//! Firecracker VM pool for fast execution.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::sleep;

use crate::firecracker_client::{BootSource, Drive, FirecrackerClient, MachineConfig, VsockDevice};
use crate::permissions::CompatibilityMode;
use crate::vsock::VsockClient;

/// VM handle returned to clients (without process ownership)
#[derive(Debug, Clone)]
pub struct VmHandle {
    /// Unique ID
    pub id: String,
    /// vsock CID
    pub cid: u32,
    /// Path to vsock UDS
    pub vsock_path: PathBuf,
}

/// Per-agent pool configuration
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AgentPoolConfig {
    /// Minimum number of warm VMs for this agent type
    pub min_warm: usize,
    /// Maximum number of warm VMs for this agent type
    pub max_warm: usize,
    /// Preferred runtime (base, python, node, etc.)
    pub runtime: String,
    /// Memory allocation for this agent's VMs (MiB)
    pub mem_size_mib: u64,
    /// Number of vCPUs for this agent's VMs
    pub vcpu_count: u32,
}

#[allow(dead_code)]
impl AgentPoolConfig {
    /// Create config for Claude Code
    pub fn claude_code() -> Self {
        Self {
            min_warm: 2,
            max_warm: 4,
            runtime: "python".to_string(), // Claude often runs Python
            mem_size_mib: 512,
            vcpu_count: 1,
        }
    }

    /// Create config for Codex
    pub fn codex() -> Self {
        Self {
            min_warm: 1,
            max_warm: 3,
            runtime: "python".to_string(),
            mem_size_mib: 512,
            vcpu_count: 1,
        }
    }

    /// Create config for Gemini
    pub fn gemini() -> Self {
        Self {
            min_warm: 1,
            max_warm: 3,
            runtime: "python".to_string(),
            mem_size_mib: 512,
            vcpu_count: 1,
        }
    }

    /// Create config for native mode
    pub fn native() -> Self {
        Self {
            min_warm: 2,
            max_warm: 5,
            runtime: "base".to_string(),
            mem_size_mib: 512,
            vcpu_count: 1,
        }
    }
}

impl Default for AgentPoolConfig {
    fn default() -> Self {
        Self::native()
    }
}

/// Pool configuration
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum number of warm VMs to maintain (global)
    pub min_warm: usize,
    /// Maximum number of warm VMs to maintain (global)
    pub max_warm: usize,
    /// Maximum age of a VM before recycling (seconds)
    pub max_age_secs: u64,
    /// Health check interval (seconds)
    pub health_interval_secs: u64,
    /// Default runtime type
    pub default_runtime: String,
    /// Per-agent configuration (overrides globals when set)
    pub agent_configs: HashMap<CompatibilityMode, AgentPoolConfig>,
    /// Which agents to pre-warm on startup
    pub prewarm_agents: Vec<CompatibilityMode>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_warm: 3,
            max_warm: 5,
            max_age_secs: 300, // 5 minutes
            health_interval_secs: 30,
            default_runtime: "base".to_string(),
            agent_configs: HashMap::new(),
            prewarm_agents: vec![],
        }
    }
}

#[allow(dead_code)]
impl PoolConfig {
    /// Create a config with pre-warming for all agent types
    pub fn with_all_agents() -> Self {
        let mut agent_configs = HashMap::new();
        agent_configs.insert(CompatibilityMode::Native, AgentPoolConfig::native());
        agent_configs.insert(
            CompatibilityMode::ClaudeCode,
            AgentPoolConfig::claude_code(),
        );
        agent_configs.insert(CompatibilityMode::Codex, AgentPoolConfig::codex());
        agent_configs.insert(CompatibilityMode::Gemini, AgentPoolConfig::gemini());

        Self {
            min_warm: 3,
            max_warm: 10,
            max_age_secs: 300,
            health_interval_secs: 30,
            default_runtime: "base".to_string(),
            agent_configs,
            prewarm_agents: vec![CompatibilityMode::Native], // Only pre-warm native by default
        }
    }

    /// Get config for a specific agent type
    pub fn get_agent_config(&self, mode: CompatibilityMode) -> AgentPoolConfig {
        self.agent_configs.get(&mode).cloned().unwrap_or_default()
    }
}

/// A VM in the pool
#[derive(Debug)]
pub struct PooledVm {
    /// Unique ID
    pub id: String,
    /// vsock CID
    pub cid: u32,
    /// Path to vsock UDS
    pub vsock_path: PathBuf,
    /// Path to Firecracker API socket
    pub api_socket_path: PathBuf,
    /// Firecracker process
    process: Child,
    /// Runtime type (base, python, etc.)
    pub runtime: String,
    /// Compatibility mode this VM was created for
    pub compatibility_mode: CompatibilityMode,
    /// When the VM was created
    pub created_at: Instant,
    /// When the VM was last used
    pub last_used: Instant,
}

