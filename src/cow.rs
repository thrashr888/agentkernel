//! Safe copy-on-write preparation for local Firecracker rootfs images.
//!
//! Firecracker consumes an ext4 image file, rather than a directory tree.  An
//! overlayfs mount therefore cannot be passed to the Firecracker drive API
//! without changing the image format and requiring a privileged loop mount.
//! We use filesystem reflinks when the host supports them and retain a full
//! copy fallback for portable behavior.  The fallback has the same persisted
//! semantics as the old implementation.

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::Arc;
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
    #[cfg(unix)]
    root_handle: Arc<File>,
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
        #[cfg(unix)]
        let root_handle = Arc::new(open_directory(&root).with_context(|| {
            format!(
                "failed to open rootfs COW directory {} without following symlinks",
                root.display()
            )
        })?);
        Ok(Self {
            root,
            capabilities: RootfsCowCapabilities::detect(),
            #[cfg(unix)]
            root_handle,
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
        #[cfg(unix)]
        let (artifact_name, artifact_identity) = {
            let artifact_name = artifact_dir
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("rootfs COW artifact has no directory name"))?
                .to_os_string();
            let artifact_handle = open_directory_at(self.root_handle.as_raw_fd(), &artifact_name)
                .with_context(|| {
                format!(
                    "failed to open rootfs COW artifact {} without following symlinks",
                    artifact_dir.display()
                )
            })?;
            (artifact_name, file_identity(&artifact_handle)?)
        };
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
            #[cfg(not(unix))]
            store_root: self.root.clone(),
            artifact_dir,
            rootfs_path,
            marker_path,
            owner_token,
            strategy,
            #[cfg(unix)]
            store_root_handle: Arc::clone(&self.root_handle),
            #[cfg(unix)]
            artifact_name,
            #[cfg(unix)]
            artifact_identity,
        })
    }
}

