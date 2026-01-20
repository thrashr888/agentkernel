//! Virtual Machine Manager
//!
//! This module provides the interface to sandboxes via Firecracker microVMs
//! or containers (Docker/Podman) as fallback when KVM is not available.

use crate::docker_backend::{ContainerRuntime, ContainerSandbox, detect_container_runtime};
use crate::firecracker_client::{BootSource, Drive, FirecrackerClient, MachineConfig, VsockDevice};
use crate::permissions::Permissions;
use crate::validation;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tokio::time::{Duration, sleep};

/// Backend type for sandbox execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Firecracker microVM (requires KVM)
    Firecracker,
    /// Container (Docker or Podman)
    Container(ContainerRuntime),
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

/// Firecracker VM configuration
#[derive(Debug, Clone)]
pub struct VmConfig {
    pub name: String,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub vsock_cid: u32,
}

/// Firecracker VM instance
pub struct FirecrackerVm {
    pub config: VmConfig,
    socket_path: PathBuf,
    process: Option<Child>,
}

// Firecracker API types are now in firecracker_client module

impl FirecrackerVm {
    /// Create a new Firecracker VM instance (does not start it)
    pub fn new(config: VmConfig) -> Result<Self> {
        // Create socket path in /tmp
        let socket_path = PathBuf::from(format!("/tmp/agentkernel-{}.sock", config.name));

        // Clean up any existing socket
        // Security: Use atomic remove to avoid TOCTOU race condition
        // where an attacker could create a symlink between exists() and remove_file()
        match std::fs::remove_file(&socket_path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        Ok(Self {
            config,
            socket_path,
            process: None,
        })
    }

    /// Start the Firecracker process
    pub async fn start(&mut self) -> Result<()> {
        // Find firecracker binary
        let firecracker_bin = Self::find_firecracker()?;

        // Start firecracker process
        let process = Command::new(&firecracker_bin)
            .arg("--api-sock")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("Failed to start firecracker: {}", firecracker_bin.display())
            })?;

        self.process = Some(process);

        // Wait for socket to be available
        self.wait_for_socket().await?;

        // Configure the VM
        self.configure().await?;

        // Start the VM
        self.start_instance().await?;

