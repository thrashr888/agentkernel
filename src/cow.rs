//! Safe copy-on-write preparation for local Firecracker rootfs images.
//!
//! Firecracker consumes an ext4 image file, rather than a directory tree.  An
//! overlayfs mount therefore cannot be passed to the Firecracker drive API
//! without changing the image format and requiring a privileged loop mount.
//! We use filesystem reflinks when the host supports them and retain a full
//! copy fallback for portable behavior.  The fallback has the same persisted
//! semantics as the old implementation.

use anyhow::{Context, Result, bail};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const OWNER_MARKER: &str = ".agentkernel-owned";
const OWNER_MARKER_CONTENT: &str = "agentkernel-rootfs-cow-v1";
const ROOTFS_FILE: &str = "rootfs.ext4";

/// How a sandbox rootfs was materialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootfsCowStrategy {
    /// A filesystem reflink was created with `cp --reflink=always`.
    Reflink,
    /// A regular byte-for-byte copy was required.
    FullCopy,
}

/// Host capabilities relevant to local rootfs COW setup.
///
/// `overlayfs_available` is reported for diagnostics and future directory
/// rootfs backends.  It is deliberately not used for ext4 image files: an
/// overlay mount would require privileged loop setup and would not produce a
/// file that Firecracker can open as its root drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootfsCowCapabilities {
    pub reflink_copy: bool,
    pub overlayfs_available: bool,
}

impl RootfsCowCapabilities {
    /// Detect capabilities without modifying the source image.
    pub fn detect() -> Self {
        Self {
            reflink_copy: cp_supports_reflink(),
            overlayfs_available: overlayfs_available(),
        }
    }
}

/// A directory that owns all temporary rootfs artifacts created by AgentKernel.
#[derive(Debug, Clone)]
pub struct RootfsCowStore {
    root: PathBuf,
    capabilities: RootfsCowCapabilities,
}

impl RootfsCowStore {
    /// Create a store rooted at `root`.
    ///
    /// The directory is created if needed and canonicalized before it is used
    /// for containment checks.  No caller-supplied sandbox name is ever used
    /// as a path component.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        ensure_private_store_root(root.as_ref())?;
        let root = fs::canonicalize(root.as_ref()).with_context(|| {
            format!(
                "failed to resolve rootfs COW directory {}",
                root.as_ref().display()
            )
        })?;
        Ok(Self {
            root,
            capabilities: RootfsCowCapabilities::detect(),
        })
    }

    /// Construct a store with explicit capabilities for deterministic tests.
    pub fn with_capabilities(
        root: impl AsRef<Path>,
        capabilities: RootfsCowCapabilities,
    ) -> Result<Self> {
        let mut store = Self::new(root)?;
        store.capabilities = capabilities;
        Ok(store)
    }

    /// The canonical directory that contains all artifacts from this store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Host capabilities observed when this store was created.
    pub fn capabilities(&self) -> RootfsCowCapabilities {
        self.capabilities
    }

    /// Prepare a unique writable rootfs for `base`.
    ///
    /// The output is first written to a private staging file and then renamed
    /// into place.  A marker is written only after the rename.  Cleanup will
    /// refuse to remove an artifact unless its marker, private directory, and
    /// containment checks all prove that AgentKernel created it.
    pub fn prepare(&self, base: &Path) -> Result<RootfsCow> {
        let base = fs::canonicalize(base)
            .with_context(|| format!("failed to resolve rootfs {}", base.display()))?;
        let metadata = fs::metadata(&base)
            .with_context(|| format!("failed to inspect rootfs {}", base.display()))?;
        if !metadata.is_file() {
            bail!("rootfs is not a regular file: {}", base.display());
        }

        let temporary_dir = tempfile::Builder::new()
            .prefix("sandbox-")
            .tempdir_in(&self.root)
            .context("failed to create private rootfs COW directory")?;
        let artifact_dir = temporary_dir.path().to_path_buf();
        let staging_path = artifact_dir.join(format!("{}.partial", ROOTFS_FILE));
        let rootfs_path = artifact_dir.join(ROOTFS_FILE);
        let marker_path = artifact_dir.join(OWNER_MARKER);
        let owner_token = Uuid::new_v4().to_string();

        let reflink_succeeded =
            self.capabilities.reflink_copy && reflink_copy(&base, &staging_path).unwrap_or(false);
        let strategy = if reflink_succeeded {
            RootfsCowStrategy::Reflink
        } else {
            // A failed reflink command may have left a partial destination.
            // It is inside our private temporary directory, so removing it
            // before the portable fallback is safe and deterministic.
            let _ = fs::remove_file(&staging_path);
            full_copy(&base, &staging_path).with_context(|| {
                format!(
                    "failed to copy rootfs {} -> {}",
                    base.display(),
                    staging_path.display()
                )
            })?;
            RootfsCowStrategy::FullCopy
        };

        // Ensure the staged image is durable before exposing it as a complete
        // artifact.  This is best-effort on platforms where sync is unusual,
        // but a failure is safer than publishing a partial rootfs.
        File::open(&staging_path)
            .with_context(|| format!("failed to open staged rootfs {}", staging_path.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync staged rootfs {}", staging_path.display()))?;
        fs::rename(&staging_path, &rootfs_path).with_context(|| {
            format!(
                "failed to publish rootfs {} -> {}",
                staging_path.display(),
                rootfs_path.display()
            )
        })?;

        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker_path)
            .with_context(|| {
                format!(
                    "failed to create ownership marker {}",
                    marker_path.display()
                )
            })?;
        marker.write_all(format!("{OWNER_MARKER_CONTENT}\n{owner_token}").as_bytes())?;
        marker.sync_all()?;

        let artifact_dir = temporary_dir.keep();
        Ok(RootfsCow {
            store_root: self.root.clone(),
            artifact_dir,
            rootfs_path,
            marker_path,
            owner_token,
            strategy,
        })
    }
}

