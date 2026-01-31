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
#[cfg(feature = "kubernetes")]
pub mod kubernetes;
#[cfg(feature = "kubernetes")]
pub mod kubernetes_operator;
#[cfg(feature = "kubernetes")]
pub mod kubernetes_pool;
#[cfg(feature = "nomad")]
pub mod nomad;
#[cfg(feature = "nomad")]
pub mod nomad_pool;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(target_os = "macos")]
pub use apple::AppleSandbox;
pub use docker::{ContainerRuntime, DockerSandbox};
pub use firecracker::FirecrackerSandbox;
pub use hyperlight::HyperlightSandbox;
#[cfg(feature = "kubernetes")]
pub use kubernetes::KubernetesSandbox;
#[cfg(feature = "nomad")]
pub use nomad::NomadSandbox;

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
    /// Kubernetes pods (requires --features kubernetes)
    Kubernetes,
    /// HashiCorp Nomad jobs (requires --features nomad)
    Nomad,
}

impl fmt::Display for BackendType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendType::Docker => write!(f, "docker"),
            BackendType::Podman => write!(f, "podman"),
            BackendType::Firecracker => write!(f, "firecracker"),
            BackendType::Apple => write!(f, "apple"),
            BackendType::Hyperlight => write!(f, "hyperlight"),
            BackendType::Kubernetes => write!(f, "kubernetes"),
            BackendType::Nomad => write!(f, "nomad"),
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
            "kubernetes" | "k8s" => Ok(BackendType::Kubernetes),
            "nomad" => Ok(BackendType::Nomad),
            _ => Err(format!(
                "Unknown backend '{}'. Valid options: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad",
                s
            )),
        }
    }
}

/// File to inject into sandbox at startup
#[derive(Debug, Clone)]
pub struct FileInjection {
    /// Content to write
    pub content: Vec<u8>,
    /// Destination path inside the sandbox (absolute)
    pub dest: String,
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
    /// Files to inject after sandbox starts
    pub files: Vec<FileInjection>,
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
            files: Vec::new(),
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

    /// Add files to inject after sandbox starts
    pub fn with_files(mut self, files: Vec<FileInjection>) -> Self {
        self.files = files;
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
/// - File operations (read/write)
/// - Stopping and cleaning up
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Start the sandbox with the given configuration
    async fn start(&mut self, config: &SandboxConfig) -> Result<()>;

    /// Execute a command in the sandbox
    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult>;

    /// Execute a command in the sandbox with environment variables
    ///
    /// # Arguments
    /// * `cmd` - Command and arguments to execute
    /// * `env` - Environment variables as KEY=VALUE pairs
    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        // Default implementation ignores env vars (for backends that don't support it)
        if !env.is_empty() {
            eprintln!(
                "Warning: This backend doesn't support environment variables, ignoring {} var(s)",
                env.len()
            );
        }
        self.exec(cmd).await
    }

    /// Stop the sandbox and clean up resources
    async fn stop(&mut self) -> Result<()>;

    /// Get the sandbox name/identifier
    fn name(&self) -> &str;

    /// Get the backend type
    fn backend_type(&self) -> BackendType;

    /// Check if the sandbox is running
    fn is_running(&self) -> bool;

    // --- File Operations ---

    /// Write a file to the sandbox filesystem
    ///
    /// # Arguments
    /// * `path` - Absolute path inside the sandbox (must start with '/')
    /// * `content` - File content as bytes
    ///
    /// # Security
    /// Path is validated to prevent traversal attacks and writes to system paths
    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()> {
        validate_sandbox_path(path)?;
        self.write_file_unchecked(path, content).await
    }

    /// Internal write implementation (no validation, called by write_file)
    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> Result<()>;

    /// Read a file from the sandbox filesystem
    ///
    /// # Arguments
    /// * `path` - Absolute path inside the sandbox (must start with '/')
    ///
    /// # Returns
    /// File content as bytes
    async fn read_file(&mut self, path: &str) -> Result<Vec<u8>> {
        validate_sandbox_path(path)?;
        self.read_file_unchecked(path).await
    }

    /// Internal read implementation (no validation, called by read_file)
    async fn read_file_unchecked(&mut self, path: &str) -> Result<Vec<u8>>;

    /// Remove a file from the sandbox filesystem
    async fn remove_file(&mut self, path: &str) -> Result<()> {
        validate_sandbox_path(path)?;
        self.remove_file_unchecked(path).await
    }

    /// Internal remove implementation
    async fn remove_file_unchecked(&mut self, path: &str) -> Result<()>;

    /// Create a directory in the sandbox filesystem
    async fn mkdir(&mut self, path: &str, recursive: bool) -> Result<()> {
        validate_sandbox_path(path)?;
        self.mkdir_unchecked(path, recursive).await
    }

