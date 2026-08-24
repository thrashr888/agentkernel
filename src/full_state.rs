//! Durable full-VM checkpoints used by Firecracker pause, resume, and fork.
//!
//! Firecracker writes the memory and device-state files, while AgentKernel
//! supplies an immutable disk image and owns the surrounding artifact
//! lifecycle. Checkpoint identifiers, rather than caller-provided paths, are
//! persisted in sandbox state so deletion can remain confined to the private
//! checkpoint store.

use crate::backend::{BackendType, FullStateSnapshot};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const FORMAT_VERSION: u32 = 1;
pub const MEMORY_FILE: &str = "memory.bin";
pub const VMSTATE_FILE: &str = "vmstate.bin";
pub const ROOTFS_FILE: &str = "rootfs.ext4";
const MANIFEST_FILE: &str = "manifest.json";
const READY_FILE: &str = "recovery-ready.json";
const STAGING_PREFIX: &str = ".staging-";
const DEFAULT_GLOBAL_CAP_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const DEFAULT_MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const GLOBAL_CAP_ENV: &str = "AGENTKERNEL_FULL_STATE_MAX_BYTES";
const MIN_FREE_ENV: &str = "AGENTKERNEL_FULL_STATE_MIN_FREE_BYTES";

/// Firecracker warns that userspace identifiers, cached random values, and
/// cryptographic tokens can be duplicated when the same VM state is resumed
/// more than once. VMGenID reseeds the supported Linux kernel RNG, but cannot
/// repair arbitrary userspace state.
pub const FORK_SECURITY_WARNING: &str = "Forking duplicates userspace memory. Rotate cached identifiers and cryptographic tokens in each child; prefer proxy-managed secrets that never enter the VM.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointArtifact {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Versioned manifest stored inside an AgentKernel-owned checkpoint directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FullStateCheckpoint {
    pub format_version: u32,
    pub id: String,
    pub source_sandbox: String,
    pub created_at: String,
    pub backend: BackendType,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub backend_snapshot: FullStateSnapshot,
    pub memory: CheckpointArtifact,
    pub vmstate: CheckpointArtifact,
    pub rootfs: CheckpointArtifact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecoveryReady {
    format_version: u32,
    id: String,
    source_sandbox: String,
    vcpus: u32,
    memory_mb: u64,
    backend_snapshot: FullStateSnapshot,
}

pub struct CheckpointStaging {
    id: String,
    directory: PathBuf,
}

impl CheckpointStaging {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.directory
    }

    /// Return the deterministic recovery path. Staging directories are
    /// deliberately persistent until explicitly committed or discarded so a
    /// cancelled async request cannot erase the only copy of paused state.
    pub fn preserve(self) -> PathBuf {
        self.directory
    }
}

#[derive(Debug, Clone)]
pub struct FullStateCheckpointStore {
    root: PathBuf,
}

