//! Safe copy-on-write preparation for local Firecracker rootfs images.
//!
//! Firecracker consumes an ext4 image file, rather than a directory tree.  An
//! overlayfs mount therefore cannot be passed to the Firecracker drive API
//! without changing the image format and requiring a privileged loop mount.
//! We use filesystem reflinks when the host supports them and retain a full
//! copy fallback for portable behavior.  The fallback has the same persisted
//! semantics as the old implementation.

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
#[cfg(unix)]
use std::ffi::{CStr, CString, OsString};
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::Arc;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const OWNER_MARKER: &str = ".agentkernel-owned";
const OWNER_MARKER_CONTENT: &str = "agentkernel-rootfs-cow-v2";
const ROOTFS_FILE: &str = "rootfs.ext4";
const PARTIAL_ROOTFS_FILE: &str = "rootfs.ext4.partial";
const LEASE_FILE: &str = ".agentkernel-lease";
const CAPACITY_LOCK_FILE: &str = ".agentkernel-capacity.lock";
const PRESERVE_MARKER: &str = ".agentkernel-preserve";
const PRESERVE_MARKER_CONTENT: &str = "agentkernel-rootfs-cow-preserve-v1";
const ARTIFACT_PREFIX: &str = "sandbox-";
const ARTIFACT_RANDOM_LEN: usize = 6;

/// Result of a conservative scan for abandoned rootfs artifacts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RootfsCowReapReport {
    /// Complete AgentKernel-owned artifact directories removed.
    pub reclaimed_artifacts: usize,
    /// Allocated bytes released by the removed files.
    pub reclaimed_bytes: u64,
    /// Artifacts whose advisory lease is held by a live process.
    pub active_artifacts: usize,
    /// Artifacts explicitly retained as snapshot inputs.
    pub preserved_artifacts: usize,
    /// Entries left intact because ownership or shape was not proven.
    pub skipped_artifacts: usize,
    /// Owned-looking entries left intact after an operating-system error.
    pub error_artifacts: usize,
}

