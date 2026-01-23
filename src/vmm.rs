//! Virtual Machine Manager
//!
//! This module provides the interface to sandboxes via Firecracker microVMs
//! or containers (Docker/Podman) as fallback when KVM is not available.

use crate::backend::{BackendType, Sandbox, SandboxConfig, create_sandbox, detect_best_backend};
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

        eprintln!("Using {} backend", backend);

        Ok(Self {
            backend,
            running: HashMap::new(),
            sandboxes,
            data_dir,
            rootfs_dir,
            next_cid: max_cid + 1,
        })
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
            image: effective_image,
            vcpus,
            memory_mb,
            vsock_cid,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);

        Ok(())
    }

    /// Start a sandbox
    pub async fn start(&mut self, name: &str) -> Result<()> {
        self.start_with_permissions(name, &Permissions::default())
            .await
    }

    /// Start a sandbox with specific permissions
    pub async fn start_with_permissions(&mut self, name: &str, perms: &Permissions) -> Result<()> {
        let state = self
            .sandboxes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?
            .clone();

        if self.running.contains_key(name) {
            bail!("Sandbox '{}' is already running", name);
        }

        // Create sandbox using unified factory
        let mut sandbox = create_sandbox(self.backend, name)?;

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
        };

        sandbox.start(&config).await?;
        self.running.insert(name.to_string(), sandbox);

        Ok(())
    }

    /// Execute a command in a sandbox
    pub async fn exec_cmd(&mut self, name: &str, cmd: &[String]) -> Result<String> {
        let sandbox = self.running.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                name,
                name
            )
        })?;

        // Convert &[String] to &[&str]
        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();

        let result = sandbox.exec(&cmd_refs).await?;

        if result.exit_code != 0 {
            bail!(
                "Command exited with code {}: {}",
                result.exit_code,
                result.output()
            );
        }

        Ok(result.output())
    }

    /// Stop a sandbox
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        if let Some(mut sandbox) = self.running.remove(name) {
            sandbox.stop().await?;
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

        Ok(())
    }

    /// List all sandboxes (persisted, with running status)
    pub fn list(&self) -> Vec<(&str, bool)> {
        self.sandboxes
            .keys()
            .map(|name| {
                let running = self
                    .running
                    .get(name)
                    .map(|s| s.is_running())
                    .unwrap_or(false);
                (name.as_str(), running)
            })
            .collect()
    }

    /// Check if a sandbox exists
    pub fn exists(&self, name: &str) -> bool {
        self.sandboxes.contains_key(name)
    }

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
    pub async fn run_ephemeral(
        &mut self,
        image: &str,
        cmd: &[String],
        perms: &Permissions,
    ) -> Result<String> {
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
        };

        // Use optimized `docker/podman run --rm` for container backends
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

        // Generic path for non-container backends (Firecracker, Apple, Hyperlight)
        let name = format!("ephemeral-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let mut sandbox = create_sandbox(self.backend, &name)?;

        // Start, exec, stop
        sandbox.start(&config).await?;

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

        sandbox.write_file(path, content).await
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

        sandbox.read_file(path).await
    }
}