impl FullStateCheckpointStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let root = data_dir.join("full-state-checkpoints");
        ensure_private_directory(&root)?;
        Ok(Self { root })
    }

    pub fn begin(&self) -> Result<CheckpointStaging> {
        let id = Uuid::new_v4().to_string();
        let directory = self.root.join(format!("{STAGING_PREFIX}{id}"));
        fs::create_dir(&directory)
            .context("failed to create full-state checkpoint staging directory")?;
        make_private(&directory)?;
        File::open(&self.root)?.sync_all()?;
        Ok(CheckpointStaging { id, directory })
    }

    /// Fail before pausing a VM unless the checkpoint store has both global
    /// quota capacity and filesystem headroom for the conservative snapshot
    /// reservation. Daemon lifecycle operations hold the authoritative VMM
    /// write lock across this check and checkpoint publication.
    pub fn ensure_capacity(&self, reservation_bytes: u64) -> Result<()> {
        let cap = capacity_setting(GLOBAL_CAP_ENV, DEFAULT_GLOBAL_CAP_BYTES)?;
        let min_free = capacity_setting(MIN_FREE_ENV, DEFAULT_MIN_FREE_BYTES)?;
        let used = directory_usage_bytes(&self.root)?;
        let available = available_space_bytes(&self.root)?;
        check_capacity(used, reservation_bytes, available, cap, min_free)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit(
        &self,
        staging: &CheckpointStaging,
        source_sandbox: &str,
        vcpus: u32,
        memory_mb: u64,
        backend_snapshot: FullStateSnapshot,
    ) -> Result<FullStateCheckpoint> {
        validate_id(staging.id())?;
        let memory = inspect_artifact(staging.path(), MEMORY_FILE)?;
        let vmstate = inspect_artifact(staging.path(), VMSTATE_FILE)?;
        let rootfs = inspect_artifact(staging.path(), ROOTFS_FILE)?;

        let manifest = FullStateCheckpoint {
            format_version: FORMAT_VERSION,
            id: staging.id.clone(),
            source_sandbox: source_sandbox.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            backend: BackendType::Firecracker,
            vcpus,
            memory_mb,
            backend_snapshot,
            memory,
            vmstate,
            rootfs,
        };
        let manifest_path = staging.path().join(MANIFEST_FILE);
        let contents = serde_json::to_vec_pretty(&manifest)?;
        fs::write(&manifest_path, contents)
            .with_context(|| format!("failed to write {}", manifest_path.display()))?;
        File::open(&manifest_path)?.sync_all()?;
        File::open(staging.path())?.sync_all()?;

        let destination = self.root.join(&manifest.id);
        if destination.exists() {
            bail!("checkpoint '{}' already exists", manifest.id);
        }
        fs::rename(staging.path(), &destination).with_context(|| {
            format!(
                "failed to publish full-state checkpoint {} -> {}",
                staging.path().display(),
                destination.display()
            )
        })?;
        // The checkpoint is already atomically visible after rename. A
        // directory fsync failure is worth reporting, but must not turn a
        // recoverable paused VM into an unpublished state artifact.
        if let Err(error) = File::open(&self.root).and_then(|root| root.sync_all()) {
            eprintln!("Warning: failed to sync checkpoint store directory: {error}");
        }
        Ok(manifest)
    }

    /// Mark a complete staging set as safe to publish after a restart.
    ///
    /// Callers may create this marker only after the original Firecracker
    /// process has been terminated. Its absence therefore keeps ambiguous
    /// staging diagnostic-only instead of risking two live copies of a VM.
    pub fn mark_recovery_ready(
        &self,
        staging: &CheckpointStaging,
        source_sandbox: &str,
        vcpus: u32,
        memory_mb: u64,
        backend_snapshot: FullStateSnapshot,
    ) -> Result<()> {
        validate_id(staging.id())?;
        // Validate and sync the full artifact set before publishing the marker.
        // Digests are computed once during commit/recovery publication rather
        // than twice while the source is paused.
        sync_artifact(staging.path(), MEMORY_FILE)?;
        sync_artifact(staging.path(), VMSTATE_FILE)?;
        sync_artifact(staging.path(), ROOTFS_FILE)?;
        let ready = RecoveryReady {
            format_version: FORMAT_VERSION,
            id: staging.id.clone(),
            source_sandbox: source_sandbox.to_string(),
            vcpus,
            memory_mb,
            backend_snapshot,
        };
        let path = staging.path().join(READY_FILE);
        let contents = serde_json::to_vec_pretty(&ready)?;
        let temporary = staging
            .path()
            .join(format!(".{READY_FILE}.tmp-{}", Uuid::new_v4()));
        let publish = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .with_context(|| format!("failed to create {}", temporary.display()))?;
            file.write_all(&contents)
                .with_context(|| format!("failed to write {}", temporary.display()))?;
            file.sync_all()?;
            fs::rename(&temporary, &path).with_context(|| {
                format!(
                    "failed to atomically publish recovery marker {} -> {}",
                    temporary.display(),
                    path.display()
                )
            })?;
            File::open(staging.path())?.sync_all()?;
            Ok(())
        })();
        if publish.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        publish?;
        Ok(())
    }

    /// Publish a deterministic staging directory whose ready marker proves
    /// the source runtime had already been terminated before interruption.
    pub fn recover_ready(
        &self,
        id: &str,
        source_sandbox: &str,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<(FullStateCheckpoint, PathBuf)> {
        let staging_path = self.staging_path(id)?;
        ensure_owned_directory(&staging_path)?;
        let ready_path = staging_path.join(READY_FILE);
        ensure_regular_file(&ready_path)?;
        let ready: RecoveryReady = serde_json::from_slice(&fs::read(&ready_path)?)?;
        if ready.format_version != FORMAT_VERSION
            || ready.id != id
            || ready.source_sandbox != source_sandbox
            || ready.vcpus != vcpus
            || ready.memory_mb != memory_mb
        {
            bail!("recovery-ready metadata does not match sandbox state");
        }
        let staging = CheckpointStaging {
            id: id.to_string(),
            directory: staging_path,
        };
        let checkpoint = self.commit(
            &staging,
            source_sandbox,
            vcpus,
            memory_mb,
            ready.backend_snapshot,
        )?;
        let path = self.checkpoint_dir(id)?;
        Ok((checkpoint, path))
    }

    pub fn recovery_is_ready(&self, id: &str) -> Result<bool> {
        let path = self.staging_path(id)?.join(READY_FILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    bail!("recovery marker is not a regular file: {}", path.display());
                }
                let ready: RecoveryReady = serde_json::from_slice(&fs::read(&path)?)
                    .with_context(|| format!("invalid recovery marker for checkpoint '{id}'"))?;
                if ready.format_version != FORMAT_VERSION || ready.id != id {
                    bail!("recovery-ready marker does not match checkpoint '{id}'");
                }
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("failed to inspect recovery marker for '{id}'"))
            }
        }
    }

    /// Validate a checkpoint once and return both its manifest and directory.
    pub fn load(&self, id: &str) -> Result<(FullStateCheckpoint, PathBuf)> {
        let directory = self.checkpoint_dir(id)?;
        let manifest_path = directory.join(MANIFEST_FILE);
        ensure_regular_file(&manifest_path)?;
        let manifest: FullStateCheckpoint = serde_json::from_slice(
            &fs::read(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?,
        )?;
        if manifest.format_version != FORMAT_VERSION {
            bail!(
                "checkpoint '{}' uses unsupported format version {} (expected {})",
                id,
                manifest.format_version,
                FORMAT_VERSION
            );
        }
        if manifest.id != id || manifest.backend != BackendType::Firecracker {
            bail!(
                "checkpoint '{}' manifest identity does not match its directory",
                id
            );
        }
        validate_artifact(&directory, &manifest.memory, MEMORY_FILE)?;
        validate_artifact(&directory, &manifest.vmstate, VMSTATE_FILE)?;
        validate_artifact(&directory, &manifest.rootfs, ROOTFS_FILE)?;
        Ok((manifest, directory))
    }

    /// Deterministic unpublished path for a checkpoint transition.
    pub fn staging_path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(format!("{STAGING_PREFIX}{id}")))
    }

    /// Reopen deterministic staging retained by an in-process recovery owner.
    pub fn open_staging(&self, id: &str) -> Result<CheckpointStaging> {
        let directory = self.staging_path(id)?;
        ensure_owned_directory(&directory)?;
        Ok(CheckpointStaging {
            id: id.to_string(),
            directory,
        })
    }

    /// Check whether an atomically published checkpoint directory exists
    /// without hashing its potentially multi-gigabyte artifacts.
    pub fn contains(&self, id: &str) -> Result<bool> {
        validate_id(id)?;
        let directory = self.root.join(id);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                bail!("checkpoint '{}' is not an owned directory", id)
            }
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("failed to inspect checkpoint directory '{id}'"))
            }
        }
    }

    /// Enumerate unpublished transitions without following directory entries.
    /// Callers use this at startup to report interrupted or orphaned pauses;
    /// automatic deletion would be unsafe while another manager may be live.
    pub fn staging_entries(&self) -> Result<Vec<(Option<String>, PathBuf)>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            let Some(raw_id) = name.strip_prefix(STAGING_PREFIX) else {
                continue;
            };
            let id = validate_id(raw_id).is_ok().then(|| raw_id.to_string());
            entries.push((id, path));
        }
        entries.sort_by(|left, right| left.1.cmp(&right.1));
        Ok(entries)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let directory = self.root.join(id);
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect checkpoint '{}'", id));
            }
        };
        // A canonical UUID directory directly beneath the private,
        // current-user-owned store is the deletion boundary. Do not require
        // valid artifacts here: removal is also the recovery path for corrupt
        // or partial checkpoints.
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("checkpoint '{}' is not an owned directory", id);
        }
        fs::remove_dir_all(&directory).with_context(|| {
            format!(
                "failed to delete checkpoint directory {}",
                directory.display()
            )
        })?;
        // The directory is already gone. A failed durability sync must not
        // make callers retain state that references a consumed checkpoint.
        if let Err(error) = File::open(&self.root).and_then(|root| root.sync_all()) {
            eprintln!("Warning: failed to sync checkpoint deletion: {error}");
        }
        Ok(())
    }

    /// Delete an unpublished staging directory retained after an ambiguous
    /// pause failure.
    ///
    /// The path comes from [`CheckpointStaging::preserve`], but the same
    /// containment and file-type boundary is revalidated here before any
    /// recursive removal.
    pub fn discard_staging(&self, path: &Path) -> Result<()> {
        let staging_id = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(STAGING_PREFIX));
        if path.parent() != Some(self.root.as_path())
            || staging_id.is_none_or(|id| validate_id(id).is_err())
        {
            bail!(
                "full-state recovery path is outside the checkpoint staging boundary: {}",
                path.display()
            );
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect recovery staging path {}", path.display())
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "full-state recovery path is not an owned directory: {}",
                path.display()
            );
        }
        fs::remove_dir_all(path).with_context(|| {
            format!(
                "failed to delete recovery staging directory {}",
                path.display()
            )
        })?;
        if let Err(error) = File::open(&self.root).and_then(|root| root.sync_all()) {
            eprintln!("Warning: failed to sync recovery staging deletion: {error}");
        }
        Ok(())
    }

    fn checkpoint_dir(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        let directory = self.root.join(id);
        let metadata = fs::symlink_metadata(&directory)
            .with_context(|| format!("checkpoint '{}' not found", id))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("checkpoint '{}' is not an owned directory", id);
        }
        Ok(directory)
    }
}

