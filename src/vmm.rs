//! Virtual Machine Manager
//!
//! This module provides the interface to sandboxes via Firecracker microVMs
//! or containers (Docker/Podman) as fallback when KVM is not available.

use crate::audit::{AuditEvent, log_event};
use crate::backend::{
    BackendType, FileInjection, PortMapping, Sandbox, SandboxConfig, create_sandbox,
    detect_best_backend,
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
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Global proxy handle registry. Proxy handles must outlive individual VmManager
/// instances since VmManager is created fresh per HTTP request.
static PROXY_HANDLES: std::sync::LazyLock<RwLock<HashMap<String, ProxyHandle>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));
use tokio::sync::OnceCell;

/// Global container pool for fast ephemeral runs
static CONTAINER_POOL: OnceCell<Arc<ContainerPool>> = OnceCell::const_new();

/// Get or initialize the global container pool
async fn get_pool() -> Result<Arc<ContainerPool>> {
    CONTAINER_POOL
        .get_or_try_init(|| async {
            let pool = ContainerPool::with_config(5, 20, "alpine:3.20")?;
            pool.start().await?;
            Ok(Arc::new(pool))
        })
        .await
        .cloned()
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
    /// Host port of the running proxy (if any)
    #[serde(default)]
    pub proxy_port: Option<u16>,
    /// Shell script to run inside the sandbox after start (from template init_script)
    #[serde(default)]
    pub init_script: Option<String>,
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
    policy_engine: Option<crate::policy::PolicyEngine>,
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

        // Use explicit backend or auto-detect
        let backend = if let Some(b) = explicit_backend {
            // Verify the requested backend is available
            if !crate::backend::backend_available(b) {
                bail!("Backend '{}' is not available on this system", b);
            }
            b
        } else {
            detect_best_backend().ok_or_else(|| {
                anyhow::anyhow!(
                    "No sandbox backend available. Need one of: KVM (Linux), Apple containers (macOS 26+), or Docker/Podman."
                )
            })?
        };

        // Find rootfs path (only needed for Firecracker)
        let rootfs_dir = if backend == BackendType::Firecracker {
            Self::find_images_dir().ok().map(|d| d.join("rootfs"))
        } else {
            None
        };

        // Load existing sandboxes
        let sandboxes = Self::load_sandboxes(&sandboxes_dir)?;

        // Find next available CID
        let max_cid = sandboxes.values().map(|s| s.vsock_cid).max().unwrap_or(2);

        // Initialize enterprise policy engine if configured
        #[cfg(feature = "enterprise")]
        let policy_engine = {
            let default_config = PathBuf::from("agentkernel.toml");
            if default_config.exists() {
                if let Ok(cfg) = Config::from_file(&default_config) {
                    if cfg.enterprise.enabled {
                        match crate::policy::PolicyEngine::new(&cfg.enterprise) {
                            Ok(engine) => {
                                eprintln!("[enterprise] Policy engine initialized");
                                Some(engine)
                            }
                            Err(e) => {
                                eprintln!("[enterprise] Failed to initialize policy engine: {}", e);
                                None
                            }
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

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
            let backend = self
                .sandboxes
                .get(&name)
                .and_then(|s| s.backend)
                .unwrap_or(self.backend);
            if let Ok(sandbox) = create_sandbox(backend, &name) {
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
                        let updated = serde_json::to_string_pretty(&state)?;
                        std::fs::write(&path, updated)?;
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
        let create_start = std::time::Instant::now();

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
        let effective_image = if self.backend == BackendType::Firecracker {
            let runtime = docker_image_to_firecracker_runtime(image);
            self.rootfs_path(runtime)?;
            runtime.to_string()
        } else {
            image.to_string()
        };

        let vsock_cid = self.next_cid;
        self.next_cid += 1;

        let created = chrono::Utc::now();
        let expires_at =
            ttl_seconds.map(|ttl| (created + chrono::Duration::seconds(ttl as i64)).to_rfc3339());

        let state = SandboxState {
            name: name.to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: effective_image.clone(),
            vcpus,
            memory_mb,
            vsock_cid,
            created_at: created.to_rfc3339(),
            backend: Some(self.backend),
            remote_id: None,
            remote_namespace: None,
            ttl_seconds,
            expires_at,
            ports,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent,
            secret_bindings: Vec::new(),
            secret_files: Vec::new(),
            proxy_port: None,
            init_script: None,
        };

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);

        log_event(AuditEvent::SandboxCreated {
            name: name.to_string(),
            image: effective_image,
            backend: self.backend.to_string(),
        });
        crate::metrics::record_sandbox_lifecycle(
            "created",
            &self.backend.to_string(),
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

        if self.running.contains_key(name) {
            bail!("Sandbox '{}' is already running", name);
        }

        // Enterprise policy check for start
        #[cfg(feature = "enterprise")]
        self.check_enterprise_policy(crate::policy::Action::Run, name, "unknown", &state.image)
            .await?;

        // Use the backend from stored state, or fall back to current backend
        let backend = state.backend.unwrap_or(self.backend);

        // Create sandbox using unified factory
        let mut sandbox = create_sandbox(backend, name)?;

        // Convert permissions to SandboxConfig
        let work_dir = if perms.mount_cwd {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
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
                };

                match crate::proxy::start_proxy(proxy_config, resolved_secrets).await {
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
                        "cat /etc/ssl/certs/ca-certificates.crt /usr/local/share/ca-certificates/agentkernel-proxy.crt > /etc/ssl/certs/agentkernel-combined.crt 2>/dev/null || \
                         cat /etc/pki/tls/certs/ca-bundle.crt /usr/local/share/ca-certificates/agentkernel-proxy.crt > /etc/ssl/certs/agentkernel-combined.crt 2>/dev/null || \
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
                "claude" => Some("npm install -g @anthropic-ai/claude-code"),
                "gemini" => Some("npm install -g @google/gemini-cli"),
                "codex" => Some("npm install -g @openai/codex"),
                "opencode" => Some("npm install -g opencode"),
                "amp" => Some("npm install -g @sourcegraph/amp"),
                "pi" => Some("npm install -g @mariozechner/pi-coding-agent"),
                "copilot" => Some("npm install -g @githubnext/github-copilot-cli"),
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

        // Run init_script if specified (from template config)
        if let Some(ref script) = state.init_script {
            eprintln!("Running init script...");
            match sandbox.exec(&["sh", "-c", script]).await {
                Ok(result) if result.exit_code == 0 => {
                    eprintln!("Init script completed successfully");
                }
                Ok(result) => {
                    eprintln!(
                        "Warning: init script exited with code {}: {}",
                        result.exit_code,
                        result.stderr.trim()
                    );
                }
                Err(e) => {
                    eprintln!("Warning: Failed to run init script: {}", e);
                }
            }
        }

        self.running.insert(name.to_string(), sandbox);

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
        crate::metrics::record_command(
            &self.backend.to_string(),
            exec_start.elapsed().as_secs_f64(),
        );

        log_event(AuditEvent::CommandExecuted {
            sandbox: name.to_string(),
            command: cmd.to_vec(),
            exit_code: Some(result.exit_code),
        });

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

        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        let id = format!("{:08x}", rand::thread_rng().r#gen::<u32>());
        let stdout_path = format!("/tmp/ak-{id}.out");
        let stderr_path = format!("/tmp/ak-{id}.err");

        // Wrap the command to run in background with output capture
        let escaped_cmd: Vec<String> = cmd.iter().map(|c| shell_escape(c)).collect();
        let wrapped = format!(
            "nohup sh -c '{} > {} 2> {} & echo $!'",
            escaped_cmd.join(" "),
            stdout_path,
            stderr_path,
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

        // Check if process is still running
        let sandbox = self
            .running
            .get_mut(&cmd.sandbox)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' is not running", cmd.sandbox))?;

        let check_cmd = format!(
            "kill -0 {} 2>/dev/null && echo running || (wait {} 2>/dev/null; echo $?)",
            cmd.pid, cmd.pid
        );
        let result = sandbox
            .exec_with_options(
                &["sh", "-c", &check_cmd],
                &crate::backend::ExecOptions::default(),
            )
            .await?;

        let output = result.stdout.trim().to_string();
        if output == "running" {
            return Ok(cmd);
        }

        // Process finished — parse exit code
        let exit_code: i32 = output.parse().unwrap_or(1);
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
        Ok(cmd)
    }

    /// Get stdout/stderr logs from a detached command.
    pub async fn detached_logs(&mut self, cmd_id: &str, stream: Option<&str>) -> Result<String> {
        let cmd = self
            .detached
            .get(cmd_id)
            .ok_or_else(|| anyhow::anyhow!("Detached command '{}' not found", cmd_id))?
            .clone();

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

        let sandbox = self
            .running
            .get_mut(&cmd.sandbox)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' is not running", cmd.sandbox))?;

        let kill_cmd = format!("kill {} 2>/dev/null || true", cmd.pid);
        sandbox
            .exec_with_options(
                &["sh", "-c", &kill_cmd],
                &crate::backend::ExecOptions::default(),
            )
            .await?;

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
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        log_event(AuditEvent::SessionAttached {
            sandbox: name.to_string(),
        });

        sandbox.attach_with_env(None, env).await
    }

    /// Stop a sandbox
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        let stop_start = std::time::Instant::now();
        // Shut down the proxy if running
        if let Some(handle) = PROXY_HANDLES.write().await.remove(name) {
            let _ = handle.shutdown_tx.send(());
        }
        if let Some(mut sandbox) = self.running.remove(name) {
            sandbox.stop().await?;
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
            let _ = sandbox.stop().await;
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
                    .unwrap_or(false);
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
            .unwrap_or(false)
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
        Self::enforce_command_policy(cmd)?;
        // Build config from permissions
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

        // Use optimized `docker/podman run --rm` for container backends
        // Note: File injection not supported in fast path; use generic path if files specified
        if files.is_empty() {
            match self.backend {
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
                _ => {
                    // Fall through to generic start→exec→stop for other backends
                }
            }
        }

        // Generic path for non-container backends or when files need injection
        let name = format!("ephemeral-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let mut sandbox = create_sandbox(self.backend, &name)?;

        // Start sandbox
        sandbox.start(&config).await?;

        // Inject files if specified
        if !files.is_empty() {
            sandbox.inject_files(files).await?;
        }

        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        let result = sandbox.exec(&cmd_refs).await;

        // Always stop, even on error
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
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        sandbox.write_file(path, content).await?;

        log_event(AuditEvent::FileWritten {
            sandbox: name.to_string(),
            path: path.to_string(),
        });

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
        let cmd = vec!["rm".to_string(), "-f".to_string(), path.to_string()];
        self.exec_cmd(name, &cmd).await?;
        Ok(())
    }

    /// Read a file from a running sandbox
    pub async fn read_file(&mut self, name: &str, path: &str) -> Result<Vec<u8>> {
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        let content = sandbox.read_file(path).await?;

        log_event(AuditEvent::FileRead {
            sandbox: name.to_string(),
            path: path.to_string(),
        });

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sandbox_state_serialize() {
        let state = SandboxState {
            name: "test-sandbox".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.20".to_string(),
            vcpus: 2,
            memory_mb: 1024,
            vsock_cid: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_files: Vec::new(),
            proxy_port: None,
            init_script: None,
        };

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("test-sandbox"));
        assert!(json.contains("alpine:3.20"));
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
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_files: Vec::new(),
            proxy_port: None,
            init_script: None,
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

        // Create a valid sandbox JSON file
        let state = SandboxState {
            name: "loaded-sandbox".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            image: "alpine:3.20".to_string(),
            vcpus: 1,
            memory_mb: 256,
            vsock_cid: 4,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
            remote_id: None,
            remote_namespace: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_files: Vec::new(),
            proxy_port: None,
            init_script: None,
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
        assert_eq!(loaded.image, "alpine:3.20");
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
            "image": "alpine:3.20",
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
                ttl_seconds: None,
                expires_at: None,
                ports: Vec::new(),
                ssh_enabled: false,
                ssh_host_port: None,
                volumes: Vec::new(),
                agent: None,
                secret_bindings: Vec::new(),
                secret_files: Vec::new(),
                proxy_port: None,
                init_script: None,
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
}