fn ensure_private_store_root(root: &Path) -> Result<()> {
    let existed = match fs::symlink_metadata(root) {
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
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = root.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create rootfs COW parent {}", parent.display())
                })?;
            }
            match fs::create_dir(root) {
                Ok(()) => false,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(root).with_context(|| {
                        format!("failed to inspect rootfs COW directory {}", root.display())
                    })?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        bail!(
                            "rootfs COW path changed to a non-directory or symlink: {}",
                            root.display()
                        );
                    }
                    true
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create rootfs COW directory {}", root.display())
                    });
                }
            }
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect rootfs COW directory {}", root.display())
            });
        }
    };

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

        if metadata.permissions().mode() & 0o777 != 0o700 {
            if existed {
                bail!(
                    "existing rootfs COW directory must have mode 0700: {}",
                    root.display()
                );
            }
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(root, permissions).with_context(|| {
                format!(
                    "failed to restrict newly-created rootfs COW directory permissions: {}",
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
    #[cfg(not(unix))]
    store_root: PathBuf,
    artifact_dir: PathBuf,
    rootfs_path: PathBuf,
    marker_path: PathBuf,
    owner_token: String,
    strategy: RootfsCowStrategy,
    #[cfg(unix)]
    store_root_handle: Arc<File>,
    #[cfg(unix)]
    artifact_name: std::ffi::OsString,
    #[cfg(unix)]
    artifact_identity: (u64, u64),
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
        #[cfg(unix)]
        {
            self.cleanup_unix()
        }

        #[cfg(not(unix))]
        {
            self.cleanup_portable()
        }
    }

    #[cfg(unix)]
    fn cleanup_unix(&self) -> Result<()> {
        let artifact = open_directory_at(self.store_root_handle.as_raw_fd(), &self.artifact_name)
            .with_context(|| {
            format!(
                "refusing to clean missing or replaced rootfs COW directory {}",
                self.artifact_dir.display()
            )
        })?;
        if file_identity(&artifact)? != self.artifact_identity {
            bail!(
                "refusing to clean replaced rootfs COW directory: {}",
                self.artifact_dir.display()
            );
        }
        verify_marker_at(artifact.as_raw_fd(), &self.owner_token).with_context(|| {
            format!(
                "refusing to clean rootfs COW directory without AgentKernel ownership marker: {}",
                self.artifact_dir.display()
            )
        })?;

        unlink_at(artifact.as_raw_fd(), ROOTFS_FILE, 0)
            .or_else(ignore_not_found)
            .with_context(|| format!("failed to remove rootfs {}", self.rootfs_path.display()))?;
        unlink_at(artifact.as_raw_fd(), OWNER_MARKER, 0).with_context(|| {
            format!(
                "failed to remove ownership marker {}",
                self.marker_path.display()
            )
        })?;
        unlink_at(
            self.store_root_handle.as_raw_fd(),
            &self.artifact_name,
            libc::AT_REMOVEDIR,
        )
        .with_context(|| {
            format!(
                "failed to remove empty rootfs COW directory {}",
                self.artifact_dir.display()
            )
        })?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn cleanup_portable(&self) -> Result<()> {
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
        if fs::symlink_metadata(&self.rootfs_path).is_ok() {
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
        #[cfg(unix)]
        {
            let _ = self.cleanup_unix();
        }
        #[cfg(not(unix))]
        {
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
}

#[cfg(not(unix))]
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

#[cfg(not(unix))]
fn is_owned_marker(path: &Path, owner_token: &str) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
        && fs::read_to_string(path)
            .map(|contents| contents == format!("{OWNER_MARKER_CONTENT}\n{owner_token}"))
            .unwrap_or(false)
}

#[cfg(unix)]
fn open_directory(path: &Path) -> std::io::Result<File> {
    let bytes = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let fd = unsafe {
        libc::open(
            bytes.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was returned by `open` and is owned by this File now.
    let file = unsafe { File::from_raw_fd(fd) };
    ensure_directory_fd(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn open_directory_at(parent: RawFd, name: &std::ffi::OsStr) -> std::io::Result<File> {
    let bytes = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            parent,
            bytes.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was returned by `openat` and is owned by this File now.
    let file = unsafe { File::from_raw_fd(fd) };
    ensure_directory_fd(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn ensure_directory_fd(file: &File) -> std::io::Result<()> {
    let (_, mode) = file_stat(file)?;
    if mode & STAT_TYPE_MASK != STAT_TYPE_DIRECTORY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "path is not a directory",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn file_identity(file: &File) -> std::io::Result<(u64, u64)> {
    let (identity, _) = file_stat(file)?;
    Ok(identity)
}

#[cfg(unix)]
fn file_stat(file: &File) -> std::io::Result<((u64, u64), u32)> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fstat initialized the struct when it returned success.
    let stat = unsafe { stat.assume_init() };
    Ok(((stat_device(&stat), stat.st_ino), stat_mode(&stat)))
}

#[cfg(target_os = "linux")]
const STAT_TYPE_MASK: u32 = libc::S_IFMT;

#[cfg(not(target_os = "linux"))]
const STAT_TYPE_MASK: u32 = libc::S_IFMT as u32;

#[cfg(target_os = "linux")]
const STAT_TYPE_DIRECTORY: u32 = libc::S_IFDIR;

#[cfg(not(target_os = "linux"))]
const STAT_TYPE_DIRECTORY: u32 = libc::S_IFDIR as u32;

#[cfg(target_os = "linux")]
const STAT_TYPE_REGULAR: u32 = libc::S_IFREG;

#[cfg(not(target_os = "linux"))]
const STAT_TYPE_REGULAR: u32 = libc::S_IFREG as u32;

#[cfg(target_os = "linux")]
fn stat_device(stat: &libc::stat) -> u64 {
    stat.st_dev
}

#[cfg(not(target_os = "linux"))]
fn stat_device(stat: &libc::stat) -> u64 {
    stat.st_dev as u64
}

#[cfg(target_os = "linux")]
fn stat_mode(stat: &libc::stat) -> u32 {
    stat.st_mode
}

#[cfg(not(target_os = "linux"))]
fn stat_mode(stat: &libc::stat) -> u32 {
    stat.st_mode as u32
}

#[cfg(unix)]
fn open_file_at(parent: RawFd, name: &str) -> std::io::Result<File> {
    let bytes = CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            parent,
            bytes.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was returned by `openat` and is owned by this File now.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn verify_marker_at(parent: RawFd, owner_token: &str) -> std::io::Result<()> {
    let mut marker = open_file_at(parent, OWNER_MARKER)?;
    let (_, mode) = file_stat(&marker)?;
    if mode & STAT_TYPE_MASK != STAT_TYPE_REGULAR {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "ownership marker is not a regular file",
        ));
    }
    let mut contents = String::new();
    marker.read_to_string(&mut contents)?;
    if contents == format!("{OWNER_MARKER_CONTENT}\n{owner_token}") {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "ownership marker does not match",
        ))
    }
}

#[cfg(unix)]
fn unlink_at(parent: RawFd, name: impl AsRef<std::ffi::OsStr>, flags: i32) -> std::io::Result<()> {
    let bytes = CString::new(name.as_ref().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains NUL"))?;
    let result = unsafe { libc::unlinkat(parent, bytes.as_ptr(), flags) };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn ignore_not_found(error: std::io::Error) -> std::io::Result<()> {
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error)
    }
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
        #[cfg(unix)]
        let unowned_handle = open_directory(&unowned).unwrap();

        let rootfs = RootfsCow {
            #[cfg(not(unix))]
            store_root: store.root().to_path_buf(),
            artifact_dir: unowned.clone(),
            rootfs_path: unowned.join(ROOTFS_FILE),
            marker_path: unowned.join(OWNER_MARKER),
            owner_token: "not-the-token".to_string(),
            strategy: RootfsCowStrategy::FullCopy,
            #[cfg(unix)]
            store_root_handle: Arc::clone(&store.root_handle),
            #[cfg(unix)]
            artifact_name: unowned.file_name().unwrap().to_os_string(),
            #[cfg(unix)]
            artifact_identity: file_identity(&unowned_handle).unwrap(),
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

    #[cfg(unix)]
    #[test]
    fn cleanup_uses_store_handle_when_store_path_is_replaced() {
        use std::os::unix::fs::symlink;

        let (tmp, store) = store();
        let base = store.root().join("base.ext4");
        fs::write(&base, b"rootfs contents").unwrap();
        let rootfs = store.prepare(&base).unwrap();
        let artifact_name = rootfs.artifact_dir.file_name().unwrap().to_os_string();
        let moved_store = tmp.path().join("cow-real");
        let replacement = tmp.path().join("replacement");
        fs::create_dir(&replacement).unwrap();
        fs::write(replacement.join("keep"), b"must survive").unwrap();

        fs::rename(store.root(), &moved_store).unwrap();
        symlink(&replacement, store.root()).unwrap();

        rootfs.cleanup().unwrap();
        assert!(!moved_store.join(&artifact_name).exists());
        assert_eq!(fs::read(replacement.join("keep")).unwrap(), b"must survive");
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_removes_replaced_rootfs_symlink_without_touching_target() {
        use std::os::unix::fs::symlink;

        let (tmp, store) = store();
        let base = store.root().join("base.ext4");
        fs::write(&base, b"rootfs contents").unwrap();
        let rootfs = store.prepare(&base).unwrap();
        let rootfs_path = rootfs.path().to_path_buf();
        let external = tmp.path().join("external-rootfs");
        fs::write(&external, b"must survive").unwrap();
        fs::remove_file(&rootfs_path).unwrap();
        symlink(&external, &rootfs_path).unwrap();

        rootfs.cleanup().unwrap();
        assert_eq!(fs::read(&external).unwrap(), b"must survive");
        assert!(!rootfs_path.exists());
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
    fn rejects_permissive_existing_store_without_changing_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cow");
        fs::create_dir(&root).unwrap();
        let mut permissions = fs::metadata(&root).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&root, permissions).unwrap();

        assert!(RootfsCowStore::new(&root).is_err());
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn configured_current_directory_is_not_chmodded() {
        let current = std::env::current_dir().unwrap();
        let before = fs::metadata(&current).unwrap().permissions().mode() & 0o777;
        let result = RootfsCowStore::new(&current);
        let after = fs::metadata(&current).unwrap().permissions().mode() & 0o777;
        assert_eq!(after, before);
        if before == 0o700 {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
    }
}