fn ensure_private_store_root(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(
                    "refusing symlink as rootfs COW directory: {}",
                    root.display()
                );
            }
            if !metadata.is_dir() {
                bail!("rootfs COW path is not a directory: {}", root.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(root).with_context(|| {
                format!("failed to create rootfs COW directory {}", root.display())
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect rootfs COW directory {}", root.display())
            });
        }
    }

    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("failed to inspect rootfs COW directory {}", root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "rootfs COW path changed to a non-directory or symlink: {}",
            root.display()
        );
    }

    #[cfg(unix)]
    {
        let euid = unsafe { libc::geteuid() };
        if metadata.uid() != euid {
            bail!(
                "rootfs COW directory is not owned by the current user: {}",
                root.display()
            );
        }

        // A user-owned but permissive directory is repaired before it is
        // canonicalized or used for artifacts.  Refuse to continue if the
        // platform cannot make it private.
        if metadata.permissions().mode() & 0o777 != 0o700 {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(root, permissions).with_context(|| {
                format!(
                    "failed to restrict rootfs COW directory permissions: {}",
                    root.display()
                )
            })?;
            let verified = fs::symlink_metadata(root)?;
            if verified.permissions().mode() & 0o777 != 0o700 {
                bail!(
                    "rootfs COW directory is not private (expected mode 0700): {}",
                    root.display()
                );
            }
        }
    }

    Ok(())
}

impl RootfsCowStore {
    /// Open the default per-user temporary rootfs store.
    pub fn open_default() -> Result<Self> {
        let root = std::env::var_os("AGENTKERNEL_ROOTFS_COW_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("agentkernel-rootfs"));
        Self::new(root)
    }
}

/// A prepared rootfs whose cleanup is restricted to AgentKernel-owned paths.
#[derive(Debug)]
pub struct RootfsCow {
    store_root: PathBuf,
    artifact_dir: PathBuf,
    rootfs_path: PathBuf,
    marker_path: PathBuf,
    owner_token: String,
    strategy: RootfsCowStrategy,
}

impl RootfsCow {
    pub fn path(&self) -> &Path {
        &self.rootfs_path
    }

