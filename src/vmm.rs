//! Virtual Machine Manager
//!
//! This module provides the interface to sandboxes via Firecracker microVMs
//! or containers (Docker/Podman) as fallback when KVM is not available.

use crate::audit::{AuditEvent, log_event};
use crate::backend::{
    BackendType, FileInjection, FullStatePauseError, FullStateSnapshot, FullStateTerminationError,
    PortMapping, RemoteSandboxContext, ResolvedEndpoint, Sandbox, SandboxConfig,
    SandboxRuntimeMetadata, backend_capabilities, create_sandbox, create_sandbox_with_state,
    detect_best_backend,
};
use crate::config::Config;
use crate::cow::RootfsCowStore;
use crate::docker_backend::detect_container_runtime;
use crate::full_state::{FullStateCheckpoint, FullStateCheckpointStore};
use crate::languages::docker_image_to_firecracker_runtime;
use crate::permissions::Permissions;
use crate::pool::ContainerPool;
use crate::proxy::{ProxyConfig, ProxyHandle, SecretBinding};
use crate::secrets::{SecretBackend, SecretVault};
use crate::validation;
use crate::volume::{VolumeManager, VolumeMount};
use anyhow::{Context, Result, bail};

/// Error returned when a command exits with a non-zero exit code.
/// Distinguished from infrastructure errors so HTTP handlers can return
/// an appropriate status code (e.g. 409 instead of 500).
#[derive(Debug, thiserror::Error)]
#[error("Command exited with code {exit_code}: {output}")]
pub struct CommandFailed {
    pub exit_code: i32,
    pub output: String,
}
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "enterprise")]
use std::sync::{LazyLock, Mutex};
use tokio::sync::RwLock;

/// Global proxy handle registry. Proxy handles must outlive individual VmManager
/// instances since VmManager is created fresh per HTTP request.
static PROXY_HANDLES: std::sync::LazyLock<RwLock<HashMap<String, ProxyHandle>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "enterprise")]
struct CachedPolicyEngine {
    file_signature: Option<crate::config::ConfigFingerprint>,
    engine: Option<Arc<crate::policy::PolicyEngine>>,
}

#[cfg(feature = "enterprise")]
static POLICY_ENGINE_CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedPolicyEngine>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(feature = "enterprise")]
fn cached_policy_engine() -> Option<Arc<crate::policy::PolicyEngine>> {
    let default_config = std::env::current_dir()
        .ok()
        .map(|dir| dir.join("agentkernel.toml"))?;
    let file_signature = Config::file_fingerprint(&default_config);

    if let Some(entry) = POLICY_ENGINE_CACHE
        .lock()
        .expect("policy engine cache lock poisoned")
        .get(&default_config)
        && entry.file_signature == file_signature
    {
        return entry.engine.clone();
    }

    let engine = Config::from_file_cached(&default_config)
        .ok()
        .filter(|cfg| cfg.enterprise.enabled)
        .and_then(
            |cfg| match crate::policy::PolicyEngine::new(&cfg.enterprise) {
                Ok(engine) => {
                    eprintln!("[enterprise] Policy engine initialized");
                    Some(Arc::new(engine))
                }
                Err(error) => {
                    eprintln!("[enterprise] Failed to initialize policy engine: {error}");
                    None
                }
            },
        );

    POLICY_ENGINE_CACHE
        .lock()
        .expect("policy engine cache lock poisoned")
        .insert(
            default_config,
            CachedPolicyEngine {
                file_signature,
                engine: engine.clone(),
            },
        );
    engine
}
use tokio::sync::OnceCell;

/// Global container pool for fast ephemeral runs
static CONTAINER_POOL: OnceCell<Arc<ContainerPool>> = OnceCell::const_new();

/// Get or initialize the global container pool
async fn get_pool() -> Result<Arc<ContainerPool>> {
    CONTAINER_POOL
        .get_or_try_init(|| async {
            let pool = ContainerPool::with_config(5, 20, "alpine:3.24")?;
            pool.start().await?;
            Ok(Arc::new(pool))
        })
        .await
        .cloned()
}

/// Declarative lifecycle automation policy for a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxLifecyclePolicy {
    /// Stop the sandbox after this much inactivity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_stop_after_seconds: Option<u64>,
    /// Archive the sandbox after this much inactivity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_archive_after_seconds: Option<u64>,
    /// Delete an archived sandbox after this duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_delete_after_seconds: Option<u64>,
}

/// Host-side Git checkout created for an agent sandbox.
///
/// The branch is intentionally retained after sandbox removal so commits
/// produced by an agent remain recoverable; only the disposable checkout is
/// cleaned up automatically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxGitWorktree {
    pub repository: String,
    pub path: String,
    pub branch: String,
    pub base_ref: String,
}

/// Action produced by lifecycle reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleAction {
    pub sandbox: String,
    pub action: String,
    pub reason: String,
}

/// Result produced by lifecycle reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LifecycleReconcileResult {
    pub dry_run: bool,
    pub stopped: Vec<String>,
    pub archived: Vec<String>,
    pub removed: Vec<String>,
    pub actions: Vec<LifecycleAction>,
}

/// Persisted sandbox state (saved to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub name: String,
    #[serde(default)]
    pub uuid: String,
    /// Docker image to use (e.g., "python:3.12-alpine")
    pub image: String,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub vsock_cid: u32,
    pub created_at: String,
    /// Backend type used to create this sandbox
    #[serde(default)]
    pub backend: Option<BackendType>,
    /// Remote resource identifier (K8s pod name or Nomad alloc ID)
    #[serde(default)]
    pub remote_id: Option<String>,
    /// Remote namespace (K8s namespace or Nomad namespace)
    #[serde(default)]
    pub remote_namespace: Option<String>,
    /// Provider-specific remote metadata used to reconnect and restore state.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub remote_metadata: HashMap<String, String>,
    /// Managed workspace revision for remote sync conflict detection.
    #[serde(default)]
    pub workspace_revision: Option<String>,
    /// Provider-resolved service endpoints for exposed ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<ResolvedEndpoint>,
    /// Local workspace path used for mount_cwd or managed remote sync.
    #[serde(default)]
    pub work_dir: Option<String>,
    /// Container-side workspace path (defaults to /workspace).
    #[serde(default)]
    pub container_work_dir: Option<String>,
    /// Managed host-side Git worktree used to isolate this sandbox's agent.
    #[serde(default)]
    pub git_worktree: Option<SandboxGitWorktree>,
    /// Original config file path used to create or start this sandbox.
    #[serde(default)]
    pub config_path: Option<String>,
    /// Server-derived tenant ownership used by LLM model governance.
    /// This is persisted at sandbox creation and cannot be supplied by the
    /// sandbox's network request payload.
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Time-to-live in seconds (None = no expiry)
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// When this sandbox expires (RFC3339). Computed from created_at + ttl_seconds.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Port mappings (host:container)
    #[serde(default)]
    pub ports: Vec<PortMapping>,
    /// Optional AgentKernel-managed Docker/Podman bridge configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_network: Option<crate::backend::ManagedNetworkConfig>,
    /// Whether SSH access is enabled
    #[serde(default)]
    pub ssh_enabled: bool,
    /// Host port mapped to sshd inside the sandbox
    #[serde(default)]
    pub ssh_host_port: Option<u16>,
    /// Volume mounts (slug:/path format)
    #[serde(default)]
    pub volumes: Vec<String>,
    /// Agent CLI to install on start (e.g., "claude", "gemini", "codex")
    #[serde(default)]
    pub agent: Option<String>,
    /// Secret bindings for proxy injection (raw CLI strings, e.g. "KEY:host")
    #[serde(default)]
    pub secret_bindings: Vec<String>,
    /// Secret keys to inject as files (e.g. ["OPENAI_API_KEY"])
    #[serde(default)]
    pub secret_files: Vec<String>,
    /// Use placeholder tokens instead of real secret values in file injection.
    /// Real values are substituted by the proxy in outbound traffic.
    #[serde(default)]
    pub placeholder_secrets: bool,
    /// Host port of the running proxy (if any)
    #[serde(default)]
    pub proxy_port: Option<u16>,
    /// Template secret mappings: env_var → target_host.
    /// Persisted so the UI can show expected secrets even when not yet configured.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub secret_mappings: HashMap<String, String>,
    /// Shell script to run inside the sandbox after start (from template init_script)
    #[serde(default)]
    pub init_script: Option<String>,
    /// Environment from a devcontainer file. Values are passed as argv to the
    /// backend and are never interpolated into log messages.
    #[serde(default)]
    pub environment: Vec<(String, String)>,
    /// Devcontainer postCreateCommand entries represented as argv vectors.
    #[serde(default)]
    pub post_create_commands: Vec<Vec<String>>,
    /// Whether all devcontainer postCreateCommand entries completed successfully.
    /// This remains false after a failure so the next start can retry them.
    #[serde(default)]
    pub post_create_completed: bool,
    /// Template name this sandbox was created from (if any).
    #[serde(default)]
    pub created_from_template: Option<String>,
    /// Human guidance text associated with the source template.
    #[serde(default)]
    pub template_help_text: Option<String>,
    /// User-defined labels for fleet management and filtering.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    /// Trusted tenant assigned by the authenticated API at creation time.
    /// Never populated from user-controlled labels or proxied request data.
    #[serde(default)]
    pub owner_org_id: Option<String>,
    /// Trusted owner assigned by the authenticated API at creation time.
    #[serde(default)]
    pub owner_user_id: Option<String>,
    /// User-defined description for the sandbox.
    #[serde(default)]
    pub description: Option<String>,
    /// Last observed sandbox activity (exec/file/attach/start), RFC3339.
    #[serde(default)]
    pub last_activity_at: Option<String>,
    /// Archive timestamp when sandbox is archived, RFC3339.
    #[serde(default)]
    pub archived_at: Option<String>,
    /// Human-readable archive reason.
    #[serde(default)]
    pub archived_reason: Option<String>,
    /// When this sandbox entered the dormant state, RFC3339.
    #[serde(default)]
    pub dormant_at: Option<String>,
    /// Human-readable reason for entering the dormant state.
    #[serde(default)]
    pub dormant_reason: Option<String>,
    /// Optional lifecycle automation policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_policy: Option<SandboxLifecyclePolicy>,
    /// Durable Firecracker full-state checkpoint used while this sandbox is paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_state_checkpoint: Option<String>,
    /// Published checkpoints awaiting best-effort deletion after consumption.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub full_state_cleanup_pending: Vec<String>,
    /// True after this sandbox has restored mutable disk state from a full-state checkpoint.
    /// This is dedicated metadata because user-editable labels cannot enforce safety.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub full_state_lineage: bool,
    /// Timestamp at which the running Firecracker VM was paused to zero compute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<String>,
    /// Source sandbox name when this sandbox was created by a full-state fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Opaque AgentKernel-owned writable Firecracker rootfs lineage. This is
    /// persisted separately from full-state checkpoint IDs so ordinary stop
    /// and start can retain guest filesystem changes without making a VM
    /// memory checkpoint cold-startable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firecracker_rootfs: Option<String>,
}

impl SandboxState {
    /// Render status from persisted archive state + runtime liveness.
    pub fn status(&self, running: bool) -> &'static str {
        if self.archived_at.is_some() {
            "archived"
        } else if self.dormant_at.is_some() {
            "dormant"
        } else if self.paused_at.is_some() {
            "paused"
        } else if running {
            "running"
        } else {
            "stopped"
        }
    }

    fn parse_rfc3339(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
    }

    fn last_activity_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.last_activity_at
            .as_deref()
            .and_then(Self::parse_rfc3339)
            .or_else(|| Self::parse_rfc3339(&self.created_at))
    }

    fn archived_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.archived_at.as_deref().and_then(Self::parse_rfc3339)
    }

    fn dormant_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.dormant_at.as_deref().and_then(Self::parse_rfc3339)
    }

    fn remote_context(&self) -> RemoteSandboxContext {
        RemoteSandboxContext {
            remote_id: self.remote_id.clone(),
            remote_namespace: self.remote_namespace.clone(),
            remote_metadata: self.remote_metadata.clone(),
            workspace_revision: self.workspace_revision.clone(),
            endpoints: self.endpoints.clone(),
            local_workspace: normalize_persisted_path(self.work_dir.clone())
                .ok()
                .flatten(),
            config_path: normalize_persisted_path(self.config_path.clone())
                .ok()
                .flatten(),
        }
    }
}

fn normalize_persisted_path(value: Option<String>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };

    let path = PathBuf::from(&value);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };

    Ok(Some(
        absolute
            .canonicalize()
            .unwrap_or(absolute)
            .to_string_lossy()
            .to_string(),
    ))
}

/// Status of a detached command
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DetachedStatus {
    Running,
    Completed,
    Failed,
}

/// A detached (background) command running in a sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachedCommand {
    /// Unique command ID
    pub id: String,
    /// Sandbox name
    pub sandbox: String,
    /// Original command
    pub command: Vec<String>,
    /// PID inside the container
    pub pid: u32,
    /// Current status
    pub status: DetachedStatus,
    /// Exit code (set when completed/failed)
    pub exit_code: Option<i32>,
    /// When the command was started (RFC3339)
    pub started_at: String,
}

/// VM Manager - manages sandboxes via unified Sandbox trait
///
/// Supports multiple backends:
/// - Firecracker microVMs (Linux with KVM)
/// - Docker/Podman containers
/// - Apple Containers (macOS 26+)
pub struct VmManager {
    /// Selected backend type
    backend: BackendType,
    /// Running sandboxes (unified interface)
    running: HashMap<String, Box<dyn Sandbox>>,
    /// Interrupted pause transitions retained after automatic recovery fails.
    /// Entries own either a possibly-live source runtime, safe completed
    /// staging metadata, or both. Only resume or explicit removal may consume
    /// them; normal exec paths must never expose these transitional states.
    pause_recovery: HashMap<String, PendingFullStateRecovery>,
    /// Running runtimes whose post-resume metadata write failed. A repeated
    /// resume retries only the atomic state publication; it never starts a
    /// second VM or destroys the retained runtime.
    resume_state_recovery: HashMap<String, SandboxState>,
    /// Persisted sandbox configurations
    sandboxes: HashMap<String, SandboxState>,
    /// Data directory for persistence
    data_dir: PathBuf,
    /// Optional explicit volume data root. Production resolves the standard
    /// home-backed location; tests can inject an isolated root.
    volume_base_dir: Option<PathBuf>,
    /// Test managers persist and validate lifecycle state without requiring a
    /// host container runtime.
    #[cfg(test)]
    bypass_backend_runtime: bool,
    /// Test-only backend injection for exercising lifecycle persistence
    /// without requiring a host Firecracker/KVM installation.
    #[cfg(test)]
    test_backend_factory:
        Option<Arc<dyn Fn(&str, BackendType) -> Result<Box<dyn Sandbox>> + Send + Sync>>,
    /// Test-only one-shot failure injection for the atomic state boundary.
    #[cfg(test)]
    fail_next_state_save: std::sync::atomic::AtomicBool,
    /// Rootfs directory for Firecracker
    rootfs_dir: Option<PathBuf>,
    /// Next vsock CID
    next_cid: u32,
    /// Detached commands tracked by ID
    detached: HashMap<String, DetachedCommand>,
    /// Enterprise policy engine (when enterprise feature is enabled)
    #[cfg(feature = "enterprise")]
    policy_engine: Option<Arc<crate::policy::PolicyEngine>>,
}

struct PendingFullStateRecovery {
    sandbox: Option<Box<dyn Sandbox>>,
    staging_path: PathBuf,
    completed_snapshot: Option<FullStateSnapshot>,
}

fn runtime_may_survive_failed_stop(error: &anyhow::Error, probe_running: bool) -> bool {
    error
        .downcast_ref::<FullStateTerminationError>()
        .map_or(probe_running, |failure| failure.process_may_be_running)
}

fn with_full_state_cleanup_intent(mut state: SandboxState, checkpoint_id: &str) -> SandboxState {
    if !state
        .full_state_cleanup_pending
        .iter()
        .any(|id| id == checkpoint_id)
    {
        state
            .full_state_cleanup_pending
            .push(checkpoint_id.to_string());
    }
    state
}