/// How a sandbox rootfs was materialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootfsCowStrategy {
    /// A filesystem reflink was created with `cp --reflink=always`.
    Reflink,
    /// A regular byte-for-byte copy was required.
    FullCopy,
    /// An existing durable AgentKernel rootfs was reopened for another VM
    /// lifetime. No new image copy was made.
    Existing,
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

    fn acquire_capacity_lease(&self) -> Result<File> {
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(CAPACITY_LOCK_FILE))?;
        #[cfg(unix)]
        {
            let result = unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX) };
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(lease)
    }

    /// Prepare a unique writable rootfs for `base`.
    ///
    /// Under an exclusive lease, a versioned ownership marker is published
    /// before the output is written to a private staging file and renamed into
    /// place. Cleanup refuses to remove an artifact unless its strict name,
    /// marker, lease, private directory, contents, and containment checks all
    /// prove that AgentKernel created it and no process is using it.
    pub fn prepare(&self, base: &Path) -> Result<RootfsCow> {
        let storage_limit = std::env::var("AGENTKERNEL_ROOTFS_COW_MAX_BYTES")
            .ok()
            .map(|value| {
                value.parse::<u64>().with_context(|| {
                    format!("AGENTKERNEL_ROOTFS_COW_MAX_BYTES is not a valid byte limit: {value}")
                })
            })
            .transpose()?;
        self.prepare_with_limit(base, storage_limit)
    }

    /// Prepare a writable rootfs after checking explicit store headroom.
    /// Production callers normally use [`Self::prepare`], which reads the
    /// process-wide `AGENTKERNEL_ROOTFS_COW_MAX_BYTES` cap.
    pub fn prepare_with_limit(&self, base: &Path, storage_limit: Option<u64>) -> Result<RootfsCow> {
        let capacity_lease = Some(self.acquire_capacity_lease()?);
        let base = fs::canonicalize(base)
            .with_context(|| format!("failed to resolve rootfs {}", base.display()))?;
        let metadata = fs::metadata(&base)
            .with_context(|| format!("failed to inspect rootfs {}", base.display()))?;
        if !metadata.is_file() {
            bail!("rootfs is not a regular file: {}", base.display());
        }
        if let Some(limit) = storage_limit {
            let used = self.usage_bytes()?;
            let requested = metadata.len();
            if used.saturating_add(requested) > limit {
                bail!(
                    "Firecracker writable rootfs storage headroom exhausted: used={} requested={} limit={}",
                    used,
                    requested,
                    limit
                );
            }
        }

        let temporary_dir = tempfile::Builder::new()
            .prefix(ARTIFACT_PREFIX)
            .rand_bytes(ARTIFACT_RANDOM_LEN)
            .tempdir_in(&self.root)
            .context("failed to create private rootfs COW directory")?;
        let artifact_dir = temporary_dir.path().to_path_buf();
        #[cfg(unix)]
        let (artifact_name, artifact_identity, lease_handle) = {
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
            make_directory_private(&artifact_handle)?;
            let lease = create_file_at(artifact_handle.as_raw_fd(), LEASE_FILE)?;
            lock_exclusive_nonblocking(&lease).context("failed to acquire rootfs COW lease")?;
            (artifact_name, file_identity(&artifact_handle)?, lease)
        };
        let staging_path = artifact_dir.join(PARTIAL_ROOTFS_FILE);
        let rootfs_path = artifact_dir.join(ROOTFS_FILE);
        let marker_path = artifact_dir.join(OWNER_MARKER);
        let owner_token = Uuid::new_v4().to_string();

        // Publish the versioned ownership marker before copying so a hard
        // crash during staging is recoverable. The live lease prevents a
        // concurrent AgentKernel process from reaping this directory.
        #[cfg(unix)]
        {
            let artifact = open_directory_at(self.root_handle.as_raw_fd(), &artifact_name)?;
            write_new_file_at(
                artifact.as_raw_fd(),
                OWNER_MARKER,
                format!("{OWNER_MARKER_CONTENT}\n{owner_token}").as_bytes(),
            )?;
            artifact.sync_all()?;
        }

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

        // Base images (especially immutable checkpoint artifacts) may be
        // read-only.  The private clone is Firecracker's writable root drive,
        // so do not inherit the source file's restrictive mode.
        #[cfg(unix)]
        fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to make {} writable", staging_path.display()))?;
        #[cfg(not(unix))]
        {
            let mut permissions = fs::metadata(&staging_path)?.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(&staging_path, permissions)?;
        }

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
        #[cfg(unix)]
        open_directory_at(self.root_handle.as_raw_fd(), &artifact_name)?.sync_all()?;

        #[cfg(not(unix))]
        {
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
        }

        // The published image is now visible to other preparers and counted
        // by usage_bytes; release the store-wide reservation before returning
        // the handle so later starts do not serialize on VM lifetime.
        drop(capacity_lease);
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
            #[cfg(unix)]
            _lease_handle: lease_handle,
            preserved: false,
        })
    }

    /// Reopen a durable per-sandbox rootfs lineage.
    ///
    /// The persisted value is an opaque artifact identifier, never a caller
    /// supplied path. The artifact must carry AgentKernel's ownership marker,
    /// durable-preservation marker, and an available lease before it can be
    /// attached to a new Firecracker process.
    pub fn adopt(&self, reference: &str) -> Result<RootfsCow> {
        self.adopt_internal(reference, true)
    }

    /// Reopen a state-owned lineage and finish publishing its durability
    /// marker. This closes the crash window between the atomic state rename
    /// and marker publication: a valid owned artifact referenced by state is
    /// recoverable even when the prior process died before the marker write.
    pub fn adopt_or_publish(&self, reference: &str) -> Result<RootfsCow> {
        let mut rootfs = self.adopt_internal(reference, false)?;
        if !rootfs.preserved {
            rootfs.preserve_for_lifecycle()?;
        }
        Ok(rootfs)
    }

    fn adopt_internal(&self, reference: &str, require_preserved: bool) -> Result<RootfsCow> {
        #[cfg(unix)]
        {
            let artifact_name = std::ffi::OsStr::new(reference);
            if !is_artifact_name(artifact_name) {
                bail!("invalid rootfs lineage reference '{reference}'");
            }
            let artifact = open_directory_at(self.root_handle.as_raw_fd(), artifact_name)
                .with_context(|| format!("rootfs lineage artifact '{reference}' is missing"))?;
            make_directory_private(&artifact)?;
            let artifact_identity = file_identity(&artifact)?;
            let lease = open_file_at_read_write(artifact.as_raw_fd(), LEASE_FILE)
                .with_context(|| format!("rootfs lineage '{reference}' has no lease"))?;
            if !is_regular_file(&lease) {
                bail!("rootfs lineage '{reference}' lease is not a regular file");
            }
            lock_exclusive_with_retry(&lease)
                .with_context(|| format!("rootfs lineage '{reference}' is already in use"))?;
            let owner_marker = open_file_at(artifact.as_raw_fd(), OWNER_MARKER)
                .with_context(|| format!("rootfs lineage '{reference}' has no ownership marker"))?;
            let Some(owner_token) = read_versioned_token(&owner_marker, OWNER_MARKER_CONTENT)
            else {
                bail!("rootfs lineage '{reference}' has an invalid ownership marker");
            };
            verify_marker_at(artifact.as_raw_fd(), &owner_token)?;
            let has_preserve = match open_file_at(artifact.as_raw_fd(), PRESERVE_MARKER) {
                Ok(preserve) => {
                    if !is_regular_file(&preserve) {
                        bail!("rootfs lineage '{reference}' has an invalid durability marker");
                    }
                    read_versioned_token(&preserve, PRESERVE_MARKER_CONTENT).as_deref()
                        == Some(owner_token.as_str())
                }
                Err(error) if error.raw_os_error() == Some(libc::ENOENT) => false,
                Err(error) => return Err(error.into()),
            };
            if require_preserved && !has_preserve {
                bail!("rootfs lineage '{reference}' has an invalid durability marker");
            }
            let rootfs_name = std::ffi::OsStr::new(ROOTFS_FILE);
            let rootfs = open_file_at(artifact.as_raw_fd(), ROOTFS_FILE)
                .with_context(|| format!("rootfs lineage '{reference}' has no rootfs image"))?;
            if !is_regular_file(&rootfs) || rootfs.metadata()?.len() == 0 {
                bail!("rootfs lineage '{reference}' image is not a non-empty regular file");
            }
            let artifact_dir = self.root.join(reference);
            Ok(RootfsCow {
                artifact_dir,
                rootfs_path: self.root.join(reference).join(rootfs_name),
                marker_path: self.root.join(reference).join(OWNER_MARKER),
                owner_token,
                strategy: RootfsCowStrategy::Existing,
                store_root_handle: Arc::clone(&self.root_handle),
                artifact_name: artifact_name.to_os_string(),
                artifact_identity,
                _lease_handle: lease,
                preserved: has_preserve,
            })
        }

        #[cfg(not(unix))]
        {
            if !is_artifact_name(std::ffi::OsStr::new(reference)) {
                bail!("invalid rootfs lineage reference '{reference}'");
            }
            let artifact_dir = self.root.join(reference);
            let artifact_metadata = fs::symlink_metadata(&artifact_dir)?;
            if artifact_metadata.file_type().is_symlink() || !artifact_metadata.is_dir() {
                bail!("rootfs lineage '{reference}' is not a regular artifact directory");
            }
            let rootfs_path = artifact_dir.join(ROOTFS_FILE);
            let marker_path = artifact_dir.join(OWNER_MARKER);
            let owner_metadata = fs::symlink_metadata(&marker_path)?;
            if owner_metadata.file_type().is_symlink() || !owner_metadata.is_file() {
                bail!("rootfs lineage '{reference}' has an invalid ownership marker");
            }
            let owner_token = fs::read_to_string(&marker_path)
                .ok()
                .and_then(|contents| contents.strip_prefix(OWNER_MARKER_CONTENT))
                .and_then(|contents| contents.strip_prefix('\n'))
                .map(str::to_owned)
                .ok_or_else(|| {
                    anyhow::anyhow!("rootfs lineage '{reference}' has an invalid ownership marker")
                })?;
            let preserve_path = artifact_dir.join(PRESERVE_MARKER);
            let preserve_metadata = fs::symlink_metadata(&preserve_path).ok();
            if preserve_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
            {
                bail!("rootfs lineage '{reference}' has an invalid durability marker");
            }
            let has_preserve = is_owned_marker(&preserve_path, &owner_token);
            if !is_owned_marker(&marker_path, &owner_token)
                || (require_preserved && !has_preserve)
                || !rootfs_path.is_file()
            {
                bail!("rootfs lineage '{reference}' is not a durable owned image");
            }
            Ok(RootfsCow {
                store_root: self.root.clone(),
                artifact_dir,
                rootfs_path,
                marker_path,
                owner_token,
                strategy: RootfsCowStrategy::Existing,
                preserved: has_preserve,
            })
        }
    }

    /// Remove a durable rootfs lineage after validating its opaque reference.
    pub fn discard(&self, reference: &str) -> Result<()> {
        let rootfs = self.adopt_or_publish(reference)?;
        rootfs.discard_persisted()
    }

    /// Idempotently remove a durable lineage during sandbox removal.
    ///
    /// A prior remove attempt may have deleted the owned artifact before a
    /// later sandbox-state deletion failed. An absent artifact is therefore
    /// already clean, while malformed, symlinked, or leased artifacts still
    /// fail closed.
    pub fn discard_if_present(&self, reference: &str) -> Result<()> {
        if !is_artifact_name(std::ffi::OsStr::new(reference)) {
            bail!("invalid rootfs lineage reference '{reference}'");
        }
        #[cfg(unix)]
        match open_directory_at(
            self.root_handle.as_raw_fd(),
            std::ffi::OsStr::new(reference),
        ) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        #[cfg(not(unix))]
        match fs::symlink_metadata(self.root.join(reference)) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!("rootfs lineage '{reference}' is not a regular artifact directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        match self.adopt_or_publish(reference) {
            Ok(rootfs) => rootfs.discard_persisted(),
            Err(error)
                if error
                    .chain()
                    .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
                    .any(|cause| cause.kind() == std::io::ErrorKind::NotFound)
                    && !self.root.join(reference).exists() =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Return bytes currently consumed by all owned rootfs artifacts.
    ///
    /// This is intentionally an accounting helper rather than a cleanup
    /// decision. Unknown names are ignored, but malformed owned-looking
    /// artifacts return an error so a capacity check never undercounts usage.
    pub fn usage_bytes(&self) -> Result<u64> {
        let mut total = 0_u64;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(reference) = file_name.to_str() else {
                continue;
            };
            if !is_artifact_name(std::ffi::OsStr::new(reference)) {
                continue;
            }
            let image = entry.path().join(ROOTFS_FILE);
            let image_metadata = fs::symlink_metadata(&image).with_context(|| {
                format!(
                    "failed to inspect rootfs image for owned artifact {}",
                    entry.path().display()
                )
            })?;
            if image_metadata.file_type().is_symlink() || !image_metadata.is_file() {
                bail!(
                    "owned rootfs artifact has a non-regular image: {}",
                    image.display()
                );
            }
            total = total.saturating_add(image_metadata.len());
        }
        Ok(total)
    }

    /// Return the size of one validated durable rootfs artifact for quota
    /// accounting. This does not acquire the artifact lease, so a running VM
    /// can be accounted for while it owns the image.
    pub fn bytes_for_reference(&self, reference: &str) -> Result<u64> {
        if !is_artifact_name(std::ffi::OsStr::new(reference)) {
            bail!("invalid rootfs lineage reference '{reference}'");
        }
        let artifact = self.root.join(reference);
        let artifact_metadata = fs::symlink_metadata(&artifact)?;
        if !artifact_metadata.is_dir() || artifact_metadata.file_type().is_symlink() {
            bail!("rootfs lineage '{reference}' is not an owned artifact directory");
        }
        let rootfs = artifact.join(ROOTFS_FILE);
        let rootfs_metadata = fs::symlink_metadata(&rootfs)?;
        if !rootfs_metadata.is_file() || rootfs_metadata.file_type().is_symlink() {
            bail!("rootfs lineage '{reference}' is not a regular rootfs image");
        }
        Ok(rootfs_metadata.len())
    }

    /// Reclaim abandoned, AgentKernel-owned artifacts in this store.
    ///
    /// On Unix this uses directory-relative, no-follow operations and a
    /// cross-process advisory lease. Other platforms conservatively report no
    /// reclamation rather than guessing whether an artifact is live.
    pub fn reap_stale(&self) -> Result<RootfsCowReapReport> {
        self.reap_stale_except(&HashSet::new())
    }

    /// Reclaim stale artifacts while retaining opaque references loaded from
    /// persisted sandbox state. A state-owned artifact may be between the
    /// atomic state rename and durability-marker publication after a crash.
    pub fn reap_stale_except(
        &self,
        retained_references: &HashSet<String>,
    ) -> Result<RootfsCowReapReport> {
        #[cfg(unix)]
        {
            self.reap_stale_unix(retained_references)
        }

        #[cfg(not(unix))]
        {
            Ok(RootfsCowReapReport::default())
        }
    }

    #[cfg(unix)]
    fn reap_stale_unix(
        &self,
        retained_references: &HashSet<String>,
    ) -> Result<RootfsCowReapReport> {
        let mut report = RootfsCowReapReport::default();
        for name in list_directory_at(self.root_handle.as_raw_fd())? {
            if name == std::ffi::OsStr::new(CAPACITY_LOCK_FILE) {
                continue;
            }
            if !is_artifact_name(&name) {
                report.skipped_artifacts += 1;
                continue;
            }
            if name
                .to_str()
                .is_some_and(|reference| retained_references.contains(reference))
            {
                report.preserved_artifacts += 1;
                continue;
            }

            let artifact = match open_directory_at(self.root_handle.as_raw_fd(), &name) {
                Ok(artifact) => artifact,
                Err(error)
                    if matches!(
                        error.raw_os_error(),
                        Some(libc::ELOOP) | Some(libc::ENOTDIR)
                    ) =>
                {
                    report.skipped_artifacts += 1;
                    continue;
                }
                Err(_) => {
                    report.error_artifacts += 1;
                    continue;
                }
            };
            let metadata = match artifact.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    report.error_artifacts += 1;
                    continue;
                }
            };
            if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                report.skipped_artifacts += 1;
                continue;
            }

            let lease = match open_file_at_read_write(artifact.as_raw_fd(), LEASE_FILE) {
                Ok(lease) => lease,
                Err(error)
                    if matches!(error.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOENT)) =>
                {
                    report.skipped_artifacts += 1;
                    continue;
                }
                Err(_) => {
                    report.error_artifacts += 1;
                    continue;
                }
            };
            if !is_regular_file(&lease) {
                report.skipped_artifacts += 1;
                continue;
            }
            match try_lock_exclusive(&lease) {
                Ok(true) => {}
                Ok(false) => {
                    report.active_artifacts += 1;
                    continue;
                }
                Err(_) => {
                    report.error_artifacts += 1;
                    continue;
                }
            }

            let inspected = match inspect_owned_artifact(&artifact) {
                Ok(Some(inspected)) => inspected,
                Ok(None) => {
                    report.skipped_artifacts += 1;
                    continue;
                }
                Err(_) => {
                    report.error_artifacts += 1;
                    continue;
                }
            };
            if inspected.preserved {
                report.preserved_artifacts += 1;
                continue;
            }

            let removed = (|| -> std::io::Result<()> {
                if inspected.has_rootfs {
                    unlink_at(artifact.as_raw_fd(), ROOTFS_FILE, 0)?;
                }
                if inspected.has_partial {
                    unlink_at(artifact.as_raw_fd(), PARTIAL_ROOTFS_FILE, 0)?;
                }
                unlink_at(artifact.as_raw_fd(), OWNER_MARKER, 0)?;
                unlink_at(artifact.as_raw_fd(), LEASE_FILE, 0)?;
                unlink_at(self.root_handle.as_raw_fd(), &name, libc::AT_REMOVEDIR)
            })();
            if removed.is_ok() {
                report.reclaimed_artifacts += 1;
                report.reclaimed_bytes = report
                    .reclaimed_bytes
                    .saturating_add(inspected.allocated_bytes);
            } else {
                report.error_artifacts += 1;
            }
        }
        Ok(report)
    }
}

