//! Cloud Hypervisor backend implementing the Sandbox trait.
//!
//! This is a modern, Rust-based VMM optimized for cloud workloads.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tokio::time::{Duration, sleep};

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};
use crate::cloud_hypervisor_client::{
    CloudHypervisorClient, CpusConfig, DiskConfig, MemoryConfig, PayloadConfig, VmConfig, VsockConfig,
};
use crate::cow::{RootfsCow, RootfsCowStore};
use crate::vsock::VsockClient;

pub fn cloud_hypervisor_available() -> bool {
    find_cloud_hypervisor().is_ok()
}

fn find_cloud_hypervisor() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CLOUD_HYPERVISOR_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let agentkernel_bin = home.join(".local/share/agentkernel/bin/cloud-hypervisor");
        if agentkernel_bin.exists() {
            return Ok(agentkernel_bin);
        }
    }

    let locations = [
        "/usr/local/bin/cloud-hypervisor",
        "/usr/bin/cloud-hypervisor",
    ];

    for loc in locations {
        let path = PathBuf::from(loc);
        if path.exists() {
            return Ok(path);
        }
    }

    bail!("Cloud Hypervisor binary not found")
}

fn find_kernel() -> Result<PathBuf> {
    let data_dir = if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/agentkernel")
    } else {
        PathBuf::from("/usr/local/share/agentkernel")
    };
    
    let kernel_dir = data_dir.join("images/kernel");
    if kernel_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&kernel_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("vmlinux-") && name_str.ends_with("-agentkernel") {
                    return Ok(entry.path());
                }
            }
        }
    }
    bail!("Kernel not found")
}

fn find_rootfs(image: &str) -> Result<PathBuf> {
    let data_dir = if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/agentkernel")
    } else {
        PathBuf::from("/usr/local/share/agentkernel")
    };
    
    // Simplistic image mapping
    let rootfs_name = match image {
        "python:3.12" | "python" => "python.ext4",
        "node:20" | "node" => "node.ext4",
        "rust:1.85" | "rust" => "rust.ext4",
        "golang:1.24" | "go" => "go.ext4",
        _ => "base.ext4",
    };
    
    let rootfs = data_dir.join("images/rootfs").join(rootfs_name);
    if rootfs.exists() {
        Ok(rootfs)
    } else {
        bail!("Rootfs {} not found", rootfs.display())
    }
}

/// Cloud Hypervisor sandbox
pub struct CloudHypervisorSandbox {
    name: String,
    running: bool,
    process: Option<Child>,
    socket_path: PathBuf,
    vsock_path: PathBuf,
    client: Option<CloudHypervisorClient>,
    sandbox_rootfs: Option<RootfsCow>,
}

impl CloudHypervisorSandbox {
    /// Create a new Cloud Hypervisor sandbox
    pub fn new(name: &str) -> Self {
        let socket_path = PathBuf::from(format!("/tmp/ch-api-{}.sock", name));
        let vsock_path = PathBuf::from(format!("/tmp/ch-vsock-{}.sock", name));
        
        Self {
            name: name.to_string(),
            running: false,
            process: None,
            socket_path,
            vsock_path,
            client: None,
            sandbox_rootfs: None,
        }
    }

    async fn wait_for_socket(&self) -> Result<()> {
        for _ in 0..100 {
            if self.socket_path.exists() {
                return Ok(());
            }
            sleep(Duration::from_millis(50)).await;
        }
        bail!("Timeout waiting for Cloud Hypervisor API socket")
    }

    async fn wait_for_agent(&mut self) -> Result<()> {
        for _ in 0..100 {
            if self.vsock_path.exists() {
                // Try to connect to guest agent
                let client = VsockClient::for_firecracker(&self.vsock_path);
                match client.ping().await {
                    Ok(_) => {
                        return Ok(());
                    }
                    Err(_) => {}
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        bail!("Timeout waiting for Cloud Hypervisor guest agent")
    }
}

#[async_trait]
impl Sandbox for CloudHypervisorSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        if self.running {
            bail!("Sandbox already running");
        }

        let ch_bin = find_cloud_hypervisor()?;
        let kernel = find_kernel()?;
        let base_rootfs = find_rootfs(&config.image)?;

        // Prepare COW rootfs
        let store = RootfsCowStore::open_default()?;
        let rootfs = store.prepare(&base_rootfs)?;
        let rootfs_path = rootfs.path().to_path_buf();
        self.sandbox_rootfs = Some(rootfs);

        // Remove old sockets
        let _ = fs::remove_file(&self.socket_path);
        let _ = fs::remove_file(&self.vsock_path);

        // Spawn Cloud Hypervisor
        let log_file = std::fs::File::create("/tmp/ch_error.log").unwrap();
        let child = Command::new(&ch_bin)
            .arg("--api-socket")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(log_file.try_clone().unwrap())
            .stderr(log_file)
            .spawn()
            .context("Failed to spawn Cloud Hypervisor")?;
            
        self.process = Some(child);

        self.wait_for_socket().await?;
        
        let client = CloudHypervisorClient::new(&self.socket_path);
        
        let vm_config = VmConfig {
            payload: PayloadConfig {
                kernel: Some(kernel.to_string_lossy().to_string()),
                cmdline: Some("console=ttyS0 reboot=k panic=1 pci=off nomodules root=/dev/vda rw init=/usr/local/bin/agentkernel-guest".to_string()),
                initramfs: None,
            },
            cpus: CpusConfig {
                boot_vcpus: config.vcpus,
                max_vcpus: config.vcpus,
                kvm_hyperv: Some(true),
            },
            memory: MemoryConfig {
                size: 512 * 1024 * 1024,
                shared: None,
            },
            disks: Some(vec![DiskConfig {
                path: rootfs_path.to_string_lossy().to_string(),
                readonly: Some(false),
                direct: Some(false),
            }]),
            net: None,
            vsock: Some(VsockConfig {
                cid: 3,
                socket: self.vsock_path.to_string_lossy().to_string(),
            }),
        };

        client.create_vm(&vm_config).await?;
        client.boot_vm().await?;
        
        self.client = Some(client);
        self.wait_for_agent().await?;
        self.running = true;

        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        if !self.running {
            bail!("Sandbox not running");
        }
        let client = VsockClient::for_firecracker(&self.vsock_path);
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
        if let Some(client) = &self.client {
            let _ = client.shutdown_vm().await;
        }
        
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
        
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.vsock_path);
        
        self.running = false;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        BackendType::CloudHypervisor
    }

    fn is_running(&self) -> bool {
        self.running
    }

    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> anyhow::Result<()> {
        let client = VsockClient::for_firecracker(&self.vsock_path);
        client.write_file(path, content).await
    }

    async fn read_file_unchecked(&mut self, path: &str) -> anyhow::Result<Vec<u8>> {
        let client = VsockClient::for_firecracker(&self.vsock_path);
        client.read_file(path).await
    }

    async fn remove_file_unchecked(&mut self, path: &str) -> anyhow::Result<()> {
        let client = VsockClient::for_firecracker(&self.vsock_path);
        client.remove_file(path).await
    }

    async fn mkdir_unchecked(&mut self, path: &str, recursive: bool) -> anyhow::Result<()> {
        let client = VsockClient::for_firecracker(&self.vsock_path);
        client.mkdir(path, recursive).await
    }
}