impl PooledVm {
    /// Check if this VM is still running
    pub fn is_alive(&self) -> bool {
        // Check if process is still running via ps
        Command::new("ps")
            .arg("-p")
            .arg(self.process.id().to_string())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Kill the VM process
    pub fn kill(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        // Clean up sockets
        let _ = std::fs::remove_file(&self.api_socket_path);
        let _ = std::fs::remove_file(&self.vsock_path);
    }
}

/// Firecracker VM pool
pub struct FirecrackerPool {
    /// Warm (ready) VMs
    warm_pool: Mutex<VecDeque<PooledVm>>,
    /// VMs currently in use
    in_use: Mutex<HashMap<String, PooledVm>>,
    /// Pool configuration
    config: PoolConfig,
    /// Semaphore to limit concurrent VM starts
    start_semaphore: Semaphore,
    /// Next CID to assign
    next_cid: AtomicU32,
    /// Kernel path
    kernel_path: PathBuf,
    /// Rootfs directory
    rootfs_dir: PathBuf,
    /// Shutdown flag
    shutdown: std::sync::atomic::AtomicBool,
}

impl FirecrackerPool {
    /// Create a new pool
    pub fn new(config: PoolConfig, kernel_path: PathBuf, rootfs_dir: PathBuf) -> Self {
        Self {
            warm_pool: Mutex::new(VecDeque::new()),
            in_use: Mutex::new(HashMap::new()),
            config,
            start_semaphore: Semaphore::new(2), // Max 2 concurrent starts
            next_cid: AtomicU32::new(100),      // Start at 100 to avoid conflicts
            kernel_path,
            rootfs_dir,
            shutdown: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Get pool statistics
    pub async fn stats(&self) -> (usize, usize) {
        // Lock separately to avoid holding both locks at once
        let warm = { self.warm_pool.lock().await.len() };
        let in_use = { self.in_use.lock().await.len() };
        (warm, in_use)
    }

    /// Get pool statistics broken down by agent compatibility mode
    pub async fn stats_by_agent(&self) -> HashMap<String, usize> {
        let pool = self.warm_pool.lock().await;
        let mut stats: HashMap<String, usize> = HashMap::new();

        for vm in pool.iter() {
            let mode_str = match vm.compatibility_mode {
                CompatibilityMode::Native => "native",
                CompatibilityMode::ClaudeCode => "claude",
                CompatibilityMode::Codex => "codex",
                CompatibilityMode::Gemini => "gemini",
            };
            *stats.entry(mode_str.to_string()).or_insert(0) += 1;
        }

        stats
    }

    /// Acquire a VM from the pool by runtime type
    #[allow(dead_code)]
    pub async fn acquire(&self, runtime: &str) -> Result<VmHandle> {
        self.acquire_with_mode(runtime, CompatibilityMode::Native)
            .await
    }

    /// Acquire a VM from the pool for a specific agent compatibility mode
    #[allow(dead_code)]
    pub async fn acquire_for_agent(&self, mode: CompatibilityMode) -> Result<VmHandle> {
        let agent_config = self.config.get_agent_config(mode);
        self.acquire_with_mode(&agent_config.runtime, mode).await
    }

    /// Acquire a VM with specific runtime and compatibility mode
    pub async fn acquire_with_mode(
        &self,
        runtime: &str,
        mode: CompatibilityMode,
    ) -> Result<VmHandle> {
        // Try to get a VM from the warm pool
        // IMPORTANT: Release warm_pool lock before acquiring in_use lock to prevent deadlock
        let vm_opt = {
            let mut pool = self.warm_pool.lock().await;

            // Find a VM with matching runtime AND compatibility mode (prefer exact match)
            // Fall back to matching runtime only if no exact match
            let exact_idx = pool.iter().position(|vm| {
                vm.runtime == runtime && vm.compatibility_mode == mode && vm.is_alive()
            });

            let idx = exact_idx.or_else(|| {
                pool.iter()
                    .position(|vm| vm.runtime == runtime && vm.is_alive())
            });

            if let Some(idx) = idx {
                let mut vm = pool.remove(idx).unwrap();
                vm.last_used = Instant::now();
                Some(vm)
            } else {
                None
            }
        }; // warm_pool lock released here

        if let Some(vm) = vm_opt {
            // Create handle before moving VM
            let handle = VmHandle {
                id: vm.id.clone(),
                cid: vm.cid,
                vsock_path: vm.vsock_path.clone(),
            };

            // Move to in_use (now safe - no nested locks)
            self.in_use.lock().await.insert(vm.id.clone(), vm);

            return Ok(handle);
        }

        // No warm VM available, start a new one
        let vm = self.start_vm_with_mode(runtime, mode).await?;

        // Create handle before moving VM
        let handle = VmHandle {
            id: vm.id.clone(),
            cid: vm.cid,
            vsock_path: vm.vsock_path.clone(),
        };

        // Track in in_use
        self.in_use.lock().await.insert(vm.id.clone(), vm);

        Ok(handle)
    }

    /// Release a VM back to the pool
    pub async fn release(&self, id: &str) -> Result<()> {
        // IMPORTANT: Release in_use lock before acquiring warm_pool lock to prevent deadlock
        let vm_opt = {
            let mut in_use = self.in_use.lock().await;
            in_use.remove(id)
        }; // in_use lock released here

        if let Some(mut vm) = vm_opt {
            // Check if VM is still healthy and not too old
            let age = vm.created_at.elapsed();
            let max_age = Duration::from_secs(self.config.max_age_secs);

            if vm.is_alive() && age < max_age {
                // Return to warm pool (now safe - no nested locks)
                vm.last_used = Instant::now();
                let mut pool = self.warm_pool.lock().await;

                // Don't exceed max_warm
                if pool.len() < self.config.max_warm {
                    pool.push_back(vm);
                } else {
                    // Pool is full, destroy the VM
                    vm.kill();
                }
            } else {
                // VM is dead or too old, destroy it
                vm.kill();
            }
        }

        Ok(())
    }

    /// Start a new VM with default compatibility mode
    async fn start_vm(&self, runtime: &str) -> Result<PooledVm> {
        self.start_vm_with_mode(runtime, CompatibilityMode::Native)
            .await
    }

    /// Start a new VM with specific compatibility mode
    async fn start_vm_with_mode(&self, runtime: &str, mode: CompatibilityMode) -> Result<PooledVm> {
        // Acquire semaphore to limit concurrent starts
        let _permit = self.start_semaphore.acquire().await?;

        // Get agent-specific config for memory/CPU
        let agent_config = self.config.get_agent_config(mode);

        let cid = self.next_cid.fetch_add(1, Ordering::SeqCst);
        let mode_str = match mode {
            CompatibilityMode::Native => "native",
            CompatibilityMode::ClaudeCode => "claude",
            CompatibilityMode::Codex => "codex",
            CompatibilityMode::Gemini => "gemini",
        };
        let id = format!("pool-{}-{}-{}", mode_str, runtime, cid);

        let api_socket_path = PathBuf::from(format!("/tmp/agentkernel-{}.sock", id));
        let vsock_path = PathBuf::from(format!("/tmp/agentkernel-{}-vsock.sock", id));

        // Clean up any existing sockets
        let _ = std::fs::remove_file(&api_socket_path);
        let _ = std::fs::remove_file(&vsock_path);

        // Find firecracker binary
        let firecracker_bin = Self::find_firecracker()?;

        // Start firecracker process
        let process = Command::new(&firecracker_bin)
            .arg("--api-sock")
            .arg(&api_socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("Failed to start firecracker: {}", firecracker_bin.display())
            })?;

        // Wait for socket
        for _ in 0..50 {
            if api_socket_path.exists() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        if !api_socket_path.exists() {
            bail!("Firecracker API socket not available after 5 seconds");
        }

        // Configure the VM
        let client = FirecrackerClient::new(&api_socket_path);

        // Boot source
        let boot_source = BootSource {
            kernel_image_path: self.kernel_path.to_string_lossy().to_string(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off init=/init quiet loglevel=4 i8042.nokbd i8042.noaux".to_string(),
        };
        client.set_boot_source(&boot_source).await?;

        // Root drive
        let rootfs_path = self.rootfs_dir.join(format!("{}.ext4", runtime));
        if !rootfs_path.exists() {
            bail!("Rootfs not found: {}", rootfs_path.display());
        }

        let drive = Drive {
            drive_id: "rootfs".to_string(),
            path_on_host: rootfs_path.to_string_lossy().to_string(),
            is_root_device: true,
            is_read_only: false,
        };
        client.set_drive("rootfs", &drive).await?;

        // Machine config (use agent-specific settings)
        let machine = MachineConfig {
            vcpu_count: agent_config.vcpu_count,
            mem_size_mib: agent_config.mem_size_mib,
        };
        client.set_machine_config(&machine).await?;

        // vsock device
        let vsock = VsockDevice {
            guest_cid: cid,
            uds_path: vsock_path.to_string_lossy().to_string(),
        };
        client.set_vsock(&vsock).await?;

        // Start instance
        client.start_instance().await?;

        // Wait for guest agent
        let vsock_client = VsockClient::for_firecracker(vsock_path.clone());
        for i in 0..100 {
            if vsock_client.ping().await.unwrap_or(false) {
                break;
            }
            if i == 99 {
                bail!("Guest agent not available after 10 seconds");
            }
            if i % 20 == 0 && i > 0 {
                eprintln!("Waiting for guest agent... ({}s)", i / 10);
            }
            sleep(Duration::from_millis(100)).await;
        }

        let now = Instant::now();

        Ok(PooledVm {
            id,
            cid,
            vsock_path,
            api_socket_path,
            process,
            runtime: runtime.to_string(),
            compatibility_mode: mode,
            created_at: now,
            last_used: now,
        })
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

    /// Pre-warm the pool to min_warm VMs (default behavior)
    pub async fn warm_up(&self) -> Result<()> {
        // If agent-specific pre-warming is configured, use that
        if !self.config.prewarm_agents.is_empty() {
            return self.warm_up_agents().await;
        }

        // Otherwise fall back to default runtime
        let runtime = &self.config.default_runtime;
        let current = self.warm_pool.lock().await.len();
        let needed = self.config.min_warm.saturating_sub(current);

        for _ in 0..needed {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            match self.start_vm(runtime).await {
                Ok(vm) => {
                    self.warm_pool.lock().await.push_back(vm);
                }
                Err(e) => {
                    eprintln!("Failed to warm up VM: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Pre-warm the pool with VMs for each configured agent type
    pub async fn warm_up_agents(&self) -> Result<()> {
        for mode in &self.config.prewarm_agents {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            let agent_config = self.config.get_agent_config(*mode);
            let mode_str = match mode {
                CompatibilityMode::Native => "native",
                CompatibilityMode::ClaudeCode => "claude",
                CompatibilityMode::Codex => "codex",
                CompatibilityMode::Gemini => "gemini",
            };

            // Count current VMs for this mode
            let current = {
                let pool = self.warm_pool.lock().await;
                pool.iter()
                    .filter(|vm| vm.compatibility_mode == *mode)
                    .count()
            };

            let needed = agent_config.min_warm.saturating_sub(current);
            if needed == 0 {
                continue;
            }

            eprintln!("Pre-warming {} VMs for {} mode...", needed, mode_str);

            for _ in 0..needed {
                if self.shutdown.load(Ordering::SeqCst) {
                    break;
                }

                match self.start_vm_with_mode(&agent_config.runtime, *mode).await {
                    Ok(vm) => {
                        self.warm_pool.lock().await.push_back(vm);
                    }
                    Err(e) => {
                        eprintln!("Failed to warm up {} VM: {}", mode_str, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Pre-warm VMs for a specific agent type
    pub async fn warm_up_for_agent(&self, mode: CompatibilityMode) -> Result<()> {
        let agent_config = self.config.get_agent_config(mode);
        let mode_str = match mode {
            CompatibilityMode::Native => "native",
            CompatibilityMode::ClaudeCode => "claude",
            CompatibilityMode::Codex => "codex",
            CompatibilityMode::Gemini => "gemini",
        };

        // Count current VMs for this mode
        let current = {
            let pool = self.warm_pool.lock().await;
            pool.iter()
                .filter(|vm| vm.compatibility_mode == mode)
                .count()
        };

        let needed = agent_config.min_warm.saturating_sub(current);
        if needed == 0 {
            return Ok(());
        }

        eprintln!("Pre-warming {} VMs for {} mode...", needed, mode_str);

        for _ in 0..needed {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            match self.start_vm_with_mode(&agent_config.runtime, mode).await {
                Ok(vm) => {
                    self.warm_pool.lock().await.push_back(vm);
                }
                Err(e) => {
                    eprintln!("Failed to warm up {} VM: {}", mode_str, e);
                }
            }
        }

        Ok(())
    }

    /// Run background health checks and replenishment
    pub async fn run_maintenance(&self) {
        let interval = Duration::from_secs(self.config.health_interval_secs);

        while !self.shutdown.load(Ordering::SeqCst) {
            sleep(interval).await;

            // Remove dead/stale VMs from warm pool
            {
                let mut pool = self.warm_pool.lock().await;
                let max_age = Duration::from_secs(self.config.max_age_secs);

                pool.retain(|vm| {
                    let alive = vm.is_alive();
                    let young = vm.created_at.elapsed() < max_age;
                    alive && young
                });
            }

            // Replenish if needed
            let _ = self.warm_up().await;
        }
    }

    /// Signal shutdown
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Destroy all VMs
    #[allow(dead_code)]
    pub async fn destroy_all(&self) {
        // Destroy warm pool
        {
            let mut pool = self.warm_pool.lock().await;
            for mut vm in pool.drain(..) {
                vm.kill();
            }
        }

        // Destroy in-use VMs
        {
            let mut in_use = self.in_use.lock().await;
            for (_, mut vm) in in_use.drain() {
                vm.kill();
            }
        }
    }
}

impl Drop for FirecrackerPool {
    fn drop(&mut self) {
        self.shutdown();
        // Note: async cleanup happens via destroy_all() before drop
    }
}