#[cfg(unix)]
struct InspectedArtifact {
    allocated_bytes: u64,
    has_rootfs: bool,
    has_partial: bool,
    preserved: bool,
}

#[cfg(unix)]
fn inspect_owned_artifact(artifact: &File) -> std::io::Result<Option<InspectedArtifact>> {
    let names = list_directory_at(artifact.as_raw_fd())?;
    let mut allocated_bytes = 0u64;
    let mut has_marker = false;
    let mut has_lease = false;
    let mut has_rootfs = false;
    let mut has_partial = false;
    let mut has_preserve = false;
    let mut preserve_token = None;

    for name in names {
        let Some(name) = name.to_str() else {
            return Ok(None);
        };
        if !matches!(
            name,
            OWNER_MARKER | LEASE_FILE | ROOTFS_FILE | PARTIAL_ROOTFS_FILE | PRESERVE_MARKER
        ) {
            return Ok(None);
        }
        let file = match open_file_at(artifact.as_raw_fd(), name) {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Ok(None),
            Err(error) => return Err(error),
        };
        if !is_regular_file(&file) {
            return Ok(None);
        }
        allocated_bytes = allocated_bytes.saturating_add(file.metadata()?.blocks() * 512);
        match name {
            OWNER_MARKER => has_marker = true,
            LEASE_FILE => has_lease = true,
            ROOTFS_FILE => has_rootfs = true,
            PARTIAL_ROOTFS_FILE => has_partial = true,
            PRESERVE_MARKER => {
                has_preserve = true;
                preserve_token = read_versioned_token(&file, PRESERVE_MARKER_CONTENT);
            }
            _ => unreachable!(),
        }
    }

    if !has_marker || !has_lease || (has_rootfs && has_partial) {
        return Ok(None);
    }
    let owner_token = read_versioned_token(
        &open_file_at(artifact.as_raw_fd(), OWNER_MARKER)?,
        OWNER_MARKER_CONTENT,
    );
    let Some(owner_token) = owner_token else {
        return Ok(None);
    };
    if let Some(preserve_token) = preserve_token.as_ref() {
        if preserve_token != &owner_token {
            return Ok(None);
        }
    } else if has_preserve {
        // A malformed preservation marker is never interpreted as permission
        // to delete the artifact.
        return Ok(None);
    }

    Ok(Some(InspectedArtifact {
        allocated_bytes,
        has_rootfs,
        has_partial,
        preserved: preserve_token.is_some(),
    }))
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
    fn default_root() -> PathBuf {
        std::env::var_os("AGENTKERNEL_ROOTFS_COW_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("agentkernel-rootfs"))
    }

    /// Open the default per-user temporary rootfs store.
    pub fn open_default() -> Result<Self> {
        let root = Self::default_root();
        Self::new(root)
    }

    /// Open the default store without reaping. A persisted state reference is
    /// authoritative during startup, including the crash window after its
    /// atomic rename but before the preservation marker was written.
    pub fn open_default_without_reap() -> Result<Self> {
        Self::new(Self::default_root())
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
    #[cfg(unix)]
    _lease_handle: File,
    preserved: bool,
}

impl RootfsCow {
    pub fn path(&self) -> &Path {
        &self.rootfs_path
    }

    /// Arrange for exactly this child to inherit the lineage lease.
    ///
    /// The parent keeps `FD_CLOEXEC` set at all times. Clearing it in the
    /// post-fork child avoids a process-wide race where an unrelated spawn on
    /// another thread could otherwise inherit the lease.
    pub fn inherit_lease_in_child(&self, command: &mut Command) {
        #[cfg(unix)]
        {
            let lease_fd = self._lease_handle.as_raw_fd();
            // SAFETY: this closure runs after fork and before exec. `fcntl`
            // operates only on the inherited descriptor and is async-signal
            // safe on the supported Unix hosts.
            unsafe {
                command.pre_exec(move || {
                    let flags = libc::fcntl(lease_fd, libc::F_GETFD);
                    if flags < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::fcntl(lease_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(not(unix))]
        let _ = command;
    }

    /// Stable opaque identifier persisted in sandbox state. It is deliberately
    /// not an absolute path and is validated by [`RootfsCowStore::adopt`].
    pub fn reference(&self) -> String {
        self.artifact_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Directory that must be used as Firecracker's working directory.
    ///
    /// The drive is deliberately configured as the relative path
    /// `rootfs.ext4`.  A restored VM can therefore use an independent writable
    /// COW image in a different directory without changing the path embedded in
    /// Firecracker's vmstate file.
    pub fn artifact_dir(&self) -> &Path {
        &self.artifact_dir
    }

    pub fn strategy(&self) -> RootfsCowStrategy {
        self.strategy
    }

    /// Copy the paused root drive into a checkpoint directory.
    ///
    /// A reflink is attempted first and a portable full copy is used when the
    /// source and checkpoint stores do not share reflink-capable storage.  The
    /// destination is published by rename only after its contents are synced.
    pub fn snapshot_to(&self, destination: &Path) -> Result<RootfsCowStrategy> {
        // Firecracker has stopped issuing guest I/O while paused, but its host
        // file can still have dirty pages. Flush the source before reflinking
        // or copying so a published checkpoint is durable as a complete set.
        File::options()
            .read(true)
            .write(true)
            .open(&self.rootfs_path)
            .with_context(|| format!("failed to open {}", self.rootfs_path.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync {}", self.rootfs_path.display()))?;

        if fs::symlink_metadata(destination).is_ok() {
            bail!(
                "refusing to overwrite checkpoint rootfs {}",
                destination.display()
            );
        }
        let parent = destination.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "checkpoint rootfs has no parent directory: {}",
                destination.display()
            )
        })?;
        let parent_metadata = fs::symlink_metadata(parent).with_context(|| {
            format!(
                "failed to inspect checkpoint directory {}",
                parent.display()
            )
        })?;
        if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
            bail!(
                "checkpoint rootfs parent is not a directory: {}",
                parent.display()
            );
        }

        let staging = parent.join(format!(".rootfs.ext4.partial-{}", Uuid::new_v4().simple()));
        let reflink_succeeded = reflink_copy(&self.rootfs_path, &staging).unwrap_or(false);
        let strategy = if reflink_succeeded {
            RootfsCowStrategy::Reflink
        } else {
            let _ = fs::remove_file(&staging);
            if let Err(error) = full_copy(&self.rootfs_path, &staging) {
                let _ = fs::remove_file(&staging);
                return Err(error).with_context(|| {
                    format!(
                        "failed to copy paused rootfs {} -> {}",
                        self.rootfs_path.display(),
                        staging.display()
                    )
                });
            }
            RootfsCowStrategy::FullCopy
        };

        let publish = (|| -> Result<()> {
            File::open(&staging)
                .with_context(|| format!("failed to open {}", staging.display()))?
                .sync_all()
                .with_context(|| format!("failed to sync {}", staging.display()))?;
            fs::rename(&staging, destination).with_context(|| {
                format!(
                    "failed to publish checkpoint rootfs {} -> {}",
                    staging.display(),
                    destination.display()
                )
            })?;
            File::open(parent)
                .with_context(|| format!("failed to open {}", parent.display()))?
                .sync_all()
                .with_context(|| format!("failed to sync {}", parent.display()))?;
            Ok(())
        })();
        if let Err(error) = publish {
            let _ = fs::remove_file(&staging);
            return Err(error);
        }
        Ok(strategy)
    }

    /// Keep this rootfs as an input for a durable Firecracker snapshot.
    ///
    /// The versioned marker is durable and checked by future reapers. After
    /// this succeeds, explicit cleanup and `Drop` intentionally leave the
    /// complete artifact intact.
    pub fn preserve_for_snapshot(&mut self) -> Result<()> {
        if self.preserved {
            return Ok(());
        }

        #[cfg(unix)]
        {
            let artifact =
                open_directory_at(self.store_root_handle.as_raw_fd(), &self.artifact_name)?;
            if file_identity(&artifact)? != self.artifact_identity {
                bail!(
                    "refusing to preserve replaced rootfs COW directory: {}",
                    self.artifact_dir.display()
                );
            }
            verify_marker_at(artifact.as_raw_fd(), &self.owner_token)?;
            ensure_preserve_marker_at(artifact.as_raw_fd(), &self.owner_token)?;
            artifact.sync_all()?;
        }

        #[cfg(not(unix))]
        ensure_preserve_marker(&self.artifact_dir.join(PRESERVE_MARKER), &self.owner_token)?;
        self.preserved = true;
        Ok(())
    }

    /// Mark this rootfs durable across ordinary Firecracker stop/start.
    pub fn preserve_for_lifecycle(&mut self) -> Result<()> {
        self.preserve_for_snapshot()
    }

    /// Remove a durable lineage, including its preservation marker.
    pub fn discard_persisted(mut self) -> Result<()> {
        if self.preserved {
            #[cfg(unix)]
            {
                let artifact =
                    open_directory_at(self.store_root_handle.as_raw_fd(), &self.artifact_name)?;
                if file_identity(&artifact)? != self.artifact_identity {
                    bail!("refusing to discard replaced rootfs COW directory");
                }
                verify_marker_at(artifact.as_raw_fd(), &self.owner_token)?;
                unlink_at(artifact.as_raw_fd(), PRESERVE_MARKER, 0).or_else(ignore_not_found)?;
                artifact.sync_all()?;
            }
            #[cfg(not(unix))]
            {
                let marker = self.artifact_dir.join(PRESERVE_MARKER);
                if marker.exists() {
                    fs::remove_file(marker)?;
                }
            }
            self.preserved = false;
        }
        self.cleanup_inner()
    }

    /// Remove the rootfs and its private directory if ownership is proven.
    pub fn cleanup(self) -> Result<()> {
        self.cleanup_inner()
    }

    fn cleanup_inner(self) -> Result<()> {
        if self.preserved {
            return Ok(());
        }
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
        if self.preserved {
            return Ok(());
        }
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
        unlink_at(artifact.as_raw_fd(), LEASE_FILE, 0).with_context(|| {
            format!(
                "failed to remove rootfs lease in {}",
                self.artifact_dir.display()
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
        if self.preserved {
            return Ok(());
        }
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
        let _ = fs::remove_file(self.artifact_dir.join(LEASE_FILE));
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
        if self.preserved {
            return;
        }
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
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
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
fn make_directory_private(directory: &File) -> std::io::Result<()> {
    let result = unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let metadata = directory.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "rootfs COW artifact directory is not private",
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
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was returned by `openat` and is owned by this File now.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_file_at_read_write(parent: RawFd, name: &str) -> std::io::Result<File> {
    let bytes = CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            parent,
            bytes.as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn create_file_at(parent: RawFd, name: &str) -> std::io::Result<File> {
    let bytes = CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            parent,
            bytes.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn write_new_file_at(parent: RawFd, name: &str, contents: &[u8]) -> std::io::Result<()> {
    let mut file = create_file_at(parent, name)?;
    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(unix)]
fn ensure_preserve_marker_at(parent: RawFd, owner_token: &str) -> Result<()> {
    let desired = format!("{PRESERVE_MARKER_CONTENT}\n{owner_token}");
    match open_file_at(parent, PRESERVE_MARKER) {
        Ok(marker) => {
            if !is_regular_file(&marker) {
                bail!("Firecracker preservation marker is not a regular file");
            }
            let mut contents = String::new();
            marker.try_clone()?.read_to_string(&mut contents)?;
            if contents == desired {
                return Ok(());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let temporary = format!("{PRESERVE_MARKER}.tmp-{}", Uuid::new_v4().simple());
    write_new_file_at(parent, &temporary, desired.as_bytes())?;
    let temporary_c = CString::new(temporary.as_bytes())?;
    let marker_c = CString::new(PRESERVE_MARKER)?;
    let result = unsafe { libc::renameat(parent, temporary_c.as_ptr(), parent, marker_c.as_ptr()) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::unlinkat(parent, temporary_c.as_ptr(), 0);
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(unix)]
fn is_regular_file(file: &File) -> bool {
    file_stat(file)
        .map(|(_, mode)| mode & STAT_TYPE_MASK == STAT_TYPE_REGULAR)
        .unwrap_or(false)
}

#[cfg(unix)]
fn lock_exclusive_nonblocking(file: &File) -> std::io::Result<()> {
    if try_lock_exclusive(file)? {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "rootfs COW lease is already held",
        ))
    }
}

#[cfg(unix)]
fn lock_exclusive_with_retry(file: &File) -> std::io::Result<()> {
    const ATTEMPTS: usize = 10;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(5);
    for attempt in 0..ATTEMPTS {
        match lock_exclusive_nonblocking(file) {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && attempt + 1 < ATTEMPTS =>
            {
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded rootfs lease retry always returns")
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> std::io::Result<bool> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn is_artifact_name(name: &std::ffi::OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == ARTIFACT_PREFIX.len() + ARTIFACT_RANDOM_LEN
        && bytes.starts_with(ARTIFACT_PREFIX.as_bytes())
        && bytes[ARTIFACT_PREFIX.len()..]
            .iter()
            .all(u8::is_ascii_alphanumeric)
}

#[cfg(not(unix))]
fn is_artifact_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name.len() == ARTIFACT_PREFIX.len() + ARTIFACT_RANDOM_LEN
        && name.starts_with(ARTIFACT_PREFIX)
        && name[ARTIFACT_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
}

#[cfg(unix)]
fn read_versioned_token(file: &File, version: &str) -> Option<String> {
    if file.metadata().ok()?.len() > 128 {
        return None;
    }
    let mut file = file.try_clone().ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;
    let token = contents.strip_prefix(version)?.strip_prefix('\n')?;
    if token.contains('\n') {
        return None;
    }
    let parsed = Uuid::parse_str(token).ok()?;
    (parsed.to_string() == token).then(|| token.to_string())
}

#[cfg(unix)]
fn list_directory_at(parent: RawFd) -> std::io::Result<Vec<OsString>> {
    let current = c".";
    let descriptor = unsafe {
        libc::openat(
            parent,
            current.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_DIRECTORY,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let directory = unsafe { libc::fdopendir(descriptor) };
    if directory.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(descriptor) };
        return Err(error);
    }

    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let directory = Directory(directory);
    let mut names = Vec::new();
    loop {
        set_directory_errno(0);
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            let error = directory_errno();
            if error == 0 {
                break;
            }
            return Err(std::io::Error::from_raw_os_error(error));
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    Ok(names)
}

#[cfg(all(unix, target_os = "linux"))]
fn set_directory_errno(value: libc::c_int) {
    unsafe { *libc::__errno_location() = value };
}

#[cfg(all(unix, target_os = "linux"))]
fn directory_errno() -> libc::c_int {
    unsafe { *libc::__errno_location() }
}

#[cfg(all(unix, target_vendor = "apple"))]
fn set_directory_errno(value: libc::c_int) {
    unsafe { *libc::__error() = value };
}

#[cfg(all(unix, target_vendor = "apple"))]
fn directory_errno() -> libc::c_int {
    unsafe { *libc::__error() }
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

#[cfg(not(unix))]
fn ensure_preserve_marker(path: &Path, owner_token: &str) -> Result<()> {
    let desired = format!("{PRESERVE_MARKER_CONTENT}\n{owner_token}");
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "refusing non-regular Firecracker preservation marker {}",
                    path.display()
                );
            }
            if fs::read_to_string(path).ok().as_deref() == Some(desired.as_str()) {
                return Ok(());
            }
            let temporary =
                path.with_file_name(format!("{PRESERVE_MARKER}.tmp-{}", Uuid::new_v4()));
            let mut marker = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            marker.write_all(desired.as_bytes())?;
            marker.sync_all()?;
            fs::rename(&temporary, path)?;
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut marker = OpenOptions::new().write(true).create_new(true).open(path)?;
            marker.write_all(desired.as_bytes())?;
            marker.sync_all()?;
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
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

    #[cfg(unix)]
    const CRASH_HELPER_ROOT: &str = "AGENTKERNEL_TEST_COW_CRASH_ROOT";
    #[cfg(unix)]
    const CRASH_HELPER_BASE: &str = "AGENTKERNEL_TEST_COW_CRASH_BASE";

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

    #[cfg(unix)]
    fn create_stale_fixture(store: &RootfsCowStore, name: &str) -> PathBuf {
        assert!(is_artifact_name(std::ffi::OsStr::new(name)));
        let artifact = store.root().join(name);
        fs::create_dir(&artifact).unwrap();
        let mut permissions = fs::metadata(&artifact).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&artifact, permissions).unwrap();
        let token = Uuid::new_v4().to_string();
        fs::write(
            artifact.join(OWNER_MARKER),
            format!("{OWNER_MARKER_CONTENT}\n{token}"),
        )
        .unwrap();
        fs::write(artifact.join(LEASE_FILE), b"").unwrap();
        fs::write(artifact.join(ROOTFS_FILE), vec![7u8; 64 * 1024]).unwrap();
        artifact
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_holds_rootfs_lease_until_crash() {
        let (Some(root), Some(base)) = (
            std::env::var_os(CRASH_HELPER_ROOT),
            std::env::var_os(CRASH_HELPER_BASE),
        ) else {
            return;
        };
        let store = RootfsCowStore::with_capabilities(
            root,
            RootfsCowCapabilities {
                reflink_copy: false,
                overlayfs_available: false,
            },
        )
        .unwrap();
        let _rootfs = store.prepare(Path::new(&base)).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[cfg(unix)]
    #[test]
    fn inherited_rootfs_lease_blocks_adoption_until_child_exit() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"inherited lease filesystem").unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        rootfs.preserve_for_lifecycle().unwrap();
        let reference = rootfs.reference();

        let mut command = std::process::Command::new("sleep");
        command.arg("1");
        rootfs.inherit_lease_in_child(&mut command);
        let mut child = command.spawn().unwrap();
        drop(rootfs);

        assert!(store.adopt(&reference).is_err());
        child.wait().unwrap();
        let adopted = store.adopt(&reference).unwrap();
        adopted.discard_persisted().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn restart_reaps_sigkill_orphan_but_not_cross_process_active_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cow");
        let base = tmp.path().join("base.ext4");
        fs::write(&base, vec![7u8; 64 * 1024]).unwrap();
        let store = RootfsCowStore::with_capabilities(
            &root,
            RootfsCowCapabilities {
                reflink_copy: false,
                overlayfs_available: false,
            },
        )
        .unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("cow::tests::subprocess_holds_rootfs_lease_until_crash")
            .arg("--nocapture")
            .env(CRASH_HELPER_ROOT, &root)
            .env(CRASH_HELPER_BASE, &base)
            .spawn()
            .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !fs::read_dir(store.root()).unwrap().any(|entry| {
            let entry = entry.unwrap();
            if !is_artifact_name(&entry.file_name()) {
                return false;
            }
            let Ok(artifact) = open_directory_at(store.root_handle.as_raw_fd(), &entry.file_name())
            else {
                return false;
            };
            matches!(
                inspect_owned_artifact(&artifact),
                Ok(Some(InspectedArtifact {
                    has_rootfs: true,
                    ..
                })) | Ok(Some(InspectedArtifact {
                    has_partial: true,
                    ..
                }))
            )
        }) {
            assert!(
                std::time::Instant::now() < deadline,
                "child did not prepare rootfs"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert_eq!(
            store.reap_stale().unwrap(),
            RootfsCowReapReport {
                active_artifacts: 1,
                ..RootfsCowReapReport::default()
            }
        );

        child.kill().unwrap();
        child.wait().unwrap();
        let report = store.reap_stale().unwrap();
        assert_eq!(report.reclaimed_artifacts, 1, "{report:?}");
        assert!(report.reclaimed_bytes > 0);
        assert_eq!(
            report,
            RootfsCowReapReport {
                reclaimed_artifacts: 1,
                reclaimed_bytes: report.reclaimed_bytes,
                ..RootfsCowReapReport::default()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicitly_preserved_snapshot_input_survives_cleanup_and_reaping() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"snapshot rootfs").unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        let path = rootfs.path().to_path_buf();
        rootfs.preserve_for_snapshot().unwrap();
        rootfs.cleanup().unwrap();

        assert_eq!(
            store.reap_stale().unwrap(),
            RootfsCowReapReport {
                preserved_artifacts: 1,
                ..RootfsCowReapReport::default()
            }
        );
        assert_eq!(fs::read(path).unwrap(), b"snapshot rootfs");
    }

    #[test]
    fn durable_lineage_can_be_reopened_after_owner_drop_and_discarded() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"durable guest filesystem").unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        let reference = rootfs.reference();
        rootfs.preserve_for_lifecycle().unwrap();
        let path = rootfs.path().to_path_buf();
        drop(rootfs);

        let adopted = store.adopt(&reference).unwrap();
        assert_eq!(adopted.strategy(), RootfsCowStrategy::Existing);
        assert_eq!(adopted.reference(), reference);
        assert_eq!(
            fs::read(adopted.path()).unwrap(),
            b"durable guest filesystem"
        );
        drop(adopted);
        assert!(path.exists());

        store.discard(&reference).unwrap();
        assert!(!path.exists());
        assert!(store.adopt(&reference).is_err());
        store.discard_if_present(&reference).unwrap();
    }

    #[test]
    fn idempotent_discard_does_not_accept_malformed_existing_artifact() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"malformed cleanup").unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        let reference = rootfs.reference();
        rootfs.preserve_for_lifecycle().unwrap();
        fs::remove_file(rootfs.artifact_dir().join(OWNER_MARKER)).unwrap();
        drop(rootfs);

        assert!(store.discard_if_present(&reference).is_err());
    }

    #[test]
    fn state_owned_lineage_finishes_missing_marker_after_crash_window() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"state-authoritative filesystem").unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        let reference = rootfs.reference();
        rootfs.preserve_for_lifecycle().unwrap();
        fs::remove_file(rootfs.artifact_dir().join(PRESERVE_MARKER)).unwrap();
        drop(rootfs);

        // Startup reconciliation must treat the state reference as
        // authoritative even before the preservation marker is repaired.
        let report = store
            .reap_stale_except(&std::collections::HashSet::from([reference.clone()]))
            .unwrap();
        assert_eq!(report.reclaimed_artifacts, 0);

        let adopted = store.adopt_or_publish(&reference).unwrap();
        assert_eq!(
            fs::read(adopted.path()).unwrap(),
            b"state-authoritative filesystem"
        );
        drop(adopted);
        store.discard(&reference).unwrap();
        assert!(store.adopt(&reference).is_err());
    }

    #[test]
    fn state_owned_lineage_repairs_partial_marker_after_crash_window() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"partial-marker filesystem").unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        let reference = rootfs.reference();
        rootfs.preserve_for_lifecycle().unwrap();
        fs::write(rootfs.artifact_dir().join(PRESERVE_MARKER), b"partial").unwrap();
        drop(rootfs);

        let adopted = store.adopt_or_publish(&reference).unwrap();
        assert_eq!(
            fs::read_to_string(adopted.artifact_dir().join(PRESERVE_MARKER)).unwrap(),
            format!("{PRESERVE_MARKER_CONTENT}\n{}", adopted.owner_token)
        );
        adopted.discard_persisted().unwrap();
    }

    #[test]
    fn writable_rootfs_storage_cap_rejects_insufficient_headroom() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"rootfs headroom").unwrap();
        let requested = fs::metadata(&base).unwrap().len();
        let error = store
            .prepare_with_limit(&base, Some(requested - 1))
            .unwrap_err();
        assert!(error.to_string().contains("storage headroom exhausted"));

        let rootfs = store.prepare_with_limit(&base, Some(requested)).unwrap();
        let reference = rootfs.reference();
        rootfs.discard_persisted().unwrap();
        assert!(store.adopt(&reference).is_err());
    }

    #[test]
    fn concurrent_rootfs_preparation_serializes_capacity_reservation() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"concurrent headroom").unwrap();
        let requested = fs::metadata(&base).unwrap().len();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first_store = store.clone();
        let first_base = base.clone();
        let first = std::thread::spawn(move || {
            let rootfs = first_store
                .prepare_with_limit(&first_base, Some(requested))
                .unwrap();
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            rootfs.discard_persisted().unwrap();
        });
        started_rx.recv().unwrap();

        let second_store = store.clone();
        let second_base = base.clone();
        let second = std::thread::spawn(move || {
            second_store.prepare_with_limit(&second_base, Some(requested))
        });
        let error = second.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("storage headroom exhausted"));
        release_tx.send(()).unwrap();
        first.join().unwrap();
    }

    #[test]
    fn paused_rootfs_snapshot_is_independent_and_never_overwritten() {
        let (tmp, store) = store();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, b"checkpoint contents").unwrap();
        let rootfs = store.prepare(&base).unwrap();
        assert_eq!(rootfs.path().parent(), Some(rootfs.artifact_dir()));

        let checkpoint = tmp.path().join("checkpoint");
        fs::create_dir(&checkpoint).unwrap();
        let destination = checkpoint.join(ROOTFS_FILE);
        rootfs.snapshot_to(&destination).unwrap();
        fs::write(rootfs.path(), b"mutated live rootfs").unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"checkpoint contents");
        assert!(rootfs.snapshot_to(&destination).is_err());
        assert!(fs::read_dir(&checkpoint).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rootfs.ext4.partial-")
        }));

        let mut permissions = fs::metadata(&destination).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&destination, permissions).unwrap();
        let restored = store.prepare(&destination).unwrap();
        assert!(
            !fs::metadata(restored.path())
                .unwrap()
                .permissions()
                .readonly()
        );
    }

    #[cfg(unix)]
    #[test]
    fn stale_artifact_with_unknown_entry_is_skipped_intact() {
        let (_tmp, store) = store();
        let artifact = create_stale_fixture(&store, "sandbox-ABC123");
        fs::write(artifact.join("unexpected"), b"keep all").unwrap();

        assert_eq!(
            store.reap_stale().unwrap(),
            RootfsCowReapReport {
                skipped_artifacts: 1,
                ..RootfsCowReapReport::default()
            }
        );
        assert!(artifact.join(ROOTFS_FILE).exists());
        assert_eq!(fs::read(artifact.join("unexpected")).unwrap(), b"keep all");
    }

    #[cfg(unix)]
    #[test]
    fn legacy_malformed_and_symlink_artifacts_are_never_reaped() {
        use std::os::unix::fs::symlink;

        let (tmp, store) = store();
        let legacy = create_stale_fixture(&store, "sandbox-LEG123");
        fs::write(
            legacy.join(OWNER_MARKER),
            format!("agentkernel-rootfs-cow-v1\n{}", Uuid::new_v4()),
        )
        .unwrap();
        fs::remove_file(legacy.join(LEASE_FILE)).unwrap();

        let malformed = create_stale_fixture(&store, "sandbox-BAD123");
        fs::write(malformed.join(OWNER_MARKER), b"bad marker").unwrap();

        let malformed_preserve = create_stale_fixture(&store, "sandbox-PRV123");
        fs::write(
            malformed_preserve.join(PRESERVE_MARKER),
            format!("{PRESERVE_MARKER_CONTENT}\n{}", Uuid::new_v4()),
        )
        .unwrap();

        let external = tmp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep"), b"untouched").unwrap();
        symlink(&external, store.root().join("sandbox-LNK123")).unwrap();
        fs::write(store.root().join("foreign-file"), b"untouched").unwrap();
        fs::create_dir(store.root().join("foreign-directory")).unwrap();

        assert_eq!(
            store.reap_stale().unwrap(),
            RootfsCowReapReport {
                skipped_artifacts: 6,
                ..RootfsCowReapReport::default()
            }
        );
        assert!(legacy.exists());
        assert!(malformed.exists());
        assert!(malformed_preserve.exists());
        assert_eq!(
            fs::read(store.root().join("foreign-file")).unwrap(),
            b"untouched"
        );
        assert!(store.root().join("foreign-directory").is_dir());
        assert_eq!(fs::read(external.join("keep")).unwrap(), b"untouched");
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
        assert_eq!(
            fs::read_dir(store.root())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| is_artifact_name(&entry.file_name()))
                .count(),
            0
        );
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
        #[cfg(unix)]
        let unowned_lease = create_file_at(unowned_handle.as_raw_fd(), LEASE_FILE).unwrap();

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
            #[cfg(unix)]
            _lease_handle: unowned_lease,
            preserved: false,
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