        Ok(())
    }

    /// Find the firecracker binary
    fn find_firecracker() -> Result<PathBuf> {
        // Check FIRECRACKER_BIN env var first
        if let Ok(path) = std::env::var("FIRECRACKER_BIN") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Ok(path);
            }
        }

        // Check agentkernel's own bin directory
        if let Some(home) = std::env::var_os("HOME") {
            let local_fc = PathBuf::from(home).join(".local/share/agentkernel/bin/firecracker");
            if local_fc.exists() {
                return Ok(local_fc);
            }
        }

        // Check common locations
        let locations = [
            "/usr/local/bin/firecracker",
            "/usr/bin/firecracker",
            "./firecracker",
        ];

        for loc in locations {
            let path = PathBuf::from(loc);
            if path.exists() {
                return Ok(path);
            }
        }

        // Try PATH
        if let Ok(output) = Command::new("which").arg("firecracker").output()
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }

        bail!(
            "Firecracker binary not found. Run 'agentkernel setup' or set FIRECRACKER_BIN.\n\
             Download from: https://github.com/firecracker-microvm/firecracker/releases"
        );
    }

    /// Wait for the API socket to be available
    async fn wait_for_socket(&self) -> Result<()> {
        for _ in 0..50 {
            if self.socket_path.exists() {
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
        bail!("Firecracker API socket not available after 5 seconds");
    }

    /// Configure the VM via the Firecracker API
    async fn configure(&self) -> Result<()> {
        let client = FirecrackerClient::new(&self.socket_path);

        // Set boot source
        let boot_source = BootSource {
            kernel_image_path: self.config.kernel_path.to_string_lossy().to_string(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off init=/init".to_string(),
        };
        client.set_boot_source(&boot_source).await?;

        // Set root drive
        let drive = Drive {
            drive_id: "rootfs".to_string(),
            path_on_host: self.config.rootfs_path.to_string_lossy().to_string(),
            is_root_device: true,
            is_read_only: false,
        };
        client.set_drive("rootfs", &drive).await?;

        // Set machine config
        let machine = MachineConfig {
            vcpu_count: self.config.vcpus,
            mem_size_mib: self.config.memory_mb,
        };
        client.set_machine_config(&machine).await?;

        // Set vsock device
        let vsock_uds = format!("/tmp/agentkernel-{}-vsock.sock", self.config.name);
        let vsock = VsockDevice {
            guest_cid: self.config.vsock_cid,
            uds_path: vsock_uds,
        };
        client.set_vsock(&vsock).await?;

        Ok(())
    }

    /// Start the VM instance
    async fn start_instance(&self) -> Result<()> {
        let client = FirecrackerClient::new(&self.socket_path);
        client.start_instance().await
    }

    /// Stop the VM
    pub async fn stop(&mut self) -> Result<()> {
        // Send shutdown signal via API
        let client = FirecrackerClient::new(&self.socket_path);
        let _ = client.send_ctrl_alt_del().await;

        // Give it a moment to shutdown gracefully
        sleep(Duration::from_millis(500)).await;

        // Kill the process if still running
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
            let _ = process.wait();
        }

        // Clean up socket (ignore errors - best effort cleanup)
        let _ = std::fs::remove_file(&self.socket_path);

        Ok(())
    }

    /// Get the vsock path for this VM
    pub fn vsock_path(&self) -> PathBuf {
        PathBuf::from(format!("/tmp/agentkernel-{}-vsock.sock", self.config.name))
    }

    /// Check if the VM is running
    pub fn is_running(&self) -> bool {
        if let Some(ref process) = self.process {
            // Try to get process status without blocking
            match Command::new("ps")
                .arg("-p")
                .arg(process.id().to_string())
                .output()
            {
                Ok(output) => output.status.success(),
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

impl Drop for FirecrackerVm {
    fn drop(&mut self) {
        // Clean up on drop
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
        }
        // Best effort socket cleanup (ignore errors)
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// VM Manager - manages sandboxes via Firecracker or containers (Docker/Podman)
pub struct VmManager {
    backend: Backend,
    vms: std::collections::HashMap<String, FirecrackerVm>,
    container_sandboxes: std::collections::HashMap<String, ContainerSandbox>,
    sandboxes: std::collections::HashMap<String, SandboxState>,
    data_dir: PathBuf,
    kernel_path: Option<PathBuf>,
    rootfs_dir: Option<PathBuf>,
    next_cid: u32,
}

impl VmManager {
    /// Create a new VM manager (auto-selects backend based on KVM availability)
    pub fn new() -> Result<Self> {
        let data_dir = Self::data_dir();
        let sandboxes_dir = data_dir.join("sandboxes");
        std::fs::create_dir_all(&sandboxes_dir)?;

        // Determine backend based on KVM availability
        let backend = if Self::check_kvm() {
            Backend::Firecracker
        } else if let Some(runtime) = detect_container_runtime() {
            Backend::Container(runtime)
        } else {
            bail!("Neither KVM nor a container runtime (Docker/Podman) is available.");
        };

        // Find kernel and rootfs paths (only needed for Firecracker)
        let (kernel_path, rootfs_dir) = if backend == Backend::Firecracker {
            let base_dir = Self::find_images_dir()?;
            let kernel = Self::find_kernel(&base_dir)?;
            let rootfs = base_dir.join("rootfs");
            (Some(kernel), Some(rootfs))
        } else {
            (None, None)
        };

        // Load existing sandboxes
        let sandboxes = Self::load_sandboxes(&sandboxes_dir)?;

        // Find next available CID
        let max_cid = sandboxes.values().map(|s| s.vsock_cid).max().unwrap_or(2);

        eprintln!(
            "Using {} backend",
            match backend {
                Backend::Firecracker => "Firecracker".to_string(),
                Backend::Container(ContainerRuntime::Docker) => "Docker".to_string(),
                Backend::Container(ContainerRuntime::Podman) => "Podman".to_string(),
            }
        );

        Ok(Self {
            backend,
            vms: std::collections::HashMap::new(),
            container_sandboxes: std::collections::HashMap::new(),
            sandboxes,
            data_dir,
            kernel_path,
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
    fn load_sandboxes(
        sandboxes_dir: &Path,
    ) -> Result<std::collections::HashMap<String, SandboxState>> {
        let mut sandboxes = std::collections::HashMap::new();

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
        // Check installed location first (preferred)
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = PathBuf::from(home).join(".local/share/agentkernel/images");
            // Check if it has actual content (kernel or rootfs)
            if home_path.join("kernel").exists() || home_path.join("rootfs").exists() {
                return Ok(home_path);
            }
        }

        // Check relative to current dir (development mode)
        let paths = [PathBuf::from("images"), PathBuf::from("../images")];

        for path in &paths {
            if path.join("kernel").exists() || path.join("rootfs").exists() {
                return Ok(path.clone());
            }
        }

        bail!(
            "Images directory not found. Run 'agentkernel setup' first, or check ~/.local/share/agentkernel/images"
        );
    }

    /// Find the kernel image
    fn find_kernel(base_dir: &Path) -> Result<PathBuf> {
        let kernel_dir = base_dir.join("kernel");

        // Look for vmlinux-*-agentkernel
        if kernel_dir.exists() {
            for entry in std::fs::read_dir(&kernel_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("vmlinux-") && name_str.ends_with("-agentkernel") {
                    return Ok(entry.path());
                }
            }
        }

        bail!(
            "Kernel not found in {}. Run 'agentkernel setup' first.",
            kernel_dir.display()
        );
    }

    /// Get rootfs path for a runtime (Firecracker only)
    ///
    /// # Security
    /// The runtime parameter is validated against an allowlist to prevent
    /// path traversal attacks (e.g., `../../../etc/passwd`).
    pub fn rootfs_path(&self, runtime: &str) -> Result<PathBuf> {
        // Security: Validate runtime against allowlist to prevent path traversal
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

        // Validate rootfs exists (only for Firecracker)
        if self.backend == Backend::Firecracker {
            self.rootfs_path(image)?;
        }

        let vsock_cid = self.next_cid;
        self.next_cid += 1;

        let state = SandboxState {
            name: name.to_string(),
            image: image.to_string(),
            vcpus,
            memory_mb,
            vsock_cid,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.save_sandbox(&state)?;
        self.sandboxes.insert(name.to_string(), state);

        Ok(())
    }

    /// Check if KVM is available
    fn check_kvm() -> bool {
        PathBuf::from("/dev/kvm").exists()
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

        match self.backend {
            Backend::Firecracker => {
                if self.vms.contains_key(name) {
                    bail!("Sandbox '{}' is already running", name);
                }

                let kernel_path = self
                    .kernel_path
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Kernel path not configured"))?;
                let rootfs_path = self.rootfs_path(&state.image)?;

                let config = VmConfig {
                    name: name.to_string(),
                    kernel_path,
                    rootfs_path,
                    vcpus: state.vcpus,
                    memory_mb: state.memory_mb,
                    vsock_cid: state.vsock_cid,
                };

                let mut vm = FirecrackerVm::new(config)?;
                vm.start().await?;
                self.vms.insert(name.to_string(), vm);
            }
            Backend::Container(runtime) => {
                if self.container_sandboxes.contains_key(name) {
                    bail!("Sandbox '{}' is already running", name);
                }

                let mut sandbox = ContainerSandbox::with_runtime(name, runtime);
                sandbox.start_with_permissions(&state.image, perms).await?;
                self.container_sandboxes.insert(name.to_string(), sandbox);
            }
        }

        Ok(())
    }

    /// Execute a command in a sandbox
    pub async fn exec_cmd(&mut self, name: &str, cmd: &[String]) -> Result<String> {
        match self.backend {
            Backend::Firecracker => {
                bail!("Exec not yet implemented for Firecracker backend (requires vsock)");
            }
            Backend::Container(runtime) => {
                // Check if container is running
                if !Self::is_container_running(name, runtime) {
                    bail!(
                        "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                        name,
                        name
                    );
                }

                // Execute directly via container exec
                let container_name = format!("agentkernel-{}", name);
                let mut args = vec!["exec", &container_name];
                let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
                args.extend(cmd_refs);

                let output = std::process::Command::new(runtime.cmd())
                    .args(&args)
                    .output()
                    .context("Failed to execute command in container")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("Command failed: {}", stderr);
                }

                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
        }
    }

    /// Stop a sandbox
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        match self.backend {
            Backend::Firecracker => {
                if let Some(mut vm) = self.vms.remove(name) {
                    vm.stop().await?;
                }
            }
            Backend::Container(runtime) => {
                // Kill container immediately (skip graceful shutdown for speed)
                let container_name = format!("agentkernel-{}", name);
                let _ = std::process::Command::new(runtime.cmd())
                    .args(["kill", &container_name])
                    .output();
                // Also remove from in-memory map if present
                self.container_sandboxes.remove(name);
            }
        }
        Ok(())
    }

    /// Remove a sandbox
    pub async fn remove(&mut self, name: &str) -> Result<()> {
        match self.backend {
            Backend::Firecracker => {
                if let Some(mut vm) = self.vms.remove(name) {
                    let _ = vm.stop().await;
                }
            }
            Backend::Container(runtime) => {
                // Remove container directly
                let container_name = format!("agentkernel-{}", name);
                let _ = std::process::Command::new(runtime.cmd())
                    .args(["rm", "-f", &container_name])
                    .output();
                // Also remove from in-memory map if present
                self.container_sandboxes.remove(name);
            }
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
                let running = match self.backend {
                    Backend::Firecracker => self.vms.get(name).is_some_and(|vm| vm.is_running()),
                    Backend::Container(runtime) => {
                        // Check container status directly (containers persist across CLI calls)
                        Self::is_container_running(name, runtime)
                    }
                };
                (name.as_str(), running)
            })
            .collect()
    }

    /// Check if a container is running (for a given sandbox name)
    fn is_container_running(name: &str, runtime: ContainerRuntime) -> bool {
        let container_name = format!("agentkernel-{}", name);
        std::process::Command::new(runtime.cmd())
            .args(["ps", "-q", "-f", &format!("name={}", container_name)])
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false)
    }

    /// Get a VM by name (only if running, Firecracker only)
    pub fn get(&self, name: &str) -> Option<&FirecrackerVm> {
        self.vms.get(name)
    }

    /// Check if a sandbox exists
    pub fn exists(&self, name: &str) -> bool {
        self.sandboxes.contains_key(name)
    }

    /// Get the current backend
    #[allow(dead_code)]
    pub fn backend(&self) -> Backend {
        self.backend
    }
}