    /// Internal mkdir implementation
    async fn mkdir_unchecked(&mut self, path: &str, recursive: bool) -> Result<()>;

    /// Inject files from config into the sandbox
    ///
    /// Called automatically after start() when files are specified in config.
    /// Creates parent directories as needed.
    async fn inject_files(&mut self, files: &[FileInjection]) -> Result<()> {
        for file in files {
            // Create parent directory if needed
            if let Some(parent) = std::path::Path::new(&file.dest).parent() {
                let parent_str = parent.to_string_lossy();
                if parent_str != "/" {
                    self.mkdir(&parent_str, true).await?;
                }
            }
            // Write the file
            self.write_file(&file.dest, &file.content).await?;
        }
        Ok(())
    }

    // --- Interactive Shell/PTY Operations ---

    /// Attach an interactive shell to the sandbox
    ///
    /// This opens a PTY session in the guest and bridges it to the host terminal.
    /// The shell runs until the user exits (Ctrl+D or exit command).
    ///
    /// # Arguments
    /// * `shell` - Shell to run (e.g., "/bin/sh", "/bin/bash"). If None, uses /bin/sh.
    ///
    /// # Returns
    /// The exit code of the shell process.
    async fn attach(&mut self, shell: Option<&str>) -> Result<i32> {
        // Default implementation returns an error since not all backends support PTY
        let _ = shell;
        anyhow::bail!("Interactive shell not supported by this backend")
    }

    /// Attach to the sandbox with an interactive shell and environment variables
    ///
    /// # Arguments
    /// * `shell` - Shell to run (e.g., "/bin/sh", "/bin/bash"). If None, uses /bin/sh.
    /// * `env` - Environment variables as KEY=VALUE pairs
    ///
    /// # Returns
    /// The exit code of the shell process.
    async fn attach_with_env(&mut self, shell: Option<&str>, env: &[String]) -> Result<i32> {
        // Default implementation ignores env vars
        if !env.is_empty() {
            eprintln!(
                "Warning: This backend doesn't support environment variables, ignoring {} var(s)",
                env.len()
            );
        }
        self.attach(shell).await
    }
}

/// Validate a path for sandbox file operations
///
/// Ensures paths are:
/// - Absolute (start with '/')
/// - No path traversal (..)
/// - Not targeting sensitive system paths
pub fn validate_sandbox_path(path: &str) -> Result<()> {
    use anyhow::bail;

    // Must be absolute path
    if !path.starts_with('/') {
        bail!("Sandbox path must be absolute, got: {}", path);
    }

    // No path traversal
    if path.contains("..") {
        bail!("Path traversal not allowed: {}", path);
    }

    // Block sensitive system paths
    const BLOCKED_PATHS: &[&str] = &[
        "/proc",
        "/sys",
        "/dev",
        "/etc/passwd",
        "/etc/shadow",
        "/etc/sudoers",
        "/root/.ssh",
    ];

    for blocked in BLOCKED_PATHS {
        if path.starts_with(blocked) {
            bail!("Cannot access system path: {}", path);
        }
    }

    Ok(())
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
        // Kubernetes and Nomad are always "available" when compiled with the feature;
        // actual connectivity is checked at start() time.
        #[cfg(feature = "kubernetes")]
        BackendType::Kubernetes => true,
        #[cfg(not(feature = "kubernetes"))]
        BackendType::Kubernetes => false,
        #[cfg(feature = "nomad")]
        BackendType::Nomad => true,
        #[cfg(not(feature = "nomad"))]
        BackendType::Nomad => false,
    }
}

/// Create a sandbox for the specified backend
///
/// For Docker/Podman, creates persistent sandboxes that survive CLI exit.
/// This is needed because the Sandbox trait workflow (create/start/stop/attach)
/// expects containers to persist between CLI invocations.
pub fn create_sandbox(backend: BackendType, name: &str) -> Result<Box<dyn Sandbox>> {
    create_sandbox_with_config(backend, name, &crate::config::OrchestratorConfig::default())
}