fn ensure_owned_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("checkpoint directory is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "checkpoint path is not an owned directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn inspect_artifact(directory: &Path, file: &str) -> Result<CheckpointArtifact> {
    let path = directory.join(file);
    ensure_regular_file(&path)?;
    let bytes = fs::metadata(&path)?.len();
    if bytes == 0 {
        bail!("checkpoint artifact '{}' is empty", file);
    }
    File::open(&path)?.sync_all()?;
    Ok(CheckpointArtifact {
        file: file.to_string(),
        bytes,
        sha256: sha256_file(&path)?,
    })
}

fn sync_artifact(directory: &Path, file: &str) -> Result<()> {
    let path = directory.join(file);
    ensure_regular_file(&path)?;
    if fs::metadata(&path)?.len() == 0 {
        bail!("checkpoint artifact '{}' is empty", file);
    }
    File::open(path)?.sync_all()?;
    Ok(())
}

fn validate_artifact(
    directory: &Path,
    artifact: &CheckpointArtifact,
    expected_file: &str,
) -> Result<()> {
    if artifact.file != expected_file {
        bail!("checkpoint artifact name mismatch for '{}'", expected_file);
    }
    let path = directory.join(expected_file);
    ensure_regular_file(&path)?;
    let bytes = fs::metadata(&path)?.len();
    if bytes != artifact.bytes || bytes == 0 {
        bail!("checkpoint artifact '{}' size changed", expected_file);
    }
    let sha256 = sha256_file(&path)?;
    if sha256 != artifact.sha256 {
        bail!("checkpoint artifact '{}' digest changed", expected_file);
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let bytes = file.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn validate_id(id: &str) -> Result<()> {
    let parsed = Uuid::parse_str(id).context("invalid full-state checkpoint id")?;
    if parsed.to_string() != id {
        bail!("full-state checkpoint id must use canonical UUID form");
    }
    Ok(())
}

fn ensure_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("checkpoint artifact is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "checkpoint artifact is not a regular file: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)
            .with_context(|| format!("failed to create checkpoint store {}", path.display()))?;
        make_private(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("checkpoint store is not a directory: {}", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!("checkpoint store is not owned by the current user");
        }
        if metadata.permissions().mode() & 0o777 != 0o700 {
            bail!(
                "existing checkpoint store must have mode 0700: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn capacity_setting(name: &str, default: u64) -> Result<u64> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .with_context(|| format!("{name} must be an unsigned byte count")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
    }
}

fn check_capacity(
    used: u64,
    reservation: u64,
    available: u64,
    cap: u64,
    min_free: u64,
) -> Result<()> {
    if used.saturating_add(reservation) > cap {
        bail!(
            "full-state checkpoint storage cap exceeded: used={} bytes, requested={} bytes, cap={} bytes; remove checkpoints or raise {}",
            used,
            reservation,
            cap,
            GLOBAL_CAP_ENV
        );
    }
    if reservation > available.saturating_sub(min_free) {
        bail!(
            "insufficient checkpoint filesystem headroom: available={} bytes, requested={} bytes, required free reserve={} bytes; free space or lower {}",
            available,
            reservation,
            min_free,
            MIN_FREE_ENV
        );
    }
    Ok(())
}

fn directory_usage_bytes(root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).with_context(|| {
            format!(
                "failed to inspect checkpoint usage in {}",
                directory.display()
            )
        })? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

#[cfg(unix)]
fn available_space_bytes(path: &Path) -> Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .context("checkpoint store path contains an interior NUL")?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a NUL-terminated CString and `stats` points to valid,
    // writable storage initialized by a successful statvfs call.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect checkpoint filesystem capacity");
    }
    // SAFETY: statvfs returned success and initialized the structure.
    let stats = unsafe { stats.assume_init() };
    // `fsblkcnt_t` is u32 on Apple targets and u64 on Linux targets. Keep the
    // widening cast for the former even though it is a no-op on the latter.
    #[allow(clippy::unnecessary_cast)]
    let available_blocks = stats.f_bavail as u64;
    Ok(available_blocks.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn available_space_bytes(_path: &Path) -> Result<u64> {
    bail!("full-state checkpoint capacity checks require a Unix host")
}

fn make_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend_snapshot() -> FullStateSnapshot {
        FullStateSnapshot {
            firecracker_version: "1.16.1".to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            host_kernel_release: "test-kernel".to_string(),
            host_identity_sha256: "test-host-id".to_string(),
            cpu_fingerprint_sha256: "test-cpu-id".to_string(),
            guest_kernel_release: "6.18.45-agentkernel".to_string(),
        }
    }

    fn write_artifacts(directory: &Path) {
        fs::write(directory.join(MEMORY_FILE), b"memory").unwrap();
        fs::write(directory.join(VMSTATE_FILE), b"state").unwrap();
        fs::write(directory.join(ROOTFS_FILE), b"rootfs").unwrap();
    }

    #[test]
    fn commit_get_and_delete_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        write_artifacts(staging.path());
        let manifest = store
            .commit(&staging, "source", 2, 1024, backend_snapshot())
            .unwrap();
        let (loaded, path) = store.load(&manifest.id).unwrap();
        assert_eq!(loaded, manifest);
        assert!(path.join(ROOTFS_FILE).is_file());
        store.delete(&manifest.id).unwrap();
        assert!(store.load(&manifest.id).is_err());
    }

    #[test]
    fn refuses_size_tampering_and_noncanonical_ids() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        write_artifacts(staging.path());
        let manifest = store
            .commit(&staging, "source", 1, 512, backend_snapshot())
            .unwrap();
        fs::write(store.root.join(&manifest.id).join(MEMORY_FILE), b"changed").unwrap();
        assert!(store.load(&manifest.id).is_err());
        assert!(store.load("../escape").is_err());

        // Corruption must not strand a sandbox that the caller is removing,
        // and a retry after successful removal is harmless.
        store.delete(&manifest.id).unwrap();
        store.delete(&manifest.id).unwrap();
    }

    #[test]
    fn refuses_same_size_artifact_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        write_artifacts(staging.path());
        let manifest = store
            .commit(&staging, "source", 1, 512, backend_snapshot())
            .unwrap();
        fs::write(store.root.join(&manifest.id).join(MEMORY_FILE), b"MEMORY").unwrap();

        let error = store.load(&manifest.id).unwrap_err();
        assert!(error.to_string().contains("digest changed"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_artifacts() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        fs::write(staging.path().join(MEMORY_FILE), b"memory").unwrap();
        fs::write(staging.path().join(VMSTATE_FILE), b"state").unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"rootfs").unwrap();
        symlink(&outside, staging.path().join(ROOTFS_FILE)).unwrap();
        assert!(
            store
                .commit(&staging, "source", 1, 512, backend_snapshot())
                .is_err()
        );
    }

    #[test]
    fn discards_only_owned_staging_directories() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let staging_path = staging.preserve();

        store.discard_staging(&staging_path).unwrap();
        assert!(!staging_path.exists());
        assert!(store.discard_staging(temp.path()).is_err());
    }

    #[test]
    fn staging_path_is_deterministic_and_scannable() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        let path = staging.path().to_path_buf();
        assert_eq!(store.staging_path(&id).unwrap(), path);

        drop(staging);
        assert!(path.is_dir(), "staging must survive async cancellation");
        assert_eq!(store.staging_entries().unwrap(), vec![(Some(id), path)]);
    }

    #[test]
    fn recovery_ready_marker_can_publish_interrupted_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        write_artifacts(staging.path());
        store
            .mark_recovery_ready(&staging, "source", 2, 1024, backend_snapshot())
            .unwrap();
        let _ = staging.preserve();

        let (checkpoint, path) = store.recover_ready(&id, "source", 2, 1024).unwrap();
        assert_eq!(checkpoint.id, id);
        assert!(path.is_dir());
        assert!(store.load(&id).is_ok());
    }

    #[test]
    fn malformed_recovery_marker_fails_closed_and_preserves_staging() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        let staging_path = staging.path().to_path_buf();
        write_artifacts(staging.path());
        fs::write(staging.path().join(READY_FILE), b"{\"id\":").unwrap();
        let _ = staging.preserve();

        assert!(store.recovery_is_ready(&id).is_err());
        assert!(store.recover_ready(&id, "source", 2, 1024).is_err());
        assert!(staging_path.is_dir());
        assert!(!store.contains(&id).unwrap());
    }

    #[test]
    fn capacity_guard_reserves_global_cap_and_free_space_headroom() {
        assert!(check_capacity(40, 10, 100, 50, 20).is_ok());

        let cap_error = check_capacity(41, 10, 100, 50, 20).unwrap_err();
        assert!(cap_error.to_string().contains("storage cap exceeded"));

        let headroom_error = check_capacity(0, 81, 100, 1_000, 20).unwrap_err();
        assert!(headroom_error.to_string().contains("filesystem headroom"));
    }
}
