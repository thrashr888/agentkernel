//! Virtual Machine Manager
//!
//! This module provides the interface to sandboxes via Firecracker microVMs
//! or containers (Docker/Podman) as fallback when KVM is not available.

use crate::audit::{AuditEvent, log_event};
use crate::backend::{
    BackendType, FileInjection, PortMapping, RemoteSandboxContext, ResolvedEndpoint, Sandbox,
    SandboxConfig, SandboxRuntimeMetadata, backend_capabilities, create_sandbox,
    create_sandbox_with_state, detect_best_backend,
};
use crate::config::Config;
use crate::docker_backend::detect_container_runtime;
use crate::languages::docker_image_to_firecracker_runtime;
use crate::permissions::Permissions;
use crate::pool::ContainerPool;
use crate::proxy::{ProxyConfig, ProxyHandle, SecretBinding};
use crate::secrets::{SecretBackend, SecretVault};
use crate::validation;
use crate::volume::{VolumeManager, VolumeMount};
use anyhow::{Result, bail};

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
use std::sync::LazyLock;
use tokio::sync::RwLock;

/// Global proxy handle registry. Proxy handles must outlive individual VmManager
/// instances since VmManager is created fresh per HTTP request.
static PROXY_HANDLES: std::sync::LazyLock<RwLock<HashMap<String, ProxyHandle>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "enterprise")]
static POLICY_ENGINE_CACHE: LazyLock<Option<Arc<crate::policy::PolicyEngine>>> =
    LazyLock::new(|| {
        let default_config = PathBuf::from("agentkernel.toml");
        if !default_config.exists() {
            return None;
        }

        let cfg = match Config::from_file(&default_config) {
            Ok(cfg) => cfg,
            Err(_) => return None,
        };

        if !cfg.enterprise.enabled {
            return None;
        }

        match crate::policy::PolicyEngine::new(&cfg.enterprise) {
            Ok(engine) => {
                eprintln!("[enterprise] Policy engine initialized");
                Some(Arc::new(engine))
            }
            Err(e) => {
                eprintln!("[enterprise] Failed to initialize policy engine: {}", e);
                None
            }
        }
    });
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
    /// Original config file path used to create or start this sandbox.
    #[serde(default)]
    pub config_path: Option<String>,
    /// Time-to-live in seconds (None = no expiry)
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// When this sandbox expires (RFC3339). Computed from created_at + ttl_seconds.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Port mappings (host:container)
    #[serde(default)]
    pub ports: Vec<PortMapping>,
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
    /// Template name this sandbox was created from (if any).
    #[serde(default)]
    pub created_from_template: Option<String>,
    /// Human guidance text associated with the source template.
    #[serde(default)]
    pub template_help_text: Option<String>,
    /// User-defined labels for fleet management and filtering.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
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
    /// Optional lifecycle automation policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_policy: Option<SandboxLifecyclePolicy>,
}

