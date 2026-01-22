//! Unified backend abstraction for sandbox execution.
//!
//! This module provides a common interface for all sandbox backends:
//! - Docker/Podman containers
//! - Firecracker microVMs
//! - Apple Containers (macOS 26+)
//! - Hyperlight WebAssembly (Linux with KVM)

// Allow dead code temporarily - this module provides the new unified interface
// that will be integrated into vmm.rs and main.rs incrementally
#![allow(dead_code)]

#[cfg(target_os = "macos")]
pub mod apple;
pub mod docker;
pub mod firecracker;
pub mod hyperlight;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(target_os = "macos")]
pub use apple::AppleSandbox;
pub use docker::{ContainerRuntime, DockerSandbox};
pub use firecracker::FirecrackerSandbox;
pub use hyperlight::HyperlightSandbox;

/// Backend type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendType {
    /// Docker or Podman container
    Docker,
    /// Podman container (explicit)
    Podman,
    /// Firecracker microVM
    Firecracker,
    /// Apple Containers (macOS 26+)
    Apple,
    /// Hyperlight WebAssembly
    Hyperlight,
}

impl fmt::Display for BackendType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendType::Docker => write!(f, "docker"),
            BackendType::Podman => write!(f, "podman"),
            BackendType::Firecracker => write!(f, "firecracker"),
            BackendType::Apple => write!(f, "apple"),
            BackendType::Hyperlight => write!(f, "hyperlight"),
        }
    }
}

impl std::str::FromStr for BackendType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "docker" => Ok(BackendType::Docker),
            "podman" => Ok(BackendType::Podman),
            "firecracker" => Ok(BackendType::Firecracker),
            "apple" => Ok(BackendType::Apple),
            "hyperlight" => Ok(BackendType::Hyperlight),
            _ => Err(format!(
                "Unknown backend '{}'. Valid options: docker, podman, firecracker, apple, hyperlight",
                s
            )),
        }
    }
}

/// Configuration for starting a sandbox
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Container/VM image to use (e.g., "python:3.12-alpine")
    pub image: String,
    /// Number of vCPUs (for VM backends)
    pub vcpus: u32,
    /// Memory in MB (for VM backends)
    pub memory_mb: u64,
    /// Whether to mount the current working directory
    pub mount_cwd: bool,
    /// Path to mount as working directory
    pub work_dir: Option<String>,
    /// Environment variables to set
    pub env: Vec<(String, String)>,
    /// Network access enabled
    pub network: bool,
    /// Make root filesystem read-only
    pub read_only: bool,
    /// Mount home directory (read-only)
    pub mount_home: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: "alpine:3.20".to_string(),
            vcpus: 1,
            memory_mb: 512,
            mount_cwd: false,
            work_dir: None,
            env: Vec::new(),
            network: true,
            read_only: false,
            mount_home: false,
        }
    }
}

impl SandboxConfig {
    /// Create a new config with the given image
    pub fn with_image(image: &str) -> Self {
        Self {
            image: image.to_string(),
            ..Default::default()
        }
    }

    /// Set resource limits
    pub fn with_resources(mut self, vcpus: u32, memory_mb: u64) -> Self {
        self.vcpus = vcpus;
        self.memory_mb = memory_mb;
        self
    }

    /// Enable/disable network
    pub fn with_network(mut self, network: bool) -> Self {
        self.network = network;
        self
    }

    /// Mount current working directory
    pub fn with_mount_cwd(mut self, mount: bool, work_dir: Option<String>) -> Self {
        self.mount_cwd = mount;
        self.work_dir = work_dir;
        self
    }

    /// Set environment variables
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.env = env;
        self
    }
}

/// Result of executing a command in a sandbox
#[derive(Debug, Clone)]
pub struct ExecResult {
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

impl ExecResult {
    /// Create a successful result
    pub fn success(stdout: String) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    /// Create a failed result
    pub fn failure(exit_code: i32, stderr: String) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr,
        }
    }

    /// Check if the command succeeded
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }

    /// Get combined output (stdout + stderr)
    pub fn output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }
}

/// Unified sandbox interface for all backends
///
/// Each backend implements this trait to provide a consistent API for:
/// - Starting sandboxes with configuration
/// - Executing commands
/// - Stopping and cleaning up
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Start the sandbox with the given configuration
    async fn start(&mut self, config: &SandboxConfig) -> Result<()>;

    /// Execute a command in the sandbox
    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult>;

    /// Stop the sandbox and clean up resources
    async fn stop(&mut self) -> Result<()>;

    /// Get the sandbox name/identifier
    fn name(&self) -> &str;

    /// Get the backend type
    fn backend_type(&self) -> BackendType;

    /// Check if the sandbox is running
    fn is_running(&self) -> bool;
}

/// Detect the best available backend for the current platform
pub fn detect_best_backend() -> Option<BackendType> {
    // On Linux, prefer Firecracker if KVM is available
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new("/dev/kvm").exists() {
            // Check if firecracker is available
            if firecracker::firecracker_available() {
                return Some(BackendType::Firecracker);
            }
        }
    }

    // On macOS 26+, check for Apple Containers
    #[cfg(target_os = "macos")]
    {
        if apple::apple_containers_available() {
            return Some(BackendType::Apple);
        }
    }

    // Fall back to containers (prefer Podman over Docker)
    if docker::podman_available() {
        return Some(BackendType::Podman);
    }
    if docker::docker_available() {
        return Some(BackendType::Docker);
    }

    None
}

/// Check if a specific backend is available
pub fn backend_available(backend: BackendType) -> bool {
    match backend {
        BackendType::Docker => docker::docker_available(),
        BackendType::Podman => docker::podman_available(),
        BackendType::Firecracker => firecracker::firecracker_available(),
        #[cfg(target_os = "macos")]
        BackendType::Apple => apple::apple_containers_available(),
        #[cfg(not(target_os = "macos"))]
        BackendType::Apple => false,
        BackendType::Hyperlight => hyperlight::hyperlight_available(),
    }
}

/// Create a sandbox for the specified backend
pub fn create_sandbox(backend: BackendType, name: &str) -> Result<Box<dyn Sandbox>> {
    match backend {
        BackendType::Docker => Ok(Box::new(DockerSandbox::new(name, ContainerRuntime::Docker))),
        BackendType::Podman => Ok(Box::new(DockerSandbox::new(name, ContainerRuntime::Podman))),
        BackendType::Firecracker => Ok(Box::new(FirecrackerSandbox::new(name)?)),
        #[cfg(target_os = "macos")]
        BackendType::Apple => Ok(Box::new(AppleSandbox::new(name))),
        #[cfg(not(target_os = "macos"))]
        BackendType::Apple => anyhow::bail!("Apple Containers only available on macOS"),
        BackendType::Hyperlight => Ok(Box::new(HyperlightSandbox::new(name))),
    }
}
