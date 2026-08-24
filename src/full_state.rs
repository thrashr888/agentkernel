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
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

// Version 2 binds a checkpoint to the immutable UUID of the source sandbox.
// Keep legacy fields deserializable with an empty default so old records can
// reach the explicit version check and fail closed instead of being treated
// as usable checkpoints.
pub const FORMAT_VERSION: u32 = 2;
pub const MEMORY_FILE: &str = "memory.bin";
pub const VMSTATE_FILE: &str = "vmstate.bin";
pub const ROOTFS_FILE: &str = "rootfs.ext4";
const MANIFEST_FILE: &str = "manifest.json";
const READY_FILE: &str = "recovery-ready.json";
const TENANT_FILE: &str = "tenant.json";
const STORE_LOCK_FILE: &str = ".agentkernel-store.lock";
const STAGING_PREFIX: &str = ".staging-";
const DEFAULT_GLOBAL_CAP_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const DEFAULT_TENANT_CAP_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const DEFAULT_MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const DEFAULT_GC_GRACE_SECONDS: u64 = 60 * 60;
const GLOBAL_CAP_ENV: &str = "AGENTKERNEL_FULL_STATE_MAX_BYTES";
const TENANT_CAP_ENV: &str = "AGENTKERNEL_FULL_STATE_TENANT_MAX_BYTES";
const MIN_FREE_ENV: &str = "AGENTKERNEL_FULL_STATE_MIN_FREE_BYTES";
const GC_GRACE_ENV: &str = "AGENTKERNEL_FULL_STATE_GC_GRACE_SECONDS";

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
    #[serde(default)]
    pub source_sandbox_uuid: String,
    /// Trusted tenant owner used for per-tenant storage accounting.
    /// `None` is retained for manifests created before tenant quotas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    pub created_at: String,
    pub backend: BackendType,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub backend_snapshot: FullStateSnapshot,
    pub memory: CheckpointArtifact,
    pub vmstate: CheckpointArtifact,
    pub rootfs: CheckpointArtifact,
}