impl SandboxState {
    /// Render status from persisted archive state + runtime liveness.
    pub fn status(&self, running: bool) -> &'static str {
        if self.archived_at.is_some() {
            "archived"
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
    /// Persisted sandbox configurations
    sandboxes: HashMap<String, SandboxState>,
    /// Data directory for persistence
    data_dir: PathBuf,
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

/// Escape a string for use inside a single-quoted shell command.
fn shell_escape(s: &str) -> String {
    // Replace ' with '\'' (end quote, escaped quote, start quote)
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run a command only when `names` is non-empty; returns `Err` (skip) when empty.
fn batch_cmd(names: &[String], cmd: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    if names.is_empty() {
        return Err(std::io::Error::other("empty"));
    }
    std::process::Command::new(cmd).args(args).output()
}

impl VmManager {
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
            Self::find_images_dir().ok().map(|d| d.join("rootfs"))
        } else {
            None
        };

        // Find next available CID
        let max_cid = sandboxes.values().map(|s| s.vsock_cid).max().unwrap_or(2);

        // Initialize enterprise policy engine once per process when configured
        #[cfg(feature = "enterprise")]
        let policy_engine = POLICY_ENGINE_CACHE.clone();

        let mut manager = Self {
            backend,
            running: HashMap::new(),
            sandboxes,
            data_dir,
            rootfs_dir,
            next_cid: max_cid + 1,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine,
        };

        // Detect already-running sandboxes
        manager.detect_running_sandboxes();

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
            if let Ok(sandbox) = create_sandbox_with_state(
                backend,
                &name,
                &crate::config::OrchestratorConfig::default(),
                backend.is_remote().then(|| state.remote_context()),
            ) {
                self.running.insert(name, sandbox);
            }
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
        let path = self
            .data_dir
            .join("sandboxes")
            .join(format!("{}.json", state.name));
        let content = serde_json::to_string_pretty(state)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Delete a sandbox state from disk
    fn delete_sandbox(&self, name: &str) -> Result<()> {
        let path = self
            .data_dir
            .join("sandboxes")
            .join(format!("{}.json", name));
        if path.exists() {
            std::fs::remove_file(path)?;
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
        self.create_internal(
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
        self.create_internal(
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

        if !crate::backend::backend_available(backend) {
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
            config_path: None,
            ttl_seconds,
            expires_at,
            ports,
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: Some(created_at),
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.lifecycle_policy = policy;
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
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

        let new_expiry = {
            let state = self
                .sandboxes
                .get_mut(name)
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

            // Update state
            state.expires_at = Some(new_expiry_str.clone());
            // Also update ttl_seconds to reflect total TTL from creation
            if let Ok(created) = state.created_at.parse::<DateTime<Utc>>() {
                let total_secs = (new_exp - created).num_seconds();
                if total_secs > 0 {
                    state.ttl_seconds = Some(total_secs as u64);
                }
            }

            Some(new_expiry_str)
        };

        // Save the updated state
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        self.save_sandbox(state)?;

        Ok(new_expiry)
    }

    /// Recover an archived sandbox back to a normal lifecycle state.
    pub fn recover(&mut self, name: &str) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

        state.archived_at = None;
        state.archived_reason = None;
        state.last_activity_at = Some(chrono::Utc::now().to_rfc3339());

        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
        Ok(())
    }

    /// Start a sandbox
    pub async fn start(&mut self, name: &str) -> Result<()> {
        self.start_with_permissions(name, &Permissions::default())
            .await
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
        let start_time = std::time::Instant::now();
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?
            .clone();

        if state.archived_at.is_some() {
            bail!(
                "Sandbox '{}' is archived. Recover it before starting (POST /sandboxes/{}/recover).",
                name,
                name
            );
        }

        if self.running.contains_key(name) {
            bail!("Sandbox '{}' is already running", name);
        }

        // Enterprise policy check for start
        #[cfg(feature = "enterprise")]
        self.check_enterprise_policy(crate::policy::Action::Run, name, "unknown", &state.image)
            .await?;

        // Use the backend from stored state, or fall back to current backend
        let backend = state.backend.unwrap_or(self.backend);
        let capabilities = backend_capabilities(backend);

        if perms.mount_home && !capabilities.mount_home {
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
        if !state.secret_bindings.is_empty() && !capabilities.proxy_secret_bindings {
            bail!(
                "Backend '{}' does not support proxy-based secret bindings; use secret env vars or secret files instead",
                backend
            );
        }

        // Create sandbox using unified factory
        let mut sandbox = create_sandbox_with_state(
            backend,
            name,
            &crate::config::OrchestratorConfig::default(),
            backend.is_remote().then(|| state.remote_context()),
        )?;

        // Convert permissions to SandboxConfig
        let work_dir = if perms.mount_cwd {
            state.work_dir.clone().or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
        } else {
            None
        };

        // Build environment variables if pass_env is enabled
        let mut env: Vec<(String, String)> = if perms.pass_env {
            ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"]
                .iter()
                .filter_map(|&var| std::env::var(var).ok().map(|val| (var.to_string(), val)))
                .collect()
        } else {
            Vec::new()
        };

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

        // Start secret injection proxy if bindings are configured
        if !state.secret_bindings.is_empty() {
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

            if !resolved_secrets.is_empty() {
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
                        eprintln!("Warning: Failed to start secret proxy: {}", e);
                    }
                }
            }
        }

        // Auto-inject org-level LLM keys from [llm_keys] config
        {
            let config_path = std::path::PathBuf::from("agentkernel.toml");
            if config_path.exists()
                && let Ok(toml_cfg) = Config::from_file(&config_path)
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
        let volume_args = if !state.volumes.is_empty() {
            let volume_manager = VolumeManager::new()?;
            let mut args = Vec::new();
            for spec in &state.volumes {
                let mount = VolumeMount::parse(spec)?;
                // Validate volume exists
                if !volume_manager.exists(&mount.slug) {
                    bail!(
                        "Volume '{}' not found. Create it with: agentkernel volume create {}",
                        mount.slug,
                        mount.slug
                    );
                }
                args.push(mount.to_docker_arg(volume_manager.volumes_dir()));
            }
            args
        } else {
            Vec::new()
        };

        let config = SandboxConfig {
            image: state.image.clone(),
            vcpus: state.vcpus,
            memory_mb: perms.max_memory_mb.unwrap_or(state.memory_mb),
            mount_cwd: perms.mount_cwd,
            work_dir,
            env,
            network: perms.network,
            read_only: perms.read_only_root,
            mount_home: perms.mount_home,
            files: files.to_vec(),
            ports: state.ports.clone(),
            ssh: ssh_config.clone(),
            volumes: volume_args,
        };

        sandbox.start(&config).await?;
        if let Some(persisted) = self.sandboxes.get_mut(name) {
            persisted.work_dir = config.work_dir.clone();
            if let Some(metadata) = sandbox.runtime_metadata() {
                Self::apply_runtime_metadata(persisted, &metadata);
            }
            let snapshot = persisted.clone();
            self.save_sandbox(&snapshot)?;
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
                    self.inject_placeholder_secrets(sandbox.as_mut(), name, &resolved, backend)
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

        self.running.insert(name.to_string(), sandbox);
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
            && let Ok(cfg) = Config::from_file(&PathBuf::from("agentkernel.toml"))
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
        crate::metrics::record_command(
            &self.backend.to_string(),
            exec_start.elapsed().as_secs_f64(),
        );

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

    /// Stop a sandbox
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        let stop_start = std::time::Instant::now();
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
        if let Some(mut sandbox) = self.running.remove(name) {
            sandbox.stop().await?;
            if let Some(metadata) = sandbox.runtime_metadata()
                && let Some(state) = self.sandboxes.get_mut(name)
            {
                Self::apply_runtime_metadata(state, &metadata);
                let snapshot = state.clone();
                self.save_sandbox(&snapshot)?;
            }
            log_event(AuditEvent::SandboxStopped {
                name: name.to_string(),
            });
            crate::metrics::record_sandbox_lifecycle(
                "stopped",
                &self.backend.to_string(),
                stop_start.elapsed().as_secs_f64(),
            );
        }
        Ok(())
    }

    /// Remove a sandbox
    pub async fn remove(&mut self, name: &str) -> Result<()> {
        let remove_start = std::time::Instant::now();
        // Shut down the proxy if running
        if let Some(handle) = PROXY_HANDLES.write().await.remove(name) {
            let _ = handle.shutdown_tx.send(());
        }
        if let Some(mut sandbox) = self.running.remove(name) {
            let _ = sandbox.remove().await;
        } else if let Some(state) = self.sandboxes.get(name).cloned() {
            let backend = state.backend.unwrap_or(self.backend);
            if let Ok(mut sandbox) = create_sandbox_with_state(
                backend,
                name,
                &crate::config::OrchestratorConfig::default(),
                backend.is_remote().then(|| state.remote_context()),
            ) {
                let _ = sandbox.remove().await;
            }
        }

        self.delete_sandbox(name)?;
        self.sandboxes.remove(name);

        log_event(AuditEvent::SandboxRemoved {
            name: name.to_string(),
        });
        crate::metrics::record_sandbox_lifecycle(
            "removed",
            &self.backend.to_string(),
            remove_start.elapsed().as_secs_f64(),
        );
        crate::metrics::dec_active_sandboxes();
        crate::llm_intercept::LLM_USAGE
            .write()
            .await
            .clear_sandbox(name);

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
                        if let Some(state) = self.sandboxes.get_mut(&decision.sandbox) {
                            state.archived_at = Some(now.to_rfc3339());
                            state.archived_reason = Some(decision.reason.clone());
                            let snapshot = state.clone();
                            self.save_sandbox(&snapshot)?;
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
                let running = self
                    .running
                    .get(name)
                    .map(|s| s.is_running())
                    .unwrap_or_else(|| {
                        state.backend.is_some_and(|backend| backend.is_remote())
                            && state
                                .remote_metadata
                                .get("last_known_status")
                                .is_some_and(|value| value == "running")
                    });
                (name.as_str(), running, state.backend)
            })
            .collect()
    }

    /// Check if a sandbox exists
    pub fn exists(&self, name: &str) -> bool {
        self.sandboxes.contains_key(name)
    }

    /// Get the backend type for a sandbox (from stored state or current default)
    /// Check if a sandbox is currently running
    pub fn is_running(&self, name: &str) -> bool {
        self.running
            .get(name)
            .map(|s| s.is_running())
            .unwrap_or_else(|| {
                self.sandboxes.get(name).is_some_and(|state| {
                    state.backend.is_some_and(|backend| backend.is_remote())
                        && state
                            .remote_metadata
                            .get("last_known_status")
                            .is_some_and(|value| value == "running")
                })
            })
    }

    /// Update persisted sandbox resource values without recreating.
    pub fn update_resources(&mut self, name: &str, vcpus: u32, memory_mb: u64) -> Result<()> {
        let state = self
            .sandboxes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;
        state.vcpus = vcpus;
        state.memory_mb = memory_mb;
        let snapshot = state.clone();
        self.save_sandbox(&snapshot)?;
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
            env,
            network: perms.network,
            read_only: perms.read_only_root,
            mount_home: perms.mount_home,
            files: files.to_vec(),
            ports: Vec::new(),
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
    use async_trait::async_trait;
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
            config_path: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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

        let state: SandboxState = serde_json::from_str(json).unwrap();
        assert_eq!(state.name, "my-sandbox");
        assert_eq!(state.image, "python:3.12-alpine");
        assert_eq!(state.vcpus, 4);
        assert_eq!(state.memory_mb, 2048);
        assert_eq!(state.vsock_cid, 10);
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
            config_path: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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
            config_path: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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
                config_path: None,
                ttl_seconds: None,
                expires_at: None,
                ports: Vec::new(),
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
                created_from_template: None,
                template_help_text: None,
                labels: HashMap::new(),
                description: None,
                last_activity_at: None,
                archived_at: None,
                archived_reason: None,
                lifecycle_policy: None,
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
            backend: BackendType::Docker,
            running: HashMap::new(),
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
            config_path: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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
            backend: BackendType::Docker,
            running: HashMap::new(),
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
                config_path: None,
                ttl_seconds: None,
                expires_at: None,
                ports: Vec::new(),
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
                created_from_template: None,
                template_help_text: None,
                labels,
                description: None,
                last_activity_at: None,
                archived_at: None,
                archived_reason: None,
                lifecycle_policy: None,
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
            backend: BackendType::Docker,
            running: HashMap::new(),
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
            config_path: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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
            config_path: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: labels.clone(),
            description: Some("Test sandbox".to_string()),
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
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
        std::fs::create_dir_all(temp_dir.path().join("sandboxes")).unwrap();
        VmManager {
            sandboxes: HashMap::new(),
            data_dir: temp_dir.path().to_path_buf(),
            backend: BackendType::Docker,
            running: HashMap::new(),
            rootfs_dir: None,
            next_cid: 3,
            detached: HashMap::new(),
            #[cfg(feature = "enterprise")]
            policy_engine: None,
        }
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
            config_path: None,
            ttl_seconds: Some(3600),
            expires_at: Some("2026-01-01T01:00:00Z".to_string()),
            ports: Vec::new(),
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
            created_from_template: None,
            template_help_text: None,
            labels: HashMap::new(),
            description: None,
            last_activity_at: Some("2026-01-01T00:00:00Z".to_string()),
            archived_at: None,
            archived_reason: None,
            lifecycle_policy: None,
        }
    }

    #[allow(dead_code)]
    struct TestSandbox {
        name: String,
        running: bool,
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
        let mut manager = new_test_manager(&temp_dir);
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