/// Escape a string for use inside a single-quoted shell command.
fn shell_escape(s: &str) -> String {
    // Replace ' with '\'' (end quote, escaped quote, start quote)
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn should_run_devcontainer_post_create(state: &SandboxState) -> bool {
    !state.post_create_completed && !state.post_create_commands.is_empty()
}

/// Run a command only when `names` is non-empty; returns `Err` (skip) when empty.
fn batch_cmd(names: &[String], cmd: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    if names.is_empty() {
        return Err(std::io::Error::other("empty"));
    }
    std::process::Command::new(cmd).args(args).output()
}

impl VmManager {
    /// Construct a manager backed by a test directory without probing host
    /// runtimes. This keeps quota accounting tests deterministic.
    #[cfg(test)]
    pub fn for_tests(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir.join("sandboxes"))?;
        Ok(Self {
            backend: BackendType::Docker,
            running: HashMap::new(),
            pause_recovery: HashMap::new(),
            resume_state_recovery: HashMap::new(),
            sandboxes: HashMap::new(),
            data_dir: data_dir.to_path_buf(),
            volume_base_dir: None,
            bypass_backend_runtime: true,
            test_backend_factory: None,
            fail_next_state_save: std::sync::atomic::AtomicBool::new(false),
            rootfs_dir: None,
            next_cid: 3,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine: None,
        })
    }

    #[cfg(test)]
    pub fn insert_state_for_tests(&mut self, state: SandboxState) {
        self.sandboxes.insert(state.name.clone(), state);
    }

    #[cfg(test)]
    pub fn set_volume_base_dir_for_tests(&mut self, volume_base_dir: PathBuf) {
        self.volume_base_dir = Some(volume_base_dir);
    }

    #[cfg(test)]
    pub fn set_backend_factory_for_tests(
        &mut self,
        factory: Arc<dyn Fn(&str, BackendType) -> Result<Box<dyn Sandbox>> + Send + Sync>,
    ) {
        self.test_backend_factory = Some(factory);
        self.bypass_backend_runtime = false;
    }

    #[cfg(test)]
    pub fn fail_next_state_save_for_tests(&self) {
        self.fail_next_state_save
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Create a new VM manager (auto-selects backend based on availability)
    pub fn new() -> Result<Self> {
        Self::with_backend(None)
    }

    /// Create a new VM manager with explicit backend selection
    ///
    /// If backend is None, auto-detects the best available backend.
    /// If backend is Some, uses the specified backend (fails if unavailable).
    pub fn with_backend(explicit_backend: Option<BackendType>) -> Result<Self> {
        let data_dir = Self::data_dir();
        let sandboxes_dir = data_dir.join("sandboxes");
        std::fs::create_dir_all(&sandboxes_dir)?;

        // Load existing sandboxes first so remote-only environments still have
        // a backend anchor when no local runtime is installed.
        let sandboxes = Self::load_sandboxes(&sandboxes_dir)?;

        // Reap only artifacts that no persisted sandbox can own. This keeps
        // the crash window between state publication and the rootfs marker
        // publication recoverable on the next manager start.
        if let Ok(store) = RootfsCowStore::open_default_without_reap() {
            let retained: std::collections::HashSet<String> = sandboxes
                .values()
                .filter_map(|state| state.firecracker_rootfs.clone())
                .collect();
            if let Err(error) = store.reap_stale_except(&retained) {
                eprintln!("[firecracker] rootfs COW cleanup scan failed: {error:#}");
            }
        }

        // Use explicit backend or auto-detect
        let backend = if let Some(b) = explicit_backend {
            // Verify the requested backend is available
            if !crate::backend::backend_available(b) {
                bail!("Backend '{}' is not available on this system", b);
            }
            b
        } else {
            detect_best_backend()
                .or_else(|| sandboxes.values().find_map(|state| state.backend))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No sandbox backend available. Need one of: KVM (Linux), Apple containers (macOS 26+), Docker/Podman, or a configured remote backend."
                    )
                })?
        };

        // Find rootfs path (only needed for Firecracker)
        let rootfs_dir = if backend == BackendType::Firecracker {
            // The access-controlled native KVM gate provides a prepared
            // rootfs file directly. Derive the managed runtime directory so
            // create() validates the same fixture that FirecrackerSandbox
            // will attach at start time.
            (cfg!(all(target_os = "linux", target_arch = "x86_64"))
                && std::env::var("AGENTKERNEL_KVM_SMOKE").as_deref() == Ok("1"))
            .then(|| std::env::var_os("AGENTKERNEL_KVM_ROOTFS").map(PathBuf::from))
            .flatten()
            .filter(|path| path.is_file())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| Self::find_images_dir().ok().map(|d| d.join("rootfs")))
        } else {
            None
        };

        // Find next available CID
        let max_cid = sandboxes.values().map(|s| s.vsock_cid).max().unwrap_or(2);

        // Reuse the enterprise policy engine for this working directory until
        // its config file changes.
        #[cfg(feature = "enterprise")]
        let policy_engine = cached_policy_engine();

        let mut manager = Self {
            backend,
            running: HashMap::new(),
            pause_recovery: HashMap::new(),
            resume_state_recovery: HashMap::new(),
            sandboxes,
            data_dir,
            volume_base_dir: None,
            #[cfg(test)]
            bypass_backend_runtime: false,
            #[cfg(test)]
            test_backend_factory: None,
            #[cfg(test)]
            fail_next_state_save: std::sync::atomic::AtomicBool::new(false),
            rootfs_dir,
            next_cid: max_cid + 1,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine,
        };

        // Detect already-running sandboxes
        manager.detect_running_sandboxes();
        manager.reconcile_full_state_cleanup();
        manager.report_full_state_recovery_state();

        crate::metrics::set_active_sandboxes(manager.sandboxes.len() as i64);

        Ok(manager)
    }

    /// Detect sandboxes that are already running (e.g., Docker containers).
    ///
    /// Uses batched queries (one call per backend) instead of per-sandbox calls
    /// to avoid O(N) subprocess overhead on startup.
    fn detect_running_sandboxes(&mut self) {
        use std::collections::HashSet;

        // Collect sandbox names grouped by backend type
        let mut docker_names: Vec<String> = Vec::new();
        let mut podman_names: Vec<String> = Vec::new();
        let mut k8s_names: Vec<String> = Vec::new();
        let mut nomad_names: Vec<String> = Vec::new();
        let mut apple_names: Vec<String> = Vec::new();

        for (name, state) in &self.sandboxes {
            match state.backend.unwrap_or(self.backend) {
                BackendType::Docker => docker_names.push(name.clone()),
                BackendType::Podman => podman_names.push(name.clone()),
                BackendType::Kubernetes => k8s_names.push(name.clone()),
                BackendType::Nomad => nomad_names.push(name.clone()),
                BackendType::Apple => apple_names.push(name.clone()),
                _ => {}
            }
        }

        let mut running_set: HashSet<String> = HashSet::new();

        // Helper: match sandbox names against a set of active prefixed names
        let match_active =
            |names: &[String], active: &HashSet<&str>, running: &mut HashSet<String>| {
                for name in names {
                    if active.contains(format!("agentkernel-{}", name).as_str()) {
                        running.insert(name.clone());
                    }
                }
            };

        // Batch Docker: one `docker ps` call for all Docker sandboxes
        if let Ok(output) = batch_cmd(&docker_names, "docker", &["ps", "--format", "{{.Names}}"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let active: HashSet<&str> = stdout.lines().collect();
            match_active(&docker_names, &active, &mut running_set);
        }

        // Batch Podman: one `podman ps` call
        if let Ok(output) = batch_cmd(&podman_names, "podman", &["ps", "--format", "{{.Names}}"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let active: HashSet<&str> = stdout.lines().collect();
            match_active(&podman_names, &active, &mut running_set);
        }

        // Batch Kubernetes: one `kubectl get pods` call
        if let Ok(output) = batch_cmd(
            &k8s_names,
            "kubectl",
            &[
                "get",
                "pods",
                "-n",
                "agentkernel",
                "--field-selector=status.phase=Running",
                "-o",
                "jsonpath={.items[*].metadata.name}",
            ],
        ) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let active: HashSet<&str> = stdout.split_whitespace().collect();
            match_active(&k8s_names, &active, &mut running_set);
        }

        // Batch Nomad: one `nomad job status` call
        if let Ok(output) = batch_cmd(&nomad_names, "nomad", &["job", "status", "-short"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let active: HashSet<&str> = stdout
                .lines()
                .filter(|line| line.contains("running"))
                .filter_map(|line| line.split_whitespace().next())
                .collect();
            match_active(&nomad_names, &active, &mut running_set);
        }

        // Batch Apple: one `container ls` call for all Apple sandboxes
        if let Ok(output) = batch_cmd(&apple_names, "container", &["ls"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for name in &apple_names {
                if stdout.contains(&format!("agentkernel-{}", name)) {
                    running_set.insert(name.clone());
                }
            }
        }

        // Create sandbox objects for running sandboxes
        for name in running_set {
            let Some(state) = self.sandboxes.get(&name) else {
                continue;
            };
            let backend = state.backend.unwrap_or(self.backend);
            if let Ok(mut sandbox) = create_sandbox_with_state(
                backend,
                &name,
                &crate::config::OrchestratorConfig::default(),
                backend.is_remote().then(|| state.remote_context()),
            ) {
                if let Some(network) = state.managed_network.as_ref()
                    && let Err(error) = sandbox.restore_managed_network(network)
                {
                    eprintln!(
                        "[vmm] warning: failed to restore network lease for '{}': {}",
                        name, error
                    );
                    continue;
                }
                self.running.insert(name, sandbox);
            }
        }
    }

    /// Report interrupted pause transitions without deleting artifacts that
    /// could still belong to another live manager. Deterministic staging names
    /// let operators correlate a paused sandbox with its recovery directory.
    fn report_full_state_recovery_state(&self) {
        let Ok(store) = FullStateCheckpointStore::new(&self.data_dir) else {
            return;
        };
        let referenced: std::collections::HashSet<String> = self
            .sandboxes
            .values()
            .filter(|state| state.paused_at.is_some())
            .filter_map(|state| state.full_state_checkpoint.clone())
            .collect();

        if let Ok(entries) = store.staging_entries() {
            for (id, path) in entries {
                if id.as_ref().is_some_and(|id| referenced.contains(id)) {
                    let recovery_ready = id
                        .as_deref()
                        .is_some_and(|id| store.recovery_is_ready(id).unwrap_or(false));
                    if recovery_ready {
                        eprintln!(
                            "Warning: interrupted full-state pause retained recovery-ready staging at {}; the next resume or fork will publish it",
                            path.display()
                        );
                    } else {
                        eprintln!(
                            "Warning: interrupted full-state pause retained diagnostic staging at {}; automatic restore is unavailable until the transition is reconciled",
                            path.display()
                        );
                    }
                } else {
                    eprintln!(
                        "Warning: orphaned full-state checkpoint staging retained at {}; it was not deleted because another manager may still own the transition",
                        path.display()
                    );
                }
            }
        }

        for id in referenced {
            let published = store.contains(&id).unwrap_or(false);
            let staged = store.staging_path(&id).is_ok_and(|path| path.is_dir());
            if !published && !staged {
                eprintln!(
                    "Warning: paused sandbox references missing full-state checkpoint '{id}'"
                );
            }
        }
    }

    fn delete_full_state_artifacts(
        store: &FullStateCheckpointStore,
        checkpoint_id: &str,
    ) -> Result<()> {
        store.delete(checkpoint_id)?;
        let staging_path = store.staging_path(checkpoint_id)?;
        store.discard_staging(&staging_path)
    }

    /// Retry durable deletion intents left after a checkpoint was consumed.
    /// The intent is cleared only after both published and staging locations
    /// are absent and the updated sandbox state is atomically persisted.
    fn reconcile_full_state_cleanup(&mut self) {
        let Ok(store) = FullStateCheckpointStore::new(&self.data_dir) else {
            return;
        };
        let names: Vec<String> = self.sandboxes.keys().cloned().collect();
        for name in names {
            let Some(current) = self.sandboxes.get(&name).cloned() else {
                continue;
            };
            if current.full_state_cleanup_pending.is_empty() {
                continue;
            }
            let mut remaining = Vec::new();
            for checkpoint_id in &current.full_state_cleanup_pending {
                if let Err(error) = Self::delete_full_state_artifacts(&store, checkpoint_id) {
                    eprintln!(
                        "Warning: failed to finish checkpoint cleanup '{}' for sandbox '{}': {error:#}",
                        checkpoint_id, name
                    );
                    remaining.push(checkpoint_id.clone());
                }
            }
            if remaining == current.full_state_cleanup_pending {
                continue;
            }
            let mut updated = current;
            updated.full_state_cleanup_pending = remaining;
            if let Err(error) = self.save_sandbox(&updated) {
                eprintln!(
                    "Warning: checkpoint artifacts were deleted for sandbox '{}', but its cleanup intent could not be cleared: {error:#}",
                    name
                );
                continue;
            }
            self.sandboxes.insert(name, updated);
        }
    }

    /// Get the data directory
    fn data_dir() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".local/share/agentkernel")
        } else {
            PathBuf::from("/tmp/agentkernel")
        }
    }

    /// Load sandboxes from disk
    fn load_sandboxes(sandboxes_dir: &Path) -> Result<HashMap<String, SandboxState>> {
        let mut sandboxes = HashMap::new();

        if sandboxes_dir.exists() {
            for entry in std::fs::read_dir(sandboxes_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json")
                    && let Ok(content) = std::fs::read_to_string(&path)
                    && let Ok(mut state) = serde_json::from_str::<SandboxState>(&content)
                {
                    // Backfill UUIDs for pre-UUID sandbox state files.
                    if state.uuid.is_empty() {
                        state.uuid = uuid::Uuid::now_v7().to_string();
                        match serde_json::to_string_pretty(&state)
                            .map_err(anyhow::Error::from)
                            .and_then(|updated| {
                                std::fs::write(&path, updated).map_err(anyhow::Error::from)
                            }) {
                            Ok(()) => {}
                            Err(e) => {
                                eprintln!(
                                    "[vmm] warning: failed to backfill UUID for {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                    sandboxes.insert(state.name.clone(), state);
                }
            }
        }

        Ok(sandboxes)
    }

    /// Save a sandbox state to disk
    fn save_sandbox(&self, state: &SandboxState) -> Result<()> {
        #[cfg(test)]
        if self
            .fail_next_state_save
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            bail!("simulated sandbox state publication failure");
        }
        let directory = self.data_dir.join("sandboxes");
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.json", state.name));
        let content = serde_json::to_vec_pretty(state)?;
        let mut staging =
            tempfile::NamedTempFile::new_in(&directory).context("failed to stage sandbox state")?;
        std::io::Write::write_all(staging.as_file_mut(), &content)?;
        staging.as_file().sync_all()?;
        staging
            .persist(&path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to publish sandbox state {}", path.display()))?;
        // The state is atomically visible after rename. A directory fsync
        // failure is advisory: reporting it as a failed write would cause
        // callers to roll back after the new state is already published.
        if let Err(error) = std::fs::File::open(&directory).and_then(|dir| dir.sync_all()) {
            eprintln!("Warning: failed to sync sandbox state directory: {error}");
        }
        Ok(())
    }

    /// Delete a sandbox state from disk
    fn delete_sandbox(&self, name: &str) -> Result<()> {
        let directory = self.data_dir.join("sandboxes");
        let path = directory.join(format!("{}.json", name));
        if path.exists() {
            std::fs::remove_file(path)?;
            if let Err(error) = std::fs::File::open(&directory).and_then(|dir| dir.sync_all()) {
                eprintln!("Warning: failed to sync sandbox state deletion: {error}");
            }
        }
        Ok(())
    }

    fn apply_runtime_metadata(state: &mut SandboxState, metadata: &SandboxRuntimeMetadata) {
        state.remote_id = metadata.remote_id.clone();
        state.remote_namespace = metadata.remote_namespace.clone();
        state.remote_metadata = metadata.remote_metadata.clone();
        state.workspace_revision = metadata.workspace_revision.clone();
        state.endpoints = metadata.endpoints.clone();
    }

    fn sync_runtime_metadata(&mut self, name: &str) -> Result<()> {
        let metadata = self
            .running
            .get(name)
            .and_then(|sandbox| sandbox.runtime_metadata());

        if let Some(metadata) = metadata {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            Self::apply_runtime_metadata(state, &metadata);
            let snapshot = state.clone();
            self.save_sandbox(&snapshot)?;
        }

        Ok(())
    }

    fn hydrate_remote_runtime(&mut self, name: &str) -> Result<()> {
        if self.running.contains_key(name) {
            return Ok(());
        }

        let Some(state) = self.sandboxes.get(name).cloned() else {
            bail!("Sandbox '{}' not found", name);
        };

        let backend = state.backend.unwrap_or(self.backend);
        if !backend.is_remote() {
            return Ok(());
        }

        let running = state
            .remote_metadata
            .get("last_known_status")
            .is_some_and(|value| value == "running");
        if !running {
            return Ok(());
        }

        let sandbox = create_sandbox_with_state(
            backend,
            name,
            &crate::config::OrchestratorConfig::default(),
            Some(state.remote_context()),
        )?;
        self.running.insert(name.to_string(), sandbox);
        Ok(())
    }

    /// Find the images directory
    fn find_images_dir() -> Result<PathBuf> {
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = PathBuf::from(home).join(".local/share/agentkernel/images");
            if home_path.join("kernel").exists() || home_path.join("rootfs").exists() {
                return Ok(home_path);
            }
        }

        let paths = [PathBuf::from("images"), PathBuf::from("../images")];
        for path in &paths {
            if path.join("kernel").exists() || path.join("rootfs").exists() {
                return Ok(path.clone());
            }
        }

        bail!("Images directory not found. Run 'agentkernel setup' first.")
    }

    /// Get rootfs path for a runtime (Firecracker only)
    pub fn rootfs_path(&self, runtime: &str) -> Result<PathBuf> {
        validation::validate_runtime(runtime)?;

        // The dedicated native gate supplies one exact, immutable fixture.
        // Keep this override opt-in and host-gated so an operator's stale
        // environment cannot change normal production image selection.
        if cfg!(all(target_os = "linux", target_arch = "x86_64"))
            && std::env::var("AGENTKERNEL_KVM_SMOKE").as_deref() == Ok("1")
            && let Some(path) = std::env::var_os("AGENTKERNEL_KVM_ROOTFS").map(PathBuf::from)
        {
            if !path.is_file() {
                bail!(
                    "AGENTKERNEL_KVM_ROOTFS does not point to a regular file: {}",
                    path.display()
                );
            }
            return Ok(path);
        }

        let rootfs_dir = self
            .rootfs_dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Rootfs directory not configured"))?;
        let path = rootfs_dir.join(format!("{}.ext4", runtime));
        if !path.exists() {
            bail!(
                "Rootfs not found: {}. Run 'agentkernel setup' first.",
                path.display()
            );
        }
        Ok(path)
    }

    /// Check enterprise policy for an action on a sandbox.
    ///
    /// Returns Ok(()) if the action is permitted (or no policy engine is active).
    /// Returns an error if the action is denied by enterprise policy.
    #[cfg(feature = "enterprise")]
    async fn check_enterprise_policy(
        &self,
        action: crate::policy::Action,
        sandbox_name: &str,
        agent_type: &str,
        runtime: &str,
    ) -> Result<()> {
        if let Some(ref engine) = self.policy_engine {
            // Build a default principal from environment
            let principal = crate::policy::Principal {
                id: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
                email: std::env::var("USER")
                    .map(|u| format!("{}@local", u))
                    .unwrap_or_else(|_| "unknown@local".to_string()),
                org_id: "local".to_string(),
                roles: vec!["developer".to_string()],
                teams: Vec::new(),
                mfa_verified: false,
            };

            let resource = crate::policy::Resource {
                name: sandbox_name.to_string(),
                agent_type: agent_type.to_string(),
                runtime: runtime.to_string(),
            };

            let decision = engine.evaluate(&principal, action, &resource).await;
            if !decision.is_permit() {
                bail!(
                    "Enterprise policy denied action '{}' on sandbox '{}': {}",
                    action,
                    sandbox_name,
                    decision.reason
                );
            }
        }
        Ok(())
    }

    /// Create a new sandbox (persisted to disk)
    pub async fn create(
        &mut self,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<()> {
        self.create_with_options(name, image, vcpus, memory_mb, None, Vec::new())
            .await
    }

    /// Create a new sandbox with an explicit backend, without mutating the
    /// manager's default backend for future operations.
    pub async fn create_with_backend(
        &mut self,
        backend: BackendType,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<()> {
        self.create_with_backend_options(
            backend,
            name,
            image,
            vcpus,
            memory_mb,
            None,
            Vec::new(),
            None,
        )
        .await
    }

    /// Create a sandbox on an explicit backend while preserving the same
    /// creation options as the automatic-backend path.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_with_backend_options(
        &mut self,
        backend: BackendType,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
        ttl_seconds: Option<u64>,
        ports: Vec<PortMapping>,
        agent: Option<String>,
    ) -> Result<()> {
        self.create_internal(
            backend,
            name,
            image,
            vcpus,
            memory_mb,
            ttl_seconds,
            ports,
            agent,
        )
        .await
    }

    /// Create a new sandbox with an optional TTL
    pub async fn create_with_ttl(
        &mut self,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
        ttl_seconds: Option<u64>,
    ) -> Result<()> {
        self.create_with_options(name, image, vcpus, memory_mb, ttl_seconds, Vec::new())
            .await
    }

    /// Create a new sandbox with TTL, port mappings, and optional agent
    pub async fn create_with_options(
        &mut self,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
        ttl_seconds: Option<u64>,
        ports: Vec<PortMapping>,
    ) -> Result<()> {
        self.create_with_agent(name, image, vcpus, memory_mb, ttl_seconds, ports, None)
            .await
    }

    /// Create a new sandbox with TTL, port mappings, and optional agent CLI
    #[allow(clippy::too_many_arguments)]
    pub async fn create_with_agent(
        &mut self,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
        ttl_seconds: Option<u64>,
        ports: Vec<PortMapping>,
        agent: Option<String>,
    ) -> Result<()> {
        self.create_with_backend_options(
            self.backend,
            name,
            image,
            vcpus,
            memory_mb,
            ttl_seconds,
            ports,
            agent,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_internal(
        &mut self,
        backend: BackendType,
        name: &str,
        image: &str,
        vcpus: u32,
        memory_mb: u64,
        ttl_seconds: Option<u64>,
        ports: Vec<PortMapping>,
        agent: Option<String>,
    ) -> Result<()> {
        let create_start = std::time::Instant::now();

        let backend_available = crate::backend::backend_available(backend);
        #[cfg(test)]
        let backend_available = backend_available || self.bypass_backend_runtime;
        if !backend_available {
            bail!("Backend '{}' is not available on this system", backend);
        }

        if self.sandboxes.contains_key(name) {
            bail!("Sandbox '{}' already exists", name);
        }

        // Enterprise policy check
        #[cfg(feature = "enterprise")]
        self.check_enterprise_policy(
            crate::policy::Action::Create,
            name,
            "unknown",
            crate::languages::docker_image_to_firecracker_runtime(image),
        )
        .await?;

        // For Firecracker, convert Docker image names to runtime names
        let effective_image = if backend == BackendType::Firecracker {
            let runtime = docker_image_to_firecracker_runtime(image);
            self.rootfs_path(runtime)?;
            runtime.to_string()
        } else {
            image.to_string()
        };

        let vsock_cid = self.next_cid;
        self.next_cid += 1;

        let created = chrono::Utc::now();
        let created_at = created.to_rfc3339();
        let expires_at =
            ttl_seconds.map(|ttl| (created + chrono::Duration::seconds(ttl as i64)).to_rfc3339());

        let state = SandboxState {
            name: name.to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: effective_image.clone(),
            vcpus,
            memory_mb,
            vsock_cid,
            created_at: created_at.clone(),
            backend: Some(backend),
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            tenant_id: None,
            ttl_seconds,
            expires_at,
            ports,
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: Some(created_at),
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);

        log_event(AuditEvent::SandboxCreated {
            name: name.to_string(),
            image: effective_image,
            backend: backend.to_string(),
            labels: self
                .sandboxes
                .get(name)
                .map(|s| s.labels.clone())
                .unwrap_or_default(),
        });
        crate::metrics::record_sandbox_lifecycle(
            "created",
            &backend.to_string(),
            create_start.elapsed().as_secs_f64(),
        );
        crate::metrics::inc_active_sandboxes();

        Ok(())
    }

    /// Set SSH enabled state for a sandbox
    pub fn set_ssh_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.ssh_enabled = enabled;
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    pub fn set_work_dir(&mut self, name: &str, work_dir: Option<String>) -> Result<()> {
        let work_dir = normalize_persisted_path(work_dir)?;
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.work_dir = work_dir;
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Persist an optional Docker/Podman-managed bridge configuration.
    pub fn set_managed_network(
        &mut self,
        name: &str,
        managed_network: Option<crate::backend::ManagedNetworkConfig>,
    ) -> Result<()> {
        if let Some(config) = managed_network.as_ref() {
            config.validate()?;
        }
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.managed_network = managed_network;
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Set the container-side workspace path for devcontainer mounts.
    pub fn set_container_work_dir(&mut self, name: &str, work_dir: Option<String>) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.container_work_dir = work_dir;
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Set environment entries originating from a devcontainer file. Values
    /// are persisted for subsequent `sandbox start` operations.
    pub fn set_environment(&mut self, name: &str, environment: &[(String, String)]) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.environment = environment.to_vec();
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Set argv-safe devcontainer post-create commands.
    pub fn set_post_create_commands(&mut self, name: &str, commands: &[Vec<String>]) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.post_create_commands = commands.to_vec();
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Create (or return) the dedicated Git checkout for a sandbox.
    ///
    /// This is intentionally explicit: existing callers that mount a host
    /// directory continue to do so unchanged. The generated checkout path is
    /// owned by AgentKernel and is persisted with the sandbox so restarts and
    /// repeated setup calls are idempotent.
    pub fn create_git_worktree(&mut self, name: &str, repository: &Path) -> Result<String> {
        let state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

        if let Some(existing) = state.git_worktree.as_ref() {
            let existing_path = PathBuf::from(&existing.path);
            if existing_path.exists() {
                let recorded = crate::git_worktree::ManagedWorktree {
                    repository: PathBuf::from(&existing.repository),
                    path: existing_path,
                    branch: existing.branch.clone(),
                    base_ref: existing.base_ref.clone(),
                };
                if crate::git_worktree::same_repository(repository, &recorded.repository)? {
                    crate::git_worktree::verify(&recorded, &self.data_dir.join("worktrees"))?;
                    return Ok(existing.path.clone());
                }
                bail!(
                    "Sandbox '{}' already has a Git worktree for '{}', not '{}'; refusing to replace it",
                    name,
                    existing.repository,
                    repository.display()
                );
            }
            bail!(
                "Sandbox '{}' records missing Git worktree '{}'; refusing to recreate it automatically",
                name,
                existing.path
            );
        }

        let managed = crate::git_worktree::create(
            repository,
            &self.data_dir.join("worktrees"),
            name,
            &state.uuid,
        )?;
        let metadata = SandboxGitWorktree {
            repository: managed.repository.to_string_lossy().into_owned(),
            path: managed.path.to_string_lossy().into_owned(),
            branch: managed.branch,
            base_ref: managed.base_ref,
        };
        let mut updated = state.clone();
        updated.work_dir = Some(metadata.path.clone());
        updated.git_worktree = Some(metadata);
        if let Err(error) = self.save_sandbox(&updated) {
            let cleanup = crate::git_worktree::remove(
                &crate::git_worktree::ManagedWorktree {
                    repository: PathBuf::from(&updated.git_worktree.as_ref().unwrap().repository),
                    path: PathBuf::from(&updated.git_worktree.as_ref().unwrap().path),
                    branch: updated.git_worktree.as_ref().unwrap().branch.clone(),
                    base_ref: updated.git_worktree.as_ref().unwrap().base_ref.clone(),
                },
                &self.data_dir.join("worktrees"),
            );
            self.sandboxes.insert(name.to_string(), state);
            if let Err(cleanup_error) = cleanup {
                return Err(error).context(format!(
                    "failed to persist Git worktree metadata; cleanup also failed: {cleanup_error}"
                ));
            }
            return Err(error).context("failed to persist Git worktree metadata");
        }
        self.sandboxes.insert(name.to_string(), updated);
        Ok(managed.path.to_string_lossy().into_owned())
    }

    fn remove_git_worktree(&self, state: &SandboxState) -> Result<()> {
        let Some(metadata) = state.git_worktree.as_ref() else {
            return Ok(());
        };
        crate::git_worktree::remove(
            &crate::git_worktree::ManagedWorktree {
                repository: PathBuf::from(&metadata.repository),
                path: PathBuf::from(&metadata.path),
                branch: metadata.branch.clone(),
                base_ref: metadata.base_ref.clone(),
            },
            &self.data_dir.join("worktrees"),
        )
    }

    fn ensure_clean_git_worktree(&self, state: &SandboxState) -> Result<()> {
        let Some(metadata) = state.git_worktree.as_ref() else {
            return Ok(());
        };
        crate::git_worktree::ensure_clean_removable(
            &crate::git_worktree::ManagedWorktree {
                repository: PathBuf::from(&metadata.repository),
                path: PathBuf::from(&metadata.path),
                branch: metadata.branch.clone(),
                base_ref: metadata.base_ref.clone(),
            },
            &self.data_dir.join("worktrees"),
        )
    }

    pub fn set_config_path(&mut self, name: &str, config_path: Option<String>) -> Result<()> {
        let config_path = normalize_persisted_path(config_path)?;
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.config_path = config_path;
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Persist the trusted tenant identity associated with a sandbox.
    pub fn set_tenant_id(&mut self, name: &str, tenant_id: Option<String>) -> Result<()> {
        let tenant_id = tenant_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.tenant_id = tenant_id;
        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Persist template secret mappings (env_var → host) so the UI can show
    /// which secrets a template expects, even before they are configured.
    pub fn set_secret_mappings(
        &mut self,
        name: &str,
        mappings: &HashMap<String, String>,
    ) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.secret_mappings = mappings.clone();
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Set user-defined labels on a sandbox for fleet management and filtering.
    pub fn set_labels(&mut self, name: &str, labels: &HashMap<String, String>) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.labels = labels.clone();
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
        Ok(())
    }

    /// Persist trusted ownership metadata supplied by the authenticated API.
    pub fn set_owner_identity(&mut self, name: &str, tenant: &str, user: &str) -> Result<()> {
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.owner_org_id = Some(tenant.to_string());
        state.owner_user_id = Some(user.to_string());
        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Atomically persist the trusted tenant and owner established by an
    /// authenticated first-start claim, then publish it in memory.
    pub fn set_trusted_ownership(
        &mut self,
        name: &str,
        tenant_id: Option<String>,
        user_id: Option<&str>,
        org_id: Option<&str>,
    ) -> Result<()> {
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.tenant_id = tenant_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        state.owner_user_id = user_id.map(ToString::to_string);
        state.owner_org_id = org_id.map(ToString::to_string);
        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Set user-defined description on a sandbox.
    pub fn set_description(&mut self, name: &str, description: Option<&str>) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.description = description.map(String::from);
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
        Ok(())
    }

    /// Restore immutable identity/creation metadata after sandbox recreation.
    ///
    /// Used by resize fallback paths to avoid changing externally visible
    /// sandbox identity and historical timestamps.
    pub fn set_identity_metadata(
        &mut self,
        name: &str,
        uuid: &str,
        created_at: &str,
        expires_at: Option<&str>,
    ) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.uuid = uuid.to_string();
        state.created_at = created_at.to_string();
        state.expires_at = expires_at.map(ToString::to_string);
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
        Ok(())
    }

    /// Persist the authenticated tenant owner for enterprise quota accounting.
    #[cfg(feature = "enterprise")]
    pub fn set_owner_metadata(
        &mut self,
        name: &str,
        user_id: Option<&str>,
        org_id: Option<&str>,
    ) -> Result<()> {
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.owner_user_id = user_id.map(ToString::to_string);
        state.owner_org_id = org_id.map(ToString::to_string);
        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Set volume mount specs for a sandbox (slug:/path or slug:/path:ro).
    pub fn set_volumes(&mut self, name: &str, volumes: &[String]) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.volumes = volumes.to_vec();
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
        Ok(())
    }

    /// Set lifecycle automation policy for a sandbox.
    pub fn set_lifecycle_policy(
        &mut self,
        name: &str,
        policy: Option<SandboxLifecyclePolicy>,
    ) -> Result<()> {
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.lifecycle_policy = policy;
        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Mark sandbox activity and persist the updated timestamp.
    pub fn touch_activity(&mut self, name: &str) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
        Ok(())
    }

    /// Mark a stopped sandbox dormant after a configured period of disuse.
    pub fn mark_dormant(&mut self, name: &str, at: &str, reason: &str) -> Result<()> {
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        if state.dormant_at.is_none() {
            state.dormant_at = Some(at.to_string());
            state.dormant_reason = Some(reason.to_string());
            self.save_sandbox(&state)?;
            self.sandboxes.insert(name.to_string(), state);
        }
        Ok(())
    }

    /// Return the last activity timestamp for scheduler evaluation.
    pub fn activity_time(&self, name: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        self.sandboxes
            .get(name)
            .and_then(SandboxState::last_activity_time)
    }

    /// Return the dormant timestamp for scheduler evaluation.
    pub fn dormant_time(&self, name: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        self.sandboxes
            .get(name)
            .and_then(SandboxState::dormant_time)
    }

    /// Return all persisted sandbox names in stable order.
    pub fn sandbox_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.sandboxes.keys().cloned().collect();
        names.sort();
        names
    }

    /// Set secret bindings for a sandbox (raw CLI strings for proxy injection).
    pub fn set_secret_bindings(&mut self, name: &str, bindings: &[String]) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.secret_bindings = bindings.to_vec();
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Set secret file keys for a sandbox (injected as files on start).
    pub fn set_secret_files(&mut self, name: &str, keys: &[String]) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.secret_files = keys.to_vec();
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    pub fn set_placeholder_secrets(&mut self, name: &str, enabled: bool) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.placeholder_secrets = enabled;
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Mark a sandbox so its next remote start restores from a provider-backed
    /// snapshot handle.
    pub fn set_remote_restore_snapshot(&mut self, name: &str, snapshot_handle: &str) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state
                .remote_metadata
                .insert("restore_snapshot".to_string(), snapshot_handle.to_string());
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Inject secrets as placeholder tokens and start a proxy for substitution.
    async fn inject_placeholder_secrets(
        &mut self,
        sandbox: &mut dyn crate::backend::Sandbox,
        name: &str,
        resolved: &HashMap<String, String>,
        backend: BackendType,
        model_governance: Option<&crate::model_governance::ModelGovernancePolicy>,
    ) -> Result<()> {
        match crate::vsock_secrets::inject_secrets_as_placeholders(
            sandbox,
            crate::vsock_secrets::DEFAULT_SECRETS_PATH,
            resolved,
        )
        .await
        {
            Ok((injected, placeholder_map)) => {
                eprintln!(
                    "Injected {} placeholder secret file(s) at {} (real values never enter VM)",
                    injected.len(),
                    crate::vsock_secrets::DEFAULT_SECRETS_PATH,
                );
                if !placeholder_map.is_empty() {
                    let proxy_config = crate::proxy::ProxyConfig {
                        listen_addr: "0.0.0.0:0".parse().unwrap(),
                        bindings: Vec::new(),
                        allowed_hosts: Vec::new(),
                        blocked_hosts: Vec::new(),
                        allowlist_only: false,
                        sandbox_name: name.to_string(),
                        hooks: Vec::new(),
                        llm_intercept: true,
                        llm_domains: Vec::new(),
                        org_managed_domains: Vec::new(),
                        llm_governance: model_governance.cloned(),
                    };
                    match crate::proxy::start_proxy(proxy_config, HashMap::new(), placeholder_map)
                        .await
                    {
                        Ok(handle) => {
                            let proxy_addr = handle.addr;
                            let proxy_host = match backend {
                                BackendType::Apple => {
                                    format!("192.168.64.1:{}", proxy_addr.port())
                                }
                                BackendType::Docker | BackendType::Podman => {
                                    if cfg!(target_os = "macos") {
                                        format!("host.docker.internal:{}", proxy_addr.port())
                                    } else {
                                        format!("172.17.0.1:{}", proxy_addr.port())
                                    }
                                }
                                _ => format!("127.0.0.1:{}", proxy_addr.port()),
                            };

                            // Inject proxy env vars into sandbox
                            let ca_pem = handle.ca_cert_pem.clone();
                            sandbox
                                .inject_files(&[crate::backend::FileInjection {
                                    dest: "/usr/local/share/ca-certificates/agentkernel-proxy.crt"
                                        .to_string(),
                                    content: ca_pem.into_bytes(),
                                }])
                                .await
                                .ok();

                            // Set proxy env vars via profile script
                            let proxy_script = format!(
                                "export HTTP_PROXY=http://{h}\nexport HTTPS_PROXY=http://{h}\nexport http_proxy=http://{h}\nexport https_proxy=http://{h}\nexport NO_PROXY=localhost,127.0.0.1\n",
                                h = proxy_host
                            );
                            sandbox
                                .inject_files(&[crate::backend::FileInjection {
                                    dest: "/etc/profile.d/agentkernel-proxy.sh".to_string(),
                                    content: proxy_script.into_bytes(),
                                }])
                                .await
                                .ok();

                            if let Some(s) = self.sandboxes.get_mut(name) {
                                s.proxy_port = Some(proxy_addr.port());
                            }
                            self.save_sandbox(self.sandboxes.get(name).unwrap())?;

                            eprintln!(
                                "Placeholder proxy started on port {} for secret substitution",
                                proxy_addr.port()
                            );
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to start placeholder proxy: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to inject placeholder secret files: {}", e);
            }
        }
        Ok(())
    }

    /// Set an init script to run inside the sandbox after start.
    pub fn set_init_script(&mut self, name: &str, script: &str) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.init_script = Some(script.to_string());
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Set optional template metadata for a sandbox.
    pub fn set_template_metadata(
        &mut self,
        name: &str,
        created_from_template: Option<&str>,
        template_help_text: Option<&str>,
    ) -> Result<()> {
        {
            let state = self
                .sandboxes
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
            state.created_from_template = created_from_template
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            state.template_help_text = template_help_text
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
        }
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;
        Ok(())
    }

    /// Extend a sandbox's time-to-live by additional seconds.
    /// Returns the new expiry time in RFC3339 format, or None if TTL is disabled.
    pub fn extend_ttl(&mut self, name: &str, additional_secs: u64) -> Result<Option<String>> {
        use chrono::{DateTime, Duration, Utc};

        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

        // Calculate new expiry based on current state
        let now = Utc::now();
        let base_time = if let Some(ref expires_at) = state.expires_at {
            // Extend from current expiry (if not already expired)
            expires_at
                .parse::<DateTime<Utc>>()
                .ok()
                .filter(|exp| *exp > now)
                .unwrap_or(now)
        } else {
            // No current expiry, extend from now
            now
        };

        let new_exp = base_time + Duration::seconds(additional_secs as i64);
        let new_expiry_str = new_exp.to_rfc3339();

        state.expires_at = Some(new_expiry_str.clone());
        if let Ok(created) = state.created_at.parse::<DateTime<Utc>>() {
            let total_secs = (new_exp - created).num_seconds();
            if total_secs > 0 {
                state.ttl_seconds = Some(total_secs as u64);
            }
        }

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);

        Ok(Some(new_expiry_str))
    }

    /// Recover an archived sandbox back to a normal lifecycle state.
    pub fn recover(&mut self, name: &str) -> Result<()> {
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

        state.archived_at = None;
        state.archived_reason = None;
        state.dormant_at = None;
        state.dormant_reason = None;
        state.last_activity_at = Some(chrono::Utc::now().to_rfc3339());

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Start a sandbox
    pub async fn start(&mut self, name: &str) -> Result<()> {
        self.start_with_permissions(name, &Permissions::default())
            .await
    }

    fn ensure_unique_firecracker_rootfs_reference(
        &self,
        name: &str,
        reference: Option<&str>,
    ) -> Result<()> {
        let Some(reference) = reference else {
            return Ok(());
        };
        if let Some((other_name, _)) = self.sandboxes.iter().find(|(other_name, state)| {
            other_name.as_str() != name && state.firecracker_rootfs.as_deref() == Some(reference)
        }) {
            bail!(
                "Firecracker writable rootfs lineage '{}' is referenced by both '{}' and '{}'; refusing destructive or mutable access",
                reference,
                name,
                other_name
            );
        }
        Ok(())
    }

    /// Resolve model governance from trusted server configuration and the
    /// sandbox's persisted ownership. Request payloads cannot select a tenant.
    fn model_governance_for_state(
        state: &SandboxState,
    ) -> Result<Option<crate::model_governance::ModelGovernancePolicy>> {
        // A persisted path is authoritative. Falling back to the process
        // working directory after that path disappears could silently switch
        // the tenant policy (or disable governance) between starts.
        let configured_path = state.config_path.as_ref().map(PathBuf::from);
        let path = configured_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("agentkernel.toml"));
        if !path.exists() {
            if configured_path.is_some() {
                bail!("sandbox governance configuration is missing");
            }
            return Ok(None);
        }

        let config = Config::from_file(&path)?;
        if !config.llm_governance.enabled {
            return Ok(None);
        }

        let tenant_id = state.tenant_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "LLM model governance is enabled but this sandbox has no trusted tenant identity"
            )
        })?;
        crate::model_governance::ModelGovernancePolicy::from_config(
            &config.llm_governance,
            tenant_id,
        )
    }

    fn resolve_volume_args(&self, state: &SandboxState) -> Result<Vec<String>> {
        if state.volumes.is_empty() {
            return Ok(Vec::new());
        }

        let volume_manager = match self.volume_base_dir.as_deref() {
            Some(base_dir) => VolumeManager::new_in(base_dir)?,
            None => VolumeManager::new()?,
        };
        state
            .volumes
            .iter()
            .map(|spec| {
                let mount = VolumeMount::parse(spec)?;
                if !volume_manager.exists(&mount.slug) {
                    bail!(
                        "Volume '{}' not found. Create it with: agentkernel volume create {}",
                        mount.slug,
                        mount.slug
                    );
                }
                Ok(mount.to_docker_arg(volume_manager.volumes_dir()))
            })
            .collect()
    }

    /// Start a sandbox with specific permissions
    pub async fn start_with_permissions(&mut self, name: &str, perms: &Permissions) -> Result<()> {
        self.start_with_permissions_and_files(name, perms, &[])
            .await
    }

    /// Start a sandbox with specific permissions and files to inject
    pub async fn start_with_permissions_and_files(
        &mut self,
        name: &str,
        perms: &Permissions,
        files: &[FileInjection],
    ) -> Result<()> {
        self.start_with_permissions_and_files_impl(name, perms, files, false)
            .await
    }

    /// Start after the HTTP layer has authorized the authenticated principal.
    /// This avoids re-evaluating Cedar against the daemon process identity.
    pub(crate) async fn start_with_permissions_and_files_authorized(
        &mut self,
        name: &str,
        perms: &Permissions,
        files: &[FileInjection],
    ) -> Result<()> {
        self.start_with_permissions_and_files_impl(name, perms, files, true)
            .await
    }

    async fn start_with_permissions_and_files_impl(
        &mut self,
        name: &str,
        perms: &Permissions,
        files: &[FileInjection],
        _policy_prechecked: bool,
    ) -> Result<()> {
        let start_time = std::time::Instant::now();
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?
            .clone();
        let backend = state.backend.unwrap_or(self.backend);
        if backend == BackendType::Firecracker {
            self.ensure_unique_firecracker_rootfs_reference(
                name,
                state.firecracker_rootfs.as_deref(),
            )?;
        }
        if self.pause_recovery.contains_key(name) || self.resume_state_recovery.contains_key(name) {
            bail!(
                "Sandbox '{}' has a pending full-state recovery; retry the recovery operation or remove it before starting another runtime",
                name
            );
        }

        crate::llm_intercept::register_sandbox_metadata(
            name,
            state
                .owner_org_id
                .clone()
                .or_else(|| Some("local".to_string())),
            state.agent.clone(),
            state.owner_user_id.clone(),
            state.labels.get("project").cloned(),
        );

        // A persisted managed checkout is the sandbox's workspace, regardless
        // of which entry point starts it (CLI restart, API, or lifecycle
        // recovery). Keep the isolation guarantee even when the original
        // `--git-worktree` flag was used with `--no-start` or the caller's
        // permission profile otherwise disables `mount_cwd`.
        let mut effective_perms = perms.clone();
        if state.git_worktree.is_some() {
            effective_perms.mount_cwd = true;
        }
        // Resolve before creating the guest so an enabled policy cannot be
        // skipped because proxy startup happens later.
        let model_governance = Self::model_governance_for_state(&state)?;

        if state.archived_at.is_some() {
            bail!(
                "Sandbox '{}' is archived. Recover it before starting (POST /sandboxes/{}/recover).",
                name,
                name
            );
        }

        if state.paused_at.is_some() {
            bail!(
                "Sandbox '{}' is paused with full VM state. Resume it instead of cold-starting it.",
                name
            );
        }

        if backend == BackendType::Firecracker && state.full_state_lineage {
            bail!(
                "Sandbox '{}' belongs to a full-state Firecracker lineage whose writable runtime is not attached; cold start would lose that state. Resume a checkpoint when available or remove the sandbox explicitly.",
                name
            );
        }

        if self.running.contains_key(name) {
            bail!("Sandbox '{}' is already running", name);
        }

        // Enterprise policy check for start
        #[cfg(feature = "enterprise")]
        if !_policy_prechecked {
            self.check_enterprise_policy(crate::policy::Action::Run, name, "unknown", &state.image)
                .await?;
        }

        // Use the backend from stored state, or fall back to current backend
        let capabilities = backend_capabilities(backend);

        if effective_perms.mount_home && !capabilities.mount_home {
            bail!(
                "Backend '{}' does not support mounting the host home directory",
                backend
            );
        }
        if state.ssh_enabled && !capabilities.ssh {
            bail!("Backend '{}' does not support SSH exposure", backend);
        }
        if !state.volumes.is_empty() && !capabilities.host_volumes {
            bail!("Backend '{}' does not support host volume mounts", backend);
        }
        if state.managed_network.is_some()
            && !matches!(backend, BackendType::Docker | BackendType::Podman)
        {
            bail!(
                "Backend '{}' does not support AgentKernel-managed bridge networking; use docker or podman",
                backend
            );
        }
        if !state.secret_bindings.is_empty() && !capabilities.proxy_secret_bindings {
            bail!(
                "Backend '{}' does not support proxy-based secret bindings; use secret env vars or secret files instead",
                backend
            );
        }

        #[cfg(test)]
        if self.bypass_backend_runtime && self.test_backend_factory.is_none() {
            self.resolve_volume_args(&state)?;
            return Ok(());
        }

        // Create sandbox using unified factory
        let mut sandbox = {
            #[cfg(test)]
            if let Some(factory) = &self.test_backend_factory {
                factory(name, backend)?
            } else {
                create_sandbox_with_state(
                    backend,
                    name,
                    &crate::config::OrchestratorConfig::default(),
                    backend.is_remote().then(|| state.remote_context()),
                )?
            }
            #[cfg(not(test))]
            {
                create_sandbox_with_state(
                    backend,
                    name,
                    &crate::config::OrchestratorConfig::default(),
                    backend.is_remote().then(|| state.remote_context()),
                )?
            }
        };
        if backend == BackendType::Firecracker {
            sandbox.set_persistent_disk_reference(state.firecracker_rootfs.as_deref())?;
        }

        // Convert permissions to SandboxConfig
        let work_dir = if effective_perms.mount_cwd {
            state.work_dir.clone().or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
        } else {
            None
        };
        let container_work_dir = state.container_work_dir.clone();

        // Build environment variables if pass_env is enabled
        let mut env: Vec<(String, String)> = if effective_perms.pass_env {
            ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"]
                .iter()
                .filter_map(|&var| std::env::var(var).ok().map(|val| (var.to_string(), val)))
                .collect()
        } else {
            Vec::new()
        };
        env.extend(state.environment.iter().cloned());

        // Pass agent-specific API keys from host environment
        if let Some(ref agent) = state.agent {
            let key_vars: &[&str] = match agent.as_str() {
                "claude" | "amp" => &["ANTHROPIC_API_KEY"],
                "copilot" => &["GITHUB_TOKEN"],
                "gemini" => &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
                "codex" => &["OPENAI_API_KEY"],
                "opencode" => &["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GOOGLE_API_KEY"],
                "pi" => &["ANTHROPIC_API_KEY", "OPENAI_API_KEY"],
                _ => &[],
            };
            for &var in key_vars {
                if let Ok(val) = std::env::var(var) {
                    env.push((var.to_string(), val));
                }
            }
        }

        // Inject a process-scoped Git identity from the sandbox's config. Git
        // reads these variables as configuration for every command, avoiding
        // writes to a mounted human ~/.gitconfig while working on every backend.
        let configured_path = state.config_path.as_ref().map(PathBuf::from);
        let fallback_path = PathBuf::from("agentkernel.toml");
        let git_config_path = configured_path
            .filter(|path| path.exists())
            .or_else(|| fallback_path.exists().then_some(fallback_path));
        let is_devcontainer_path = git_config_path.as_ref().is_some_and(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| matches!(extension, "json" | "jsonc"))
        });
        if let Some(path) = git_config_path
            && !is_devcontainer_path
        {
            match Config::from_file_cached(&path) {
                Ok(config) => env.extend(config.agent.git_config_env()),
                Err(error) => eprintln!(
                    "Warning: Failed to load agent Git identity from {}: {}",
                    path.display(),
                    error
                ),
            }
        }

        // Start secret injection proxy if bindings are configured
        if !state.secret_bindings.is_empty() || model_governance.is_some() {
            let vault = SecretVault::new(SecretBackend::default());

            // Parse bindings and resolve secrets
            let mut bindings = Vec::new();
            let mut resolved_secrets = HashMap::new();
            for raw in &state.secret_bindings {
                let (binding, inline_value) = SecretBinding::parse_cli(raw)?;
                if let Some(val) = inline_value {
                    vault.set(&binding.secret_key, &val)?;
                }
                // Resolve the secret value
                if let Ok(Some(secret_val)) = vault.get(&binding.secret_key) {
                    let header_value = format!("{}{}", binding.header_prefix, secret_val);
                    resolved_secrets.insert(
                        binding.target_host.clone(),
                        (binding.header_name.clone(), header_value),
                    );
                    // Set placeholder env var (real secret never enters VM)
                    env.push((binding.secret_key.clone(), "ak-proxy-managed".to_string()));
                } else {
                    eprintln!(
                        "Warning: Secret '{}' not found in vault, skipping binding",
                        binding.secret_key
                    );
                }
                bindings.push(binding);
            }

            if !resolved_secrets.is_empty() || model_governance.is_some() {
                let allowed_hosts: Vec<String> =
                    bindings.iter().map(|b| b.target_host.clone()).collect();
                // Bind proxy to 0.0.0.0 so VMs on the host network can reach it
                let proxy_config = ProxyConfig {
                    listen_addr: "0.0.0.0:0".parse().unwrap(),
                    bindings,
                    allowed_hosts,
                    blocked_hosts: Vec::new(),
                    allowlist_only: false,
                    sandbox_name: name.to_string(),
                    hooks: Vec::new(),
                    llm_intercept: true,
                    llm_domains: Vec::new(),
                    org_managed_domains: Vec::new(),
                    llm_governance: model_governance.clone(),
                };

                match crate::proxy::start_proxy(
                    proxy_config,
                    resolved_secrets,
                    crate::vsock_secrets::PlaceholderMap::new(),
                )
                .await
                {
                    Ok(handle) => {
                        let proxy_addr = handle.addr;
                        // Determine the proxy host for the sandbox to reach
                        let proxy_host = match backend {
                            BackendType::Apple => {
                                // Apple VMs reach host at gateway 192.168.64.1
                                format!("192.168.64.1:{}", proxy_addr.port())
                            }
                            BackendType::Docker | BackendType::Podman => {
                                if cfg!(target_os = "macos") {
                                    format!("host.docker.internal:{}", proxy_addr.port())
                                } else {
                                    format!("172.17.0.1:{}", proxy_addr.port())
                                }
                            }
                            _ => {
                                // Firecracker, Hyperlight, etc. — use loopback
                                format!("127.0.0.1:{}", proxy_addr.port())
                            }
                        };

                        // Inject proxy env vars
                        env.push(("HTTP_PROXY".to_string(), format!("http://{}", proxy_host)));
                        env.push(("HTTPS_PROXY".to_string(), format!("http://{}", proxy_host)));
                        env.push(("http_proxy".to_string(), format!("http://{}", proxy_host)));
                        env.push(("https_proxy".to_string(), format!("http://{}", proxy_host)));
                        env.push(("NO_PROXY".to_string(), "localhost,127.0.0.1".to_string()));

                        // NODE_EXTRA_CA_CERTS is additive — just points to proxy CA
                        env.push((
                            "NODE_EXTRA_CA_CERTS".to_string(),
                            "/usr/local/share/ca-certificates/agentkernel-proxy.crt".to_string(),
                        ));
                        // SSL_CERT_FILE / REQUESTS_CA_BUNDLE replace the trust store.
                        // We'll create a combined bundle post-start. Point to it here.
                        env.push((
                            "REQUESTS_CA_BUNDLE".to_string(),
                            "/etc/ssl/certs/agentkernel-combined.crt".to_string(),
                        ));
                        env.push((
                            "SSL_CERT_FILE".to_string(),
                            "/etc/ssl/certs/agentkernel-combined.crt".to_string(),
                        ));

                        // Save proxy port to state
                        if let Some(s) = self.sandboxes.get_mut(name) {
                            s.proxy_port = Some(proxy_addr.port());
                        }
                        self.save_sandbox(self.sandboxes.get(name).unwrap())?;

                        eprintln!(
                            "Secret proxy started on port {} ({} binding(s))",
                            proxy_addr.port(),
                            state.secret_bindings.len()
                        );

                        PROXY_HANDLES.write().await.insert(name.to_string(), handle);
                    }
                    Err(e) => {
                        if model_governance.is_some() {
                            return Err(e.context("LLM governance proxy failed to start"));
                        }
                        eprintln!("Warning: Failed to start secret proxy: {}", e);
                    }
                }
            }
        }

        // Auto-inject org-level LLM keys from [llm_keys] config
        {
            let config_path = std::path::PathBuf::from("agentkernel.toml");
            if config_path.exists()
                && let Ok(toml_cfg) = Config::from_file_cached(&config_path)
                && !toml_cfg.llm_keys.is_empty()
            {
                let vault = SecretVault::new(SecretBackend::default());
                let llm_registry = crate::llm_intercept::LlmDomainRegistry::default_registry();

                // Collect domains already bound by sandbox-specific secrets
                let already_bound: std::collections::HashSet<String> = state
                    .secret_bindings
                    .iter()
                    .filter_map(|raw| {
                        crate::proxy::SecretBinding::parse_cli(raw)
                            .ok()
                            .map(|(b, _)| b.target_host.clone())
                    })
                    .collect();

                let mut org_bindings = Vec::new();
                let mut org_resolved = HashMap::new();

                for (domain, vault_key_name) in &toml_cfg.llm_keys {
                    if already_bound.contains(domain) {
                        continue; // Sandbox binding takes precedence
                    }
                    if let Ok(Some(secret_val)) = vault.get(vault_key_name) {
                        // Determine header format from LLM domain registry
                        let (header_name, header_prefix) =
                            if let Some(provider) = llm_registry.lookup(domain) {
                                if provider.name == "anthropic" {
                                    ("x-api-key".to_string(), String::new())
                                } else {
                                    ("Authorization".to_string(), "Bearer ".to_string())
                                }
                            } else {
                                ("Authorization".to_string(), "Bearer ".to_string())
                            };

                        let header_value = format!("{}{}", header_prefix, secret_val);
                        org_resolved.insert(domain.clone(), (header_name.clone(), header_value));
                        org_bindings.push(SecretBinding {
                            secret_key: vault_key_name.clone(),
                            target_host: domain.clone(),
                            header_name,
                            header_prefix,
                        });
                    }
                }

                if !org_resolved.is_empty() {
                    // If no proxy is running yet, start one for org keys
                    let has_proxy = PROXY_HANDLES.read().await.contains_key(name);
                    if !has_proxy {
                        let org_domains: Vec<String> =
                            org_bindings.iter().map(|b| b.target_host.clone()).collect();
                        let allowed_hosts = org_domains.clone();
                        let proxy_config = ProxyConfig {
                            listen_addr: "0.0.0.0:0".parse().unwrap(),
                            bindings: org_bindings,
                            allowed_hosts,
                            blocked_hosts: Vec::new(),
                            allowlist_only: false,
                            sandbox_name: name.to_string(),
                            hooks: Vec::new(),
                            llm_intercept: true,
                            llm_domains: Vec::new(),
                            org_managed_domains: org_domains,
                            llm_governance: model_governance.clone(),
                        };

                        match crate::proxy::start_proxy(
                            proxy_config,
                            org_resolved,
                            crate::vsock_secrets::PlaceholderMap::new(),
                        )
                        .await
                        {
                            Ok(handle) => {
                                let proxy_addr = handle.addr;
                                let proxy_host = match backend {
                                    BackendType::Apple => {
                                        format!("192.168.64.1:{}", proxy_addr.port())
                                    }
                                    BackendType::Docker | BackendType::Podman => {
                                        if cfg!(target_os = "macos") {
                                            format!("host.docker.internal:{}", proxy_addr.port())
                                        } else {
                                            format!("172.17.0.1:{}", proxy_addr.port())
                                        }
                                    }
                                    _ => format!("127.0.0.1:{}", proxy_addr.port()),
                                };

                                env.push((
                                    "HTTP_PROXY".to_string(),
                                    format!("http://{}", proxy_host),
                                ));
                                env.push((
                                    "HTTPS_PROXY".to_string(),
                                    format!("http://{}", proxy_host),
                                ));
                                env.push((
                                    "http_proxy".to_string(),
                                    format!("http://{}", proxy_host),
                                ));
                                env.push((
                                    "https_proxy".to_string(),
                                    format!("http://{}", proxy_host),
                                ));
                                env.push((
                                    "NO_PROXY".to_string(),
                                    "localhost,127.0.0.1".to_string(),
                                ));
                                env.push((
                                    "NODE_EXTRA_CA_CERTS".to_string(),
                                    "/usr/local/share/ca-certificates/agentkernel-proxy.crt"
                                        .to_string(),
                                ));
                                env.push((
                                    "REQUESTS_CA_BUNDLE".to_string(),
                                    "/etc/ssl/certs/agentkernel-combined.crt".to_string(),
                                ));
                                env.push((
                                    "SSL_CERT_FILE".to_string(),
                                    "/etc/ssl/certs/agentkernel-combined.crt".to_string(),
                                ));

                                if let Some(s) = self.sandboxes.get_mut(name) {
                                    s.proxy_port = Some(proxy_addr.port());
                                }
                                self.save_sandbox(self.sandboxes.get(name).unwrap())?;

                                eprintln!(
                                    "Org LLM key proxy started on port {} ({} key(s))",
                                    proxy_addr.port(),
                                    toml_cfg.llm_keys.len()
                                );

                                PROXY_HANDLES.write().await.insert(name.to_string(), handle);
                            }
                            Err(e) => {
                                if model_governance.is_some() {
                                    return Err(e.context("LLM governance proxy failed to start"));
                                }
                                eprintln!("Warning: Failed to start org LLM key proxy: {}", e);
                            }
                        }
                    }
                    // If proxy already running, we can't add more secrets dynamically yet
                    // Future: add hot-reload support
                }
            }
        }

        // If secret files are configured, set the env var so sandbox knows where to find them
        if !state.secret_files.is_empty() {
            env.push((
                "AGENTKERNEL_SECRETS_PATH".to_string(),
                crate::vsock_secrets::DEFAULT_SECRETS_PATH.to_string(),
            ));
        }

        // Build SSH config if enabled
        let ssh_config = if state.ssh_enabled {
            let mut ssh_cfg = crate::ssh::SshConfig {
                enabled: true,
                ..Default::default()
            };
            // Check for VAULT_ADDR env var
            if let Ok(vault_addr) = std::env::var("VAULT_ADDR") {
                ssh_cfg.vault_addr = Some(vault_addr);
            }
            Some(ssh_cfg)
        } else {
            None
        };

        // Resolve volume mounts to docker -v arguments
        let volume_args = self.resolve_volume_args(&state)?;

        let config = SandboxConfig {
            image: state.image.clone(),
            vcpus: state.vcpus,
            memory_mb: effective_perms.max_memory_mb.unwrap_or(state.memory_mb),
            mount_cwd: effective_perms.mount_cwd,
            work_dir,
            container_work_dir,
            env,
            network: effective_perms.network,
            read_only: effective_perms.read_only_root,
            mount_home: effective_perms.mount_home,
            files: files.to_vec(),
            ports: state.ports.clone(),
            managed_network: state.managed_network.clone(),
            ssh: ssh_config.clone(),
            volumes: volume_args,
        };

        if let Err(error) = sandbox.start(&config).await {
            if runtime_may_survive_failed_stop(&error, sandbox.is_running()) {
                self.running.insert(name.to_string(), sandbox);
                return Err(error).context(format!(
                    "sandbox '{name}' failed to start cleanly, but its runtime may still exist and remains under manager ownership; remove it before retrying"
                ));
            }
            return Err(error);
        }
        let previous_firecracker_rootfs = state.firecracker_rootfs.clone();
        let setup_result = async {
        if let Some(persisted) = self.sandboxes.get_mut(name) {
            persisted.work_dir = config.work_dir.clone();
            if backend == BackendType::Firecracker {
                persisted.firecracker_rootfs = sandbox.persistent_disk_reference();
            }
            if let Some(metadata) = sandbox.runtime_metadata() {
                Self::apply_runtime_metadata(persisted, &metadata);
            }
            let snapshot = persisted.clone();
            if let Err(error) = self.save_sandbox(&snapshot) {
                if backend == BackendType::Firecracker
                    && let Some(persisted) = self.sandboxes.get_mut(name)
                {
                    persisted.firecracker_rootfs = previous_firecracker_rootfs.clone();
                }
                return Err(error);
            }
        }
        if backend == BackendType::Firecracker
            && let Err(publication_error) = sandbox.publish_persistent_disk_reference()
        {
                // State and the rootfs durability marker form a small
                // two-phase publication. If marker publication fails, first
                // restore the prior opaque state reference; only then permit
                // the ordinary setup cleanup to discard a newly-prepared
                // image. If rollback cannot be written, retain the newly
                // published state reference and let the backend fail closed
                // toward retention instead of creating a dangling reference.
                let new_reference = sandbox.persistent_disk_reference();
                let rollback = if let Some(persisted) = self.sandboxes.get_mut(name) {
                    persisted.firecracker_rootfs = previous_firecracker_rootfs.clone();
                    let snapshot = persisted.clone();
                    self.save_sandbox(&snapshot)
                } else {
                    bail!("sandbox '{}' disappeared during disk lineage publication", name);
                };
                if rollback.is_ok() {
                    sandbox
                        .rollback_persistent_disk_reference()
                        .context("failed to roll back unpublished Firecracker disk lineage")?;
                    return Err(publication_error)
                        .context("failed to publish Firecracker disk lineage");
                }
                if let Some(persisted) = self.sandboxes.get_mut(name) {
                    persisted.firecracker_rootfs = new_reference;
                }
                return Err(publication_error).context(
                    "failed to publish Firecracker disk lineage and could not roll back its state reference; lineage retained for retry",
                );
        }

        // Inject non-SSH files first
        if !files.is_empty() {
            sandbox.inject_files(files).await?;
        }

        // Inject proxy CA certificate into sandbox trust store
        {
            let handles = PROXY_HANDLES.read().await;
            if let Some(handle) = handles.get(name) {
                let ca_files = vec![FileInjection {
                    dest: "/usr/local/share/ca-certificates/agentkernel-proxy.crt".to_string(),
                    content: handle.ca_cert_pem.as_bytes().to_vec(),
                }];
                sandbox.inject_files(&ca_files).await?;

                // Create a combined CA bundle: system certs + our proxy CA.
                // This works across distros (Debian: /etc/ssl/certs/ca-certificates.crt,
                // Alpine: /etc/ssl/certs/ca-certificates.crt, RHEL: /etc/pki/tls/certs/ca-bundle.crt).
                // Fallback: if no system bundle exists, use only our CA cert.
                let _ = sandbox
                    .exec(&[
                        "sh",
                        "-c",
                        "{ cat /etc/ssl/certs/ca-certificates.crt /usr/local/share/ca-certificates/agentkernel-proxy.crt > /etc/ssl/certs/agentkernel-combined.crt && [ -s /etc/ssl/certs/agentkernel-combined.crt ]; } 2>/dev/null || \
                         { cat /etc/pki/tls/certs/ca-bundle.crt /usr/local/share/ca-certificates/agentkernel-proxy.crt > /etc/ssl/certs/agentkernel-combined.crt && [ -s /etc/ssl/certs/agentkernel-combined.crt ]; } 2>/dev/null || \
                         cp /usr/local/share/ca-certificates/agentkernel-proxy.crt /etc/ssl/certs/agentkernel-combined.crt",
                    ])
                    .await;

                // Also run update-ca-certificates for tools that use system store directly
                let _ = sandbox
                    .exec(&[
                        "sh",
                        "-c",
                        "update-ca-certificates 2>/dev/null || update-ca-trust 2>/dev/null || true",
                    ])
                    .await;
            }
        }

        // Inject secrets as files if configured
        if !state.secret_files.is_empty() {
            let vault = SecretVault::new(SecretBackend::default());
            let mut resolved = HashMap::new();
            for key in &state.secret_files {
                if let Ok(Some(val)) = vault.get(key) {
                    resolved.insert(key.clone(), val);
                } else {
                    eprintln!(
                        "Warning: Secret '{}' not found in vault, skipping file injection",
                        key
                    );
                }
            }
            if !resolved.is_empty() {
                if state.placeholder_secrets {
                    self.inject_placeholder_secrets(
                        sandbox.as_mut(),
                        name,
                        &resolved,
                        backend,
                        model_governance.as_ref(),
                    )
                    .await?;
                } else {
                    match crate::vsock_secrets::inject_secrets_as_files(
                        sandbox.as_mut(),
                        crate::vsock_secrets::DEFAULT_SECRETS_PATH,
                        &resolved,
                    )
                    .await
                    {
                        Ok(injected) => {
                            eprintln!(
                                "Injected {} secret file(s) at {}",
                                injected.len(),
                                crate::vsock_secrets::DEFAULT_SECRETS_PATH,
                            );
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to inject secret files: {}", e);
                        }
                    }
                }
            }
        }

        // If SSH is enabled, install sshd THEN inject SSH files
        // (apk add openssh-server installs a default sshd_config that would
        // overwrite our custom config if we inject first)
        if let Some(ref ssh_cfg) = ssh_config {
            // Install openssh-server (Alpine/Debian/RHEL) — must come BEFORE file injection
            let install_result = sandbox
                .exec(&[
                    "sh",
                    "-c",
                    "apk add --no-cache openssh-server 2>/dev/null || \
                     apt-get update -qq && apt-get install -y -qq openssh-server 2>/dev/null || \
                     yum install -y openssh-server 2>/dev/null || true",
                ])
                .await;
            if let Err(e) = install_result {
                eprintln!("Warning: Failed to install sshd: {}", e);
            }

            // Now inject SSH config files (overwrites package defaults)
            let (ca_priv, ca_pub) = crate::ssh::generate_ca_keypair()?;
            let ssh_files = crate::ssh::sshd_file_injections(&ca_pub, ssh_cfg)?;
            sandbox.inject_files(&ssh_files).await?;

            // Store CA private key for later signing
            let ca_key_path = self.data_dir.join(format!("{}-ssh-ca.key", name));
            std::fs::write(&ca_key_path, ca_priv)?;

            // Make startup script executable and run it
            let _ = sandbox.exec(&["chmod", "+x", "/tmp/start-sshd.sh"]).await;
            let start_result = sandbox.exec(&["sh", "/tmp/start-sshd.sh"]).await;
            if let Err(e) = start_result {
                eprintln!("Warning: Failed to start sshd: {}", e);
            } else if let Ok(ref result) = start_result
                && !result.stderr.is_empty()
            {
                eprintln!("sshd: {}", result.stderr.trim());
            }

            // Find the mapped host port for SSH.
            // If the port was auto-assigned (host_port: None), query Docker for the actual port.
            let mut ssh_port = state
                .ports
                .iter()
                .find(|p| p.container_port == 22)
                .and_then(|p| p.host_port);

            if ssh_port.is_none() {
                // Query Docker for the auto-assigned host port
                let container_name = format!("agentkernel-{}", name);
                if let Ok(output) = std::process::Command::new("docker")
                    .args(["port", &container_name, "22"])
                    .output()
                {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    // Output format: "0.0.0.0:32768" or "[::]:32768"
                    if let Some(port_str) = stdout.trim().rsplit(':').next()
                        && let Ok(port) = port_str.parse::<u16>()
                    {
                        ssh_port = Some(port);
                    }
                }
            }

            // Save the resolved SSH port to state
            if let Some(port) = ssh_port {
                if let Some(s) = self.sandboxes.get_mut(name) {
                    s.ssh_host_port = Some(port);
                    // Also update the port mapping so it's visible in list/info
                    if let Some(pm) = s.ports.iter_mut().find(|p| p.container_port == 22) {
                        pm.host_port = Some(port);
                    }
                }
                self.save_sandbox(self.sandboxes.get(name).unwrap())?;
                eprintln!("SSH access: agentkernel ssh {}", name);
                eprintln!("  or: ssh -p {} sandbox@localhost", port);
            } else {
                eprintln!("SSH access: enabled on port 22 inside sandbox");
                eprintln!("  (host port could not be resolved — try explicit: -p 2222:22)");
            }
        }

        // Auto-install agent CLI if specified in sandbox state
        if let Some(ref agent) = state.agent {
            let install_cmd = match agent.as_str() {
                "claude" => Some("npm install -g @anthropic-ai/claude-code@2.1.239"),
                "gemini" => Some("npm install -g @google/gemini-cli@0.56.0"),
                "codex" => Some("npm install -g @openai/codex@0.149.0"),
                "opencode" => Some("npm install -g opencode-ai@1.18.21"),
                "amp" => Some(
                    "npm install -g --allow-scripts=@ampcode/cli @ampcode/cli@0.0.1787342526-gc11bfb",
                ),
                "pi" => Some("npm install -g @earendil-works/pi-coding-agent@0.84.2"),
                "copilot" => Some("npm install -g @github/copilot@1.0.80"),
                _ => None,
            };
            if let Some(cmd) = install_cmd {
                eprintln!("Installing {} agent CLI...", agent);
                // Install runs inside the sandbox, not on the host — safe from injection
                // since agent values are validated against the known set above
                match sandbox.exec(&["sh", "-c", cmd]).await {
                    Ok(result) if result.exit_code == 0 => {
                        eprintln!("{} agent CLI installed successfully", agent);
                    }
                    Ok(result) => {
                        eprintln!(
                            "Warning: {} agent install exited with code {}: {}",
                            agent,
                            result.exit_code,
                            result.stderr.trim()
                        );
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to install {} agent CLI: {}", agent, e);
                    }
                }
            }
        }

        // Run init_script if specified (from template config).
        // On failure, stop the sandbox and propagate the error so the user
        // does not end up with a broken but "started" sandbox.
        if let Some(ref script) = state.init_script {
            eprintln!("Running init script...");
            match sandbox.exec(&["sh", "-c", script]).await {
                Ok(result) if result.exit_code == 0 => {
                    eprintln!("Init script completed successfully");
                }
                Ok(result) => {
                    let stderr = result.stderr.trim().to_string();
                    eprintln!(
                        "Error: init script exited with code {}: {}",
                        result.exit_code, stderr
                    );
                    log_event(AuditEvent::SandboxError {
                        name: name.to_string(),
                        error: format!(
                            "init script exited with code {}: {}",
                            result.exit_code, stderr
                        ),
                    });
                    let _ = sandbox.stop().await;
                    anyhow::bail!(
                        "init script failed (exit code {}): {}",
                        result.exit_code,
                        stderr
                    );
                }
                Err(e) => {
                    eprintln!("Error: Failed to run init script: {}", e);
                    log_event(AuditEvent::SandboxError {
                        name: name.to_string(),
                        error: format!("failed to run init script: {}", e),
                    });
                    let _ = sandbox.stop().await;
                    anyhow::bail!("failed to run init script: {}", e);
                }
            }
        }

        // Run devcontainer postCreateCommand entries as argv vectors once. String
        // commands are represented by ["sh", "-c", value] during parsing; no
        // command is concatenated into a larger shell string here. The completion
        // flag is persisted only after every command succeeds, leaving failed
        // creates retryable on the next start.
        if should_run_devcontainer_post_create(&state) {
            for command in &state.post_create_commands {
                if command.is_empty() {
                    let _ = sandbox.stop().await;
                    bail!("devcontainer postCreateCommand contains an empty command");
                }
                let argv = command.iter().map(String::as_str).collect::<Vec<_>>();
                match sandbox.exec(&argv).await {
                    Ok(result) if result.exit_code == 0 => {}
                    Ok(result) => {
                        let stderr = result.stderr.trim().to_string();
                        let _ = sandbox.stop().await;
                        bail!(
                            "devcontainer postCreateCommand failed (exit code {}): {}",
                            result.exit_code,
                            stderr
                        );
                    }
                    Err(error) => {
                        let _ = sandbox.stop().await;
                        return Err(error).context("failed to run devcontainer postCreateCommand");
                    }
                }
            }

            let snapshot = {
                let stored = self
                    .sandboxes
                    .get_mut(name)
                    .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
                stored.post_create_completed = true;
                stored.clone()
            };
            if let Err(error) = self.save_sandbox(&snapshot) {
                if let Some(stored) = self.sandboxes.get_mut(name) {
                    stored.post_create_completed = false;
                }
                let _ = sandbox.stop().await;
                return Err(error)
                    .context("failed to persist devcontainer postCreateCommand completion");
            }
        }

            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(setup_error) = setup_result {
            if let Err(stop_error) = sandbox.stop().await {
                if runtime_may_survive_failed_stop(&stop_error, sandbox.is_running()) {
                    self.running.insert(name.to_string(), sandbox);
                    return Err(setup_error).context(format!(
                        "post-start setup failed and cleanup could not prove sandbox '{name}' exited ({stop_error:#}); the runtime remains under manager ownership and must be removed before retrying"
                    ));
                }
                return Err(setup_error).context(format!(
                    "post-start setup failed; sandbox '{name}' exited, but cleanup also failed ({stop_error:#})"
                ));
            }
            return Err(setup_error);
        }

        self.running.insert(name.to_string(), sandbox);
        // A manual start is an explicit revival of a dormant workspace.
        if let Some(state) = self.sandboxes.get_mut(name) {
            state.dormant_at = None;
            state.dormant_reason = None;
            let snapshot = state.clone();
            self.save_sandbox(&snapshot)?;
        }
        if let Err(e) = self.sync_runtime_metadata(name) {
            eprintln!(
                "Warning: failed to sync remote metadata for '{}': {}",
                name, e
            );
        }
        self.touch_activity(name)?;

        log_event(AuditEvent::SandboxStarted {
            name: name.to_string(),
            profile: Some(format!("{:?}", perms)),
        });
        crate::metrics::record_sandbox_lifecycle(
            "started",
            &backend.to_string(),
            start_time.elapsed().as_secs_f64(),
        );

        Ok(())
    }

    /// Check if a command is allowed by the security policy in agentkernel.toml.
    /// Logs a PolicyViolation audit event and returns an error if blocked.
    fn enforce_command_policy(cmd: &[String]) -> Result<()> {
        if let Some(binary) = cmd.first()
            && let Ok(cfg) = Config::from_file_cached(&PathBuf::from("agentkernel.toml"))
            && !cfg.security.commands.is_allowed(binary)
        {
            log_event(AuditEvent::PolicyViolation {
                sandbox: "ephemeral".to_string(),
                policy: "commands".to_string(),
                details: format!("blocked command: {}", binary),
            });
            bail!(
                "Command '{}' blocked by security policy. Check [security.commands] in agentkernel.toml",
                binary
            );
        }
        Ok(())
    }

    /// Execute a command in a sandbox
    pub async fn exec_cmd(&mut self, name: &str, cmd: &[String]) -> Result<String> {
        self.exec_cmd_with_env(name, cmd, &[]).await
    }

    /// Execute a command in a sandbox with environment variables
    pub async fn exec_cmd_with_env(
        &mut self,
        name: &str,
        cmd: &[String],
        env: &[String],
    ) -> Result<String> {
        self.exec_cmd_full(
            name,
            cmd,
            &crate::backend::ExecOptions {
                env: env.to_vec(),
                ..Default::default()
            },
        )
        .await
    }

    /// Execute a command with full options (env, workdir, user)
    pub async fn exec_cmd_full(
        &mut self,
        name: &str,
        cmd: &[String],
        opts: &crate::backend::ExecOptions,
    ) -> Result<String> {
        Self::enforce_command_policy(cmd)?;

        // Enterprise policy check for exec
        #[cfg(feature = "enterprise")]
        {
            let image = self
                .sandboxes
                .get(name)
                .map(|s| s.image.clone())
                .unwrap_or_default();
            self.check_enterprise_policy(crate::policy::Action::Exec, name, "unknown", &image)
                .await?;
        }

        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();

        let exec_start = std::time::Instant::now();
        let result = sandbox.exec_with_options(&cmd_refs, opts).await?;
        if let Err(e) = self.sync_runtime_metadata(name) {
            eprintln!(
                "Warning: failed to sync remote metadata for '{}': {}",
                name, e
            );
        }
        let backend = self
            .sandboxes
            .get(name)
            .and_then(|state| state.backend)
            .unwrap_or(self.backend);
        crate::metrics::record_command(&backend.to_string(), exec_start.elapsed().as_secs_f64());

        log_event(AuditEvent::CommandExecuted {
            sandbox: name.to_string(),
            command: cmd.to_vec(),
            exit_code: Some(result.exit_code),
        });

        // Any command execution counts as sandbox activity.
        let _ = self.touch_activity(name);

        if result.exit_code != 0 {
            return Err(CommandFailed {
                exit_code: result.exit_code,
                output: result.output(),
            }
            .into());
        }

        Ok(result.output())
    }

    /// Start a detached (background) command in a sandbox.
    ///
    /// The command runs in the background with stdout/stderr captured to files
    /// inside the container. Returns a `DetachedCommand` with an ID and PID
    /// that can be used to check status, retrieve logs, or kill the process.
    pub async fn exec_detached(
        &mut self,
        name: &str,
        cmd: &[String],
        opts: &crate::backend::ExecOptions,
    ) -> Result<DetachedCommand> {
        Self::enforce_command_policy(cmd)?;

        #[cfg(feature = "enterprise")]
        {
            let image = self
                .sandboxes
                .get(name)
                .map(|s| s.image.clone())
                .unwrap_or_default();
            self.check_enterprise_policy(crate::policy::Action::Exec, name, "unknown", &image)
                .await?;
        }

        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        let id = format!("{:08x}", rand::random::<u32>());
        let stdout_path = format!("/tmp/ak-{id}.out");
        let stderr_path = format!("/tmp/ak-{id}.err");
        let exit_path = format!("/tmp/ak-{id}.exit");

        // Wrap the command to run in background with output capture
        let escaped_cmd: Vec<String> = cmd.iter().map(|c| shell_escape(c)).collect();
        let wrapped = format!(
            "nohup sh -c '{}' > {} 2> {} & pid=$!; (wait $pid; echo $? > {}) >/dev/null 2>&1 & echo $pid",
            escaped_cmd.join(" "),
            stdout_path,
            stderr_path,
            exit_path,
        );
        let wrapper_cmd: Vec<&str> = vec!["sh", "-c", &wrapped];

        let result = sandbox.exec_with_options(&wrapper_cmd, opts).await?;

        if result.exit_code != 0 {
            bail!("Failed to start detached command: {}", result.output());
        }

        let pid: u32 = result.stdout.trim().parse().map_err(|_| {
            anyhow::anyhow!(
                "Failed to parse PID from detached command output: '{}'",
                result.stdout.trim()
            )
        })?;

        let now = chrono::Utc::now().to_rfc3339();
        let detached_cmd = DetachedCommand {
            id: id.clone(),
            sandbox: name.to_string(),
            command: cmd.to_vec(),
            pid,
            status: DetachedStatus::Running,
            exit_code: None,
            started_at: now,
        };

        log_event(AuditEvent::CommandExecuted {
            sandbox: name.to_string(),
            command: cmd.to_vec(),
            exit_code: None,
        });

        self.detached.insert(id, detached_cmd.clone());
        let _ = self.touch_activity(name);
        Ok(detached_cmd)
    }

    /// Get the status of a detached command, refreshing from the container.
    pub async fn detached_status(&mut self, cmd_id: &str) -> Result<DetachedCommand> {
        let cmd = self
            .detached
            .get(cmd_id)
            .ok_or_else(|| anyhow::anyhow!("Detached command '{}' not found", cmd_id))?
            .clone();

        // If already finished, return cached status
        if cmd.status != DetachedStatus::Running {
            return Ok(cmd);
        }

        let exit_path = format!("/tmp/ak-{}.exit", cmd_id);
        let pid_str = cmd.pid.to_string();
        if let Err(e) = self.hydrate_remote_runtime(&cmd.sandbox) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                cmd.sandbox, e
            );
        }
        let sandbox = self
            .running
            .get_mut(&cmd.sandbox)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' is not running", cmd.sandbox))?;

        // Process still running: ignore exit file to avoid spoofing/stale reads.
        let probe = sandbox
            .exec_with_options(
                &["kill", "-0", &pid_str],
                &crate::backend::ExecOptions::default(),
            )
            .await?;
        if probe.exit_code == 0 {
            return Ok(cmd);
        }

        // Process is no longer running; now consume the exit-code file if available.
        let read_exit = sandbox
            .exec_with_options(
                &["cat", &exit_path],
                &crate::backend::ExecOptions::default(),
            )
            .await;
        if let Ok(exit_result) = read_exit
            && exit_result.exit_code == 0
        {
            let exit_code = exit_result.stdout.trim().parse::<i32>().unwrap_or(1);
            let status = if exit_code == 0 {
                DetachedStatus::Completed
            } else {
                DetachedStatus::Failed
            };
            if let Some(tracked) = self.detached.get_mut(cmd_id) {
                tracked.status = status;
                tracked.exit_code = Some(exit_code);
                return Ok(tracked.clone());
            }
            return Ok(cmd);
        }

        // Process is no longer running but exit status file is unavailable.
        if let Some(tracked) = self.detached.get_mut(cmd_id) {
            tracked.status = DetachedStatus::Failed;
            tracked.exit_code = None;
            return Ok(tracked.clone());
        }
        Ok(cmd)
    }

    /// Get stdout/stderr logs from a detached command.
    pub async fn detached_logs(&mut self, cmd_id: &str, stream: Option<&str>) -> Result<String> {
        let cmd = self
            .detached
            .get(cmd_id)
            .ok_or_else(|| anyhow::anyhow!("Detached command '{}' not found", cmd_id))?
            .clone();

        if let Err(e) = self.hydrate_remote_runtime(&cmd.sandbox) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                cmd.sandbox, e
            );
        }
        let sandbox = self
            .running
            .get_mut(&cmd.sandbox)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' is not running", cmd.sandbox))?;

        let file_path = match stream {
            Some("stderr") => format!("/tmp/ak-{}.err", cmd_id),
            _ => format!("/tmp/ak-{}.out", cmd_id),
        };

        let result = sandbox
            .exec_with_options(
                &["cat", &file_path],
                &crate::backend::ExecOptions::default(),
            )
            .await?;

        Ok(result.stdout)
    }

    /// Kill a detached command.
    pub async fn detached_kill(&mut self, cmd_id: &str) -> Result<()> {
        let cmd = self
            .detached
            .get(cmd_id)
            .ok_or_else(|| anyhow::anyhow!("Detached command '{}' not found", cmd_id))?
            .clone();

        if cmd.status != DetachedStatus::Running {
            return Ok(());
        }

        if let Err(e) = self.hydrate_remote_runtime(&cmd.sandbox) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                cmd.sandbox, e
            );
        }
        let sandbox = self
            .running
            .get_mut(&cmd.sandbox)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' is not running", cmd.sandbox))?;

        let pid_str = cmd.pid.to_string();
        let _ = sandbox
            .exec_with_options(&["kill", &pid_str], &crate::backend::ExecOptions::default())
            .await;

        if let Some(tracked) = self.detached.get_mut(cmd_id) {
            tracked.status = DetachedStatus::Failed;
            tracked.exit_code = Some(137);
        }

        Ok(())
    }

    /// List detached commands, optionally filtered by sandbox name.
    pub fn detached_list(&self, sandbox: Option<&str>) -> Vec<DetachedCommand> {
        self.detached
            .values()
            .filter(|c| sandbox.is_none() || Some(c.sandbox.as_str()) == sandbox)
            .cloned()
            .collect()
    }

    /// Attach to a sandbox's interactive shell with optional environment variables
    pub async fn attach_with_env(&mut self, name: &str, env: &[String]) -> Result<i32> {
        log_event(AuditEvent::SessionAttached {
            sandbox: name.to_string(),
        });

        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        let result = {
            let sandbox = self.running.get_mut(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                    name,
                    name
                )
            })?;
            sandbox.attach_with_env(None, env).await
        };

        if result.is_ok() {
            if let Err(e) = self.sync_runtime_metadata(name) {
                eprintln!(
                    "Warning: failed to sync remote metadata for '{}': {}",
                    name, e
                );
            }
            let _ = self.touch_activity(name);
        }

        result
    }

    /// Pause a running Firecracker sandbox into a durable, full-VM checkpoint.
    ///
    /// The backend keeps the source VM running if snapshot creation fails. A
    /// successful call terminates the paused Firecracker process only after
    /// memory, device state, and an immutable disk have been captured.
    pub async fn pause(&mut self, name: &str) -> Result<FullStateCheckpoint> {
        self.pause_impl(name, false).await
    }

    pub(crate) async fn pause_authorized(&mut self, name: &str) -> Result<FullStateCheckpoint> {
        self.pause_impl(name, true).await
    }

    async fn pause_impl(
        &mut self,
        name: &str,
        _policy_prechecked: bool,
    ) -> Result<FullStateCheckpoint> {
        let pause_start = std::time::Instant::now();
        validation::validate_sandbox_name(name)?;
        let state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        let backend = state.backend.unwrap_or(self.backend);
        if backend != BackendType::Firecracker {
            bail!(
                "Backend '{}' does not support full-state pause/resume; Firecracker on Linux/KVM is required",
                backend
            );
        }
        if state.paused_at.is_some() {
            bail!("Sandbox '{}' is already paused", name);
        }
        if state.proxy_port.is_some() || !state.secret_bindings.is_empty() {
            bail!(
                "Sandbox '{}' uses a host-side secret or governance proxy whose live state cannot yet be checkpointed",
                name
            );
        }

        #[cfg(feature = "enterprise")]
        if !_policy_prechecked {
            self.check_enterprise_policy(crate::policy::Action::Run, name, "unknown", &state.image)
                .await?;
        }

        if !self.running.contains_key(name) {
            bail!("Sandbox '{}' is not running", name);
        }
        let store = FullStateCheckpointStore::new(&self.data_dir)?;
        let reservation_bytes = self
            .running
            .get(name)
            .expect("running presence was validated before capacity reservation")
            .full_state_reservation_bytes(state.memory_mb)?;
        store
            .ensure_capacity(reservation_bytes)
            .with_context(|| format!("cannot pause sandbox '{name}'"))?;
        let staging = store.begin()?;
        // Persist the checkpoint identity before changing the VM. If the
        // service exits after Firecracker stops but before publication, the
        // sandbox remains conservatively paused instead of silently becoming
        // a cold-startable state with an orphaned memory image.
        let mut paused_state = state.clone();
        paused_state.full_state_checkpoint = Some(staging.id().to_string());
        // The checkpoint now owns the immutable disk copy. Do not leave the
        // ordinary-stop lineage reference pointing at the runtime image that
        // pause will destroy after publication.
        paused_state.firecracker_rootfs = None;
        paused_state.full_state_lineage = true;
        paused_state.paused_at = Some(chrono::Utc::now().to_rfc3339());
        paused_state.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
        if let Err(error) = self.save_sandbox(&paused_state) {
            if let Err(cleanup_error) = store.discard_staging(staging.path()) {
                return Err(error).context(format!(
                    "failed to persist pause transition and failed to discard staging: {cleanup_error:#}"
                ));
            }
            return Err(error).context("failed to persist pause transition");
        }
        self.sandboxes
            .insert(name.to_string(), paused_state.clone());

        let mut sandbox = self
            .running
            .remove(name)
            .expect("running presence was validated before pause transition");
        let backend_snapshot = match sandbox.pause_to(staging.path()).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(recovery) = error.downcast_ref::<FullStatePauseError>().cloned() {
                    debug_assert!(recovery.source_resume_failed);

                    if recovery.artifacts_complete {
                        let snapshot = recovery.snapshot.clone();
                        // Never publish a cloneable checkpoint until process
                        // death is confirmed. Otherwise a failed kill could
                        // leave the original VM alive beside a resumed copy.
                        if let Err(stop_error) = sandbox.stop().await {
                            let termination_confirmed = stop_error
                                .downcast_ref::<FullStateTerminationError>()
                                .is_some_and(|failure| !failure.process_may_be_running);
                            if !termination_confirmed {
                                let recovery_path = staging.preserve();
                                self.pause_recovery.insert(
                                    name.to_string(),
                                    PendingFullStateRecovery {
                                        sandbox: Some(sandbox),
                                        staging_path: recovery_path.clone(),
                                        completed_snapshot: Some(snapshot),
                                    },
                                );
                                return Err(error).context(format!(
                                    "complete checkpoint artifacts were produced, but source termination could not be confirmed ({stop_error:#}); checkpoint remains unpublished at {} and the source stays under recovery ownership; call resume to retry termination and publication or remove to abandon it",
                                    recovery_path.display()
                                ));
                            }
                            eprintln!(
                                "Warning: source termination was confirmed, but runtime cleanup failed before checkpoint publication: {stop_error:#}"
                            );
                        }

                        if let Err(marker_error) = store.mark_recovery_ready(
                            &staging,
                            name,
                            &state.uuid,
                            state.vcpus,
                            state.memory_mb,
                            snapshot.clone(),
                        ) {
                            match store.commit(
                                &staging,
                                name,
                                &state.uuid,
                                state.vcpus,
                                state.memory_mb,
                                snapshot.clone(),
                            ) {
                                Ok(checkpoint) => {
                                    crate::metrics::record_sandbox_lifecycle(
                                        "paused",
                                        "firecracker",
                                        pause_start.elapsed().as_secs_f64(),
                                    );
                                    return Ok(checkpoint);
                                }
                                Err(commit_error) => {
                                    let recovery_path = staging.preserve();
                                    self.pause_recovery.insert(
                                        name.to_string(),
                                        PendingFullStateRecovery {
                                            sandbox: None,
                                            staging_path: recovery_path.clone(),
                                            completed_snapshot: Some(snapshot),
                                        },
                                    );
                                    return Err(commit_error).context(format!(
                                        "source was terminated, but complete checkpoint staging could neither be marked recovery-ready ({marker_error:#}) nor published after pause recovery ({error:#}); safe completed artifacts retained under recovery ownership at {}",
                                        recovery_path.display()
                                    ));
                                }
                            }
                        }

                        match store.commit(
                            &staging,
                            name,
                            &state.uuid,
                            state.vcpus,
                            state.memory_mb,
                            snapshot,
                        ) {
                            Ok(checkpoint) => {
                                crate::metrics::record_sandbox_lifecycle(
                                    "paused",
                                    "firecracker",
                                    pause_start.elapsed().as_secs_f64(),
                                );
                                return Ok(checkpoint);
                            }
                            Err(commit_error) => {
                                let recovery_path = staging.preserve();
                                return Err(commit_error).context(format!(
                                    "source was terminated after pause recovery, but checkpoint publication failed ({error:#}); recovery-ready artifacts retained at {}",
                                    recovery_path.display()
                                ));
                            }
                        }
                    }

                    // Partial artifacts cannot restore independently. Retry
                    // the live source once now and retain exact ownership if
                    // the retry is still ambiguous or unsuccessful.
                    match sandbox.retry_full_state_resume().await {
                        Ok(()) => {
                            let running_state =
                                with_full_state_cleanup_intent(state.clone(), staging.id());
                            if let Err(rollback_error) = self.save_sandbox(&running_state) {
                                let recovery_path = staging.preserve();
                                self.running.insert(name.to_string(), sandbox);
                                self.resume_state_recovery
                                    .insert(name.to_string(), running_state);
                                return Err(error).context(format!(
                                    "failed to pause Firecracker sandbox; source resumed on retry, but failed to roll back persisted pause transition ({rollback_error:#}); the live runtime and desired metadata remain under recovery ownership and staging remains at {}; call resume to retry metadata publication",
                                    recovery_path.display()
                                ));
                            }
                            self.running.insert(name.to_string(), sandbox);
                            self.sandboxes.insert(name.to_string(), running_state);
                            self.reconcile_full_state_cleanup();
                            return Err(error).context(
                                "failed to create a complete checkpoint; source resumed on retry",
                            );
                        }
                        Err(retry_error) => {
                            let recovery_path = staging.preserve();
                            self.pause_recovery.insert(
                                name.to_string(),
                                PendingFullStateRecovery {
                                    sandbox: Some(sandbox),
                                    staging_path: recovery_path.clone(),
                                    completed_snapshot: None,
                                },
                            );
                            return Err(error).context(format!(
                                "checkpoint is incomplete and source resume retry failed ({retry_error:#}); source remains paused under manager ownership and diagnostic artifacts are retained at {}; call resume to retry in place",
                                recovery_path.display()
                            ));
                        }
                    }
                }

                // Ordinary backend errors guarantee a confirmed running
                // source. Restore both in-memory and persisted state.
                let running_state = with_full_state_cleanup_intent(state.clone(), staging.id());
                if let Err(rollback_error) = self.save_sandbox(&running_state) {
                    let recovery_path = staging.preserve();
                    self.running.insert(name.to_string(), sandbox);
                    self.resume_state_recovery
                        .insert(name.to_string(), running_state);
                    return Err(error).context(format!(
                        "failed to pause Firecracker sandbox; failed to roll back persisted pause transition ({rollback_error:#}); the live runtime and desired metadata remain under recovery ownership and staging remains at {}; call resume to retry metadata publication",
                        recovery_path.display()
                    ));
                }
                self.running.insert(name.to_string(), sandbox);
                self.sandboxes.insert(name.to_string(), running_state);
                self.reconcile_full_state_cleanup();
                return Err(error).context("failed to pause Firecracker sandbox");
            }
        };

        // The backend has completed all artifacts and terminated the source.
        // Publish a durable ready marker before the manifest rename so a
        // crash in the narrow commit window can be recovered without risking
        // a duplicate of a still-live VM.
        let marker_failure = store
            .mark_recovery_ready(
                &staging,
                name,
                &state.uuid,
                state.vcpus,
                state.memory_mb,
                backend_snapshot.clone(),
            )
            .err()
            .map(|error| format!("{error:#}"));

        let checkpoint = match store.commit(
            &staging,
            name,
            &state.uuid,
            state.vcpus,
            state.memory_mb,
            backend_snapshot.clone(),
        ) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                let marker_context = marker_failure.as_deref().map_or(String::new(), |failure| {
                    format!("; recovery-ready marker also failed: {failure}")
                });
                let retained_kind = if marker_failure.is_some() {
                    "diagnostic"
                } else {
                    "recovery-ready"
                };
                // Snapshot artifacts still live in staging. Restore from them
                // so a publication failure does not destroy a running session.
                match sandbox
                    .restore_from(staging.path(), &backend_snapshot)
                    .await
                {
                    Ok(()) => {
                        let running_state =
                            with_full_state_cleanup_intent(state.clone(), staging.id());
                        if let Err(rollback_error) = self.save_sandbox(&running_state) {
                            let recovery_path = staging.preserve();
                            self.running.insert(name.to_string(), sandbox);
                            self.resume_state_recovery
                                .insert(name.to_string(), running_state);
                            return Err(error).context(format!(
                                "failed to publish checkpoint{marker_context}; source resumed but persisted pause transition rollback failed ({rollback_error:#}); the live runtime and desired metadata remain under recovery ownership and {retained_kind} staging is retained at {}; call resume to retry metadata publication",
                                recovery_path.display()
                            ));
                        }
                        self.running.insert(name.to_string(), sandbox);
                        self.sandboxes.insert(name.to_string(), running_state);
                        self.reconcile_full_state_cleanup();
                        return Err(error).context(format!(
                            "failed to publish checkpoint{marker_context}; the source sandbox was resumed"
                        ));
                    }
                    Err(restore_error) => {
                        let recovery_path = staging.preserve();
                        let sandbox =
                            runtime_may_survive_failed_stop(&restore_error, sandbox.is_running())
                                .then_some(sandbox);
                        self.pause_recovery.insert(
                            name.to_string(),
                            PendingFullStateRecovery {
                                sandbox,
                                staging_path: recovery_path.clone(),
                                completed_snapshot: Some(backend_snapshot),
                            },
                        );
                        return Err(error).context(format!(
                            "failed to publish checkpoint{marker_context} and failed to resume source ({restore_error:#}); safe completed {retained_kind} artifacts retained under recovery ownership at {}",
                            recovery_path.display()
                        ));
                    }
                }
            }
        };
        debug_assert_eq!(
            paused_state.full_state_checkpoint.as_deref(),
            Some(checkpoint.id.as_str())
        );
        crate::metrics::record_sandbox_lifecycle(
            "paused",
            "firecracker",
            pause_start.elapsed().as_secs_f64(),
        );
        Ok(checkpoint)
    }

    /// Resume a paused Firecracker sandbox from its durable checkpoint.
    pub async fn resume(&mut self, name: &str) -> Result<()> {
        self.resume_impl(name, false).await
    }

    pub(crate) async fn resume_authorized(&mut self, name: &str) -> Result<()> {
        self.resume_impl(name, true).await
    }

    async fn resume_impl(&mut self, name: &str, _policy_prechecked: bool) -> Result<()> {
        let resume_start = std::time::Instant::now();
        validation::validate_sandbox_name(name)?;
        let state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        let backend = state.backend.unwrap_or(self.backend);
        if backend != BackendType::Firecracker {
            bail!(
                "Backend '{}' does not support full-state pause/resume; Firecracker on Linux/KVM is required",
                backend
            );
        }
        if state.paused_at.is_none() {
            bail!("Sandbox '{}' is not paused", name);
        }
        if state.archived_at.is_some() || state.dormant_at.is_some() {
            bail!(
                "Sandbox '{}' is archived or dormant; recover it before resuming",
                name
            );
        }
        let checkpoint_id = state
            .full_state_checkpoint
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' has no full-state checkpoint", name))?;
        if state.proxy_port.is_some() || !state.secret_bindings.is_empty() {
            bail!(
                "Sandbox '{}' requires host-side proxy rehydration, which full-state resume does not yet support",
                name
            );
        }

        #[cfg(feature = "enterprise")]
        if !_policy_prechecked {
            self.check_enterprise_policy(crate::policy::Action::Run, name, "unknown", &state.image)
                .await?;
        }

        if let Some(running_state) = self.resume_state_recovery.get(name).cloned() {
            if !self
                .running
                .get(name)
                .is_some_and(|sandbox| sandbox.is_running())
            {
                bail!(
                    "Sandbox '{}' no longer has a confirmed-live runtime for its pending resume-state recovery; remove it before retrying",
                    name
                );
            }
            self.save_sandbox(&running_state).with_context(|| {
                format!("failed to retry running-state publication for sandbox '{name}'")
            })?;
            self.sandboxes.insert(name.to_string(), running_state);
            self.resume_state_recovery.remove(name);
            self.reconcile_full_state_cleanup();
            log_event(AuditEvent::SandboxStarted {
                name: name.to_string(),
                profile: Some("full-state-resume-metadata-recovery".to_string()),
            });
            crate::metrics::record_sandbox_lifecycle(
                "resumed",
                "firecracker",
                resume_start.elapsed().as_secs_f64(),
            );
            return Ok(());
        }
        if self.running.contains_key(name) {
            bail!("Sandbox '{}' is already running", name);
        }

        let store = FullStateCheckpointStore::new(&self.data_dir)?;
        if let Some(mut recovery) = self.pause_recovery.remove(name) {
            if let Some(snapshot) = recovery.completed_snapshot.clone() {
                if let Some(mut sandbox) = recovery.sandbox.take()
                    && let Err(error) = sandbox.stop().await
                {
                    let termination_confirmed = error
                        .downcast_ref::<FullStateTerminationError>()
                        .is_some_and(|failure| !failure.process_may_be_running);
                    if !termination_confirmed {
                        recovery.sandbox = Some(sandbox);
                        self.pause_recovery.insert(name.to_string(), recovery);
                        return Err(error).with_context(|| {
                            format!(
                                "failed to confirm termination of recovery-pending sandbox '{name}'; retry resume or remove"
                            )
                        });
                    }
                    eprintln!(
                        "Warning: source termination was confirmed during recovery, but runtime cleanup failed: {error:#}"
                    );
                }

                let staging = match store.open_staging(checkpoint_id) {
                    Ok(staging) => staging,
                    Err(error) => {
                        self.pause_recovery.insert(name.to_string(), recovery);
                        return Err(error).with_context(|| {
                            format!(
                                "failed to reopen completed checkpoint staging for sandbox '{name}'"
                            )
                        });
                    }
                };
                let marker_error = store
                    .mark_recovery_ready(
                        &staging,
                        name,
                        &state.uuid,
                        state.vcpus,
                        state.memory_mb,
                        snapshot.clone(),
                    )
                    .err();
                if let Err(error) = store.commit(
                    &staging,
                    name,
                    &state.uuid,
                    state.vcpus,
                    state.memory_mb,
                    snapshot,
                ) {
                    let marker_context = marker_error.as_ref().map_or(String::new(), |marker| {
                        format!("; recovery-ready marker also failed: {marker:#}")
                    });
                    self.pause_recovery.insert(name.to_string(), recovery);
                    return Err(error).context(format!(
                        "failed to publish completed checkpoint for sandbox '{name}'{marker_context}; safe staging remains at {}",
                        staging.path().display()
                    ));
                }
            } else {
                let Some(mut sandbox) = recovery.sandbox.take() else {
                    self.pause_recovery.insert(name.to_string(), recovery);
                    bail!(
                        "Sandbox '{}' has invalid partial recovery ownership without a source runtime",
                        name
                    );
                };
                if let Err(error) = sandbox.retry_full_state_resume().await {
                    recovery.sandbox = Some(sandbox);
                    self.pause_recovery.insert(name.to_string(), recovery);
                    return Err(error).with_context(|| {
                        format!("failed to resume recovery-pending sandbox '{name}' in place")
                    });
                }

                let mut running_state = state.clone();
                running_state.full_state_checkpoint = None;
                if !running_state
                    .full_state_cleanup_pending
                    .iter()
                    .any(|id| id == checkpoint_id)
                {
                    running_state
                        .full_state_cleanup_pending
                        .push(checkpoint_id.to_string());
                }
                running_state.paused_at = None;
                running_state.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
                if let Err(error) = self.save_sandbox(&running_state) {
                    self.running.insert(name.to_string(), sandbox);
                    self.resume_state_recovery
                        .insert(name.to_string(), running_state);
                    return Err(error).context(format!(
                        "sandbox '{name}' resumed in place, but its running state could not be persisted; the runtime and desired metadata remain under recovery ownership, and diagnostic artifacts remain at {}; call resume to retry metadata publication",
                        recovery.staging_path.display()
                    ));
                }
                self.running.insert(name.to_string(), sandbox);
                self.sandboxes.insert(name.to_string(), running_state);
                self.reconcile_full_state_cleanup();
                log_event(AuditEvent::SandboxStarted {
                    name: name.to_string(),
                    profile: Some("full-state-in-place-recovery".to_string()),
                });
                crate::metrics::record_sandbox_lifecycle(
                    "resumed",
                    "firecracker",
                    resume_start.elapsed().as_secs_f64(),
                );
                return Ok(());
            }
        }

        let (checkpoint, checkpoint_path) = if store.contains(checkpoint_id)? {
            store.load(checkpoint_id)?
        } else {
            let staging_path = store.staging_path(checkpoint_id)?;
            if staging_path.is_dir() {
                if store.recovery_is_ready(checkpoint_id)? {
                    store
                        .recover_ready(
                            checkpoint_id,
                            name,
                            &state.uuid,
                            state.vcpus,
                            state.memory_mb,
                        )
                        .with_context(|| {
                            format!(
                                "failed to publish recovery-ready checkpoint for sandbox '{name}'"
                            )
                        })?
                } else {
                    bail!(
                        "Sandbox '{}' has an interrupted pause transition at {} that cannot be restored automatically; keep these artifacts for operator recovery",
                        name,
                        staging_path.display()
                    );
                }
            } else {
                bail!(
                    "Sandbox '{}' references missing full-state checkpoint '{}'",
                    name,
                    checkpoint_id
                );
            }
        };
        checkpoint.validate_source(name, &state.uuid)?;
        if checkpoint.vcpus != state.vcpus || checkpoint.memory_mb != state.memory_mb {
            bail!(
                "Sandbox '{}' resources no longer match its checkpoint",
                name
            );
        }
        let mut sandbox = create_sandbox(BackendType::Firecracker, name)?;
        if let Err(error) = sandbox
            .restore_from(&checkpoint_path, &checkpoint.backend_snapshot)
            .await
        {
            if runtime_may_survive_failed_stop(&error, sandbox.is_running()) {
                self.pause_recovery.insert(
                    name.to_string(),
                    PendingFullStateRecovery {
                        sandbox: Some(sandbox),
                        staging_path: checkpoint_path.clone(),
                        completed_snapshot: None,
                    },
                );
                return Err(error).context(format!(
                    "failed to resume sandbox '{name}', and restore cleanup could not prove the new VMM exited; it remains under recovery ownership while the durable checkpoint stays paused; call resume to retry in place or remove to abandon it"
                ));
            }
            return Err(error).with_context(|| format!("failed to resume sandbox '{name}'"));
        }

        let mut running_state = state.clone();
        running_state.full_state_checkpoint = None;
        running_state.firecracker_rootfs = None;
        if !running_state
            .full_state_cleanup_pending
            .iter()
            .any(|id| id == checkpoint_id)
        {
            running_state
                .full_state_cleanup_pending
                .push(checkpoint_id.to_string());
        }
        running_state.paused_at = None;
        running_state.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
        running_state.labels.insert(
            "agentkernel.full-state-lineage".to_string(),
            "true".to_string(),
        );
        running_state.full_state_lineage = true;
        if let Err(error) = self.save_sandbox(&running_state) {
            if let Err(stop_error) = sandbox.stop().await {
                if runtime_may_survive_failed_stop(&stop_error, sandbox.is_running()) {
                    // Fail closed: retain ownership of an ambiguously live
                    // runtime, but keep the persisted paused state and its
                    // checkpoint visible for operator recovery.
                    self.running.insert(name.to_string(), sandbox);
                    self.resume_state_recovery
                        .insert(name.to_string(), running_state);
                    return Err(error).context(format!(
                        "failed to persist resumed state and failed to stop the restored runtime ({stop_error:#}); an ambiguously live runtime and its desired metadata remain under recovery ownership while the sandbox stays conservatively paused; call resume to retry metadata publication"
                    ));
                }
                return Err(error).context(format!(
                    "failed to persist resumed state; the restored runtime exited, but cleanup also failed ({stop_error:#}); the checkpoint remains available"
                ));
            }
            return Err(error).context(
                "failed to persist resumed state; restored runtime was stopped and the checkpoint remains available",
            );
        }

        self.sandboxes.insert(name.to_string(), running_state);
        self.running.insert(name.to_string(), sandbox);
        self.reconcile_full_state_cleanup();
        log_event(AuditEvent::SandboxStarted {
            name: name.to_string(),
            profile: Some("full-state-resume".to_string()),
        });
        crate::metrics::record_sandbox_lifecycle(
            "resumed",
            "firecracker",
            resume_start.elapsed().as_secs_f64(),
        );
        Ok(())
    }

    /// Fork a paused Firecracker sandbox into a running child.
    pub async fn fork_sandbox(&mut self, source: &str, child: &str) -> Result<()> {
        self.fork_sandbox_impl(source, child, false).await
    }

    pub(crate) async fn fork_sandbox_authorized(
        &mut self,
        source: &str,
        child: &str,
    ) -> Result<()> {
        self.fork_sandbox_impl(source, child, true).await
    }

    async fn fork_sandbox_impl(
        &mut self,
        source: &str,
        child: &str,
        _policy_prechecked: bool,
    ) -> Result<()> {
        let fork_start = std::time::Instant::now();
        validation::validate_sandbox_name(source)?;
        validation::validate_sandbox_name(child)?;
        if source == child {
            bail!("Fork child name must differ from the source sandbox");
        }
        if self.sandboxes.contains_key(child) {
            bail!("Sandbox '{}' already exists", child);
        }
        if self.pause_recovery.contains_key(source)
            || self.resume_state_recovery.contains_key(source)
        {
            bail!(
                "Sandbox '{}' has pending full-state recovery; resume or remove it before forking",
                source
            );
        }
        let source_state = self
            .sandboxes
            .get(source)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", source))?;
        let backend = source_state.backend.unwrap_or(self.backend);
        if backend != BackendType::Firecracker {
            bail!(
                "Backend '{}' does not support full-state fork; Firecracker on Linux/KVM is required",
                backend
            );
        }
        if source_state.paused_at.is_none() {
            bail!(
                "Sandbox '{}' must be paused before it can be forked",
                source
            );
        }
        if source_state.archived_at.is_some() || source_state.dormant_at.is_some() {
            bail!(
                "Sandbox '{}' is archived or dormant; recover it before forking",
                source
            );
        }
        if source_state.proxy_port.is_some() || !source_state.secret_bindings.is_empty() {
            bail!(
                "Sandbox '{}' uses a host-side proxy whose live state cannot yet be forked",
                source
            );
        }
        let checkpoint_id = source_state
            .full_state_checkpoint
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' has no full-state checkpoint", source))?;

        #[cfg(feature = "enterprise")]
        if !_policy_prechecked {
            self.check_enterprise_policy(
                crate::policy::Action::Run,
                source,
                "unknown",
                &source_state.image,
            )
            .await?;
            self.check_enterprise_policy(
                crate::policy::Action::Create,
                child,
                "unknown",
                &source_state.image,
            )
            .await?;
            self.check_enterprise_policy(
                crate::policy::Action::Run,
                child,
                "unknown",
                &source_state.image,
            )
            .await?;
        }

        let store = FullStateCheckpointStore::new(&self.data_dir)?;
        let (checkpoint, checkpoint_path) = if store.contains(checkpoint_id)? {
            store.load(checkpoint_id)?
        } else {
            let staging_path = store.staging_path(checkpoint_id)?;
            if staging_path.is_dir() && store.recovery_is_ready(checkpoint_id)? {
                store
                    .recover_ready(
                        checkpoint_id,
                        source,
                        &source_state.uuid,
                        source_state.vcpus,
                        source_state.memory_mb,
                    )
                    .with_context(|| {
                        format!(
                            "failed to publish recovery-ready checkpoint for sandbox '{source}'"
                        )
                    })?
            } else {
                bail!(
                    "Sandbox '{}' has no published, recovery-ready full-state checkpoint",
                    source
                );
            }
        };
        checkpoint.validate_source(source, &source_state.uuid)?;
        if checkpoint.vcpus != source_state.vcpus || checkpoint.memory_mb != source_state.memory_mb
        {
            bail!(
                "Sandbox '{}' resources no longer match its checkpoint",
                source
            );
        }
        let now = chrono::Utc::now();
        let mut child_state = source_state.clone();
        child_state.name = child.to_string();
        child_state.uuid = uuid::Uuid::now_v7().to_string();
        child_state.vsock_cid = self.next_cid;
        self.next_cid += 1;
        child_state.created_at = now.to_rfc3339();
        child_state.expires_at = child_state
            .ttl_seconds
            .map(|ttl| (now + chrono::Duration::seconds(ttl as i64)).to_rfc3339());
        child_state.remote_id = None;
        child_state.remote_namespace = None;
        child_state.remote_metadata.clear();
        child_state.workspace_revision = None;
        child_state.endpoints.clear();
        child_state.work_dir = None;
        child_state.container_work_dir = None;
        child_state.git_worktree = None;
        child_state.ports.clear();
        child_state.ssh_enabled = false;
        child_state.ssh_host_port = None;
        child_state.volumes.clear();
        child_state.proxy_port = None;
        child_state.full_state_checkpoint = None;
        child_state.firecracker_rootfs = None;
        child_state.paused_at = None;
        child_state.forked_from = Some(source.to_string());
        child_state.archived_at = None;
        child_state.archived_reason = None;
        child_state.dormant_at = None;
        child_state.dormant_reason = None;
        child_state.last_activity_at = Some(now.to_rfc3339());
        child_state
            .labels
            .insert("agentkernel.forked-from".to_string(), source.to_string());
        child_state.labels.insert(
            "agentkernel.full-state-lineage".to_string(),
            "true".to_string(),
        );
        child_state.full_state_lineage = true;

        let mut sandbox = create_sandbox(BackendType::Firecracker, child)?;
        if let Err(error) = sandbox
            .restore_from(&checkpoint_path, &checkpoint.backend_snapshot)
            .await
        {
            if runtime_may_survive_failed_stop(&error, sandbox.is_running()) {
                let persistence_error = self.save_sandbox(&child_state).err();
                self.sandboxes
                    .insert(child.to_string(), child_state.clone());
                self.pause_recovery.insert(
                    child.to_string(),
                    PendingFullStateRecovery {
                        sandbox: Some(sandbox),
                        // This is diagnostic ownership only. The published
                        // source checkpoint is never consumed by child removal.
                        staging_path: checkpoint_path.clone(),
                        completed_snapshot: None,
                    },
                );
                crate::metrics::inc_active_sandboxes();
                let persistence_context =
                    persistence_error.as_ref().map_or(String::new(), |persist| {
                        format!("; transitional child state also failed to persist: {persist:#}")
                    });
                return Err(error).context(format!(
                    "failed to restore fork '{child}' from sandbox '{source}', and cleanup could not prove the child VMM exited{persistence_context}; the child remains under recovery ownership and must be removed before retrying"
                ));
            }
            return Err(error).with_context(|| {
                format!(
                    "failed to restore fork '{}' from sandbox '{}'",
                    child, source
                )
            });
        }

        // Publish a normal child only after its runtime has restored. If the
        // atomic state write fails, stop the unadvertised runtime so callers
        // never observe a persisted fork that did not actually start.
        if let Err(error) = self.save_sandbox(&child_state) {
            if let Err(stop_error) = sandbox.stop().await {
                if runtime_may_survive_failed_stop(&stop_error, sandbox.is_running()) {
                    self.sandboxes
                        .insert(child.to_string(), child_state.clone());
                    self.running.insert(child.to_string(), sandbox);
                    crate::metrics::inc_active_sandboxes();
                    return Err(error).context(format!(
                        "failed to persist restored fork '{}'; failed to stop an ambiguously live runtime ({stop_error:#}), so it remains under manager ownership and must be explicitly removed",
                        child
                    ));
                }
                return Err(error).context(format!(
                    "failed to persist restored fork '{}'; its runtime exited, but cleanup also failed ({stop_error:#})",
                    child
                ));
            }
            return Err(error)
                .with_context(|| format!("failed to persist restored fork '{child}'"));
        }
        self.sandboxes
            .insert(child.to_string(), child_state.clone());
        self.running.insert(child.to_string(), sandbox);

        log_event(AuditEvent::SandboxCreated {
            name: child.to_string(),
            image: child_state.image.clone(),
            backend: "firecracker".to_string(),
            labels: child_state.labels.clone(),
        });
        log_event(AuditEvent::SandboxStarted {
            name: child.to_string(),
            profile: Some(format!("fork-of:{source}")),
        });
        crate::metrics::inc_active_sandboxes();
        crate::metrics::record_sandbox_lifecycle(
            "forked",
            "firecracker",
            fork_start.elapsed().as_secs_f64(),
        );
        Ok(())
    }

    /// Stop a sandbox
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        let stop_start = std::time::Instant::now();
        let backend = self
            .sandboxes
            .get(name)
            .and_then(|state| state.backend)
            .unwrap_or(self.backend);
        if backend == BackendType::Firecracker
            && self.running.contains_key(name)
            && self
                .sandboxes
                .get(name)
                .is_some_and(|state| state.full_state_lineage)
        {
            bail!(
                "Refusing to stop full-state Firecracker sandbox '{}': ordinary stop would discard its writable disk; use full-state pause to preserve it or remove to discard it",
                name
            );
        }
        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        // Shut down the proxy if running
        if let Some(handle) = PROXY_HANDLES.write().await.remove(name) {
            let _ = handle.shutdown_tx.send(());
        }
        // A recovery-pending source is already paused. Keep its exact backend
        // handle alive so `resume` can retry in place; `remove` is the explicit
        // operation that abandons it.
        if self.pause_recovery.contains_key(name) || self.resume_state_recovery.contains_key(name) {
            return Ok(());
        }
        if let Some(mut sandbox) = self.running.remove(name) {
            if let Err(error) = sandbox.stop().await {
                if runtime_may_survive_failed_stop(&error, sandbox.is_running()) {
                    self.running.insert(name.to_string(), sandbox);
                }
                return Err(error).with_context(|| format!("failed to stop sandbox '{name}'"));
            }
            if let Some(metadata) = sandbox.runtime_metadata()
                && let Some(state) = self.sandboxes.get_mut(name)
            {
                Self::apply_runtime_metadata(state, &metadata);
                let snapshot = state.clone();
                self.save_sandbox(&snapshot)?;
            }
            if backend == BackendType::Firecracker
                && let Some(state) = self.sandboxes.get_mut(name)
            {
                state.firecracker_rootfs = sandbox.persistent_disk_reference();
                let snapshot = state.clone();
                self.save_sandbox(&snapshot)?;
            }
            log_event(AuditEvent::SandboxStopped {
                name: name.to_string(),
            });
            crate::metrics::record_sandbox_lifecycle(
                "stopped",
                &backend.to_string(),
                stop_start.elapsed().as_secs_f64(),
            );
        }
        Ok(())
    }

    /// Remove a sandbox
    pub async fn remove(&mut self, name: &str) -> Result<()> {
        let remove_start = std::time::Instant::now();
        let backend = self
            .sandboxes
            .get(name)
            .and_then(|state| state.backend)
            .unwrap_or(self.backend);
        if backend == BackendType::Firecracker {
            self.ensure_unique_firecracker_rootfs_reference(
                name,
                self.sandboxes
                    .get(name)
                    .and_then(|state| state.firecracker_rootfs.as_deref()),
            )?;
        }
        // Preflight before stopping/removing the backend. A dirty managed
        // checkout must leave the running sandbox and its persisted state
        // untouched so the agent's work remains accessible for cleanup.
        if let Some(state) = self.sandboxes.get(name).cloned()
            && let Err(error) = self.ensure_clean_git_worktree(&state)
        {
            return Err(error).with_context(|| format!("refusing to remove sandbox '{name}'"));
        }
        // Shut down the proxy if running
        if let Some(handle) = PROXY_HANDLES.write().await.remove(name) {
            let _ = handle.shutdown_tx.send(());
        }
        #[cfg(test)]
        let bypass_backend_runtime = self.bypass_backend_runtime;
        #[cfg(not(test))]
        let bypass_backend_runtime = false;

        if let Some(mut recovery) = self.pause_recovery.remove(name) {
            if let Some(mut sandbox) = recovery.sandbox.take()
                && let Err(error) = sandbox.remove().await
            {
                recovery.sandbox = Some(sandbox);
                self.pause_recovery.insert(name.to_string(), recovery);
                return Err(error).with_context(|| {
                    format!("failed to abandon recovery-pending sandbox '{name}'")
                });
            }
        } else if let Some(mut sandbox) = self.running.remove(name) {
            if let Err(error) = sandbox.remove().await {
                self.running.insert(name.to_string(), sandbox);
                return Err(error).with_context(|| format!("failed to remove sandbox '{name}'"));
            }
        } else if !bypass_backend_runtime && let Some(state) = self.sandboxes.get(name).cloned() {
            let mut sandbox = {
                #[cfg(test)]
                if let Some(factory) = &self.test_backend_factory {
                    factory(name, backend)?
                } else {
                    create_sandbox_with_state(
                        backend,
                        name,
                        &crate::config::OrchestratorConfig::default(),
                        backend.is_remote().then(|| state.remote_context()),
                    )?
                }
                #[cfg(not(test))]
                {
                    create_sandbox_with_state(
                        backend,
                        name,
                        &crate::config::OrchestratorConfig::default(),
                        backend.is_remote().then(|| state.remote_context()),
                    )?
                }
            };
            if backend == BackendType::Firecracker {
                sandbox.set_persistent_disk_reference(state.firecracker_rootfs.as_deref())?;
            }
            sandbox
                .remove()
                .await
                .with_context(|| format!("failed to remove sandbox '{name}'"))?;
        }

        // Remove only the AgentKernel-owned checkout. Keep its dedicated
        // branch so any commits made by the agent remain recoverable.
        if let Some(state) = self.sandboxes.get(name).cloned()
            && let Err(error) = self.remove_git_worktree(&state)
        {
            return Err(error)
                .with_context(|| format!("failed to clean up Git worktree for sandbox '{name}'"));
        }

        if let Some(state) = self.sandboxes.get(name).cloned() {
            if let Some(network) = state.managed_network.as_ref() {
                crate::backend::NetworkAllocator::new(self.data_dir.clone())
                    .release(name, network)?;
            }

            let mut checkpoint_ids = state.full_state_cleanup_pending;
            if let Some(checkpoint_id) = state.full_state_checkpoint {
                checkpoint_ids.push(checkpoint_id);
            }
            checkpoint_ids.sort();
            checkpoint_ids.dedup();
            if !checkpoint_ids.is_empty() {
                let store = FullStateCheckpointStore::new(&self.data_dir)?;
                for checkpoint_id in checkpoint_ids {
                    Self::delete_full_state_artifacts(&store, &checkpoint_id).with_context(|| {
                        format!(
                            "failed to delete full-state artifacts '{}' for sandbox '{}' after its runtime was removed; retry removal to finish cleanup",
                            checkpoint_id, name
                        )
                    })?;
                }
            }
        }

        self.delete_sandbox(name)?;
        self.sandboxes.remove(name);
        self.resume_state_recovery.remove(name);

        log_event(AuditEvent::SandboxRemoved {
            name: name.to_string(),
        });
        crate::metrics::record_sandbox_lifecycle(
            "removed",
            &backend.to_string(),
            remove_start.elapsed().as_secs_f64(),
        );
        crate::metrics::dec_active_sandboxes();
        crate::llm_intercept::LLM_USAGE
            .write()
            .await
            .clear_sandbox(name);
        crate::llm_intercept::clear_sandbox_metadata(name);

        Ok(())
    }

    /// Reconcile lifecycle automation policies across all sandboxes.
    ///
    /// This applies inactivity/archive/delete policies declared on each sandbox.
    /// Use `dry_run=true` to preview actions without mutating state.
    pub async fn reconcile_lifecycle(&mut self, dry_run: bool) -> Result<LifecycleReconcileResult> {
        #[derive(Debug, Clone, Copy)]
        enum DecisionKind {
            Stop,
            Archive,
            Delete,
        }

        #[derive(Debug, Clone)]
        struct Decision {
            sandbox: String,
            kind: DecisionKind,
            reason: String,
        }

        let now = chrono::Utc::now();
        let mut decisions: Vec<Decision> = Vec::new();

        for (name, state) in &self.sandboxes {
            let Some(policy) = state.lifecycle_policy.as_ref() else {
                continue;
            };

            if let Some(archived_time) = state.archived_time() {
                if let Some(delete_after) = policy.auto_delete_after_seconds {
                    let archived_secs = now.signed_duration_since(archived_time).num_seconds();
                    if archived_secs >= delete_after as i64 {
                        decisions.push(Decision {
                            sandbox: name.clone(),
                            kind: DecisionKind::Delete,
                            reason: format!(
                                "archived for {}s (threshold={}s)",
                                archived_secs, delete_after
                            ),
                        });
                    }
                }
                continue;
            }

            let inactivity_secs = state
                .last_activity_time()
                .map(|ts| now.signed_duration_since(ts).num_seconds().max(0) as u64)
                .unwrap_or(0);

            if let Some(archive_after) = policy.auto_archive_after_seconds
                && inactivity_secs >= archive_after
            {
                decisions.push(Decision {
                    sandbox: name.clone(),
                    kind: DecisionKind::Archive,
                    reason: format!(
                        "inactive for {}s (threshold={}s)",
                        inactivity_secs, archive_after
                    ),
                });
                continue;
            }

            if let Some(stop_after) = policy.auto_stop_after_seconds
                && inactivity_secs >= stop_after
                && self.is_running(name)
            {
                decisions.push(Decision {
                    sandbox: name.clone(),
                    kind: DecisionKind::Stop,
                    reason: format!(
                        "inactive for {}s (threshold={}s)",
                        inactivity_secs, stop_after
                    ),
                });
            }
        }

        // Stable order for predictable dry-run and testability.
        decisions.sort_by(|a, b| {
            a.sandbox
                .cmp(&b.sandbox)
                .then_with(|| a.reason.cmp(&b.reason))
        });

        let mut result = LifecycleReconcileResult {
            dry_run,
            ..Default::default()
        };

        for decision in decisions {
            let action_name = match decision.kind {
                DecisionKind::Stop => "stop",
                DecisionKind::Archive => "archive",
                DecisionKind::Delete => "delete",
            }
            .to_string();

            result.actions.push(LifecycleAction {
                sandbox: decision.sandbox.clone(),
                action: action_name,
                reason: decision.reason.clone(),
            });

            match decision.kind {
                DecisionKind::Stop => {
                    if !dry_run && self.is_running(&decision.sandbox) {
                        self.stop(&decision.sandbox).await?;
                    }
                    result.stopped.push(decision.sandbox);
                }
                DecisionKind::Archive => {
                    if !dry_run {
                        if self.is_running(&decision.sandbox) {
                            self.stop(&decision.sandbox).await?;
                        }
                        if let Some(mut state) = self.sandboxes.get(&decision.sandbox).cloned() {
                            state.archived_at = Some(now.to_rfc3339());
                            state.archived_reason = Some(decision.reason.clone());
                            self.save_sandbox(&state)?;
                            self.sandboxes.insert(decision.sandbox.clone(), state);
                        }
                    }
                    result.archived.push(decision.sandbox);
                }
                DecisionKind::Delete => {
                    if !dry_run {
                        self.remove(&decision.sandbox).await?;
                    }
                    result.removed.push(decision.sandbox);
                }
            }
        }

        Ok(result)
    }

    /// Return names of sandboxes that have expired (past their TTL).
    pub fn expired(&self) -> Vec<String> {
        let now = chrono::Utc::now();
        self.sandboxes
            .iter()
            .filter_map(|(name, state)| {
                if let Some(ref exp) = state.expires_at
                    && let Ok(dt) = chrono::DateTime::parse_from_rfc3339(exp)
                    && dt < now
                {
                    return Some(name.clone());
                }
                None
            })
            .collect()
    }

    /// Return names of sandboxes matching all given label key=value pairs.
    pub fn list_matching_labels(&self, filters: &[(String, String)]) -> Vec<String> {
        self.sandboxes
            .iter()
            .filter(|(_, state)| {
                filters
                    .iter()
                    .all(|(k, v)| state.labels.get(k).map(|lv| lv == v).unwrap_or(false))
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Garbage-collect expired sandboxes. Returns names of removed sandboxes.
    pub async fn gc(&mut self) -> Result<Vec<String>> {
        let expired = self.expired();
        let mut removed = Vec::new();
        for name in expired {
            self.remove(&name).await?;
            removed.push(name);
        }
        Ok(removed)
    }

    /// List all sandboxes (persisted, with running status and backend)
    pub fn list(&self) -> Vec<(&str, bool, Option<BackendType>)> {
        self.sandboxes
            .iter()
            .map(|(name, state)| {
                let running = self.is_running(name) || {
                    state.backend.is_some_and(|backend| backend.is_remote())
                        && state
                            .remote_metadata
                            .get("last_known_status")
                            .is_some_and(|value| value == "running")
                };
                (name.as_str(), running, state.backend)
            })
            .collect()
    }

    /// Check if a sandbox exists
    pub fn exists(&self, name: &str) -> bool {
        self.sandboxes.contains_key(name)
    }

    /// Import a sandbox that another AgentKernel process persisted after this
    /// manager started.
    ///
    /// The long-lived HTTP service uses this narrow refresh path when a CLI
    /// creates a Firecracker sandbox and then delegates runtime ownership to
    /// the service. Existing entries are never overwritten, so live runtime
    /// state cannot be replaced by a stale file.
    pub fn refresh_sandbox_if_missing(&mut self, name: &str) -> Result<bool> {
        validation::validate_sandbox_name(name)?;
        if self.sandboxes.contains_key(name) {
            return Ok(false);
        }

        let Some(mut state) = self.read_persisted_sandbox(name)? else {
            return Ok(false);
        };
        if state.uuid.is_empty() {
            state.uuid = uuid::Uuid::now_v7().to_string();
            self.save_sandbox(&state)?;
        }
        self.sandboxes.insert(name.to_string(), state);
        Ok(true)
    }

    /// Reload a persisted sandbox only when no runtime or recovery transition
    /// is owned by this manager. This is the second half of the CLI-to-daemon
    /// start handoff: a one-shot manifest binds the exact disk state, then the
    /// daemon atomically adopts that final configuration before starting it.
    pub fn refresh_stopped_sandbox_from_disk(&mut self, name: &str) -> Result<bool> {
        validation::validate_sandbox_name(name)?;
        if self.running.contains_key(name)
            || self.pause_recovery.contains_key(name)
            || self.resume_state_recovery.contains_key(name)
        {
            bail!(
                "Sandbox '{}' owns a runtime or full-state recovery and cannot be refreshed from disk",
                name
            );
        }
        let Some(state) = self.read_persisted_sandbox(name)? else {
            return Ok(false);
        };
        if state.uuid.is_empty() {
            bail!("persisted sandbox '{}' has no generation UUID", name);
        }
        if let Some(current) = self.sandboxes.get(name)
            && current.uuid != state.uuid
        {
            bail!(
                "sandbox generation changed while refreshing '{}': daemon={}, disk={}",
                name,
                current.uuid,
                state.uuid
            );
        }
        self.sandboxes.insert(name.to_string(), state);
        Ok(true)
    }

    fn read_persisted_sandbox(&self, name: &str) -> Result<Option<SandboxState>> {
        let path = self.data_dir.join("sandboxes").join(format!("{name}.json"));
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read sandbox state {}", path.display()));
            }
        };
        let state: SandboxState = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse sandbox state {}", path.display()))?;
        if state.name != name {
            bail!(
                "sandbox state identity mismatch: requested '{}' but {} contains '{}'",
                name,
                path.display(),
                state.name
            );
        }
        Ok(Some(state))
    }

    /// Get the backend type for a sandbox (from stored state or current default)
    /// Check if a sandbox is currently running
    pub fn is_running(&self, name: &str) -> bool {
        self.running
            .get(name)
            .map(|s| s.is_running())
            .unwrap_or(false)
            || self
                .pause_recovery
                .get(name)
                .and_then(|recovery| recovery.sandbox.as_ref())
                // A recovery-owned handle is charged conservatively: its
                // typed cleanup failure means termination is not yet proved.
                .is_some()
            || {
                self.sandboxes.get(name).is_some_and(|state| {
                    state.backend.is_some_and(|backend| backend.is_remote())
                        && state
                            .remote_metadata
                            .get("last_known_status")
                            .is_some_and(|value| value == "running")
                })
            }
    }

    /// Update persisted sandbox resource values without recreating.
    pub fn update_resources(&mut self, name: &str, vcpus: u32, memory_mb: u64) -> Result<()> {
        if self.pause_recovery.contains_key(name) || self.resume_state_recovery.contains_key(name) {
            bail!(
                "Sandbox '{}' has a pending full-state recovery; resume or remove it before changing resources",
                name
            );
        }
        let mut state = self
            .sandboxes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        if state.paused_at.is_some() || state.full_state_checkpoint.is_some() {
            bail!(
                "Sandbox '{}' is paused with a full-state checkpoint; resume or remove it before changing resources",
                name
            );
        }
        state.vcpus = vcpus;
        state.memory_mb = memory_mb;
        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);
        Ok(())
    }

    /// Attempt an in-place resize of a running sandbox.
    ///
    /// Returns `Ok(true)` if resized in-place, `Ok(false)` if backend does not
    /// support live resize or sandbox is not running.
    pub async fn try_resize_in_place(
        &mut self,
        name: &str,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<bool> {
        let Some(sandbox) = self.running.get_mut(name) else {
            return Ok(false);
        };
        if sandbox.resize(vcpus, memory_mb).await? {
            self.update_resources(name, vcpus, memory_mb)?;
            let _ = self.touch_activity(name);
            return Ok(true);
        }
        Ok(false)
    }

    /// Get the current backend
    #[allow(dead_code)]
    pub fn backend(&self) -> BackendType {
        self.backend
    }

    /// Get a reference to the global proxy handles registry.
    pub fn proxy_handles_registry() -> &'static RwLock<HashMap<String, ProxyHandle>> {
        &PROXY_HANDLES
    }

    /// Run a command using the container pool (fast path for ephemeral runs)
    pub async fn run_pooled(cmd: &[String]) -> Result<String> {
        Self::enforce_command_policy(cmd)?;
        let pool = get_pool().await?;
        let container = pool.acquire().await?;
        let exec_start = std::time::Instant::now();
        let result = container.run_command(cmd).await;
        pool.release(container).await;
        crate::metrics::record_command("pool", exec_start.elapsed().as_secs_f64());
        result
    }

    /// Check if pooled execution is available
    #[allow(dead_code)]
    pub fn pool_available() -> bool {
        detect_container_runtime().is_some()
    }

    /// Run a command in an ephemeral sandbox (optimized single-operation path)
    #[allow(dead_code)]
    pub async fn run_ephemeral(
        &mut self,
        image: &str,
        cmd: &[String],
        perms: &Permissions,
    ) -> Result<String> {
        self.run_ephemeral_with_files(image, cmd, perms, &[]).await
    }

    /// Run a command in an ephemeral sandbox with file injection
    pub async fn run_ephemeral_with_files(
        &mut self,
        image: &str,
        cmd: &[String],
        perms: &Permissions,
        files: &[FileInjection],
    ) -> Result<String> {
        Self::run_ephemeral_with_backend(self.backend, image, cmd, perms, files).await
    }

    pub async fn run_ephemeral_with_backend(
        backend: BackendType,
        image: &str,
        cmd: &[String],
        perms: &Permissions,
        files: &[FileInjection],
    ) -> Result<String> {
        Self::enforce_command_policy(cmd)?;
        let work_dir = if perms.mount_cwd {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        let env = if perms.pass_env {
            ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"]
                .iter()
                .filter_map(|&var| std::env::var(var).ok().map(|val| (var.to_string(), val)))
                .collect()
        } else {
            Vec::new()
        };

        let config = SandboxConfig {
            image: image.to_string(),
            vcpus: 1,
            memory_mb: perms.max_memory_mb.unwrap_or(512),
            mount_cwd: perms.mount_cwd,
            work_dir,
            container_work_dir: None,
            env,
            network: perms.network,
            read_only: perms.read_only_root,
            mount_home: perms.mount_home,
            files: files.to_vec(),
            ports: Vec::new(),
            managed_network: None,
            ssh: None,
            volumes: Vec::new(),
        };

        if files.is_empty() {
            match backend {
                BackendType::Docker => {
                    use crate::docker_backend::{ContainerRuntime, ContainerSandbox};
                    let (exit_code, stdout, stderr) = ContainerSandbox::run_ephemeral_cmd(
                        ContainerRuntime::Docker,
                        image,
                        cmd,
                        perms,
                    )?;
                    if exit_code != 0 {
                        bail!("Command failed (exit {}): {}{}", exit_code, stdout, stderr);
                    }
                    return Ok(format!("{}{}", stdout, stderr));
                }
                BackendType::Podman => {
                    use crate::docker_backend::{ContainerRuntime, ContainerSandbox};
                    let (exit_code, stdout, stderr) = ContainerSandbox::run_ephemeral_cmd(
                        ContainerRuntime::Podman,
                        image,
                        cmd,
                        perms,
                    )?;
                    if exit_code != 0 {
                        bail!("Command failed (exit {}): {}{}", exit_code, stdout, stderr);
                    }
                    return Ok(format!("{}{}", stdout, stderr));
                }
                #[cfg(target_os = "macos")]
                BackendType::Apple => {
                    let result =
                        crate::backend::apple::AppleSandbox::run_ephemeral_cmd(cmd, &config)?;
                    if result.exit_code != 0 {
                        bail!(
                            "Command failed (exit {}): {}{}",
                            result.exit_code,
                            result.stdout,
                            result.stderr
                        );
                    }
                    return Ok(result.output());
                }
                _ => {}
            }
        }

        let name = format!("ephemeral-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let mut sandbox = create_sandbox(backend, &name)?;
        sandbox.start(&config).await?;

        if !files.is_empty() {
            sandbox.inject_files(files).await?;
        }

        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        let result = sandbox.exec(&cmd_refs).await;
        let _ = sandbox.stop().await;

        let result = result?;
        if !result.is_success() {
            bail!("Command failed: {}", result.output());
        }

        Ok(result.output())
    }

    /// Get pool statistics (for debugging/monitoring)
    #[allow(dead_code)]
    pub async fn pool_stats() -> Option<crate::pool::PoolStats> {
        CONTAINER_POOL.get().map(|pool| {
            // Use blocking because stats() is async
            tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(pool.stats()))
        })
    }

    /// Write a file to a running sandbox
    pub async fn write_file(&mut self, name: &str, path: &str, content: &[u8]) -> Result<()> {
        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        sandbox.write_file(path, content).await?;
        if let Err(e) = self.sync_runtime_metadata(name) {
            eprintln!(
                "Warning: failed to sync remote metadata for '{}': {}",
                name, e
            );
        }

        log_event(AuditEvent::FileWritten {
            sandbox: name.to_string(),
            path: path.to_string(),
        });

        let _ = self.touch_activity(name);

        Ok(())
    }

    /// Get the stored state for a sandbox
    pub fn get_state(&self, name: &str) -> Option<&SandboxState> {
        self.sandboxes.get(name)
    }

    /// Get the stored state for a sandbox by UUID.
    pub fn get_state_by_uuid(&self, uuid: &str) -> Option<&SandboxState> {
        self.sandboxes.values().find(|state| state.uuid == uuid)
    }

    /// Get a reference to the sandbox state (alias for get_state).
    ///
    /// Used by the SSH command to read ssh_enabled and ssh_host_port.
    #[allow(dead_code)]
    pub fn get_sandbox_state(&self, name: &str) -> Option<&SandboxState> {
        self.sandboxes.get(name)
    }

    /// Get the IP address of a running sandbox.
    pub fn get_container_ip(&self, name: &str) -> Option<String> {
        let container_name = format!("agentkernel-{}", name);
        let backend = self.sandboxes.get(name).and_then(|s| s.backend);
        match backend {
            #[cfg(target_os = "macos")]
            Some(BackendType::Apple) => crate::backend::apple::get_container_ip(&container_name),
            Some(BackendType::Podman) => crate::backend::docker::get_container_ip_with_runtime(
                crate::backend::ContainerRuntime::Podman,
                &container_name,
            ),
            _ => crate::backend::docker::get_container_ip(&container_name),
        }
    }

    /// Get the data directory path.
    ///
    /// Used by the SSH command to locate the stored CA private key.
    #[allow(dead_code)]
    pub fn get_data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Delete a file from a running sandbox
    pub async fn delete_file(&mut self, name: &str, path: &str) -> Result<()> {
        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        sandbox.remove_file(path).await?;
        if let Err(e) = self.sync_runtime_metadata(name) {
            eprintln!(
                "Warning: failed to sync remote metadata for '{}': {}",
                name, e
            );
        }
        Ok(())
    }

    /// Read a file from a running sandbox
    pub async fn read_file(&mut self, name: &str, path: &str) -> Result<Vec<u8>> {
        if let Err(e) = self.hydrate_remote_runtime(name) {
            eprintln!(
                "Warning: failed to hydrate remote runtime for '{}': {}",
                name, e
            );
        }
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        let content = sandbox.read_file(path).await?;
        if let Err(e) = self.sync_runtime_metadata(name) {
            eprintln!(
                "Warning: failed to sync remote metadata for '{}': {}",
                name, e
            );
        }

        log_event(AuditEvent::FileRead {
            sandbox: name.to_string(),
            path: path.to_string(),
        });

        let _ = self.touch_activity(name);

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ExecResult;
    use crate::cow::{RootfsCow, RootfsCowStore};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[test]
    fn test_sandbox_state_serialize() {
        let state = SandboxState {
            name: "test-sandbox".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.24".to_string(),
            vcpus: 2,
            memory_mb: 1024,
            vsock_cid: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: None,
            tenant_id: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("test-sandbox"));
        assert!(json.contains("alpine:3.24"));
        assert!(json.contains("1024"));
    }

    #[test]
    fn test_sandbox_state_deserialize() {
        let json = r#"{
            "name": "my-sandbox",
            "image": "python:3.12-alpine",
            "vcpus": 4,
            "memory_mb": 2048,
            "vsock_cid": 10,
            "created_at": "2024-01-01T00:00:00Z"
        }"#;

        let mut state: SandboxState = serde_json::from_str(json).unwrap();
        assert_eq!(state.name, "my-sandbox");
        assert_eq!(state.image, "python:3.12-alpine");
        assert_eq!(state.vcpus, 4);
        assert_eq!(state.memory_mb, 2048);
        assert_eq!(state.vsock_cid, 10);
        assert!(!state.post_create_completed);
        assert!(state.full_state_checkpoint.is_none());
        assert!(state.full_state_cleanup_pending.is_empty());
        assert!(!state.full_state_lineage);
        assert!(state.paused_at.is_none());
        assert!(state.forked_from.is_none());
        assert_eq!(state.status(false), "stopped");

        state.full_state_checkpoint = Some("checkpoint-id".to_string());
        state.paused_at = Some("2026-08-23T00:00:00Z".to_string());
        assert_eq!(state.status(false), "paused");
        assert_eq!(state.status(true), "paused");
    }

    #[test]
    fn test_post_create_completion_is_persisted_and_gates_rerun() {
        let mut state: SandboxState = serde_json::from_str(
            r#"{
                "name":"devcontainer",
                "image":"alpine:3.24",
                "vcpus":1,
                "memory_mb":512,
                "vsock_cid":2,
                "created_at":"2024-01-01T00:00:00Z",
                "post_create_commands":[["echo","ready"]]
            }"#,
        )
        .unwrap();
        assert!(should_run_devcontainer_post_create(&state));

        state.post_create_completed = true;
        let restored: SandboxState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(!should_run_devcontainer_post_create(&restored));
        assert_eq!(
            restored.post_create_commands,
            vec![vec!["echo".to_string(), "ready".to_string()]]
        );
    }

    #[test]
    fn test_normalize_persisted_path_makes_relative_absolute() {
        let current = std::env::current_dir().unwrap();
        let expected = std::fs::canonicalize(current.join("examples/remote-modal"))
            .unwrap_or_else(|_| current.join("examples/remote-modal"));
        let normalized = normalize_persisted_path(Some("examples/remote-modal".to_string()))
            .unwrap()
            .unwrap();
        assert!(std::path::Path::new(&normalized).is_absolute());
        assert_eq!(std::path::PathBuf::from(normalized), expected);
    }

    #[test]
    fn test_normalize_persisted_path_preserves_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let absolute = temp_dir.path().join("agentkernel.toml");
        let normalized = normalize_persisted_path(Some(absolute.to_string_lossy().to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(std::path::PathBuf::from(normalized), absolute);
    }

    #[test]
    fn test_sandbox_state_roundtrip() {
        let original = SandboxState {
            name: "roundtrip-test".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "node:20-alpine".to_string(),
            vcpus: 1,
            memory_mb: 512,
            vsock_cid: 3,
            created_at: "2024-06-15T12:30:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: None,
            tenant_id: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: SandboxState = serde_json::from_str(&json).unwrap();

        assert_eq!(original.name, restored.name);
        assert_eq!(original.uuid, restored.uuid);
        assert_eq!(original.image, restored.image);
        assert_eq!(original.vcpus, restored.vcpus);
        assert_eq!(original.memory_mb, restored.memory_mb);
        assert_eq!(original.vsock_cid, restored.vsock_cid);
        assert_eq!(original.created_at, restored.created_at);
    }

    #[test]
    fn test_data_dir_uses_home() {
        // data_dir should use HOME when available
        let data_dir = VmManager::data_dir();
        if std::env::var_os("HOME").is_some() {
            assert!(
                data_dir
                    .to_string_lossy()
                    .contains(".local/share/agentkernel")
            );
        }
    }

    #[test]
    fn test_load_sandboxes_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let sandboxes = VmManager::load_sandboxes(temp_dir.path()).unwrap();
        assert!(sandboxes.is_empty());
    }

    #[test]
    fn test_load_sandboxes_with_files() {
        let temp_dir = TempDir::new().unwrap();

        // Legacy compatibility: persisted old image selections are loaded unchanged.
        let state = SandboxState {
            name: "loaded-sandbox".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.20".to_string(), // legacy compatibility
            vcpus: 1,
            memory_mb: 256,
            vsock_cid: 4,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: None,
            tenant_id: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        std::fs::write(temp_dir.path().join("loaded-sandbox.json"), &json).unwrap();

        // Create an invalid file that should be ignored
        std::fs::write(temp_dir.path().join("invalid.json"), "not valid json").unwrap();

        // Create a non-json file that should be ignored
        std::fs::write(temp_dir.path().join("readme.txt"), "hello").unwrap();

        let sandboxes = VmManager::load_sandboxes(temp_dir.path()).unwrap();
        assert_eq!(sandboxes.len(), 1);
        assert!(sandboxes.contains_key("loaded-sandbox"));

        let loaded = &sandboxes["loaded-sandbox"];
        assert_eq!(loaded.image, "alpine:3.20"); // legacy compatibility
        assert_eq!(loaded.memory_mb, 256);
    }

    #[test]
    fn test_load_sandboxes_nonexistent_dir() {
        let sandboxes = VmManager::load_sandboxes(Path::new("/nonexistent/path")).unwrap();
        assert!(sandboxes.is_empty());
    }

    #[test]
    fn test_load_sandboxes_backfills_uuid() {
        let temp_dir = TempDir::new().unwrap();

        // Legacy state without UUID should be backfilled on load.
        let legacy = r#"{
            "name": "legacy-box",
            "image": "alpine:3.24",
            "vcpus": 1,
            "memory_mb": 256,
            "vsock_cid": 4,
            "created_at": "2026-02-16T00:00:00Z"
        }"#;
        let file = temp_dir.path().join("legacy-box.json");
        std::fs::write(&file, legacy).unwrap();

        let sandboxes = VmManager::load_sandboxes(temp_dir.path()).unwrap();
        let loaded = sandboxes.get("legacy-box").unwrap();
        assert!(!loaded.uuid.is_empty());

        let file_state = std::fs::read_to_string(&file).unwrap();
        assert!(file_state.contains("\"uuid\""));
    }

    #[test]
    fn test_next_cid_calculation() {
        let temp_dir = TempDir::new().unwrap();

        // Create sandboxes with various CIDs
        for (name, cid) in [("sb1", 5), ("sb2", 10), ("sb3", 3)] {
            let state = SandboxState {
                name: name.to_string(),
                uuid: uuid::Uuid::now_v7().to_string(),
                image: "alpine".to_string(),
                vcpus: 1,
                memory_mb: 256,
                vsock_cid: cid,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                backend: None,
                remote_id: None,
                remote_namespace: None,
                remote_metadata: HashMap::new(),
                workspace_revision: None,
                endpoints: Vec::new(),
                work_dir: None,
                container_work_dir: None,
                git_worktree: None,
                config_path: None,
                ttl_seconds: None,
                tenant_id: None,
                expires_at: None,
                ports: Vec::new(),
                managed_network: None,
                ssh_enabled: false,
                ssh_host_port: None,
                volumes: Vec::new(),
                agent: None,
                secret_bindings: Vec::new(),
                secret_mappings: HashMap::new(),
                secret_files: Vec::new(),
                placeholder_secrets: false,
                proxy_port: None,
                init_script: None,
                environment: Vec::new(),
                post_create_commands: Vec::new(),
                post_create_completed: false,
                created_from_template: None,
                template_help_text: None,
                labels: HashMap::new(),
                owner_org_id: None,
                owner_user_id: None,
                description: None,
                last_activity_at: None,
                archived_at: None,
                archived_reason: None,
                dormant_at: None,
                dormant_reason: None,
                lifecycle_policy: None,
                full_state_checkpoint: None,
                full_state_cleanup_pending: Vec::new(),
                full_state_lineage: false,
                paused_at: None,
                forked_from: None,
                firecracker_rootfs: None,
            };
            let json = serde_json::to_string(&state).unwrap();
            std::fs::write(temp_dir.path().join(format!("{}.json", name)), &json).unwrap();
        }

        let sandboxes = VmManager::load_sandboxes(temp_dir.path()).unwrap();
        let max_cid = sandboxes.values().map(|s| s.vsock_cid).max().unwrap_or(2);

        // Next CID should be max + 1 = 11
        assert_eq!(max_cid, 10);
    }

    #[test]
    fn test_sandbox_state_default_values() {
        // Test that missing fields in JSON cause parse failures (strict)
        let incomplete_json = r#"{"name": "test"}"#;
        let result: Result<SandboxState, _> = serde_json::from_str(incomplete_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_labels_and_retrieve() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = VmManager {
            sandboxes: HashMap::new(),
            data_dir: temp_dir.path().to_path_buf(),
            volume_base_dir: None,
            bypass_backend_runtime: false,
            test_backend_factory: None,
            fail_next_state_save: std::sync::atomic::AtomicBool::new(false),
            backend: BackendType::Docker,
            running: HashMap::new(),
            pause_recovery: HashMap::new(),
            resume_state_recovery: HashMap::new(),
            rootfs_dir: None,
            next_cid: 3,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine: None,
        };

        // Insert a sandbox manually
        let state = SandboxState {
            name: "label-test".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.24".to_string(),
            vcpus: 1,
            memory_mb: 512,
            vsock_cid: 3,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: None,
            tenant_id: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };
        std::fs::create_dir_all(temp_dir.path().join("sandboxes")).unwrap();
        manager.sandboxes.insert("label-test".to_string(), state);

        // Set labels
        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        labels.insert("team".to_string(), "ml".to_string());
        manager.set_labels("label-test", &labels).unwrap();

        // Retrieve and verify
        let state = manager.get_state("label-test").unwrap();
        assert_eq!(state.labels.get("env").unwrap(), "prod");
        assert_eq!(state.labels.get("team").unwrap(), "ml");
    }

    #[test]
    fn test_list_matching_labels() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = VmManager {
            sandboxes: HashMap::new(),
            data_dir: temp_dir.path().to_path_buf(),
            volume_base_dir: None,
            bypass_backend_runtime: false,
            test_backend_factory: None,
            fail_next_state_save: std::sync::atomic::AtomicBool::new(false),
            backend: BackendType::Docker,
            running: HashMap::new(),
            pause_recovery: HashMap::new(),
            resume_state_recovery: HashMap::new(),
            rootfs_dir: None,
            next_cid: 3,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine: None,
        };
        std::fs::create_dir_all(temp_dir.path().join("sandboxes")).unwrap();

        // Create sandboxes with different labels
        for (name, env) in [("s1", "prod"), ("s2", "staging"), ("s3", "prod")] {
            let mut labels = HashMap::new();
            labels.insert("env".to_string(), env.to_string());
            let state = SandboxState {
                name: name.to_string(),
                uuid: uuid::Uuid::now_v7().to_string(),
                image: "alpine:3.24".to_string(),
                vcpus: 1,
                memory_mb: 512,
                vsock_cid: 3,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                backend: None,
                remote_id: None,
                remote_namespace: None,
                remote_metadata: HashMap::new(),
                workspace_revision: None,
                endpoints: Vec::new(),
                work_dir: None,
                container_work_dir: None,
                git_worktree: None,
                config_path: None,
                ttl_seconds: None,
                tenant_id: None,
                expires_at: None,
                ports: Vec::new(),
                managed_network: None,
                ssh_enabled: false,
                ssh_host_port: None,
                volumes: Vec::new(),
                agent: None,
                secret_bindings: Vec::new(),
                secret_mappings: HashMap::new(),
                secret_files: Vec::new(),
                placeholder_secrets: false,
                proxy_port: None,
                init_script: None,
                environment: Vec::new(),
                post_create_commands: Vec::new(),
                post_create_completed: false,
                created_from_template: None,
                template_help_text: None,
                labels,
                owner_org_id: None,
                owner_user_id: None,
                description: None,
                last_activity_at: None,
                archived_at: None,
                archived_reason: None,
                dormant_at: None,
                dormant_reason: None,
                lifecycle_policy: None,
                full_state_checkpoint: None,
                full_state_cleanup_pending: Vec::new(),
                full_state_lineage: false,
                paused_at: None,
                forked_from: None,
                firecracker_rootfs: None,
            };
            manager.sandboxes.insert(name.to_string(), state);
        }

        // Filter by env=prod
        let filters = vec![("env".to_string(), "prod".to_string())];
        let mut matched = manager.list_matching_labels(&filters);
        matched.sort();
        assert_eq!(matched, vec!["s1", "s3"]);

        // Filter by env=staging
        let filters = vec![("env".to_string(), "staging".to_string())];
        let matched = manager.list_matching_labels(&filters);
        assert_eq!(matched, vec!["s2"]);

        // Filter by non-existent label
        let filters = vec![("team".to_string(), "ml".to_string())];
        let matched = manager.list_matching_labels(&filters);
        assert!(matched.is_empty());
    }

    #[test]
    fn test_set_description() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = VmManager {
            sandboxes: HashMap::new(),
            data_dir: temp_dir.path().to_path_buf(),
            volume_base_dir: None,
            bypass_backend_runtime: false,
            test_backend_factory: None,
            fail_next_state_save: std::sync::atomic::AtomicBool::new(false),
            backend: BackendType::Docker,
            running: HashMap::new(),
            pause_recovery: HashMap::new(),
            resume_state_recovery: HashMap::new(),
            rootfs_dir: None,
            next_cid: 3,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine: None,
        };
        std::fs::create_dir_all(temp_dir.path().join("sandboxes")).unwrap();

        let state = SandboxState {
            name: "desc-test".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.24".to_string(),
            vcpus: 1,
            memory_mb: 512,
            vsock_cid: 3,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: None,
            tenant_id: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };
        manager.sandboxes.insert("desc-test".to_string(), state);

        manager
            .set_description("desc-test", Some("My sandbox"))
            .unwrap();
        assert_eq!(
            manager
                .get_state("desc-test")
                .unwrap()
                .description
                .as_deref(),
            Some("My sandbox")
        );

        manager.set_description("desc-test", None).unwrap();
        assert!(
            manager
                .get_state("desc-test")
                .unwrap()
                .description
                .is_none()
        );
    }

    #[test]
    fn test_labels_persist_across_reload() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join("sandboxes")).unwrap();

        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());

        let state = SandboxState {
            name: "persist-test".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.24".to_string(),
            vcpus: 1,
            memory_mb: 512,
            vsock_cid: 3,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: None,
            tenant_id: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: labels.clone(),
            owner_org_id: None,
            owner_user_id: None,
            description: Some("Test sandbox".to_string()),
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        };

        // Save to disk
        let path = temp_dir.path().join("sandboxes").join("persist-test.json");
        std::fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

        // Reload from disk
        let loaded = VmManager::load_sandboxes(&temp_dir.path().join("sandboxes")).unwrap();
        let loaded_state = loaded.get("persist-test").unwrap();
        assert_eq!(loaded_state.labels.get("env").unwrap(), "prod");
        assert_eq!(loaded_state.description.as_deref(), Some("Test sandbox"));
    }

    fn new_test_manager(temp_dir: &TempDir) -> VmManager {
        VmManager::for_tests(temp_dir.path()).unwrap()
    }

    fn durable_backend_factory(
        store_root: &Path,
        fail_publication: Arc<std::sync::atomic::AtomicBool>,
    ) -> Arc<dyn Fn(&str, BackendType) -> Result<Box<dyn Sandbox>> + Send + Sync> {
        let store_root = store_root.to_path_buf();
        Arc::new(move |name, backend| {
            if backend != BackendType::Firecracker {
                bail!("durable test factory only supports Firecracker");
            }
            Ok(Box::new(DurableTestSandbox::new(
                name,
                store_root.clone(),
                Arc::clone(&fail_publication),
            )?))
        })
    }

    fn durable_state(name: &str) -> SandboxState {
        let mut state = lifecycle_state(name);
        state.backend = Some(BackendType::Firecracker);
        state
    }

    #[test]
    fn duplicated_firecracker_rootfs_reference_fails_closed() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut first = durable_state("first");
        first.firecracker_rootfs = Some("sandbox-shared123456".to_string());
        let mut second = durable_state("second");
        second.firecracker_rootfs = first.firecracker_rootfs.clone();
        manager.sandboxes.insert(first.name.clone(), first);
        manager.sandboxes.insert(second.name.clone(), second);

        let error = manager
            .ensure_unique_firecracker_rootfs_reference("first", Some("sandbox-shared123456"))
            .unwrap_err();
        assert!(error.to_string().contains("referenced by both"));
    }

    fn configure_durable_manager(
        manager: &mut VmManager,
        store_root: &Path,
        fail_publication: Arc<std::sync::atomic::AtomicBool>,
    ) {
        manager
            .set_backend_factory_for_tests(durable_backend_factory(store_root, fail_publication));
    }

    #[tokio::test]
    async fn ordinary_start_stop_start_reopens_persisted_rootfs_lineage() {
        let temp_dir = TempDir::new().unwrap();
        let store_dir = TempDir::new().unwrap();
        let store_root = store_dir.path().join("cow");
        let fail_publication = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut manager = new_test_manager(&temp_dir);
        configure_durable_manager(&mut manager, &store_root, Arc::clone(&fail_publication));
        manager.sandboxes.insert(
            "durable-lifecycle".to_string(),
            durable_state("durable-lifecycle"),
        );
        manager
            .save_sandbox(manager.get_state("durable-lifecycle").unwrap())
            .unwrap();

        manager.start("durable-lifecycle").await.unwrap();
        let reference = manager
            .get_state("durable-lifecycle")
            .unwrap()
            .firecracker_rootfs
            .clone()
            .unwrap();
        manager.stop("durable-lifecycle").await.unwrap();
        assert!(
            RootfsCowStore::new(&store_root)
                .unwrap()
                .adopt(&reference)
                .is_ok()
        );

        let mut restarted = new_test_manager(&temp_dir);
        restarted.sandboxes =
            VmManager::load_sandboxes(&temp_dir.path().join("sandboxes")).unwrap();
        configure_durable_manager(&mut restarted, &store_root, fail_publication);
        restarted.start("durable-lifecycle").await.unwrap();
        restarted.stop("durable-lifecycle").await.unwrap();
        assert_eq!(
            restarted
                .get_state("durable-lifecycle")
                .unwrap()
                .firecracker_rootfs
                .as_deref(),
            Some(reference.as_str())
        );
    }

    #[tokio::test]
    async fn remove_after_manager_restart_discards_persisted_rootfs_lineage() {
        let temp_dir = TempDir::new().unwrap();
        let store_dir = TempDir::new().unwrap();
        let store_root = store_dir.path().join("cow");
        let fail_publication = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut manager = new_test_manager(&temp_dir);
        configure_durable_manager(&mut manager, &store_root, Arc::clone(&fail_publication));
        manager.sandboxes.insert(
            "durable-remove".to_string(),
            durable_state("durable-remove"),
        );
        manager
            .save_sandbox(manager.get_state("durable-remove").unwrap())
            .unwrap();
        manager.start("durable-remove").await.unwrap();
        let reference = manager
            .get_state("durable-remove")
            .unwrap()
            .firecracker_rootfs
            .clone()
            .unwrap();
        manager.stop("durable-remove").await.unwrap();

        let mut restarted = new_test_manager(&temp_dir);
        restarted.sandboxes =
            VmManager::load_sandboxes(&temp_dir.path().join("sandboxes")).unwrap();
        configure_durable_manager(&mut restarted, &store_root, fail_publication);
        restarted.remove("durable-remove").await.unwrap();
        assert!(!store_root.join(&reference).exists());
        assert!(restarted.get_state("durable-remove").is_none());
    }

    #[tokio::test]
    async fn rootfs_publication_failure_rolls_back_state_and_artifact() {
        let temp_dir = TempDir::new().unwrap();
        let store_dir = TempDir::new().unwrap();
        let store_root = store_dir.path().join("cow");
        let fail_publication = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut manager = new_test_manager(&temp_dir);
        configure_durable_manager(&mut manager, &store_root, Arc::clone(&fail_publication));
        manager.sandboxes.insert(
            "durable-publish-failure".to_string(),
            durable_state("durable-publish-failure"),
        );
        manager
            .save_sandbox(manager.get_state("durable-publish-failure").unwrap())
            .unwrap();

        let error = manager.start("durable-publish-failure").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("publish Firecracker disk lineage")
        );
        assert!(
            manager
                .get_state("durable-publish-failure")
                .unwrap()
                .firecracker_rootfs
                .is_none()
        );
        assert_eq!(
            RootfsCowStore::new(&store_root)
                .unwrap()
                .usage_bytes()
                .unwrap(),
            0
        );
        let persisted: SandboxState = serde_json::from_slice(
            &std::fs::read(
                temp_dir
                    .path()
                    .join("sandboxes/durable-publish-failure.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(persisted.firecracker_rootfs.is_none());
    }

    #[tokio::test]
    async fn rootfs_state_save_failure_discards_unpublished_artifact() {
        let temp_dir = TempDir::new().unwrap();
        let store_dir = TempDir::new().unwrap();
        let store_root = store_dir.path().join("cow");
        let fail_publication = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut manager = new_test_manager(&temp_dir);
        configure_durable_manager(&mut manager, &store_root, fail_publication);
        manager.sandboxes.insert(
            "durable-save-failure".to_string(),
            durable_state("durable-save-failure"),
        );
        manager
            .save_sandbox(manager.get_state("durable-save-failure").unwrap())
            .unwrap();
        manager.fail_next_state_save_for_tests();

        let error = manager.start("durable-save-failure").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("simulated sandbox state publication failure")
        );
        assert!(
            manager
                .get_state("durable-save-failure")
                .unwrap()
                .firecracker_rootfs
                .is_none()
        );
        assert_eq!(
            RootfsCowStore::new(&store_root)
                .unwrap()
                .usage_bytes()
                .unwrap(),
            0
        );
    }

    fn lifecycle_state(name: &str) -> SandboxState {
        SandboxState {
            name: name.to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.24".to_string(),
            vcpus: 1,
            memory_mb: 256,
            vsock_cid: 3,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            ttl_seconds: Some(3600),
            tenant_id: None,
            expires_at: Some("2026-01-01T01:00:00Z".to_string()),
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            owner_org_id: None,
            owner_user_id: None,
            description: None,
            last_activity_at: Some("2026-01-01T00:00:00Z".to_string()),
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
        }
    }

    #[test]
    fn refresh_sandbox_imports_only_missing_persisted_state() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let state = lifecycle_state("created-by-cli");
        let state_path = temp_dir
            .path()
            .join("sandboxes")
            .join("created-by-cli.json");
        std::fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

        assert!(
            manager
                .refresh_sandbox_if_missing("created-by-cli")
                .unwrap()
        );
        assert_eq!(
            manager.get_state("created-by-cli").unwrap().uuid,
            state.uuid
        );

        let mut stale = state.clone();
        stale.image = "should-not-overwrite-live-state".to_string();
        std::fs::write(&state_path, serde_json::to_vec_pretty(&stale).unwrap()).unwrap();
        assert!(
            !manager
                .refresh_sandbox_if_missing("created-by-cli")
                .unwrap()
        );
        assert_eq!(
            manager.get_state("created-by-cli").unwrap().image,
            "alpine:3.24"
        );
    }

    #[allow(dead_code)]
    struct TestSandbox {
        name: String,
        running: bool,
    }

    struct RecoverySandbox {
        name: String,
        running: bool,
        stop_attempts: Option<Arc<AtomicUsize>>,
        stop_failures_before_success: usize,
    }

    /// A filesystem-only Firecracker double used to exercise VmManager's
    /// state publication boundary without requiring KVM or a VMM binary.
    struct DurableTestSandbox {
        name: String,
        store_root: PathBuf,
        base_rootfs: PathBuf,
        rootfs: Option<RootfsCow>,
        reference: Option<String>,
        published: bool,
        running: bool,
        fail_publication: Arc<std::sync::atomic::AtomicBool>,
    }

    impl DurableTestSandbox {
        fn new(
            name: &str,
            store_root: PathBuf,
            fail_publication: Arc<std::sync::atomic::AtomicBool>,
        ) -> Result<Self> {
            std::fs::create_dir_all(&store_root)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&store_root, std::fs::Permissions::from_mode(0o700))?;
            }
            let base_rootfs = store_root.join("test-base.ext4");
            if !base_rootfs.exists() {
                std::fs::write(&base_rootfs, b"durable test rootfs")?;
            }
            Ok(Self {
                name: name.to_string(),
                store_root,
                base_rootfs,
                rootfs: None,
                reference: None,
                published: false,
                running: false,
                fail_publication,
            })
        }
    }

    #[async_trait]
    impl Sandbox for DurableTestSandbox {
        async fn start(&mut self, _config: &SandboxConfig) -> Result<()> {
            let store = RootfsCowStore::new(&self.store_root)?;
            let rootfs = if let Some(reference) = self.reference.as_deref() {
                store.adopt(reference)?
            } else {
                store.prepare(&self.base_rootfs)?
            };
            self.rootfs = Some(rootfs);
            self.running = true;
            Ok(())
        }

        async fn exec(&mut self, _cmd: &[&str]) -> Result<ExecResult> {
            Ok(ExecResult::success(String::new()))
        }

        async fn stop(&mut self) -> Result<()> {
            if let Some(mut rootfs) = self.rootfs.take() {
                if self.published {
                    rootfs.preserve_for_lifecycle()?;
                    self.reference = Some(rootfs.reference());
                    drop(rootfs);
                } else {
                    rootfs.discard_persisted()?;
                    self.reference = None;
                }
            }
            self.running = false;
            Ok(())
        }

        async fn remove(&mut self) -> Result<()> {
            if self.rootfs.is_none()
                && let Some(reference) = self.reference.as_deref()
            {
                self.rootfs = Some(RootfsCowStore::new(&self.store_root)?.adopt(reference)?);
            }
            if let Some(rootfs) = self.rootfs.take() {
                rootfs.discard_persisted()?;
            }
            self.reference = None;
            self.published = false;
            self.running = false;
            Ok(())
        }

        fn set_persistent_disk_reference(&mut self, reference: Option<&str>) -> Result<()> {
            if self.running || self.rootfs.is_some() {
                bail!("test Firecracker runtime is active");
            }
            self.reference = reference.map(str::to_owned);
            self.published = reference.is_some();
            Ok(())
        }

        fn persistent_disk_reference(&self) -> Option<String> {
            self.reference
                .clone()
                .or_else(|| self.rootfs.as_ref().map(RootfsCow::reference))
        }

        fn publish_persistent_disk_reference(&mut self) -> Result<()> {
            if self.fail_publication.swap(false, Ordering::SeqCst) {
                bail!("simulated rootfs marker publication failure");
            }
            let rootfs = self
                .rootfs
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("test rootfs is not prepared"))?;
            rootfs.preserve_for_lifecycle()?;
            self.reference = Some(rootfs.reference());
            self.published = true;
            Ok(())
        }

        fn rollback_persistent_disk_reference(&mut self) -> Result<()> {
            if self.reference.is_none() {
                self.published = false;
            }
            Ok(())
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn backend_type(&self) -> BackendType {
            BackendType::Firecracker
        }

        fn is_running(&self) -> bool {
            self.running
        }

        async fn write_file_unchecked(&mut self, _path: &str, _content: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn read_file_unchecked(&mut self, _path: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn remove_file_unchecked(&mut self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn mkdir_unchecked(&mut self, _path: &str, _recursive: bool) -> Result<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    #[derive(Clone, Copy)]
    enum PauseRollbackMode {
        Ordinary,
        Partial,
        Commit,
        CommitAmbiguousRestore,
    }

    #[cfg(unix)]
    struct PauseRollbackSandbox {
        name: String,
        running: bool,
        state_directory: PathBuf,
        mode: PauseRollbackMode,
        stop_attempts: Option<Arc<AtomicUsize>>,
    }

    #[cfg(unix)]
    impl PauseRollbackSandbox {
        fn snapshot() -> FullStateSnapshot {
            FullStateSnapshot {
                firecracker_version: "1.16.1".to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                host_kernel_release: "test-host-kernel".to_string(),
                host_identity_sha256: "test-host".to_string(),
                cpu_fingerprint_sha256: "test-cpu".to_string(),
                guest_kernel_release: "6.18.45-agentkernel".to_string(),
            }
        }

        fn block_state_writes(&self) -> Result<()> {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&self.state_directory)?.permissions();
            permissions.set_mode(0o500);
            std::fs::set_permissions(&self.state_directory, permissions)?;
            Ok(())
        }
    }

    #[async_trait]
    impl Sandbox for TestSandbox {
        async fn start(&mut self, _config: &SandboxConfig) -> Result<()> {
            self.running = true;
            Ok(())
        }

        async fn exec(&mut self, _cmd: &[&str]) -> Result<ExecResult> {
            Ok(ExecResult::success(String::new()))
        }

        async fn stop(&mut self) -> Result<()> {
            self.running = false;
            Ok(())
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn backend_type(&self) -> BackendType {
            BackendType::Docker
        }

        fn is_running(&self) -> bool {
            self.running
        }

        async fn write_file_unchecked(&mut self, _path: &str, _content: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn read_file_unchecked(&mut self, _path: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn remove_file_unchecked(&mut self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn mkdir_unchecked(&mut self, _path: &str, _recursive: bool) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl Sandbox for RecoverySandbox {
        async fn start(&mut self, _config: &SandboxConfig) -> Result<()> {
            self.running = true;
            Ok(())
        }

        async fn exec(&mut self, _cmd: &[&str]) -> Result<ExecResult> {
            Ok(ExecResult::success(String::new()))
        }

        async fn stop(&mut self) -> Result<()> {
            if let Some(attempts) = &self.stop_attempts
                && attempts.fetch_add(1, Ordering::SeqCst) < self.stop_failures_before_success
            {
                return Err(FullStateTerminationError {
                    process_may_be_running: true,
                    detail: "simulated ambiguous wait after kill".to_string(),
                }
                .into());
            }
            self.running = false;
            Ok(())
        }

        async fn retry_full_state_resume(&mut self) -> Result<()> {
            self.running = true;
            Ok(())
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn backend_type(&self) -> BackendType {
            BackendType::Firecracker
        }

        fn is_running(&self) -> bool {
            self.running
        }

        async fn write_file_unchecked(&mut self, _path: &str, _content: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn read_file_unchecked(&mut self, _path: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn remove_file_unchecked(&mut self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn mkdir_unchecked(&mut self, _path: &str, _recursive: bool) -> Result<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    #[async_trait]
    impl Sandbox for PauseRollbackSandbox {
        async fn start(&mut self, _config: &SandboxConfig) -> Result<()> {
            self.running = true;
            Ok(())
        }

        async fn exec(&mut self, _cmd: &[&str]) -> Result<ExecResult> {
            Ok(ExecResult::success(String::new()))
        }

        async fn stop(&mut self) -> Result<()> {
            if let Some(attempts) = &self.stop_attempts {
                attempts.fetch_add(1, Ordering::SeqCst);
            }
            self.running = false;
            Ok(())
        }

        async fn pause_to(&mut self, checkpoint_dir: &Path) -> Result<FullStateSnapshot> {
            match self.mode {
                PauseRollbackMode::Ordinary => {
                    self.block_state_writes()?;
                    bail!("simulated ordinary snapshot failure")
                }
                PauseRollbackMode::Partial => {
                    self.running = false;
                    Err(FullStatePauseError::source_resume_failed(
                        Self::snapshot(),
                        false,
                        "simulated partial snapshot failure",
                        "simulated first resume failure",
                    )
                    .into())
                }
                PauseRollbackMode::Commit | PauseRollbackMode::CommitAmbiguousRestore => {
                    std::fs::write(checkpoint_dir.join("memory.bin"), b"memory")?;
                    std::fs::write(checkpoint_dir.join("vmstate.bin"), b"vmstate")?;
                    std::fs::write(checkpoint_dir.join("rootfs.ext4"), b"rootfs")?;
                    let staging_name = checkpoint_dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .context("missing checkpoint staging name")?;
                    let checkpoint_id = staging_name
                        .strip_prefix(".staging-")
                        .context("invalid checkpoint staging name")?;
                    std::fs::create_dir(checkpoint_dir.parent().unwrap().join(checkpoint_id))?;
                    self.running = false;
                    Ok(Self::snapshot())
                }
            }
        }

        async fn retry_full_state_resume(&mut self) -> Result<()> {
            self.running = true;
            self.block_state_writes()
        }

        async fn restore_from(
            &mut self,
            _checkpoint_dir: &Path,
            _snapshot: &FullStateSnapshot,
        ) -> Result<()> {
            if matches!(self.mode, PauseRollbackMode::CommitAmbiguousRestore) {
                self.running = false;
                return Err(FullStateTerminationError {
                    process_may_be_running: true,
                    detail: "simulated ambiguous cleanup after rollback restore".to_string(),
                }
                .into());
            }
            self.running = true;
            self.block_state_writes()
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn backend_type(&self) -> BackendType {
            BackendType::Firecracker
        }

        fn is_running(&self) -> bool {
            self.running
        }

        async fn write_file_unchecked(&mut self, _path: &str, _content: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn read_file_unchecked(&mut self, _path: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn remove_file_unchecked(&mut self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn mkdir_unchecked(&mut self, _path: &str, _recursive: bool) -> Result<()> {
            Ok(())
        }
    }

    fn insert_pause_recovery(manager: &mut VmManager, name: &str) -> PathBuf {
        let store = FullStateCheckpointStore::new(&manager.data_dir).unwrap();
        let staging = store.begin().unwrap();
        let id = staging.id().to_string();
        let staging_path = staging.preserve();
        let mut state = lifecycle_state(name);
        state.backend = Some(BackendType::Firecracker);
        state.paused_at = Some(chrono::Utc::now().to_rfc3339());
        state.full_state_checkpoint = Some(id);
        manager.sandboxes.insert(name.to_string(), state);
        manager.pause_recovery.insert(
            name.to_string(),
            PendingFullStateRecovery {
                sandbox: Some(Box::new(RecoverySandbox {
                    name: name.to_string(),
                    running: false,
                    stop_attempts: None,
                    stop_failures_before_success: 0,
                })),
                staging_path: staging_path.clone(),
                completed_snapshot: None,
            },
        );
        staging_path
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn pause_recovery_owned_runtime_counts_toward_running_quota() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        insert_pause_recovery(&mut manager, "quota-recovery");

        assert!(manager.is_running("quota-recovery"));
        let subject = crate::quota::QuotaSubject {
            user_id: "anonymous".to_string(),
            org_id: "default".to_string(),
        };
        let quota = crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
            enabled: true,
            default_limits: crate::config::ResourceQuotaLimits {
                max_running_sandboxes: Some(1),
                ..Default::default()
            },
            ..Default::default()
        });

        let status = quota.status(&manager, &subject);
        assert_eq!(status.user.usage.running_sandboxes, 1);
        let error = quota.check_create(&manager, &subject, 1, 512).unwrap_err();
        assert!(error.to_string().contains("max_running_sandboxes"));
    }

    #[tokio::test]
    async fn recovery_pending_resume_retries_live_source_and_cleans_staging() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let staging_path = insert_pause_recovery(&mut manager, "recover-live");

        manager.resume("recover-live").await.unwrap();

        assert!(manager.pause_recovery.is_empty());
        assert!(manager.is_running("recover-live"));
        assert!(!staging_path.exists());
        let state = manager.get_state("recover-live").unwrap();
        assert!(state.paused_at.is_none());
        assert!(state.full_state_checkpoint.is_none());
        assert!(state.full_state_cleanup_pending.is_empty());
    }

    #[tokio::test]
    async fn resumed_runtime_metadata_failure_is_retryable_without_a_second_vm() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let staging_path = insert_pause_recovery(&mut manager, "resume-metadata-retry");
        let sandboxes_dir = temp_dir.path().join("sandboxes");
        std::fs::remove_dir(&sandboxes_dir).unwrap();
        std::fs::write(&sandboxes_dir, b"blocks state directory").unwrap();

        let error = manager.resume("resume-metadata-retry").await.unwrap_err();
        assert!(error.to_string().contains("could not be persisted"));
        assert!(manager.is_running("resume-metadata-retry"));
        assert!(
            manager
                .resume_state_recovery
                .contains_key("resume-metadata-retry")
        );
        assert!(staging_path.exists());

        #[cfg(feature = "enterprise")]
        {
            let quota = crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
                enabled: true,
                default_limits: crate::config::ResourceQuotaLimits {
                    max_running_sandboxes: Some(1),
                    ..Default::default()
                },
                ..Default::default()
            });
            quota
                .check_start(&manager, "resume-metadata-retry")
                .expect("metadata recovery must not consume a second running slot");
        }

        std::fs::remove_file(&sandboxes_dir).unwrap();
        std::fs::create_dir(&sandboxes_dir).unwrap();
        manager.resume("resume-metadata-retry").await.unwrap();

        assert!(manager.is_running("resume-metadata-retry"));
        assert!(manager.resume_state_recovery.is_empty());
        assert!(!staging_path.exists());
        assert!(
            manager
                .get_state("resume-metadata-retry")
                .unwrap()
                .paused_at
                .is_none()
        );
    }

    #[cfg(unix)]
    async fn assert_failed_pause_rollback_is_metadata_retryable(
        name: &str,
        mode: PauseRollbackMode,
    ) {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let state_directory = temp_dir.path().join("sandboxes");
        let mut state = lifecycle_state(name);
        state.backend = Some(BackendType::Firecracker);
        manager.save_sandbox(&state).unwrap();
        manager.sandboxes.insert(name.to_string(), state);
        manager.running.insert(
            name.to_string(),
            Box::new(PauseRollbackSandbox {
                name: name.to_string(),
                running: true,
                state_directory: state_directory.clone(),
                mode,
                stop_attempts: None,
            }),
        );

        let error = manager.pause(name).await.unwrap_err();
        assert!(error.to_string().contains("recovery ownership"));
        assert!(manager.running.contains_key(name));
        assert!(manager.resume_state_recovery.contains_key(name));
        let paused = manager.get_state(name).unwrap();
        assert!(paused.paused_at.is_some());
        let checkpoint_id = paused.full_state_checkpoint.clone().unwrap();
        let staging_path = FullStateCheckpointStore::new(&manager.data_dir)
            .unwrap()
            .staging_path(&checkpoint_id)
            .unwrap();
        assert!(staging_path.is_dir());

        let persisted: SandboxState = serde_json::from_slice(
            &std::fs::read(state_directory.join(format!("{name}.json"))).unwrap(),
        )
        .unwrap();
        assert!(persisted.paused_at.is_some());
        assert_eq!(
            persisted.full_state_checkpoint.as_deref(),
            Some(checkpoint_id.as_str())
        );

        let mut permissions = std::fs::metadata(&state_directory).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&state_directory, permissions).unwrap();
        manager.resume(name).await.unwrap();

        assert!(manager.running.contains_key(name));
        assert!(!manager.resume_state_recovery.contains_key(name));
        assert!(!staging_path.exists());
        let running = manager.get_state(name).unwrap();
        assert!(running.paused_at.is_none());
        assert!(running.full_state_checkpoint.is_none());
        assert!(running.full_state_cleanup_pending.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ordinary_pause_failure_and_rollback_write_failure_retain_live_recovery() {
        assert_failed_pause_rollback_is_metadata_retryable(
            "ordinary-rollback-retry",
            PauseRollbackMode::Ordinary,
        )
        .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn partial_pause_failure_and_rollback_write_failure_retain_live_recovery() {
        assert_failed_pause_rollback_is_metadata_retryable(
            "partial-rollback-retry",
            PauseRollbackMode::Partial,
        )
        .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commit_failure_and_rollback_write_failure_retain_live_recovery() {
        assert_failed_pause_rollback_is_metadata_retryable(
            "commit-rollback-retry",
            PauseRollbackMode::Commit,
        )
        .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ambiguous_commit_rollback_retains_runtime_and_blocks_fork_until_reconciled() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let name = "ambiguous-commit-rollback";
        let mut state = lifecycle_state(name);
        state.backend = Some(BackendType::Firecracker);
        manager.save_sandbox(&state).unwrap();
        manager.sandboxes.insert(name.to_string(), state);
        let stop_attempts = Arc::new(AtomicUsize::new(0));
        manager.running.insert(
            name.to_string(),
            Box::new(PauseRollbackSandbox {
                name: name.to_string(),
                running: true,
                state_directory: temp_dir.path().join("sandboxes"),
                mode: PauseRollbackMode::CommitAmbiguousRestore,
                stop_attempts: Some(Arc::clone(&stop_attempts)),
            }),
        );

        let pause_error = manager.pause(name).await.unwrap_err();
        assert!(pause_error.to_string().contains("failed to resume source"));
        let recovery = manager.pause_recovery.get(name).unwrap();
        assert!(recovery.sandbox.is_some());
        assert!(recovery.completed_snapshot.is_some());

        let fork_error = manager
            .fork_sandbox(name, "unsafe-child")
            .await
            .unwrap_err();
        assert!(
            fork_error
                .to_string()
                .contains("pending full-state recovery")
        );
        assert!(!manager.sandboxes.contains_key("unsafe-child"));

        let checkpoint_id = manager
            .get_state(name)
            .unwrap()
            .full_state_checkpoint
            .clone()
            .unwrap();
        std::fs::remove_dir(
            manager
                .data_dir
                .join("full-state-checkpoints")
                .join(&checkpoint_id),
        )
        .unwrap();

        // Reconciliation must stop the retained possibly-live runtime before
        // publishing the checkpoint. Loading then fails on the mock host
        // fingerprint, leaving the source safely paused and reusable.
        assert!(manager.resume(name).await.is_err());
        assert_eq!(stop_attempts.load(Ordering::SeqCst), 1);
        assert!(!manager.pause_recovery.contains_key(name));
        assert!(
            FullStateCheckpointStore::new(&manager.data_dir)
                .unwrap()
                .contains(&checkpoint_id)
                .unwrap()
        );
    }

    #[tokio::test]
    async fn completed_recovery_reconfirms_termination_then_publishes() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let store = FullStateCheckpointStore::new(&manager.data_dir).unwrap();
        let staging = store.begin().unwrap();
        let checkpoint_id = staging.id().to_string();
        std::fs::write(staging.path().join("memory.bin"), b"memory").unwrap();
        std::fs::write(staging.path().join("vmstate.bin"), b"vmstate").unwrap();
        std::fs::write(staging.path().join("rootfs.ext4"), b"rootfs").unwrap();
        let staging_path = staging.preserve();

        let mut state = lifecycle_state("complete-recovery");
        state.backend = Some(BackendType::Firecracker);
        state.paused_at = Some(chrono::Utc::now().to_rfc3339());
        state.full_state_checkpoint = Some(checkpoint_id.clone());
        manager
            .sandboxes
            .insert("complete-recovery".to_string(), state);

        let stop_attempts = Arc::new(AtomicUsize::new(0));
        manager.pause_recovery.insert(
            "complete-recovery".to_string(),
            PendingFullStateRecovery {
                sandbox: Some(Box::new(RecoverySandbox {
                    name: "complete-recovery".to_string(),
                    running: false,
                    stop_attempts: Some(Arc::clone(&stop_attempts)),
                    stop_failures_before_success: 1,
                })),
                staging_path: staging_path.clone(),
                completed_snapshot: Some(FullStateSnapshot {
                    firecracker_version: "1.16.1".to_string(),
                    architecture: "incompatible-test-architecture".to_string(),
                    host_kernel_release: "test-host-kernel".to_string(),
                    host_identity_sha256: "test-host".to_string(),
                    cpu_fingerprint_sha256: "test-cpu".to_string(),
                    guest_kernel_release: "6.18.45-agentkernel".to_string(),
                }),
            },
        );

        let first_error = manager.resume("complete-recovery").await.unwrap_err();
        assert!(first_error.to_string().contains("confirm termination"));
        assert!(staging_path.exists());
        assert!(!store.contains(&checkpoint_id).unwrap());
        assert!(manager.pause_recovery.contains_key("complete-recovery"));

        // The second termination probe succeeds. Restore then fails at the
        // deliberately incompatible architecture, proving publication
        // happened before any VMM was allowed to load the checkpoint.
        assert!(manager.resume("complete-recovery").await.is_err());
        assert_eq!(stop_attempts.load(Ordering::SeqCst), 2);
        assert!(!staging_path.exists());
        assert!(store.contains(&checkpoint_id).unwrap());
        assert!(!manager.pause_recovery.contains_key("complete-recovery"));
    }

    #[test]
    fn pending_checkpoint_cleanup_is_retried_and_persistently_cleared() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let store = FullStateCheckpointStore::new(&manager.data_dir).unwrap();
        let staging = store.begin().unwrap();
        let checkpoint_id = staging.id().to_string();
        let staging_path = staging.preserve();

        let mut state = lifecycle_state("cleanup-retry");
        state.full_state_cleanup_pending = vec![checkpoint_id];
        manager.save_sandbox(&state).unwrap();
        manager.sandboxes.insert("cleanup-retry".to_string(), state);

        manager.reconcile_full_state_cleanup();

        assert!(!staging_path.exists());
        assert!(
            manager
                .get_state("cleanup-retry")
                .unwrap()
                .full_state_cleanup_pending
                .is_empty()
        );
        let reloaded = VmManager::load_sandboxes(&temp_dir.path().join("sandboxes")).unwrap();
        assert!(
            reloaded["cleanup-retry"]
                .full_state_cleanup_pending
                .is_empty()
        );
    }

    #[tokio::test]
    async fn removing_recovery_pending_source_discards_live_handle_and_staging() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let staging_path = insert_pause_recovery(&mut manager, "remove-recovery");

        manager.remove("remove-recovery").await.unwrap();

        assert!(!manager.exists("remove-recovery"));
        assert!(manager.pause_recovery.is_empty());
        assert!(!staging_path.exists());
    }

    #[tokio::test]
    async fn archived_or_dormant_paused_sandboxes_require_recovery_first() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);

        let mut archived = lifecycle_state("archived-pause");
        archived.backend = Some(BackendType::Firecracker);
        archived.paused_at = Some(chrono::Utc::now().to_rfc3339());
        archived.full_state_checkpoint = Some(uuid::Uuid::new_v4().to_string());
        archived.archived_at = Some(chrono::Utc::now().to_rfc3339());
        manager
            .sandboxes
            .insert("archived-pause".to_string(), archived);
        let resume_error = manager.resume("archived-pause").await.unwrap_err();
        assert!(
            resume_error
                .to_string()
                .contains("recover it before resuming")
        );

        let mut dormant = lifecycle_state("dormant-pause");
        dormant.backend = Some(BackendType::Firecracker);
        dormant.paused_at = Some(chrono::Utc::now().to_rfc3339());
        dormant.full_state_checkpoint = Some(uuid::Uuid::new_v4().to_string());
        dormant.dormant_at = Some(chrono::Utc::now().to_rfc3339());
        manager
            .sandboxes
            .insert("dormant-pause".to_string(), dormant);
        let fork_error = manager
            .fork_sandbox("dormant-pause", "child")
            .await
            .unwrap_err();
        assert!(fork_error.to_string().contains("recover it before forking"));
    }

    #[test]
    fn paused_and_recovery_pending_sandboxes_reject_resource_changes() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);

        let mut paused = lifecycle_state("paused-resize");
        paused.backend = Some(BackendType::Firecracker);
        paused.paused_at = Some(chrono::Utc::now().to_rfc3339());
        paused.full_state_checkpoint = Some(uuid::Uuid::new_v4().to_string());
        manager
            .sandboxes
            .insert("paused-resize".to_string(), paused);
        let error = manager
            .update_resources("paused-resize", 8, 16_384)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("paused with a full-state checkpoint")
        );
        assert_eq!(manager.get_state("paused-resize").unwrap().vcpus, 1);

        insert_pause_recovery(&mut manager, "recovery-resize");
        let error = manager
            .update_resources("recovery-resize", 8, 16_384)
            .unwrap_err();
        assert!(error.to_string().contains("pending full-state recovery"));
        assert_eq!(manager.get_state("recovery-resize").unwrap().vcpus, 1);
    }

    #[tokio::test]
    async fn ambiguous_fork_restore_owner_rejects_a_second_start() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let name = "ambiguous-fork-child";
        let mut state = lifecycle_state(name);
        state.backend = Some(BackendType::Firecracker);
        state.full_state_lineage = true;
        manager.sandboxes.insert(name.to_string(), state);
        manager.pause_recovery.insert(
            name.to_string(),
            PendingFullStateRecovery {
                sandbox: Some(Box::new(RecoverySandbox {
                    name: name.to_string(),
                    running: false,
                    stop_attempts: None,
                    stop_failures_before_success: 0,
                })),
                staging_path: temp_dir.path().join("published-source-checkpoint"),
                completed_snapshot: None,
            },
        );

        let error = manager.start(name).await.unwrap_err();
        assert!(error.to_string().contains("pending full-state recovery"));
        assert!(manager.pause_recovery.contains_key(name));
        assert!(!manager.running.contains_key(name));
    }

    #[tokio::test]
    async fn full_state_lineage_rejects_lossy_ordinary_stop() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("full-state-child");
        state.backend = Some(BackendType::Firecracker);
        state.full_state_lineage = true;
        manager
            .sandboxes
            .insert("full-state-child".to_string(), state);
        manager.running.insert(
            "full-state-child".to_string(),
            Box::new(RecoverySandbox {
                name: "full-state-child".to_string(),
                running: true,
                stop_attempts: None,
                stop_failures_before_success: 0,
            }),
        );

        manager
            .set_labels(
                "full-state-child",
                &HashMap::from([("team".to_string(), "sandbox".to_string())]),
            )
            .unwrap();
        assert!(
            manager
                .get_state("full-state-child")
                .unwrap()
                .full_state_lineage
        );

        let error = manager.stop("full-state-child").await.unwrap_err();
        assert!(error.to_string().contains("use full-state pause"));
        assert!(manager.is_running("full-state-child"));
    }

    #[tokio::test]
    async fn persisted_full_state_lineage_rejects_cold_start_after_manager_restart() {
        let temp_dir = TempDir::new().unwrap();
        let manager = new_test_manager(&temp_dir);
        let name = "restarted-full-state-child";
        let mut state = lifecycle_state(name);
        state.backend = Some(BackendType::Firecracker);
        state.full_state_lineage = true;
        manager.save_sandbox(&state).unwrap();
        drop(manager);

        let persisted = VmManager::load_sandboxes(&temp_dir.path().join("sandboxes")).unwrap();
        let mut restarted = new_test_manager(&temp_dir);
        restarted.sandboxes = persisted;

        let error = restarted.start(name).await.unwrap_err();
        assert!(error.to_string().contains("cold start would lose"));
        assert!(!restarted.running.contains_key(name));
    }

    #[tokio::test]
    async fn ordinary_stop_retains_handle_when_typed_error_says_runtime_may_survive() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("ambiguous-stop");
        state.backend = Some(BackendType::Firecracker);
        manager
            .sandboxes
            .insert("ambiguous-stop".to_string(), state);
        let stop_attempts = Arc::new(AtomicUsize::new(0));
        manager.running.insert(
            "ambiguous-stop".to_string(),
            Box::new(RecoverySandbox {
                name: "ambiguous-stop".to_string(),
                // Simulate a best-effort process probe returning false even
                // though the strict termination result remains ambiguous.
                running: false,
                stop_attempts: Some(Arc::clone(&stop_attempts)),
                stop_failures_before_success: 1,
            }),
        );

        let error = manager.stop("ambiguous-stop").await.unwrap_err();
        assert!(error.to_string().contains("failed to stop sandbox"));
        assert!(manager.running.contains_key("ambiguous-stop"));
        assert_eq!(stop_attempts.load(Ordering::SeqCst), 1);

        manager.stop("ambiguous-stop").await.unwrap();
        assert!(!manager.running.contains_key("ambiguous-stop"));
        assert_eq!(stop_attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_touch_activity_updates_timestamp() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("touch-test");
        state.last_activity_at = Some("2026-01-01T00:00:00Z".to_string());
        manager.sandboxes.insert("touch-test".to_string(), state);

        manager.touch_activity("touch-test").unwrap();
        let updated = manager
            .get_state("touch-test")
            .unwrap()
            .last_activity_at
            .clone()
            .unwrap();
        assert_ne!(updated, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn test_recover_clears_archive_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("recover-test");
        state.archived_at = Some("2026-01-01T02:00:00Z".to_string());
        state.archived_reason = Some("manual archive".to_string());
        manager.sandboxes.insert("recover-test".to_string(), state);

        manager.recover("recover-test").unwrap();
        let recovered = manager.get_state("recover-test").unwrap();
        assert!(recovered.archived_at.is_none());
        assert!(recovered.archived_reason.is_none());
        assert!(recovered.last_activity_at.is_some());
    }

    #[test]
    fn test_mark_dormant_persists_state_and_status() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        manager
            .sandboxes
            .insert("dormant-test".to_string(), lifecycle_state("dormant-test"));

        manager
            .mark_dormant("dormant-test", "2026-02-01T00:00:00Z", "unused for 30 days")
            .unwrap();
        let state = manager.get_state("dormant-test").unwrap();
        assert_eq!(state.dormant_at.as_deref(), Some("2026-02-01T00:00:00Z"));
        assert_eq!(state.dormant_reason.as_deref(), Some("unused for 30 days"));
        assert_eq!(state.status(false), "dormant");
        assert_eq!(
            manager.dormant_time("dormant-test").unwrap().to_rfc3339(),
            "2026-02-01T00:00:00+00:00"
        );
    }

    #[test]
    fn failed_lifecycle_policy_save_does_not_mutate_manager_state() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        manager.sandboxes.insert(
            "policy-save-failure".to_string(),
            lifecycle_state("policy-save-failure"),
        );
        let sandboxes_dir = temp_dir.path().join("sandboxes");
        std::fs::remove_dir(&sandboxes_dir).unwrap();
        std::fs::write(&sandboxes_dir, b"blocks state directory").unwrap();

        let result = manager.set_lifecycle_policy(
            "policy-save-failure",
            Some(SandboxLifecyclePolicy {
                auto_stop_after_seconds: None,
                auto_archive_after_seconds: Some(0),
                auto_delete_after_seconds: Some(0),
            }),
        );

        assert!(result.is_err());
        assert!(
            manager
                .get_state("policy-save-failure")
                .unwrap()
                .lifecycle_policy
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_reconcile_lifecycle_dry_run_archive_does_not_mutate() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("archive-dry-run");
        state.last_activity_at =
            Some((chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339());
        state.lifecycle_policy = Some(SandboxLifecyclePolicy {
            auto_stop_after_seconds: None,
            auto_archive_after_seconds: Some(60),
            auto_delete_after_seconds: None,
        });
        manager
            .sandboxes
            .insert("archive-dry-run".to_string(), state.clone());

        let result = manager.reconcile_lifecycle(true).await.unwrap();
        assert!(result.archived.contains(&"archive-dry-run".to_string()));
        assert!(
            manager
                .get_state("archive-dry-run")
                .unwrap()
                .archived_at
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_reconcile_lifecycle_archives_when_threshold_hit() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("archive-now");
        state.last_activity_at =
            Some((chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339());
        state.lifecycle_policy = Some(SandboxLifecyclePolicy {
            auto_stop_after_seconds: None,
            auto_archive_after_seconds: Some(0),
            auto_delete_after_seconds: None,
        });
        manager.sandboxes.insert("archive-now".to_string(), state);

        let result = manager.reconcile_lifecycle(false).await.unwrap();
        assert!(result.archived.contains(&"archive-now".to_string()));
        let archived = manager.get_state("archive-now").unwrap();
        assert!(archived.archived_at.is_some());
        assert!(archived.archived_reason.is_some());
    }

    #[tokio::test]
    async fn test_reconcile_lifecycle_stops_running_sandbox() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        let mut state = lifecycle_state("stop-now");
        state.last_activity_at =
            Some((chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339());
        state.lifecycle_policy = Some(SandboxLifecyclePolicy {
            auto_stop_after_seconds: Some(10),
            auto_archive_after_seconds: None,
            auto_delete_after_seconds: None,
        });
        manager.sandboxes.insert("stop-now".to_string(), state);
        manager.running.insert(
            "stop-now".to_string(),
            Box::new(TestSandbox {
                name: "stop-now".to_string(),
                running: true,
            }),
        );

        let result = manager.reconcile_lifecycle(false).await.unwrap();
        assert!(result.stopped.contains(&"stop-now".to_string()));
        assert!(!manager.is_running("stop-now"));
    }

    #[tokio::test]
    async fn test_reconcile_lifecycle_deletes_archived_sandbox() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = VmManager::for_tests(temp_dir.path()).unwrap();
        let mut state = lifecycle_state("delete-now");
        state.archived_at = Some((chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339());
        state.archived_reason = Some("stale".to_string());
        state.lifecycle_policy = Some(SandboxLifecyclePolicy {
            auto_stop_after_seconds: None,
            auto_archive_after_seconds: None,
            auto_delete_after_seconds: Some(60),
        });
        manager.sandboxes.insert("delete-now".to_string(), state);

        let result = manager.reconcile_lifecycle(false).await.unwrap();
        assert!(result.removed.contains(&"delete-now".to_string()));
        assert!(!manager.exists("delete-now"));
    }

    #[test]
    fn test_set_identity_metadata_preserves_uuid_and_timestamps() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = new_test_manager(&temp_dir);
        manager
            .sandboxes
            .insert("id-test".to_string(), lifecycle_state("id-test"));

        manager
            .set_identity_metadata(
                "id-test",
                "fixed-uuid",
                "2020-01-01T00:00:00Z",
                Some("2020-01-01T01:00:00Z"),
            )
            .unwrap();

        let state = manager.get_state("id-test").unwrap();
        assert_eq!(state.uuid, "fixed-uuid");
        assert_eq!(state.created_at, "2020-01-01T00:00:00Z");
        assert_eq!(state.expires_at.as_deref(), Some("2020-01-01T01:00:00Z"));
    }
}