    pub fn strategy(&self) -> RootfsCowStrategy {
        self.strategy
    }

    /// Remove the rootfs and its private directory if ownership is proven.
    pub fn cleanup(self) -> Result<()> {
        self.cleanup_inner()
    }

    fn cleanup_inner(self) -> Result<()> {
        if !is_owned_artifact_dir(&self.store_root, &self.artifact_dir) {
            bail!(
                "refusing to clean rootfs COW directory outside store: {}",
                self.artifact_dir.display()
            );
        }
        if !is_owned_marker(&self.marker_path, &self.owner_token) {
            bail!(
                "refusing to clean rootfs COW directory without AgentKernel ownership marker: {}",
                self.artifact_dir.display()
            );
        }
        if self.rootfs_path.exists() {
            fs::remove_file(&self.rootfs_path).with_context(|| {
                format!("failed to remove rootfs {}", self.rootfs_path.display())
            })?;
        }
        fs::remove_file(&self.marker_path).with_context(|| {
            format!(
                "failed to remove ownership marker {}",
                self.marker_path.display()
            )
        })?;
        fs::remove_dir(&self.artifact_dir).with_context(|| {
            format!(
                "failed to remove empty rootfs COW directory {}",
                self.artifact_dir.display()
            )
        })?;
        Ok(())
    }
}

impl Drop for RootfsCow {
    fn drop(&mut self) {
        // Drop is intentionally best-effort.  It still applies exactly the
        // same ownership checks as explicit cleanup and never removes a path
        // merely because it has an AgentKernel-looking filename.
        let owned = is_owned_artifact_dir(&self.store_root, &self.artifact_dir)
            && is_owned_marker(&self.marker_path, &self.owner_token);
        if !owned {
            return;
        }
        let _ = fs::remove_file(&self.rootfs_path);
        let _ = fs::remove_file(&self.marker_path);
        let _ = fs::remove_dir(&self.artifact_dir);
    }
}

fn is_owned_artifact_dir(root: &Path, child: &Path) -> bool {
    let Ok(root_metadata) = fs::symlink_metadata(root) else {
        return false;
    };
    let Ok(child_metadata) = fs::symlink_metadata(child) else {
        // An already-removed artifact is safe to treat as cleaned up.
        return !child.exists();
    };
    if !root_metadata.is_dir()
        || root_metadata.file_type().is_symlink()
        || !child_metadata.is_dir()
        || child_metadata.file_type().is_symlink()
    {
        return false;
    }
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    let Ok(child) = fs::canonicalize(child) else {
        return false;
    };
    child.starts_with(&root) && child != root
}

fn is_owned_marker(path: &Path, owner_token: &str) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
        && fs::read_to_string(path)
            .map(|contents| contents == format!("{OWNER_MARKER_CONTENT}\n{owner_token}"))
            .unwrap_or(false)
}

fn full_copy(base: &Path, destination: &Path) -> std::io::Result<u64> {
    fs::copy(base, destination)
}

fn reflink_copy(base: &Path, destination: &Path) -> Option<bool> {
    let status = Command::new("cp")
        .arg("--reflink=always")
        .arg(base)
        .arg(destination)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

fn cp_supports_reflink() -> bool {
    Command::new("cp")
        .arg("--help")
        .output()
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("--reflink")
        })
        .unwrap_or(false)
}