/// Create a sandbox with orchestrator configuration
///
/// Used by Kubernetes/Nomad backends to pass namespace, runtime class, etc.
pub fn create_sandbox_with_config(
    backend: BackendType,
    name: &str,
    #[allow(unused_variables)] orch_config: &crate::config::OrchestratorConfig,
) -> Result<Box<dyn Sandbox>> {
    match backend {
        // Use new_persistent for Docker/Podman so containers survive CLI exit
        BackendType::Docker => Ok(Box::new(DockerSandbox::new_persistent(
            name,
            ContainerRuntime::Docker,
        ))),
        BackendType::Podman => Ok(Box::new(DockerSandbox::new_persistent(
            name,
            ContainerRuntime::Podman,
        ))),
        BackendType::Firecracker => Ok(Box::new(FirecrackerSandbox::new(name)?)),
        #[cfg(target_os = "macos")]
        BackendType::Apple => Ok(Box::new(AppleSandbox::new(name))),
        #[cfg(not(target_os = "macos"))]
        BackendType::Apple => anyhow::bail!("Apple Containers only available on macOS"),
        BackendType::Hyperlight => Ok(Box::new(HyperlightSandbox::new(name))),
        #[cfg(feature = "kubernetes")]
        BackendType::Kubernetes => Ok(Box::new(KubernetesSandbox::new(name, orch_config))),
        #[cfg(not(feature = "kubernetes"))]
        BackendType::Kubernetes => {
            anyhow::bail!("Kubernetes backend not compiled. Rebuild with --features kubernetes")
        }
        #[cfg(feature = "nomad")]
        BackendType::Nomad => Ok(Box::new(NomadSandbox::new(name, orch_config))),
        #[cfg(not(feature = "nomad"))]
        BackendType::Nomad => {
            anyhow::bail!("Nomad backend not compiled. Rebuild with --features nomad")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === BackendType tests ===

    #[test]
    fn test_backend_type_display() {
        assert_eq!(format!("{}", BackendType::Docker), "docker");
        assert_eq!(format!("{}", BackendType::Podman), "podman");
        assert_eq!(format!("{}", BackendType::Firecracker), "firecracker");
        assert_eq!(format!("{}", BackendType::Apple), "apple");
        assert_eq!(format!("{}", BackendType::Hyperlight), "hyperlight");
        assert_eq!(format!("{}", BackendType::Kubernetes), "kubernetes");
        assert_eq!(format!("{}", BackendType::Nomad), "nomad");
    }

    #[test]
    fn test_backend_type_from_str() {
        assert_eq!(
            "docker".parse::<BackendType>().unwrap(),
            BackendType::Docker
        );
        assert_eq!(
            "podman".parse::<BackendType>().unwrap(),
            BackendType::Podman
        );
        assert_eq!(
            "firecracker".parse::<BackendType>().unwrap(),
            BackendType::Firecracker
        );
        assert_eq!("apple".parse::<BackendType>().unwrap(), BackendType::Apple);
        assert_eq!(
            "hyperlight".parse::<BackendType>().unwrap(),
            BackendType::Hyperlight
        );
        assert_eq!(
            "kubernetes".parse::<BackendType>().unwrap(),
            BackendType::Kubernetes
        );
        assert_eq!(
            "k8s".parse::<BackendType>().unwrap(),
            BackendType::Kubernetes
        );
        assert_eq!("nomad".parse::<BackendType>().unwrap(), BackendType::Nomad);
    }

    #[test]
    fn test_backend_type_from_str_case_insensitive() {
        assert_eq!(
            "DOCKER".parse::<BackendType>().unwrap(),
            BackendType::Docker
        );
        assert_eq!(
            "Docker".parse::<BackendType>().unwrap(),
            BackendType::Docker
        );
        assert_eq!(
            "PODMAN".parse::<BackendType>().unwrap(),
            BackendType::Podman
        );
    }

    #[test]
    fn test_backend_type_from_str_invalid() {
        assert!("invalid".parse::<BackendType>().is_err());
        assert!("".parse::<BackendType>().is_err());
        assert!("dock".parse::<BackendType>().is_err());
    }

    #[test]
    fn test_backend_type_serialize() {
        let backend = BackendType::Docker;
        let json = serde_json::to_string(&backend).unwrap();
        assert_eq!(json, "\"Docker\"");
    }

    #[test]
    fn test_backend_type_deserialize() {
        let backend: BackendType = serde_json::from_str("\"Podman\"").unwrap();
        assert_eq!(backend, BackendType::Podman);
    }

    // === SandboxConfig tests ===

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert_eq!(config.image, "alpine:3.20");
        assert_eq!(config.vcpus, 1);
        assert_eq!(config.memory_mb, 512);
        assert!(!config.mount_cwd);
        assert!(config.work_dir.is_none());
        assert!(config.env.is_empty());
        assert!(config.network);
        assert!(!config.read_only);
        assert!(!config.mount_home);
        assert!(config.files.is_empty());
    }

    #[test]
    fn test_sandbox_config_with_image() {
        let config = SandboxConfig::with_image("python:3.12-alpine");
        assert_eq!(config.image, "python:3.12-alpine");
        // Other fields should be default
        assert_eq!(config.vcpus, 1);
        assert_eq!(config.memory_mb, 512);
    }

    #[test]
    fn test_sandbox_config_builder() {
        let config = SandboxConfig::with_image("node:20")
            .with_resources(4, 2048)
            .with_network(false)
            .with_mount_cwd(true, Some("/workspace".to_string()))
            .with_env(vec![("NODE_ENV".to_string(), "production".to_string())]);

        assert_eq!(config.image, "node:20");
        assert_eq!(config.vcpus, 4);
        assert_eq!(config.memory_mb, 2048);
        assert!(!config.network);
        assert!(config.mount_cwd);
        assert_eq!(config.work_dir, Some("/workspace".to_string()));
        assert_eq!(config.env.len(), 1);
        assert_eq!(
            config.env[0],
            ("NODE_ENV".to_string(), "production".to_string())
        );
    }

    // === ExecResult tests ===

    #[test]
    fn test_exec_result_success() {
        let result = ExecResult::success("hello world".to_string());
        assert!(result.is_success());
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello world");
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn test_exec_result_failure() {
        let result = ExecResult::failure(1, "error message".to_string());
        assert!(!result.is_success());
        assert_eq!(result.exit_code, 1);
        assert!(result.stdout.is_empty());
        assert_eq!(result.stderr, "error message");
    }

    #[test]
    fn test_exec_result_output_stdout_only() {
        let result = ExecResult {
            exit_code: 0,
            stdout: "stdout output".to_string(),
            stderr: String::new(),
        };
        assert_eq!(result.output(), "stdout output");
    }

    #[test]
    fn test_exec_result_output_stderr_only() {
        let result = ExecResult {
            exit_code: 1,
            stdout: String::new(),
            stderr: "stderr output".to_string(),
        };
        assert_eq!(result.output(), "stderr output");
    }

    #[test]
    fn test_exec_result_output_combined() {
        let result = ExecResult {
            exit_code: 0,
            stdout: "stdout".to_string(),
            stderr: "stderr".to_string(),
        };
        assert_eq!(result.output(), "stdout\nstderr");
    }

    // === Path validation tests ===

    #[test]
    fn test_validate_sandbox_path_valid() {
        assert!(validate_sandbox_path("/home/user/file.txt").is_ok());
        assert!(validate_sandbox_path("/workspace/project/src/main.rs").is_ok());
        assert!(validate_sandbox_path("/tmp/test").is_ok());
        assert!(validate_sandbox_path("/app/data.json").is_ok());
    }

    #[test]
    fn test_validate_sandbox_path_relative() {
        assert!(validate_sandbox_path("relative/path").is_err());
        assert!(validate_sandbox_path("./file.txt").is_err());
        assert!(validate_sandbox_path("file.txt").is_err());
    }

    #[test]
    fn test_validate_sandbox_path_traversal() {
        assert!(validate_sandbox_path("/home/../etc/passwd").is_err());
        assert!(validate_sandbox_path("/workspace/..").is_err());
        assert!(validate_sandbox_path("/../root").is_err());
    }

    #[test]
    fn test_validate_sandbox_path_blocked_paths() {
        assert!(validate_sandbox_path("/proc/1/cmdline").is_err());
        assert!(validate_sandbox_path("/sys/kernel").is_err());
        assert!(validate_sandbox_path("/dev/null").is_err());
        assert!(validate_sandbox_path("/etc/passwd").is_err());
        assert!(validate_sandbox_path("/etc/shadow").is_err());
        assert!(validate_sandbox_path("/etc/sudoers").is_err());
        assert!(validate_sandbox_path("/root/.ssh/id_rsa").is_err());
    }

    #[test]
    fn test_validate_sandbox_path_similar_but_allowed() {
        // These look similar to blocked paths but should be allowed
        assert!(validate_sandbox_path("/etc/hosts").is_ok());
        assert!(validate_sandbox_path("/home/root/.ssh").is_ok());
        assert!(validate_sandbox_path("/myproc/data").is_ok());
    }

    // === FileInjection tests ===

    #[test]
    fn test_file_injection_creation() {
        let injection = FileInjection {
            content: b"hello world".to_vec(),
            dest: "/app/config.txt".to_string(),
        };
        assert_eq!(injection.content, b"hello world");
        assert_eq!(injection.dest, "/app/config.txt");
    }

    #[test]
    fn test_sandbox_config_with_files() {
        let files = vec![
            FileInjection {
                content: b"content1".to_vec(),
                dest: "/app/file1.txt".to_string(),
            },
            FileInjection {
                content: b"content2".to_vec(),
                dest: "/app/file2.txt".to_string(),
            },
        ];

        let config = SandboxConfig::default().with_files(files);
        assert_eq!(config.files.len(), 2);
    }
}