impl FullStateCheckpoint {
    /// Verify that a checkpoint belongs to the currently persisted sandbox.
    /// Names are retained for operator-facing diagnostics, but the UUID is
    /// the immutable identity that prevents name reuse from adopting state.
    pub fn validate_source(&self, source_sandbox: &str, source_sandbox_uuid: &str) -> Result<()> {
        validate_sandbox_uuid(&self.source_sandbox_uuid)?;
        validate_sandbox_uuid(source_sandbox_uuid)?;
        if self.source_sandbox != source_sandbox {
            bail!(
                "Sandbox '{}' references checkpoint '{}' owned by sandbox '{}'",
                source_sandbox,
                self.id,
                self.source_sandbox
            );
        }
        if self.source_sandbox_uuid != source_sandbox_uuid {
            bail!(
                "Sandbox '{}' references checkpoint '{}' owned by sandbox UUID '{}', not current UUID '{}'",
                source_sandbox,
                self.id,
                self.source_sandbox_uuid,
                source_sandbox_uuid
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecoveryReady {
    format_version: u32,
    id: String,
    source_sandbox: String,
    #[serde(default)]
    source_sandbox_uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
    vcpus: u32,
    memory_mb: u64,
    backend_snapshot: FullStateSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TenantMarker {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
    #[serde(default)]
    reserved_bytes: u64,
}

pub struct CheckpointStaging {
    id: String,
    directory: PathBuf,
    tenant_id: Option<String>,
    store_lease: RefCell<Option<CheckpointStoreLease>>,
}

/// Opaque proof that this process holds the checkpoint store's exclusive
/// advisory lock. The root binding prevents accidentally authorizing an
/// operation against a different store.
pub(crate) struct CheckpointStoreLease {
    root: PathBuf,
    _file: File,
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

/// Storage usage observed while scanning one checkpoint store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointStorageUsage {
    pub global_bytes: u64,
    pub published_bytes: u64,
    pub staging_bytes: u64,
    pub tenant_bytes: BTreeMap<String, u64>,
    pub published_checkpoints: u64,
    pub staging_entries: u64,
}

/// Result of a reference-aware checkpoint garbage-collection pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointGcResult {
    pub removed_published: u64,
    pub removed_staging: u64,
    pub freed_bytes: u64,
    pub skipped_referenced: u64,
    pub skipped_invalid: u64,
}

impl FullStateCheckpointStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let root = data_dir.join("full-state-checkpoints");
        ensure_private_directory(&root)?;
        Ok(Self { root })
    }

    // Retained as the tenant-neutral public API; the binary uses the
    // tenant-aware variant directly while library consumers may not.
    #[allow(dead_code)]
    pub fn begin(&self) -> Result<CheckpointStaging> {
        self.begin_for_tenant(None)
    }

    /// Begin a checkpoint transition and persist its tenant before any large
    /// artifacts are written. This lets quota scans account for interrupted
    /// staging even when the process exits before a ready marker or manifest.
    pub fn begin_for_tenant(&self, tenant_id: Option<&str>) -> Result<CheckpointStaging> {
        let lease = self.acquire_store_lease()?;
        self.begin_locked(tenant_id, 0, lease)
    }

    /// Atomically reserve quota and begin a durable checkpoint transition.
    /// The returned staging object keeps the process lock through artifact
    /// creation and publication, fencing concurrent writers and GC passes.
    pub fn reserve_for_tenant(
        &self,
        tenant_id: Option<&str>,
        reservation_bytes: u64,
    ) -> Result<CheckpointStaging> {
        let lease = self.acquire_store_lease()?;
        self.ensure_capacity_locked(tenant_id, reservation_bytes)?;
        self.begin_locked(tenant_id, reservation_bytes, lease)
    }

    fn begin_locked(
        &self,
        tenant_id: Option<&str>,
        reserved_bytes: u64,
        lease: CheckpointStoreLease,
    ) -> Result<CheckpointStaging> {
        let tenant_id = tenant_id.map(validate_tenant_id).transpose()?;
        let id = Uuid::new_v4().to_string();
        let directory = self.root.join(format!("{STAGING_PREFIX}{id}"));
        fs::create_dir(&directory)
            .context("failed to create full-state checkpoint staging directory")?;
        make_private(&directory)?;
        let contents = serde_json::to_vec(&TenantMarker {
            tenant_id: tenant_id.clone(),
            reserved_bytes,
        })?;
        fs::write(directory.join(TENANT_FILE), contents)?;
        File::open(directory.join(TENANT_FILE))?.sync_all()?;
        File::open(&self.root)?.sync_all()?;
        let staging = CheckpointStaging {
            id,
            directory,
            tenant_id,
            store_lease: RefCell::new(Some(lease)),
        };
        self.refresh_metrics();
        Ok(staging)
    }

    /// Fail before pausing a VM unless both global and tenant-specific quota
    /// capacity plus filesystem headroom are available.
    #[allow(dead_code)]
    pub fn ensure_capacity_for_tenant(
        &self,
        tenant_id: Option<&str>,
        reservation_bytes: u64,
    ) -> Result<()> {
        let _lease = self.acquire_store_lease()?;
        self.ensure_capacity_locked(tenant_id, reservation_bytes)
    }

    fn ensure_capacity_locked(
        &self,
        tenant_id: Option<&str>,
        reservation_bytes: u64,
    ) -> Result<()> {
        let tenant_id = tenant_id.map(validate_tenant_id).transpose()?;
        let cap = capacity_setting(GLOBAL_CAP_ENV, DEFAULT_GLOBAL_CAP_BYTES)?;
        let tenant_cap = capacity_setting(TENANT_CAP_ENV, DEFAULT_TENANT_CAP_BYTES)?;
        let min_free = capacity_setting(MIN_FREE_ENV, DEFAULT_MIN_FREE_BYTES)?;
        let actual_used = directory_usage_bytes(&self.root)?;
        let usage = self.storage_usage()?;
        let outstanding_reservations = usage.global_bytes.saturating_sub(actual_used);
        let available = available_space_bytes(&self.root)?.saturating_sub(outstanding_reservations);
        let (result, denial_scope) = match check_capacity(
            usage.global_bytes,
            reservation_bytes,
            available,
            cap,
            min_free,
        ) {
            Err(error) => (Err(error), Some("global")),
            Ok(()) => match tenant_id.as_deref() {
                Some(tenant_id) => {
                    let tenant_used = usage.tenant_bytes.get(tenant_id).copied().unwrap_or(0);
                    match check_tenant_capacity(tenant_used, reservation_bytes, tenant_cap) {
                        Ok(()) => (Ok(()), None),
                        Err(error) => (Err(error), Some("tenant")),
                    }
                }
                None => (Ok(()), None),
            },
        };
        crate::metrics::set_full_state_storage(
            usage.global_bytes,
            usage.published_bytes,
            usage.staging_bytes,
            cap,
            tenant_cap,
        );
        if let Err(error) = &result {
            crate::metrics::record_full_state_quota_denial(
                denial_scope.expect("quota errors always carry a scope"),
            );
            eprintln!("Warning: full-state checkpoint quota alert: {error:#}");
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub fn commit(
        &self,
        staging: &CheckpointStaging,
        source_sandbox: &str,
        source_sandbox_uuid: &str,
        vcpus: u32,
        memory_mb: u64,
        backend_snapshot: FullStateSnapshot,
    ) -> Result<FullStateCheckpoint> {
        self.commit_for_tenant(
            staging,
            source_sandbox,
            source_sandbox_uuid,
            vcpus,
            memory_mb,
            backend_snapshot,
            staging.tenant_id.as_deref(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_for_tenant(
        &self,
        staging: &CheckpointStaging,
        source_sandbox: &str,
        source_sandbox_uuid: &str,
        vcpus: u32,
        memory_mb: u64,
        backend_snapshot: FullStateSnapshot,
        tenant_id: Option<&str>,
    ) -> Result<FullStateCheckpoint> {
        validate_id(staging.id())?;
        validate_sandbox_uuid(source_sandbox_uuid)?;
        let tenant_id = tenant_id.map(validate_tenant_id).transpose()?;
        if staging.tenant_id.is_some() && staging.tenant_id != tenant_id {
            bail!("checkpoint staging tenant does not match publication tenant");
        }
        let memory = inspect_artifact(staging.path(), MEMORY_FILE)?;
        let vmstate = inspect_artifact(staging.path(), VMSTATE_FILE)?;
        let rootfs = inspect_artifact(staging.path(), ROOTFS_FILE)?;

        let manifest = FullStateCheckpoint {
            format_version: FORMAT_VERSION,
            id: staging.id.clone(),
            source_sandbox: source_sandbox.to_string(),
            source_sandbox_uuid: source_sandbox_uuid.to_string(),
            tenant_id,
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
        drop(staging.store_lease.borrow_mut().take());
        self.refresh_metrics();
        Ok(manifest)
    }

    /// Mark a complete staging set as safe to publish after a restart.
    ///
    /// Callers may create this marker only after the original Firecracker
    /// process has been terminated. Its absence therefore keeps ambiguous
    /// staging diagnostic-only instead of risking two live copies of a VM.
    #[allow(dead_code)]
    pub fn mark_recovery_ready(
        &self,
        staging: &CheckpointStaging,
        source_sandbox: &str,
        source_sandbox_uuid: &str,
        vcpus: u32,
        memory_mb: u64,
        backend_snapshot: FullStateSnapshot,
    ) -> Result<()> {
        self.mark_recovery_ready_for_tenant(
            staging,
            source_sandbox,
            source_sandbox_uuid,
            vcpus,
            memory_mb,
            backend_snapshot,
            staging.tenant_id.as_deref(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_recovery_ready_for_tenant(
        &self,
        staging: &CheckpointStaging,
        source_sandbox: &str,
        source_sandbox_uuid: &str,
        vcpus: u32,
        memory_mb: u64,
        backend_snapshot: FullStateSnapshot,
        tenant_id: Option<&str>,
    ) -> Result<()> {
        validate_id(staging.id())?;
        validate_sandbox_uuid(source_sandbox_uuid)?;
        let tenant_id = tenant_id.map(validate_tenant_id).transpose()?;
        if staging.tenant_id.is_some() && staging.tenant_id != tenant_id {
            bail!("checkpoint staging tenant does not match recovery tenant");
        }
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
            source_sandbox_uuid: source_sandbox_uuid.to_string(),
            tenant_id,
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
    #[allow(dead_code)]
    pub fn recover_ready(
        &self,
        id: &str,
        source_sandbox: &str,
        source_sandbox_uuid: &str,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<(FullStateCheckpoint, PathBuf)> {
        let tenant_id = read_staging_tenant(&self.staging_path(id)?)?;
        self.recover_ready_for_tenant(
            id,
            source_sandbox,
            source_sandbox_uuid,
            vcpus,
            memory_mb,
            tenant_id.as_deref(),
        )
    }

    pub fn recover_ready_for_tenant(
        &self,
        id: &str,
        source_sandbox: &str,
        source_sandbox_uuid: &str,
        vcpus: u32,
        memory_mb: u64,
        tenant_id: Option<&str>,
    ) -> Result<(FullStateCheckpoint, PathBuf)> {
        validate_sandbox_uuid(source_sandbox_uuid)?;
        let tenant_id = tenant_id.map(validate_tenant_id).transpose()?;
        let lease = self.acquire_store_lease()?;
        let staging_path = self.staging_path(id)?;
        ensure_owned_directory(&staging_path)?;
        let ready_path = staging_path.join(READY_FILE);
        ensure_regular_file(&ready_path)?;
        let ready: RecoveryReady = serde_json::from_slice(&fs::read(&ready_path)?)?;
        if ready.format_version != FORMAT_VERSION
            || ready.id != id
            || ready.source_sandbox != source_sandbox
            || ready.source_sandbox_uuid != source_sandbox_uuid
            || (ready.tenant_id.is_some() && ready.tenant_id != tenant_id)
            || ready.vcpus != vcpus
            || ready.memory_mb != memory_mb
        {
            bail!("recovery-ready metadata does not match sandbox state");
        }
        let tenant_id = ready.tenant_id.clone().or(tenant_id);
        let staging = CheckpointStaging {
            id: id.to_string(),
            directory: staging_path,
            tenant_id: tenant_id.clone(),
            store_lease: RefCell::new(Some(lease)),
        };
        let checkpoint = self.commit_for_tenant(
            &staging,
            source_sandbox,
            source_sandbox_uuid,
            vcpus,
            memory_mb,
            ready.backend_snapshot,
            tenant_id.as_deref(),
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
                validate_sandbox_uuid(&ready.source_sandbox_uuid)?;
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
        validate_sandbox_uuid(&manifest.source_sandbox_uuid)?;
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
        let lease = self.acquire_store_lease()?;
        let directory = self.staging_path(id)?;
        ensure_owned_directory(&directory)?;
        let tenant_id = read_staging_tenant(&directory)?;
        Ok(CheckpointStaging {
            id: id.to_string(),
            directory,
            tenant_id,
            store_lease: RefCell::new(Some(lease)),
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

    #[allow(dead_code)]
    pub fn delete(&self, id: &str) -> Result<()> {
        let lease = self.acquire_store_lease()?;
        self.delete_with_lease(id, &lease)
    }

    pub(crate) fn delete_with_lease(&self, id: &str, lease: &CheckpointStoreLease) -> Result<()> {
        self.validate_store_lease(lease)?;
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
        self.refresh_metrics();
        Ok(())
    }

    /// Delete an unpublished staging directory retained after an ambiguous
    /// pause failure.
    ///
    /// The path comes from [`CheckpointStaging::preserve`], but the same
    /// containment and file-type boundary is revalidated here before any
    /// recursive removal.
    pub fn discard_staging(&self, path: &Path) -> Result<()> {
        let lease = self.acquire_store_lease()?;
        self.discard_staging_with_lease(path, &lease)
    }

    pub(crate) fn discard_staging_with_lease(
        &self,
        path: &Path,
        lease: &CheckpointStoreLease,
    ) -> Result<()> {
        self.validate_store_lease(lease)?;
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
        self.refresh_metrics();
        Ok(())
    }

    /// Return byte usage for published checkpoints and all retained staging.
    /// The global value deliberately includes metadata and unknown staging so
    /// an interrupted transition can never evade the host-wide cap.
    pub fn storage_usage(&self) -> Result<CheckpointStorageUsage> {
        let mut usage = CheckpointStorageUsage::default();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let actual_bytes = path_usage_bytes(&path)?;
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                usage.global_bytes = usage.global_bytes.saturating_add(actual_bytes);
                continue;
            };
            if let Some(id) = name.strip_prefix(STAGING_PREFIX) {
                if validate_id(id).is_ok() {
                    let bytes = actual_bytes.max(read_staging_reservation(&path)?);
                    usage.global_bytes = usage.global_bytes.saturating_add(bytes);
                    usage.staging_entries = usage.staging_entries.saturating_add(1);
                    usage.staging_bytes = usage.staging_bytes.saturating_add(bytes);
                    if let Some(tenant_id) = read_staging_tenant(&path)? {
                        let tenant_bytes = usage.tenant_bytes.entry(tenant_id).or_default();
                        *tenant_bytes = tenant_bytes.saturating_add(bytes);
                    }
                } else {
                    usage.global_bytes = usage.global_bytes.saturating_add(actual_bytes);
                }
            } else if validate_id(&name).is_ok() {
                usage.global_bytes = usage.global_bytes.saturating_add(actual_bytes);
                usage.published_checkpoints = usage.published_checkpoints.saturating_add(1);
                usage.published_bytes = usage.published_bytes.saturating_add(actual_bytes);
                if let Some(tenant_id) = read_manifest_tenant(&path)? {
                    let tenant_bytes = usage.tenant_bytes.entry(tenant_id).or_default();
                    *tenant_bytes = tenant_bytes.saturating_add(actual_bytes);
                }
            } else {
                usage.global_bytes = usage.global_bytes.saturating_add(actual_bytes);
            }
        }
        Ok(usage)
    }

    fn refresh_metrics(&self) {
        let Ok(usage) = self.storage_usage() else {
            return;
        };
        let Ok(global_cap) = capacity_setting(GLOBAL_CAP_ENV, DEFAULT_GLOBAL_CAP_BYTES) else {
            return;
        };
        let Ok(tenant_cap) = capacity_setting(TENANT_CAP_ENV, DEFAULT_TENANT_CAP_BYTES) else {
            return;
        };
        crate::metrics::set_full_state_storage(
            usage.global_bytes,
            usage.published_bytes,
            usage.staging_bytes,
            global_cap,
            tenant_cap,
        );
    }

    /// Remove only published checkpoints and staging directories that have no
    /// live sandbox reference. Callers must construct references from every
    /// persisted sandbox, including running forks, while holding lifecycle
    /// serialization.
    #[allow(dead_code)]
    pub fn gc(&self, referenced: &HashSet<String>) -> Result<CheckpointGcResult> {
        let lease = self.acquire_store_lease()?;
        self.gc_with_lease(
            referenced,
            Duration::from_secs(capacity_setting(GC_GRACE_ENV, DEFAULT_GC_GRACE_SECONDS)?),
            &lease,
        )
    }

    pub(crate) fn gc_with_default_grace_and_lease(
        &self,
        referenced: &HashSet<String>,
        lease: &CheckpointStoreLease,
    ) -> Result<CheckpointGcResult> {
        self.gc_with_lease(
            referenced,
            Duration::from_secs(capacity_setting(GC_GRACE_ENV, DEFAULT_GC_GRACE_SECONDS)?),
            lease,
        )
    }

    /// Reference-aware GC with an optional age grace period for another
    /// process that may still be completing a pause transition.
    #[allow(dead_code)]
    pub fn gc_with_grace(
        &self,
        referenced: &HashSet<String>,
        grace: Duration,
    ) -> Result<CheckpointGcResult> {
        let lease = self.acquire_store_lease()?;
        self.gc_with_lease(referenced, grace, &lease)
    }

    fn gc_with_lease(
        &self,
        referenced: &HashSet<String>,
        grace: Duration,
        lease: &CheckpointStoreLease,
    ) -> Result<CheckpointGcResult> {
        self.validate_store_lease(lease)?;
        let mut result = CheckpointGcResult::default();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                result.skipped_invalid = result.skipped_invalid.saturating_add(1);
                continue;
            };
            let (id, staging) = if let Some(id) = name.strip_prefix(STAGING_PREFIX) {
                (id, true)
            } else {
                (name.as_str(), false)
            };
            if validate_id(id).is_err() {
                result.skipped_invalid = result.skipped_invalid.saturating_add(1);
                continue;
            }
            if referenced.contains(id) {
                result.skipped_referenced = result.skipped_referenced.saturating_add(1);
                continue;
            }
            if grace > Duration::ZERO && !older_than(&metadata, grace) {
                continue;
            }
            let bytes = path_usage_bytes(&path)?;
            if staging {
                self.discard_staging_with_lease(&path, lease)?;
                result.removed_staging = result.removed_staging.saturating_add(1);
            } else {
                self.delete_with_lease(id, lease)?;
                result.removed_published = result.removed_published.saturating_add(1);
            }
            result.freed_bytes = result.freed_bytes.saturating_add(bytes);
        }
        let usage = self.storage_usage()?;
        crate::metrics::record_full_state_gc(
            result.removed_published,
            result.removed_staging,
            result.freed_bytes,
        );
        crate::metrics::set_full_state_storage(
            usage.global_bytes,
            usage.published_bytes,
            usage.staging_bytes,
            capacity_setting(GLOBAL_CAP_ENV, DEFAULT_GLOBAL_CAP_BYTES)?,
            capacity_setting(TENANT_CAP_ENV, DEFAULT_TENANT_CAP_BYTES)?,
        );
        Ok(result)
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

    fn validate_store_lease(&self, lease: &CheckpointStoreLease) -> Result<()> {
        if lease.root != self.root {
            bail!("checkpoint store lease belongs to a different store");
        }
        Ok(())
    }

    pub(crate) fn acquire_store_lease(&self) -> Result<CheckpointStoreLease> {
        let path = self.root.join(STORE_LOCK_FILE);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        }
        let lease = options
            .open(&path)
            .with_context(|| format!("failed to open checkpoint store lock {}", path.display()))?;
        let metadata = lease.metadata()?;
        if !metadata.is_file() {
            bail!(
                "checkpoint store lock is not a regular file: {}",
                path.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o777 != 0o600
            {
                bail!("checkpoint store lock must be current-user owned with mode 0600");
            }
            if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to lock checkpoint store");
            }
        }
        Ok(CheckpointStoreLease {
            root: self.root.clone(),
            _file: lease,
        })
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

fn read_manifest_tenant(directory: &Path) -> Result<Option<String>> {
    let path = directory.join(MANIFEST_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let contents = fs::read(path)?;
    let manifest = match serde_json::from_slice::<FullStateCheckpoint>(&contents) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(None),
    };
    Ok(manifest.tenant_id)
}

fn read_staging_tenant(directory: &Path) -> Result<Option<String>> {
    let marker = directory.join(TENANT_FILE);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "checkpoint tenant marker is not a regular file: {}",
                    marker.display()
                );
            }
            let marker: TenantMarker = serde_json::from_slice(&fs::read(&marker)?)?;
            if let Some(tenant_id) = marker.tenant_id {
                validate_tenant_id(&tenant_id)?;
                return Ok(Some(tenant_id));
            }
            return Ok(None);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let ready = directory.join(READY_FILE);
    match fs::symlink_metadata(&ready) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "checkpoint recovery marker is not a regular file: {}",
                    ready.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let ready: RecoveryReady = match serde_json::from_slice(&fs::read(ready)?) {
        Ok(ready) => ready,
        Err(_) => return Ok(None),
    };
    if let Some(tenant_id) = ready.tenant_id {
        validate_tenant_id(&tenant_id)?;
        Ok(Some(tenant_id))
    } else {
        Ok(None)
    }
}

fn read_staging_reservation(directory: &Path) -> Result<u64> {
    let marker = directory.join(TENANT_FILE);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "checkpoint tenant marker is not a regular file: {}",
                    marker.display()
                );
            }
            let marker: TenantMarker = serde_json::from_slice(&fs::read(&marker)?)?;
            if let Some(tenant_id) = marker.tenant_id {
                validate_tenant_id(&tenant_id)?;
            }
            Ok(marker.reserved_bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn path_usage_bytes(root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

#[allow(dead_code)]
fn older_than(metadata: &fs::Metadata, grace: Duration) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= grace)
}

fn validate_id(id: &str) -> Result<()> {
    let parsed = Uuid::parse_str(id).context("invalid full-state checkpoint id")?;
    if parsed.to_string() != id {
        bail!("full-state checkpoint id must use canonical UUID form");
    }
    Ok(())
}

fn validate_sandbox_uuid(uuid: &str) -> Result<()> {
    let parsed = Uuid::parse_str(uuid).context("invalid source sandbox UUID")?;
    if parsed.to_string() != uuid {
        bail!("source sandbox UUID must use canonical UUID form");
    }
    Ok(())
}

fn validate_tenant_id(tenant_id: &str) -> Result<String> {
    let tenant_id = tenant_id.trim();
    if tenant_id.is_empty() || tenant_id.len() > 256 {
        bail!("checkpoint tenant ID must contain 1-256 non-whitespace bytes");
    }
    if tenant_id.chars().any(char::is_control) {
        bail!("checkpoint tenant ID must not contain control characters");
    }
    Ok(tenant_id.to_string())
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

fn check_tenant_capacity(used: u64, reservation: u64, cap: u64) -> Result<()> {
    if used.saturating_add(reservation) > cap {
        bail!(
            "full-state checkpoint tenant quota exceeded: used={} bytes, requested={} bytes, cap={} bytes; remove checkpoints or raise {}",
            used,
            reservation,
            cap,
            TENANT_CAP_ENV
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

    #[cfg(unix)]
    const STORE_HELPER_ROOT: &str = "AGENTKERNEL_TEST_CHECKPOINT_STORE_ROOT";
    #[cfg(unix)]
    const STORE_HELPER_EXPECT_DENIAL: &str = "AGENTKERNEL_TEST_CHECKPOINT_EXPECT_DENIAL";
    #[cfg(unix)]
    const STORE_HELPER_READY: &str = "checkpoint-helper-ready";

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

    fn source_uuid() -> String {
        Uuid::new_v4().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_holds_checkpoint_store_reservation() {
        let Some(root) = std::env::var_os(STORE_HELPER_ROOT) else {
            return;
        };
        let root = PathBuf::from(root);
        let store = FullStateCheckpointStore::new(&root).unwrap();
        let result = store.reserve_for_tenant(Some("tenant-a"), 600);
        if std::env::var_os(STORE_HELPER_EXPECT_DENIAL).is_some() {
            let Err(error) = result else {
                panic!("a competing durable reservation must exhaust the configured quota");
            };
            assert!(
                error.to_string().contains("storage cap exceeded")
                    || error.to_string().contains("tenant quota exceeded")
            );
            return;
        }
        let _staging = result.unwrap();
        fs::write(root.join(STORE_HELPER_READY), b"ready").unwrap();
        std::thread::sleep(Duration::from_secs(30));
    }

    #[cfg(unix)]
    fn spawn_store_helper(root: &Path, expect_denial: bool) -> std::process::Child {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("full_state::tests::subprocess_holds_checkpoint_store_reservation")
            .arg("--nocapture")
            .env(STORE_HELPER_ROOT, root)
            .env(GLOBAL_CAP_ENV, "1000")
            .env(TENANT_CAP_ENV, "1000")
            .env(MIN_FREE_ENV, "0");
        if expect_denial {
            command.env(STORE_HELPER_EXPECT_DENIAL, "1");
        }
        command.spawn().unwrap()
    }

    #[cfg(unix)]
    fn wait_for_store_helper(root: &Path) {
        let ready = root.join(STORE_HELPER_READY);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ready.is_file() {
            assert!(
                std::time::Instant::now() < deadline,
                "checkpoint store helper did not acquire its lease"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn cross_process_reservation_is_serialized_and_durable() {
        let temp = tempfile::tempdir().unwrap();
        let mut first = spawn_store_helper(temp.path(), false);
        wait_for_store_helper(temp.path());

        let mut second = spawn_store_helper(temp.path(), true);
        std::thread::sleep(Duration::from_millis(150));
        assert!(second.try_wait().unwrap().is_none());

        first.kill().unwrap();
        first.wait().unwrap();
        assert!(second.wait().unwrap().success());

        let usage = FullStateCheckpointStore::new(temp.path())
            .unwrap()
            .storage_usage()
            .unwrap();
        assert!(usage.global_bytes >= 600);
        assert!(usage.tenant_bytes["tenant-a"] >= 600);
    }

    #[test]
    fn tenant_neutral_reservation_counts_toward_global_capacity() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.reserve_for_tenant(None, 600).unwrap();
        let usage = store.storage_usage().unwrap();
        assert!(usage.global_bytes >= 600);
        assert!(usage.tenant_bytes.is_empty());
        let path = staging.preserve();
        store.discard_staging(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn gc_waits_for_cross_process_active_writer() {
        let temp = tempfile::tempdir().unwrap();
        let mut writer = spawn_store_helper(temp.path(), false);
        wait_for_store_helper(temp.path());

        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let collector = std::thread::spawn(move || {
            result_tx
                .send(store.gc_with_grace(&HashSet::new(), Duration::ZERO))
                .unwrap();
        });
        assert!(
            result_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "GC must block on the active writer's process lease"
        );

        writer.kill().unwrap();
        writer.wait().unwrap();
        let result = result_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        collector.join().unwrap();
        assert_eq!(result.removed_staging, 1);
    }

    #[cfg(unix)]
    #[test]
    fn direct_staging_delete_waits_for_cross_process_active_writer() {
        let temp = tempfile::tempdir().unwrap();
        let mut writer = spawn_store_helper(temp.path(), false);
        wait_for_store_helper(temp.path());

        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging_path = store.staging_entries().unwrap()[0].1.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let deleter = std::thread::spawn(move || {
            result_tx
                .send(store.discard_staging(&staging_path))
                .unwrap();
        });
        assert!(
            result_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "direct deletion must block on the active writer's process lease"
        );

        writer.kill().unwrap();
        writer.wait().unwrap();
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        deleter.join().unwrap();
    }

    #[test]
    fn commit_get_and_delete_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        write_artifacts(staging.path());
        let source_uuid = source_uuid();
        let manifest = store
            .commit(
                &staging,
                "source",
                &source_uuid,
                2,
                1024,
                backend_snapshot(),
            )
            .unwrap();
        let (loaded, path) = store.load(&manifest.id).unwrap();
        assert_eq!(loaded, manifest);
        assert_eq!(loaded.source_sandbox_uuid, source_uuid);
        assert!(loaded.validate_source("source", &source_uuid).is_ok());
        assert!(loaded.validate_source("renamed", &source_uuid).is_err());
        assert!(
            loaded
                .validate_source("source", &Uuid::new_v4().to_string())
                .is_err()
        );
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
        let source_uuid = source_uuid();
        let manifest = store
            .commit(&staging, "source", &source_uuid, 1, 512, backend_snapshot())
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
        let source_uuid = source_uuid();
        let manifest = store
            .commit(&staging, "source", &source_uuid, 1, 512, backend_snapshot())
            .unwrap();
        fs::write(store.root.join(&manifest.id).join(MEMORY_FILE), b"MEMORY").unwrap();

        let error = store.load(&manifest.id).unwrap_err();
        assert!(error.to_string().contains("digest changed"));
    }

    #[test]
    fn refuses_tampered_source_uuid_even_when_the_replacement_is_valid() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        write_artifacts(staging.path());
        let source_uuid = source_uuid();
        let manifest = store
            .commit(&staging, "source", &source_uuid, 1, 512, backend_snapshot())
            .unwrap();
        let manifest_path = store.root.join(&manifest.id).join(MANIFEST_FILE);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let replacement = Uuid::new_v4().to_string();
        json["source_sandbox_uuid"] = serde_json::Value::String(replacement);
        fs::write(&manifest_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

        let (tampered, _) = store.load(&manifest.id).unwrap();
        assert!(tampered.validate_source("source", &source_uuid).is_err());
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
        let source_uuid = source_uuid();
        assert!(
            store
                .commit(&staging, "source", &source_uuid, 1, 512, backend_snapshot())
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
    fn tenant_usage_and_gc_preserve_referenced_published_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let source_uuid = source_uuid();

        let referenced_staging = store.begin_for_tenant(Some("tenant-a")).unwrap();
        write_artifacts(referenced_staging.path());
        let referenced = store
            .commit_for_tenant(
                &referenced_staging,
                "paused-source",
                &source_uuid,
                1,
                512,
                backend_snapshot(),
                Some("tenant-a"),
            )
            .unwrap();

        let orphan_staging = store.begin_for_tenant(Some("tenant-a")).unwrap();
        fs::write(orphan_staging.path().join(MEMORY_FILE), b"orphan").unwrap();
        let orphan_staging_path = orphan_staging.preserve();

        let orphan_published_staging = store.begin_for_tenant(Some("tenant-a")).unwrap();
        write_artifacts(orphan_published_staging.path());
        let orphan_published = store
            .commit_for_tenant(
                &orphan_published_staging,
                "removed-source",
                &Uuid::new_v4().to_string(),
                1,
                512,
                backend_snapshot(),
                Some("tenant-a"),
            )
            .unwrap();

        let usage = store.storage_usage().unwrap();
        assert!(usage.global_bytes > 0);
        assert!(usage.tenant_bytes["tenant-a"] >= usage.global_bytes / 2);

        let references = HashSet::from([referenced.id.clone()]);
        let grace_result = store.gc(&references).unwrap();
        assert_eq!(grace_result.removed_published, 0);
        assert_eq!(grace_result.removed_staging, 0);
        assert!(store.contains(&orphan_published.id).unwrap());
        assert!(orphan_staging_path.exists());

        let gc = store.gc_with_grace(&references, Duration::ZERO).unwrap();
        assert!(store.contains(&referenced.id).unwrap());
        assert!(!store.contains(&orphan_published.id).unwrap());
        assert!(!orphan_staging_path.exists());
        assert_eq!(gc.removed_published, 1);
        assert_eq!(gc.removed_staging, 1);
        assert!(gc.freed_bytes > 0);
    }

    #[test]
    fn tenant_capacity_is_checked_independently_of_global_capacity() {
        assert!(check_tenant_capacity(90, 10, 100).is_ok());
        let error = check_tenant_capacity(91, 10, 100).unwrap_err();
        assert!(error.to_string().contains("tenant quota exceeded"));
    }

    #[test]
    fn recovery_ready_marker_can_publish_interrupted_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        write_artifacts(staging.path());
        let source_uuid = source_uuid();
        store
            .mark_recovery_ready(
                &staging,
                "source",
                &source_uuid,
                2,
                1024,
                backend_snapshot(),
            )
            .unwrap();
        let _ = staging.preserve();

        let (checkpoint, path) = store
            .recover_ready(&id, "source", &source_uuid, 2, 1024)
            .unwrap();
        assert_eq!(checkpoint.id, id);
        assert_eq!(checkpoint.source_sandbox_uuid, source_uuid);
        assert!(path.is_dir());
        assert!(store.load(&id).is_ok());
    }

    #[test]
    fn recovery_ready_marker_rejects_source_uuid_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        let staging_path = staging.path().to_path_buf();
        write_artifacts(staging.path());
        let source_uuid = source_uuid();
        store
            .mark_recovery_ready(
                &staging,
                "source",
                &source_uuid,
                2,
                1024,
                backend_snapshot(),
            )
            .unwrap();
        let _ = staging.preserve();

        let error = store
            .recover_ready(&id, "source", &Uuid::new_v4().to_string(), 2, 1024)
            .unwrap_err();
        assert!(error.to_string().contains("metadata does not match"));
        assert!(staging_path.is_dir());
        assert!(!store.contains(&id).unwrap());
    }

    #[test]
    fn legacy_manifest_format_fails_closed_before_uuid_migration() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        write_artifacts(staging.path());
        let manifest = store
            .commit(
                &staging,
                "source",
                &source_uuid(),
                1,
                512,
                backend_snapshot(),
            )
            .unwrap();
        let manifest_path = store.root.join(&manifest.id).join(MANIFEST_FILE);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        json["format_version"] = serde_json::Value::from(1);
        json.as_object_mut().unwrap().remove("source_sandbox_uuid");
        fs::write(&manifest_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

        let error = store.load(&manifest.id).unwrap_err();
        assert!(error.to_string().contains("unsupported format version 1"));
    }

    #[test]
    fn legacy_recovery_marker_format_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = FullStateCheckpointStore::new(temp.path()).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        let staging_path = staging.path().to_path_buf();
        write_artifacts(staging.path());
        let source_uuid = source_uuid();
        store
            .mark_recovery_ready(
                &staging,
                "source",
                &source_uuid,
                2,
                1024,
                backend_snapshot(),
            )
            .unwrap();
        let marker_path = staging.path().join(READY_FILE);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        json["format_version"] = serde_json::Value::from(1);
        json.as_object_mut().unwrap().remove("source_sandbox_uuid");
        fs::write(&marker_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        let _ = staging.preserve();

        assert!(store.recovery_is_ready(&id).is_err());
        assert!(
            store
                .recover_ready(&id, "source", &source_uuid, 2, 1024)
                .is_err()
        );
        assert!(staging_path.is_dir());
        assert!(!store.contains(&id).unwrap());
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
        assert!(
            store
                .recover_ready(&id, "source", &source_uuid(), 2, 1024)
                .is_err()
        );
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