fn overlayfs_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/filesystems")
            .map(|filesystems| {
                filesystems
                    .lines()
                    .any(|line| line.split_whitespace().last() == Some("overlay"))
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn store() -> (tempfile::TempDir, RootfsCowStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RootfsCowStore::with_capabilities(
            dir.path().join("cow"),
            RootfsCowCapabilities {
                reflink_copy: false,
                overlayfs_available: false,
            },
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn full_copy_is_unique_contained_and_restores_contents() {
        let (_tmp, store) = store();
        let base = store.root().join("base.ext4");
        fs::write(&base, b"rootfs contents").unwrap();

        let first = store.prepare(&base).unwrap();
        let second = store.prepare(&base).unwrap();
        assert_eq!(first.strategy(), RootfsCowStrategy::FullCopy);
        assert_ne!(first.path(), second.path());
        assert!(first.path().starts_with(store.root()));
        assert_eq!(fs::read(first.path()).unwrap(), b"rootfs contents");
        assert_eq!(fs::read(second.path()).unwrap(), b"rootfs contents");
        let first_path = first.path().to_path_buf();
        first.cleanup().unwrap();
        assert!(!first_path.exists());
        assert!(second.path().exists());
        second.cleanup().unwrap();
    }

    #[test]
    fn rejects_non_file_sources_without_publishing_artifact() {
        let (_tmp, store) = store();
        let source_dir = store.root().join("source-dir");
        fs::create_dir(&source_dir).unwrap();
        assert!(store.prepare(&source_dir).is_err());
        assert_eq!(fs::read_dir(store.root()).unwrap().count(), 1);
    }

    #[test]
    fn unowned_directory_is_never_deleted() {
        let (_tmp, store) = store();
        let unowned = store.root().join("sandbox-unowned");
        fs::create_dir(&unowned).unwrap();
        fs::write(unowned.join(ROOTFS_FILE), b"keep").unwrap();
        fs::write(unowned.join(OWNER_MARKER), b"not-agentkernel").unwrap();

        let rootfs = RootfsCow {
            store_root: store.root().to_path_buf(),
            artifact_dir: unowned.clone(),
            rootfs_path: unowned.join(ROOTFS_FILE),
            marker_path: unowned.join(OWNER_MARKER),
            owner_token: "not-the-token".to_string(),
            strategy: RootfsCowStrategy::FullCopy,
        };
        assert!(rootfs.cleanup().is_err());
        assert!(unowned.join(ROOTFS_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_artifact_directory_symlink_escape() {
        use std::os::unix::fs::symlink;

        let (tmp, store) = store();
        let base = store.root().join("base.ext4");
        fs::write(&base, b"rootfs contents").unwrap();
        let rootfs = store.prepare(&base).unwrap();
        let artifact_dir = rootfs.artifact_dir.clone();
        let external_dir = tmp.path().join("external");
        fs::create_dir(&external_dir).unwrap();
        fs::write(
            external_dir.join(OWNER_MARKER),
            OWNER_MARKER_CONTENT.as_bytes(),
        )
        .unwrap();
        fs::write(external_dir.join(ROOTFS_FILE), b"must survive").unwrap();
        fs::remove_dir_all(&artifact_dir).unwrap();
        symlink(&external_dir, &artifact_dir).unwrap();

        assert!(rootfs.cleanup().is_err());
        assert_eq!(
            fs::read(external_dir.join(ROOTFS_FILE)).unwrap(),
            b"must survive"
        );
        assert!(artifact_dir.is_symlink());
    }

    #[test]
    fn detects_capabilities_without_mounting_or_touching_source() {
        let capabilities = RootfsCowCapabilities::detect();
        // The result is platform-dependent, but detection itself must be
        // total and must not imply that an overlay mount was created.
        let _ = capabilities.reflink_copy;
        let _ = capabilities.overlayfs_available;
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_store_root() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        let root = tmp.path().join("cow");
        fs::create_dir(&target).unwrap();
        symlink(&target, &root).unwrap();

        assert!(RootfsCowStore::new(&root).is_err());
        assert!(target.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn repairs_permissive_store_root_to_private_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cow");
        fs::create_dir(&root).unwrap();
        let mut permissions = fs::metadata(&root).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&root, permissions).unwrap();

        RootfsCowStore::new(&root).unwrap();
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
