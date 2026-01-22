//! Firecracker microVM backend implementing the Sandbox trait.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tokio::time::{Duration, sleep};

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};
use crate::firecracker_client::{BootSource, Drive, FirecrackerClient, MachineConfig, VsockDevice};
use crate::languages::docker_image_to_firecracker_runtime;
use crate::vsock::VsockClient;

/// Check if Firecracker is available
pub fn firecracker_available() -> bool {
    find_firecracker().is_ok()
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

    bail!("Firecracker binary not found")
}

/// Firecracker microVM sandbox
pub struct FirecrackerSandbox {
    name: String,
    socket_path: PathBuf,
    vsock_path: PathBuf,
    process: Option<Child>,
    vsock_cid: u32,
    kernel_path: Option<PathBuf>,
    rootfs_path: Option<PathBuf>,
    running: bool,
}

impl FirecrackerSandbox {
    /// Create a new Firecracker sandbox
    pub fn new(name: &str) -> Result<Self> {
        let socket_path = PathBuf::from(format!("/tmp/agentkernel-{}.sock", name));
        let vsock_path = PathBuf::from(format!("/tmp/agentkernel-{}-vsock.sock", name));

        // Clean up any existing sockets
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&vsock_path);

        // Generate a unique CID (use hash of name + timestamp)
        let vsock_cid = 100
            + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u32
                % 1000);

        Ok(Self {
            name: name.to_string(),
            socket_path,
            vsock_path,
            process: None,
            vsock_cid,
            kernel_path: None,
            rootfs_path: None,
            running: false,
        })
    }

    /// Set kernel path
    pub fn with_kernel(mut self, path: PathBuf) -> Self {
        self.kernel_path = Some(path);
        self
    }

    /// Set rootfs path
    pub fn with_rootfs(mut self, path: PathBuf) -> Self {
        self.rootfs_path = Some(path);
        self
    }

    /// Find kernel path
    fn find_kernel() -> Result<PathBuf> {
        if let Some(home) = std::env::var_os("HOME") {
            let kernel_dir = PathBuf::from(home).join(".local/share/agentkernel/kernel");
            if kernel_dir.exists() {
                // Find first vmlinux file
                for entry in std::fs::read_dir(&kernel_dir)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    if name.to_string_lossy().starts_with("vmlinux") {
                        return Ok(entry.path());
                    }
                }
            }
        }
        bail!("Kernel not found. Run 'agentkernel setup' to install.")
    }

    /// Find rootfs path for an image
    fn find_rootfs(image: &str) -> Result<PathBuf> {
        let runtime = docker_image_to_firecracker_runtime(image);
        if let Some(home) = std::env::var_os("HOME") {
            let rootfs_dir = PathBuf::from(home).join(".local/share/agentkernel/rootfs");
            let rootfs_path = rootfs_dir.join(format!("{}.ext4", runtime));
            if rootfs_path.exists() {
                return Ok(rootfs_path);
            }
        }
        bail!(
            "Rootfs for '{}' not found. Run 'agentkernel setup'.",
            runtime
        )
    }

    /// Wait for the API socket to be available
    async fn wait_for_socket(&self) -> Result<()> {
        for _ in 0..50 {
            if self.socket_path.exists() {
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
        bail!("Firecracker API socket not available after 5 seconds")
    }

    /// Configure the VM via the Firecracker API
    async fn configure(&self, config: &SandboxConfig) -> Result<()> {
        let client = FirecrackerClient::new(&self.socket_path);

        // Get kernel and rootfs paths
        let kernel_path = self
            .kernel_path
            .clone()
            .or_else(|| Self::find_kernel().ok())
            .ok_or_else(|| anyhow::anyhow!("Kernel path not set"))?;

        let rootfs_path = self
            .rootfs_path
            .clone()
            .or_else(|| Self::find_rootfs(&config.image).ok())
            .ok_or_else(|| anyhow::anyhow!("Rootfs path not set"))?;

        // Set boot source with optimized boot args
        let boot_source = BootSource {
            kernel_image_path: kernel_path.to_string_lossy().to_string(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off init=/init quiet loglevel=4 i8042.nokbd i8042.noaux".to_string(),
        };
        client.set_boot_source(&boot_source).await?;

        // Set root drive
        let drive = Drive {
            drive_id: "rootfs".to_string(),
            path_on_host: rootfs_path.to_string_lossy().to_string(),
            is_root_device: true,
            is_read_only: false,
        };
        client.set_drive("rootfs", &drive).await?;

        // Set machine config
        let machine = MachineConfig {
            vcpu_count: config.vcpus,
            mem_size_mib: config.memory_mb,
        };
        client.set_machine_config(&machine).await?;

        // Set vsock device
        let vsock = VsockDevice {
            guest_cid: self.vsock_cid,
            uds_path: self.vsock_path.to_string_lossy().to_string(),
        };
        client.set_vsock(&vsock).await?;

        Ok(())
    }

    /// Start the VM instance
    async fn start_instance(&self) -> Result<()> {
        let client = FirecrackerClient::new(&self.socket_path);
        client.start_instance().await
    }

    /// Wait for the guest agent to become available
    async fn wait_for_agent(&self) -> Result<()> {
        let client = VsockClient::for_firecracker(&self.vsock_path);

        for i in 0..100 {
            if client.ping().await.unwrap_or(false) {
                return Ok(());
            }
            if i % 20 == 0 && i > 0 {
                eprintln!("Waiting for guest agent... ({}s)", i / 10);
            }
            sleep(Duration::from_millis(100)).await;
        }

        bail!("Guest agent not available after 10 seconds")
    }
}

#[async_trait]
impl Sandbox for FirecrackerSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        let firecracker_bin = find_firecracker()?;

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

        // Wait for socket
        self.wait_for_socket().await?;

        // Configure the VM
        self.configure(config).await?;

        // Start the VM instance
        self.start_instance().await?;

        // Wait for guest agent
        self.wait_for_agent().await?;

        self.running = true;
        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        let client = VsockClient::for_firecracker(&self.vsock_path);

        // Convert &str to String
        let command: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();

        match client.run_command(&command).await {
            Ok(result) => Ok(ExecResult {
                exit_code: result.exit_code,
                stdout: result.stdout,
                stderr: result.stderr,
            }),
            Err(e) => Ok(ExecResult::failure(1, e.to_string())),
        }
    }

    async fn stop(&mut self) -> Result<()> {
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

        // Clean up sockets
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.vsock_path);

        self.running = false;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Firecracker
    }

    fn is_running(&self) -> bool {
        if !self.running {
            return false;
        }

        if let Some(ref process) = self.process {
            Command::new("ps")
                .arg("-p")
                .arg(process.id().to_string())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        } else {
            false
        }
    }
}

impl Drop for FirecrackerSandbox {
    fn drop(&mut self) {
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
        }
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.vsock_path);
    }
}
