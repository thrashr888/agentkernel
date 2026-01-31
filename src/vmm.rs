//! Virtual Machine Manager
//!
//! This module provides the interface to sandboxes via Firecracker microVMs
//! or containers (Docker/Podman) as fallback when KVM is not available.

use crate::audit::{AuditEvent, log_event};
use crate::backend::{
    BackendType, FileInjection, Sandbox, SandboxConfig, create_sandbox, detect_best_backend,
};
use crate::config::Config;
use crate::docker_backend::detect_container_runtime;
use crate::languages::docker_image_to_firecracker_runtime;
use crate::permissions::Permissions;
use crate::pool::ContainerPool;
use crate::validation;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    /// Docker image to use (e.g., "python:3.12-alpine")
    pub image: String,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub vsock_cid: u32,
    pub created_at: String,
    /// Backend type used to create this sandbox
    #[serde(default)]
    pub backend: Option<BackendType>,
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
    /// Enterprise policy engine (when enterprise feature is enabled)
    #[cfg(feature = "enterprise")]
    policy_engine: Option<crate::policy::PolicyEngine>,
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
            #[cfg(feature = "enterprise")]
            policy_engine,
        };

        // Detect already-running sandboxes
        manager.detect_running_sandboxes();

        Ok(manager)
    }

    /// Detect sandboxes that are already running (e.g., Docker containers)
    fn detect_running_sandboxes(&mut self) {
        // Need to collect names first to avoid borrow checker issues
        let sandboxes_to_check: Vec<_> = self
            .sandboxes
            .iter()
            .map(|(name, state)| (name.clone(), state.backend.unwrap_or(self.backend)))
            .collect();

        for (name, sandbox_backend) in sandboxes_to_check {
            // Check if the sandbox is running
            let is_running = match sandbox_backend {
                BackendType::Docker | BackendType::Podman => {
                    self.detect_docker_sandbox_running(&name, sandbox_backend)
                }
                _ => false, // Other backends need more complex detection
            };

            if is_running {
                // Recreate the sandbox object for the running container
                // Note: DockerSandbox::is_running() checks Docker directly
                if let Ok(sandbox) = create_sandbox(sandbox_backend, &name) {
                    self.running.insert(name.clone(), sandbox);
                }
            }
        }
    }

    /// Check if a Docker/Podman sandbox is currently running
    fn detect_docker_sandbox_running(&self, name: &str, backend: BackendType) -> bool {
        use std::process::Command;

        let cmd = match backend {
            BackendType::Docker => "docker",
            BackendType::Podman => "podman",
            _ => return false,
        };

        let container_name = format!("agentkernel-{}", name);

        Command::new(cmd)
            .args(["ps", "-q", "-f", &format!("name={}", container_name)])
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false)
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
                    && let Ok(state) = serde_json::from_str::<SandboxState>(&content)
                {
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

        let state = SandboxState {
            name: name.to_string(),
            image: effective_image.clone(),
            vcpus,
            memory_mb,
            vsock_cid,
            created_at: chrono::Utc::now().to_rfc3339(),
            backend: Some(self.backend),
        };

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);

        log_event(AuditEvent::SandboxCreated {
            name: name.to_string(),
            image: effective_image,
            backend: self.backend.to_string(),
        });

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
        let env = if perms.pass_env {
            ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"]
                .iter()
                .filter_map(|&var| std::env::var(var).ok().map(|val| (var.to_string(), val)))
                .collect()
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
        };

        sandbox.start(&config).await?;

        // Inject files if any were specified
        if !files.is_empty() {
            sandbox.inject_files(files).await?;
        }

        self.running.insert(name.to_string(), sandbox);

        log_event(AuditEvent::SandboxStarted {
            name: name.to_string(),
            profile: Some(format!("{:?}", perms)),
        });

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

        // Convert &[String] to &[&str]
        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();

        let result = sandbox.exec_with_env(&cmd_refs, env).await?;

        log_event(AuditEvent::CommandExecuted {
            sandbox: name.to_string(),
            command: cmd.to_vec(),
            exit_code: Some(result.exit_code),
        });

        if result.exit_code != 0 {
            bail!(
                "Command exited with code {}: {}",
                result.exit_code,
                result.output()
            );
        }

        Ok(result.output())
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
        if let Some(mut sandbox) = self.running.remove(name) {
            sandbox.stop().await?;
            log_event(AuditEvent::SandboxStopped {
                name: name.to_string(),
            });
        }
        Ok(())
    }

    /// Remove a sandbox
    pub async fn remove(&mut self, name: &str) -> Result<()> {
        if let Some(mut sandbox) = self.running.remove(name) {
            let _ = sandbox.stop().await;
        }

        self.delete_sandbox(name)?;
        self.sandboxes.remove(name);

        log_event(AuditEvent::SandboxRemoved {
            name: name.to_string(),
        });

        Ok(())
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

    /// Run a command using the container pool (fast path for ephemeral runs)
    pub async fn run_pooled(cmd: &[String]) -> Result<String> {
        Self::enforce_command_policy(cmd)?;
        let pool = get_pool().await?;
        let container = pool.acquire().await?;
        let result = container.run_command(cmd).await;
        pool.release(container).await;
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
            image: "alpine:3.20".to_string(),
            vcpus: 2,
            memory_mb: 1024,
            vsock_cid: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
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
            image: "node:20-alpine".to_string(),
            vcpus: 1,
            memory_mb: 512,
            vsock_cid: 3,
            created_at: "2024-06-15T12:30:00Z".to_string(),
            backend: None,
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: SandboxState = serde_json::from_str(&json).unwrap();

        assert_eq!(original.name, restored.name);
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
            image: "alpine:3.20".to_string(),
            vcpus: 1,
            memory_mb: 256,
            vsock_cid: 4,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            backend: None,
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
    fn test_next_cid_calculation() {
        let temp_dir = TempDir::new().unwrap();

        // Create sandboxes with various CIDs
        for (name, cid) in [("sb1", 5), ("sb2", 10), ("sb3", 3)] {
            let state = SandboxState {
                name: name.to_string(),
                image: "alpine".to_string(),
                vcpus: 1,
                memory_mb: 256,
                vsock_cid: cid,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                backend: None,
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
