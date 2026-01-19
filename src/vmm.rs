//! Firecracker Virtual Machine Manager
//!
//! This module provides the interface to Firecracker microVMs.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tokio::time::{Duration, sleep};

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

// Firecracker API request/response types
#[derive(Debug, Serialize)]
struct BootSource {
    kernel_image_path: String,
    boot_args: String,
}

#[derive(Debug, Serialize)]
struct Drive {
    drive_id: String,
    path_on_host: String,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Debug, Serialize)]
struct MachineConfig {
    vcpu_count: u32,
    mem_size_mib: u64,
}

#[derive(Debug, Serialize)]
struct VsockDevice {
    guest_cid: u32,
    uds_path: String,
}

#[derive(Debug, Serialize)]
struct InstanceAction {
    action_type: String,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    fault_message: Option<String>,
}

impl FirecrackerVm {
    /// Create a new Firecracker VM instance (does not start it)
    pub fn new(config: VmConfig) -> Result<Self> {
        // Create socket path in /tmp
        let socket_path = PathBuf::from(format!("/tmp/agentkernel-{}.sock", config.name));

        // Clean up any existing socket
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
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
            "Firecracker binary not found. Install it or set FIRECRACKER_BIN environment variable.\n\
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
        // Set boot source
        let boot_source = BootSource {
            kernel_image_path: self.config.kernel_path.to_string_lossy().to_string(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off init=/init".to_string(),
        };
        self.api_put("/boot-source", &boot_source).await?;

        // Set root drive
        let drive = Drive {
            drive_id: "rootfs".to_string(),
            path_on_host: self.config.rootfs_path.to_string_lossy().to_string(),
            is_root_device: true,
            is_read_only: false,
        };
        self.api_put("/drives/rootfs", &drive).await?;

        // Set machine config
        let machine = MachineConfig {
            vcpu_count: self.config.vcpus,
            mem_size_mib: self.config.memory_mb,
        };
        self.api_put("/machine-config", &machine).await?;

        // Set vsock device
        let vsock_uds = format!("/tmp/agentkernel-{}-vsock.sock", self.config.name);
        let vsock = VsockDevice {
            guest_cid: self.config.vsock_cid,
            uds_path: vsock_uds,
        };
        self.api_put("/vsock", &vsock).await?;

        Ok(())
    }

    /// Start the VM instance
    async fn start_instance(&self) -> Result<()> {
        let action = InstanceAction {
            action_type: "InstanceStart".to_string(),
        };
        self.api_put("/actions", &action).await?;
        Ok(())
    }

    /// Stop the VM
    pub async fn stop(&mut self) -> Result<()> {
        // Send shutdown signal via API
        let action = InstanceAction {
            action_type: "SendCtrlAltDel".to_string(),
        };
        let _ = self.api_put("/actions", &action).await;

        // Give it a moment to shutdown gracefully
        sleep(Duration::from_millis(500)).await;

        // Kill the process if still running
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
            let _ = process.wait();
        }

        // Clean up socket
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }

        Ok(())
    }

    /// Make a PUT request to the Firecracker API
    async fn api_put<T: Serialize>(&self, path: &str, body: &T) -> Result<()> {
        let body_json = serde_json::to_string(body)?;

        // Use curl for simplicity (works with Unix sockets)
        let output = Command::new("curl")
            .arg("--unix-socket")
            .arg(&self.socket_path)
            .arg("-X")
            .arg("PUT")
            .arg("-H")
            .arg("Content-Type: application/json")
            .arg("-d")
            .arg(&body_json)
            .arg(format!("http://localhost{}", path))
            .output()
            .context("Failed to call Firecracker API")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!("Firecracker API error: {} {}", stderr, stdout);
        }

        // Check for API error in response
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.is_empty()
            && let Ok(error) = serde_json::from_str::<ApiError>(&stdout)
            && let Some(msg) = error.fault_message
        {
            bail!("Firecracker API error: {}", msg);
        }

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
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

/// VM Manager - manages multiple Firecracker VMs
pub struct VmManager {
    vms: std::collections::HashMap<String, FirecrackerVm>,
    kernel_path: PathBuf,
    rootfs_dir: PathBuf,
    next_cid: u32,
}

impl VmManager {
    /// Create a new VM manager
    pub fn new() -> Result<Self> {
        // Find kernel and rootfs paths
        let base_dir = Self::find_images_dir()?;
        let kernel_path = Self::find_kernel(&base_dir)?;
        let rootfs_dir = base_dir.join("rootfs");

        Ok(Self {
            vms: std::collections::HashMap::new(),
            kernel_path,
            rootfs_dir,
            next_cid: 3, // CID 0-2 are reserved
        })
    }

    /// Find the images directory
    fn find_images_dir() -> Result<PathBuf> {
        // Check relative to current dir
        let paths = [PathBuf::from("images"), PathBuf::from("../images")];

        for path in &paths {
            if path.exists() {
                return Ok(path.clone());
            }
        }

        // Check home directory
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = PathBuf::from(home).join(".local/share/agentkernel/images");
            if home_path.exists() {
                return Ok(home_path);
            }
        }

        bail!(
            "Images directory not found. Expected at ./images or ~/.local/share/agentkernel/images"
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
            "Kernel not found in {}. Build it with:\n  \
             cd images/build && docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder . && \
             docker run --rm -v $(pwd)/../kernel:/kernel agentkernel-kernel-builder",
            kernel_dir.display()
        );
    }

    /// Get rootfs path for a runtime
    pub fn rootfs_path(&self, runtime: &str) -> Result<PathBuf> {
        let path = self.rootfs_dir.join(format!("{}.ext4", runtime));
        if !path.exists() {
            bail!(
                "Rootfs not found: {}. Build it with:\n  cd images/build && ./build-rootfs.sh {}",
                path.display(),
                runtime
            );
        }
        Ok(path)
    }

    /// Create a new VM
    pub async fn create(
        &mut self,
        name: &str,
        runtime: &str,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<()> {
        if self.vms.contains_key(name) {
            bail!("VM '{}' already exists", name);
        }

        let rootfs_path = self.rootfs_path(runtime)?;
        let vsock_cid = self.next_cid;
        self.next_cid += 1;

        let config = VmConfig {
            name: name.to_string(),
            kernel_path: self.kernel_path.clone(),
            rootfs_path,
            vcpus,
            memory_mb,
            vsock_cid,
        };

        let vm = FirecrackerVm::new(config)?;
        self.vms.insert(name.to_string(), vm);

        Ok(())
    }

    /// Start a VM
    pub async fn start(&mut self, name: &str) -> Result<()> {
        let vm = self
            .vms
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("VM '{}' not found", name))?;
        vm.start().await
    }

    /// Stop a VM
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        let vm = self
            .vms
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("VM '{}' not found", name))?;
        vm.stop().await
    }

    /// Remove a VM
    pub async fn remove(&mut self, name: &str) -> Result<()> {
        if let Some(mut vm) = self.vms.remove(name) {
            vm.stop().await?;
        }
        Ok(())
    }

    /// List all VMs
    pub fn list(&self) -> Vec<(&str, bool)> {
        self.vms
            .iter()
            .map(|(name, vm)| (name.as_str(), vm.is_running()))
            .collect()
    }

    /// Get a VM by name
    pub fn get(&self, name: &str) -> Option<&FirecrackerVm> {
        self.vms.get(name)
    }
}
