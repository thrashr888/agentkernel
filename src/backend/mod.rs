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
pub mod remote;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::ssh::SshConfig;

pub use crate::container_network::{ManagedNetworkConfig, ManagedNetworkLease, NetworkAllocator};

#[cfg(target_os = "macos")]
pub use apple::AppleSandbox;
pub use docker::{ContainerRuntime, DockerSandbox};
pub use firecracker::FirecrackerSandbox;
pub use hyperlight::HyperlightSandbox;
#[cfg(feature = "kubernetes")]
pub use kubernetes::KubernetesSandbox;
#[cfg(feature = "nomad")]
pub use nomad::NomadSandbox;
pub use remote::{RemoteProvider, RemoteSandbox, remote_bridge_available};
use remote::{remote_bridge_configured, remote_bridge_custom_configured};

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
    /// Daytona hosted sandboxes
    Daytona,
    /// Runloop hosted devboxes
    Runloop,
    /// E2B hosted sandboxes
    E2B,
    /// Modal hosted sandboxes
    Modal,
    /// Agent Computer hosted machines
    AgentComputer,
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
            BackendType::Daytona => write!(f, "daytona"),
            BackendType::Runloop => write!(f, "runloop"),
            BackendType::E2B => write!(f, "e2b"),
            BackendType::Modal => write!(f, "modal"),
            BackendType::AgentComputer => write!(f, "agentcomputer"),
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
            "daytona" => Ok(BackendType::Daytona),
            "runloop" => Ok(BackendType::Runloop),
            "e2b" => Ok(BackendType::E2B),
            "modal" => Ok(BackendType::Modal),
            "agentcomputer" | "agent-computer" => Ok(BackendType::AgentComputer),
            _ => Err(format!(
                "Unknown backend '{}'. Valid options: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad, daytona, runloop, e2b, modal, agentcomputer",
                s
            )),
        }
    }
}

impl BackendType {
    /// All backend identifiers exposed by the public API, in stable display
    /// order. Availability is reported separately by backend discovery.
    pub const fn all() -> [Self; 12] {
        [
            Self::Docker,
            Self::Podman,
            Self::Firecracker,
            Self::Apple,
            Self::Hyperlight,
            Self::Kubernetes,
            Self::Nomad,
            Self::Daytona,
            Self::Runloop,
            Self::E2B,
            Self::Modal,
            Self::AgentComputer,
        ]
    }

    pub fn is_remote(self) -> bool {
        matches!(
            self,
            BackendType::Daytona
                | BackendType::Runloop
                | BackendType::E2B
                | BackendType::Modal
                | BackendType::AgentComputer
        )
    }
}

/// Protocol for port mappings
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    #[default]
    Tcp,
    Udp,
}

impl fmt::Display for PortProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortProtocol::Tcp => write!(f, "tcp"),
            PortProtocol::Udp => write!(f, "udp"),
        }
    }
}

/// A port mapping from host to container
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    /// Host port (None = auto-assign)
    pub host_port: Option<u16>,
    /// Container port (required)
    pub container_port: u16,
    /// Protocol (default: tcp)
    #[serde(default)]
    pub protocol: PortProtocol,
}

/// Provider-resolved endpoint for an exposed sandbox port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResolvedEndpoint {
    pub container_port: u16,
    #[serde(default)]
    pub protocol: PortProtocol,
    pub url: String,
}

/// Provider/runtime state discovered while a sandbox is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SandboxRuntimeMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub remote_metadata: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<ResolvedEndpoint>,
}

/// Persisted remote state used to reconnect to a remote sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteSandboxContext {
    pub remote_id: Option<String>,
    pub remote_namespace: Option<String>,
    pub remote_metadata: HashMap<String, String>,
    pub workspace_revision: Option<String>,
    pub endpoints: Vec<ResolvedEndpoint>,
    pub local_workspace: Option<String>,
    pub config_path: Option<String>,
}

/// Backend-specific compatibility metadata for a full VM state snapshot.
///
/// Filesystem-only snapshots do not use this type.  A backend that implements
/// full-state pause/resume must persist enough information here to reject a
/// restore on an incompatible VMM or host before loading guest memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullStateSnapshot {
    pub firecracker_version: String,
    pub architecture: String,
    pub host_kernel_release: String,
    /// SHA-256 of the host machine identity; the raw identifier is never persisted.
    pub host_identity_sha256: String,
    /// SHA-256 of snapshot-relevant CPU identity and feature flags.
    pub cpu_fingerprint_sha256: String,
    /// Exact kernel release reported by the guest at snapshot time.
    pub guest_kernel_release: String,
}

/// A full-state pause failed and the source VM could not be confirmed resumed.
///
/// Callers can downcast an [`anyhow::Error`] to this type to decide whether the
/// staging directory must be retained. `artifacts_complete` means all memory,
/// vmstate, and rootfs artifacts were durably produced; when it is false the
/// directory may still contain useful partial recovery data and must not be
/// discarded automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullStatePauseError {
    pub source_resume_failed: bool,
    pub artifacts_complete: bool,
    pub snapshot: FullStateSnapshot,
    pub operation_error: String,
    pub resume_error: Option<String>,
}

impl FullStatePauseError {
    pub fn source_resume_failed(
        snapshot: FullStateSnapshot,
        artifacts_complete: bool,
        operation_error: impl fmt::Display,
        resume_error: impl fmt::Display,
    ) -> Self {
        Self {
            source_resume_failed: true,
            artifacts_complete,
            snapshot,
            operation_error: operation_error.to_string(),
            resume_error: Some(resume_error.to_string()),
        }
    }
}

impl fmt::Display for FullStatePauseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "full-state pause failed and source VM could not be resumed: {}",
            self.operation_error
        )?;
        if let Some(resume_error) = &self.resume_error {
            write!(f, "; resume failed: {resume_error}")?;
        }
        write!(
            f,
            "; checkpoint artifacts complete: {}",
            self.artifacts_complete
        )
    }
}

impl std::error::Error for FullStatePauseError {}

/// A Firecracker termination attempt failed after a full-state snapshot.
/// `process_may_be_running` is false only when process exit was confirmed and
/// the remaining failure is cleanup-only, so checkpoint publication is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullStateTerminationError {
    pub process_may_be_running: bool,
    pub detail: String,
}

impl fmt::Display for FullStateTerminationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for FullStateTerminationError {}

/// Outcome-level backend capabilities used for compatibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BackendCapabilities {
    pub mount_cwd: bool,
    pub mount_home: bool,
    pub attach: bool,
    pub host_volumes: bool,
    pub ssh: bool,
    pub proxy_secret_bindings: bool,
    pub secret_files: bool,
    pub snapshots: bool,
    pub resume: bool,
    /// Preserves guest memory/process/device state, not only the filesystem.
    pub full_state_pause_resume: bool,
    /// Restores multiple independent running children from one paused state.
    pub full_state_fork: bool,
    pub endpoints: bool,
}

pub fn backend_capabilities(backend: BackendType) -> BackendCapabilities {
    if backend.is_remote() {
        return BackendCapabilities {
            mount_cwd: true,
            mount_home: false,
            attach: true,
            host_volumes: false,
            ssh: false,
            proxy_secret_bindings: false,
            secret_files: true,
            snapshots: true,
            resume: true,
            full_state_pause_resume: false,
            full_state_fork: false,
            endpoints: true,
        };
    }

    let full_state = backend == BackendType::Firecracker
        && cfg!(all(target_os = "linux", target_arch = "x86_64"));
    BackendCapabilities {
        mount_cwd: true,
        mount_home: true,
        attach: !matches!(backend, BackendType::Firecracker | BackendType::Hyperlight),
        // Named persistent volumes are currently translated into Docker/Podman
        // `-v` arguments by the VMM. Other local backends must reject them
        // rather than advertising support and silently dropping the mounts.
        host_volumes: matches!(backend, BackendType::Docker | BackendType::Podman),
        ssh: true,
        proxy_secret_bindings: true,
        secret_files: true,
        snapshots: !matches!(backend, BackendType::Hyperlight),
        resume: !matches!(backend, BackendType::Hyperlight),
        full_state_pause_resume: full_state,
        full_state_fork: full_state,
        endpoints: true,
    }
}

impl PortMapping {
    /// Parse a Docker-style port string: "host:container", "container", "host:container/udp"
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let (port_part, protocol) = if let Some(stripped) = s.strip_suffix("/udp") {
            (stripped, PortProtocol::Udp)
        } else if let Some(stripped) = s.strip_suffix("/tcp") {
            (stripped, PortProtocol::Tcp)
        } else {
            (s, PortProtocol::Tcp)
        };

        if let Some((host, container)) = port_part.split_once(':') {
            let host_port: u16 = host
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid host port '{}' in '{}'", host, s))?;
            let container_port: u16 = container.parse().map_err(|_| {
                anyhow::anyhow!("Invalid container port '{}' in '{}'", container, s)
            })?;
            Ok(PortMapping {
                host_port: Some(host_port),
                container_port,
                protocol,
            })
        } else {
            let container_port: u16 = port_part
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid port '{}' in '{}'", port_part, s))?;
            Ok(PortMapping {
                host_port: None,
                container_port,
                protocol,
            })
        }
    }
}

impl fmt::Display for PortMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.host_port {
            Some(hp) => write!(f, "{}:{}", hp, self.container_port)?,
            None => write!(f, "{}", self.container_port)?,
        }
        if self.protocol == PortProtocol::Udp {
            write!(f, "/udp")?;
        }
        Ok(())
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
    /// Container-side working directory for the workspace mount.
    pub container_work_dir: Option<String>,
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
    /// Port mappings (host:container)
    pub ports: Vec<PortMapping>,
    /// SSH configuration (None = SSH disabled)
    pub ssh: Option<SshConfig>,
    /// Volume mounts (slug:/path or slug:/path:ro)
    pub volumes: Vec<String>,
    /// Optional AgentKernel-managed bridge network (Docker/Podman only).
    pub managed_network: Option<ManagedNetworkConfig>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: "alpine:3.24".to_string(),
            vcpus: 1,
            memory_mb: 512,
            mount_cwd: false,
            work_dir: None,
            container_work_dir: None,
            env: Vec::new(),
            network: true,
            read_only: false,
            mount_home: false,
            files: Vec::new(),
            ports: Vec::new(),
            ssh: None,
            volumes: Vec::new(),
            managed_network: None,
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

    /// Set the container-side working directory for the workspace mount.
    pub fn with_container_work_dir(mut self, work_dir: Option<String>) -> Self {
        self.container_work_dir = work_dir;
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

    /// Set port mappings
    pub fn with_ports(mut self, ports: Vec<PortMapping>) -> Self {
        self.ports = ports;
        self
    }

    /// Set SSH configuration
    pub fn with_ssh(mut self, ssh: Option<SshConfig>) -> Self {
        self.ssh = ssh;
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

/// Options for executing a command in a sandbox
#[derive(Debug, Default, Clone)]
pub struct ExecOptions {
    /// Environment variables as KEY=VALUE pairs
    pub env: Vec<String>,
    /// Working directory inside the sandbox
    pub workdir: Option<String>,
    /// User to run the command as (e.g., "root")
    pub user: Option<String>,
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

    /// Execute a command with environment variables
    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        if !env.is_empty() {
            eprintln!(
                "Warning: This backend doesn't support environment variables, ignoring {} var(s)",
                env.len()
            );
        }
        self.exec(cmd).await
    }

    /// Execute a command with full options (env, workdir, user)
    async fn exec_with_options(&mut self, cmd: &[&str], opts: &ExecOptions) -> Result<ExecResult> {
        if opts.workdir.is_some() || opts.user.is_some() {
            eprintln!("Warning: This backend doesn't support workdir/user options, ignoring");
        }
        self.exec_with_env(cmd, &opts.env).await
    }

    /// Stop the sandbox and clean up resources
    async fn stop(&mut self) -> Result<()>;

    /// Supply an opaque durable Firecracker disk lineage to use on start.
    /// Backends that do not have a host-side writable disk ignore this hook.
    fn set_persistent_disk_reference(&mut self, _reference: Option<&str>) -> Result<()> {
        Ok(())
    }

    /// Return the opaque durable disk lineage currently owned by this backend.
    fn persistent_disk_reference(&self) -> Option<String> {
        None
    }

    /// Atomically publish a newly-created durable disk reference after the
    /// owning sandbox state has been written. Backends without such a disk do
    /// nothing.
    fn publish_persistent_disk_reference(&mut self) -> Result<()> {
        Ok(())
    }

    /// Abort an unpublished durable disk reference after the corresponding
    /// state write has been rolled back. The runtime remains owned by the
    /// caller and ordinary stop may then discard its transient disk.
    fn rollback_persistent_disk_reference(&mut self) -> Result<()> {
        Ok(())
    }

    /// Pause a running sandbox into a durable, full-state checkpoint.
    ///
    /// Backends must attempt to leave the sandbox running if checkpoint
    /// creation fails before the original runtime is terminated. If that
    /// recovery cannot be confirmed, return [`FullStatePauseError`] and retain
    /// ownership so the caller can publish complete artifacts or retry the
    /// live source in place. The default is deliberately unsupported so
    /// filesystem snapshot support is not mistaken for full VM state support.
    async fn pause_to(&mut self, _checkpoint_dir: &Path) -> Result<FullStateSnapshot> {
        anyhow::bail!(
            "Backend '{}' does not support full-state pause/resume",
            self.backend_type()
        )
    }

    /// Conservative bytes to reserve before creating a full-state checkpoint.
    /// Backends with a writable disk should include its logical size in
    /// addition to configured guest memory and snapshot metadata overhead.
    fn full_state_reservation_bytes(&self, memory_mb: u64) -> Result<u64> {
        memory_mb
            .checked_mul(1024 * 1024)
            .and_then(|bytes| bytes.checked_add(64 * 1024 * 1024))
            .ok_or_else(|| anyhow::anyhow!("full-state checkpoint reservation overflow"))
    }

    /// Retry resuming the still-live source after a failed full-state pause.
    ///
    /// This exists for the recovery path represented by
    /// [`FullStatePauseError`]. Callers must retain the backend object while a
    /// partial checkpoint is not independently restorable.
    async fn retry_full_state_resume(&mut self) -> Result<()> {
        anyhow::bail!(
            "Backend '{}' does not support in-place full-state resume",
            self.backend_type()
        )
    }

    /// Restore a sandbox from a previously-created full-state checkpoint.
    async fn restore_from(
        &mut self,
        _checkpoint_dir: &Path,
        _snapshot: &FullStateSnapshot,
    ) -> Result<()> {
        anyhow::bail!(
            "Backend '{}' does not support full-state pause/resume",
            self.backend_type()
        )
    }

    /// Permanently delete the sandbox and provider-side resources.
    async fn remove(&mut self) -> Result<()> {
        self.stop().await
    }

    /// Attempt to resize sandbox resources in-place.
    ///
    /// Returns:
    /// - `Ok(true)` when resize succeeded in-place
    /// - `Ok(false)` when backend does not support in-place resize
    async fn resize(&mut self, _vcpus: u32, _memory_mb: u64) -> Result<bool> {
        Ok(false)
    }

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

    /// Runtime metadata for provider-backed sandboxes.
    fn runtime_metadata(&self) -> Option<SandboxRuntimeMetadata> {
        None
    }

    /// Restore a durable managed-network lease when reconnecting to a
    /// container that survived an AgentKernel process restart.
    fn restore_managed_network(&mut self, _config: &ManagedNetworkConfig) -> Result<()> {
        Ok(())
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

    // A configured hosted provider is a valid automatic default when no local
    // runtime is installed. Keep local runtimes preferred so adding provider
    // credentials never changes an existing local default.
    [
        BackendType::Daytona,
        BackendType::Runloop,
        BackendType::E2B,
        BackendType::Modal,
        BackendType::AgentComputer,
    ]
    .into_iter()
    .find(|&backend| backend_readiness(backend).usable)
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
        BackendType::Daytona
        | BackendType::Runloop
        | BackendType::E2B
        | BackendType::Modal
        | BackendType::AgentComputer => remote_bridge_available(),
    }
}

/// Server-side backend readiness, separating an installed/configured backend
/// from one that is ready to create a sandbox right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendReadiness {
    pub configured: bool,
    pub usable: bool,
    pub reason: String,
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn configured_env_values(
    value: Option<&str>,
    named_env_value: Option<&str>,
    fallback_env_value: Option<&str>,
) -> bool {
    [value, named_env_value, fallback_env_value]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
}

fn configured_env(value: Option<&str>, env_name: Option<&str>, fallback_env: &str) -> bool {
    configured_env_values(
        value,
        env_name
            .and_then(|name| std::env::var(name).ok())
            .as_deref(),
        std::env::var(fallback_env).ok().as_deref(),
    )
}

fn project_config() -> Option<crate::config::Config> {
    let path = Path::new("agentkernel.toml");
    path.exists()
        .then(|| crate::config::Config::from_file(path).ok())
        .flatten()
}

fn remote_credentials_configured(backend: BackendType) -> bool {
    let config = project_config();
    remote_credentials_configured_from_config(backend, config.as_ref())
}

fn remote_credentials_configured_from_config(
    backend: BackendType,
    config: Option<&crate::config::Config>,
) -> bool {
    let provider = config.map(|config| match backend {
        BackendType::Daytona => &config.remote.daytona,
        BackendType::Runloop => &config.remote.runloop,
        BackendType::E2B => &config.remote.e2b,
        BackendType::Modal => &config.remote.modal,
        BackendType::AgentComputer => &config.remote.agentcomputer,
        _ => unreachable!("remote credential probe called for local backend"),
    });

    match backend {
        BackendType::Modal => {
            let token_id = provider
                .map(|provider| {
                    configured_env(
                        provider.token_id.as_deref(),
                        provider.token_id_env.as_deref(),
                        "MODAL_TOKEN_ID",
                    )
                })
                .unwrap_or_else(|| configured_env(None, None, "MODAL_TOKEN_ID"));
            let token_secret = provider
                .map(|provider| {
                    configured_env(
                        provider.token_secret.as_deref(),
                        provider.token_secret_env.as_deref(),
                        "MODAL_TOKEN_SECRET",
                    )
                })
                .unwrap_or_else(|| configured_env(None, None, "MODAL_TOKEN_SECRET"));
            token_id && token_secret
        }
        BackendType::Daytona => {
            provider.is_some_and(|provider| {
                configured_env(
                    provider.api_key.as_deref(),
                    provider.api_key_env.as_deref(),
                    "DAYTONA_API_KEY",
                )
            }) || std::env::var("DAYTONA_API_KEY").is_ok_and(|value| !value.trim().is_empty())
        }
        BackendType::Runloop => {
            provider.is_some_and(|provider| {
                configured_env(
                    provider.api_key.as_deref(),
                    provider.api_key_env.as_deref(),
                    "RUNLOOP_API_KEY",
                )
            }) || std::env::var("RUNLOOP_API_KEY").is_ok_and(|value| !value.trim().is_empty())
        }
        BackendType::E2B => {
            provider.is_some_and(|provider| {
                configured_env(
                    provider.api_key.as_deref(),
                    provider.api_key_env.as_deref(),
                    "E2B_API_KEY",
                )
            }) || std::env::var("E2B_API_KEY").is_ok_and(|value| !value.trim().is_empty())
        }
        BackendType::AgentComputer => {
            provider.is_some_and(|provider| {
                configured_env(
                    provider.api_key.as_deref(),
                    provider.api_key_env.as_deref(),
                    "AGENTCOMPUTER_API_KEY",
                )
            }) || std::env::var("AGENTCOMPUTER_API_KEY").is_ok_and(|value| !value.trim().is_empty())
        }
        _ => false,
    }
}

fn bridge_configured_for_backend(
    backend: BackendType,
    bridge_configured: bool,
    custom_bridge_configured: bool,
) -> bool {
    bridge_configured && (backend != BackendType::AgentComputer || custom_bridge_configured)
}

fn expand_user_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn kubeconfig_path() -> Option<PathBuf> {
    project_config()
        .and_then(|config| config.orchestrator.kubeconfig)
        .map(|path| expand_user_path(&path))
        .filter(|path| path.exists())
        .or_else(|| {
            std::env::var_os("KUBECONFIG")
                .map(PathBuf::from)
                .filter(|path| path.exists())
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".kube/config"))
                .filter(|path| path.exists())
        })
}

fn kubeconfig_configured() -> bool {
    std::env::var_os("KUBERNETES_SERVICE_HOST").is_some()
        || kubeconfig_path().is_some_and(|path| path.exists())
}

#[cfg(feature = "kubernetes")]
fn kubernetes_api_configured() -> bool {
    if std::env::var_os("KUBERNETES_SERVICE_HOST").is_some() {
        return true;
    }
    let Some(path) = kubeconfig_path() else {
        return false;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(&contents) else {
        return false;
    };
    let current_context = document
        .get("current-context")
        .and_then(|value| value.as_str());
    let cluster_name = document
        .get("contexts")
        .and_then(|value| value.as_sequence())
        .and_then(|contexts| {
            contexts.iter().find_map(|context| {
                let matches = current_context.is_none()
                    || context.get("name").and_then(|value| value.as_str()) == current_context;
                matches
                    .then(|| context.get("context")?.get("cluster")?.as_str())
                    .flatten()
            })
        });
    document
        .get("clusters")
        .and_then(|value| value.as_sequence())
        .and_then(|clusters| {
            clusters.iter().find_map(|cluster| {
                let matches = cluster_name.is_none()
                    || cluster.get("name").and_then(|value| value.as_str()) == cluster_name;
                matches
                    .then(|| cluster.get("cluster")?.get("server")?.as_str())
                    .flatten()
            })
        })
        .is_some_and(|server| !server.trim().is_empty())
}

#[cfg(not(feature = "kubernetes"))]
fn kubernetes_api_configured() -> bool {
    false
}

fn nomad_address() -> Option<String> {
    std::env::var("NOMAD_ADDR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| project_config().and_then(|config| config.orchestrator.nomad_addr))
        .filter(|value| !value.trim().is_empty())
}

fn nomad_configured() -> bool {
    nomad_address().is_some()
}

fn nomad_address_is_valid() -> bool {
    nomad_address().is_some_and(|address| {
        let authority = address
            .split_once("://")
            .map(|(_, authority)| authority)
            .unwrap_or(&address)
            .split('/')
            .next()
            .unwrap_or_default();
        !authority.trim().is_empty() && !authority.contains(char::is_whitespace)
    })
}

/// Probe backend configuration and readiness without returning credentials or
/// provider-specific secret values.
pub fn backend_readiness(backend: BackendType) -> BackendReadiness {
    let (configured, usable, reason) = match backend {
        BackendType::Docker => {
            let configured = command_available("docker");
            let usable = backend_available(backend);
            let reason = if usable {
                "ready"
            } else if configured {
                "Docker CLI is installed but the daemon is unavailable"
            } else {
                "Docker CLI is not installed"
            };
            (configured, usable, reason)
        }
        BackendType::Podman => {
            let configured = command_available("podman");
            let usable = backend_available(backend);
            let reason = if usable {
                "ready"
            } else if configured {
                "Podman CLI is installed but its service is unavailable"
            } else {
                "Podman CLI is not installed"
            };
            (configured, usable, reason)
        }
        BackendType::Firecracker => {
            let configured = backend_available(backend);
            #[cfg(target_os = "linux")]
            let usable = configured && Path::new("/dev/kvm").exists();
            #[cfg(not(target_os = "linux"))]
            let usable = false;
            let reason = if usable {
                "ready"
            } else if !configured {
                "Firecracker is not installed"
            } else {
                "Firecracker is installed but KVM is unavailable"
            };
            (configured, usable, reason)
        }
        #[cfg(target_os = "macos")]
        BackendType::Apple => {
            let configured = apple::apple_containers_available();
            let usable = configured && apple::macos_version_supported();
            let reason = if usable {
                "ready; Apple Containers service starts on demand"
            } else if !configured {
                "Apple Containers CLI is not installed"
            } else if !apple::macos_version_supported() {
                "Apple Containers requires macOS 26 or newer"
            } else {
                "Apple Containers CLI is installed but the host readiness probe failed"
            };
            (configured, usable, reason)
        }
        #[cfg(not(target_os = "macos"))]
        BackendType::Apple => (false, false, "Apple Containers are only available on macOS"),
        BackendType::Hyperlight => {
            let configured = cfg!(all(target_os = "linux", feature = "hyperlight"));
            let usable = backend_available(backend);
            let reason = if usable {
                "ready"
            } else if !configured {
                "Hyperlight support is not enabled for this build"
            } else {
                "Hyperlight is compiled in but no KVM hypervisor is available"
            };
            (configured, usable, reason)
        }
        BackendType::Kubernetes => {
            let configured = cfg!(feature = "kubernetes") && kubeconfig_configured();
            let usable = configured && kubernetes_api_configured();
            let reason = if usable {
                "ready; cluster connectivity is checked when a sandbox starts"
            } else if !cfg!(feature = "kubernetes") {
                "Kubernetes support is not enabled for this build"
            } else if !configured {
                "Kubernetes support is enabled but no kubeconfig or in-cluster credentials were found"
            } else {
                "Kubernetes credentials are configured but the kubeconfig has no API server"
            };
            (configured, usable, reason)
        }
        BackendType::Nomad => {
            let configured = cfg!(feature = "nomad") && nomad_configured();
            let usable = configured && nomad_address_is_valid();
            let reason = if usable {
                "ready; Nomad connectivity is checked when a sandbox starts"
            } else if !cfg!(feature = "nomad") {
                "Nomad support is not enabled for this build"
            } else if !configured {
                "Nomad support is enabled but NOMAD_ADDR is not configured"
            } else {
                "Nomad is configured but its API address is invalid"
            };
            (configured, usable, reason)
        }
        BackendType::Daytona
        | BackendType::Runloop
        | BackendType::E2B
        | BackendType::Modal
        | BackendType::AgentComputer => {
            let credentials = remote_credentials_configured(backend);
            let bridge = bridge_configured_for_backend(
                backend,
                remote_bridge_configured(),
                remote_bridge_custom_configured(),
            );
            let configured = bridge && credentials;
            let usable = configured && remote_bridge_available();
            let reason = if usable {
                "ready"
            } else if backend == BackendType::AgentComputer && !remote_bridge_custom_configured() {
                "Agent Computer requires a custom provider-aware remote bridge"
            } else if !bridge {
                "Remote bridge is not configured"
            } else if !credentials {
                "Provider credentials are not configured"
            } else {
                "Remote bridge is configured but Node.js or the bridge command is unavailable"
            };
            (configured, usable, reason)
        }
    };

    BackendReadiness {
        configured,
        usable,
        reason: reason.to_string(),
    }
}

/// Create a sandbox for the specified backend
///
/// For Docker/Podman, creates persistent sandboxes that survive CLI exit.
/// This is needed because the Sandbox trait workflow (create/start/stop/attach)
/// expects containers to persist between CLI invocations.
pub fn create_sandbox(backend: BackendType, name: &str) -> Result<Box<dyn Sandbox>> {
    create_sandbox_with_state(
        backend,
        name,
        &crate::config::OrchestratorConfig::default(),
        None,
    )
}

/// Create a sandbox with orchestrator configuration
///
/// Used by Kubernetes/Nomad backends to pass namespace, runtime class, etc.
pub fn create_sandbox_with_config(
    backend: BackendType,
    name: &str,
    #[allow(unused_variables)] orch_config: &crate::config::OrchestratorConfig,
) -> Result<Box<dyn Sandbox>> {
    create_sandbox_with_state(backend, name, orch_config, None)
}

pub fn create_sandbox_with_state(
    backend: BackendType,
    name: &str,
    #[allow(unused_variables)] orch_config: &crate::config::OrchestratorConfig,
    remote: Option<RemoteSandboxContext>,
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
        BackendType::Apple => Ok(Box::new(AppleSandbox::new_persistent(name))),
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
        BackendType::Daytona => Ok(Box::new(RemoteSandbox::new(
            RemoteProvider::Daytona,
            name,
            remote.unwrap_or_default(),
        ))),
        BackendType::Runloop => Ok(Box::new(RemoteSandbox::new(
            RemoteProvider::Runloop,
            name,
            remote.unwrap_or_default(),
        ))),
        BackendType::E2B => Ok(Box::new(RemoteSandbox::new(
            RemoteProvider::E2B,
            name,
            remote.unwrap_or_default(),
        ))),
        BackendType::Modal => Ok(Box::new(RemoteSandbox::new(
            RemoteProvider::Modal,
            name,
            remote.unwrap_or_default(),
        ))),
        BackendType::AgentComputer => Ok(Box::new(RemoteSandbox::new(
            RemoteProvider::AgentComputer,
            name,
            remote.unwrap_or_default(),
        ))),
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
        assert_eq!(format!("{}", BackendType::Daytona), "daytona");
        assert_eq!(format!("{}", BackendType::Runloop), "runloop");
        assert_eq!(format!("{}", BackendType::E2B), "e2b");
        assert_eq!(format!("{}", BackendType::Modal), "modal");
        assert_eq!(format!("{}", BackendType::AgentComputer), "agentcomputer");
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
        assert_eq!(
            "daytona".parse::<BackendType>().unwrap(),
            BackendType::Daytona
        );
        assert_eq!(
            "runloop".parse::<BackendType>().unwrap(),
            BackendType::Runloop
        );
        assert_eq!("e2b".parse::<BackendType>().unwrap(), BackendType::E2B);
        assert_eq!("modal".parse::<BackendType>().unwrap(), BackendType::Modal);
        assert_eq!(
            "agentcomputer".parse::<BackendType>().unwrap(),
            BackendType::AgentComputer
        );
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

    #[test]
    fn test_backend_capabilities_remote() {
        let caps = backend_capabilities(BackendType::Daytona);
        assert!(caps.mount_cwd);
        assert!(caps.attach);
        assert!(caps.snapshots);
        assert!(!caps.proxy_secret_bindings);
        assert!(!caps.host_volumes);
        assert!(!caps.full_state_pause_resume);
        assert!(!caps.full_state_fork);
    }

    #[test]
    fn full_state_capabilities_match_the_linux_x86_64_firecracker_boundary() {
        for backend in BackendType::all() {
            let capabilities = backend_capabilities(backend);
            let expected = backend == BackendType::Firecracker
                && cfg!(all(target_os = "linux", target_arch = "x86_64"));
            assert_eq!(capabilities.full_state_pause_resume, expected, "{backend}");
            assert_eq!(capabilities.full_state_fork, expected, "{backend}");
        }
        assert!(!backend_capabilities(BackendType::Firecracker).attach);
    }

    #[test]
    fn test_host_volume_capability_matches_implemented_backends() {
        assert!(backend_capabilities(BackendType::Docker).host_volumes);
        assert!(backend_capabilities(BackendType::Podman).host_volumes);
        for backend in [
            BackendType::Apple,
            BackendType::Firecracker,
            BackendType::Hyperlight,
        ] {
            assert!(!backend_capabilities(backend).host_volumes);
        }
    }

    #[test]
    fn test_backend_readiness_is_truthful_and_never_hides_reason() {
        for backend in BackendType::all() {
            let readiness = backend_readiness(backend);
            assert!(!readiness.reason.trim().is_empty());
            assert!(!readiness.usable || readiness.configured);
        }
    }

    #[test]
    fn test_remote_credentials_probe_uses_config_without_exposing_values() {
        let mut config = crate::config::Config::minimal("test", "codex");
        config.remote.daytona.api_key = Some("daytona-secret-value".to_string());
        assert!(remote_credentials_configured_from_config(
            BackendType::Daytona,
            Some(&config)
        ));
        assert!(configured_env_values(Some("configured"), None, None));
        assert!(!configured_env_values(None, None, None));
        assert!(!configured_env_values(None, Some("  "), Some("")));
    }

    #[test]
    fn test_agentcomputer_requires_a_custom_bridge() {
        assert!(bridge_configured_for_backend(
            BackendType::Daytona,
            true,
            false
        ));
        assert!(!bridge_configured_for_backend(
            BackendType::AgentComputer,
            true,
            false
        ));
        assert!(bridge_configured_for_backend(
            BackendType::AgentComputer,
            true,
            true
        ));
    }

    // === SandboxConfig tests ===

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert_eq!(config.image, "alpine:3.24");
        assert_eq!(config.vcpus, 1);
        assert_eq!(config.memory_mb, 512);
        assert!(!config.mount_cwd);
        assert!(config.work_dir.is_none());
        assert!(config.env.is_empty());
        assert!(config.network);
        assert!(!config.read_only);
        assert!(!config.mount_home);
        assert!(config.files.is_empty());
        assert!(config.ports.is_empty());
        assert!(config.ssh.is_none());
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

    // === PortMapping tests ===

    #[test]
    fn test_port_mapping_parse_host_container() {
        let pm = PortMapping::parse("8080:80").unwrap();
        assert_eq!(pm.host_port, Some(8080));
        assert_eq!(pm.container_port, 80);
        assert_eq!(pm.protocol, PortProtocol::Tcp);
    }

    #[test]
    fn test_port_mapping_parse_container_only() {
        let pm = PortMapping::parse("3000").unwrap();
        assert_eq!(pm.host_port, None);
        assert_eq!(pm.container_port, 3000);
        assert_eq!(pm.protocol, PortProtocol::Tcp);
    }

    #[test]
    fn test_port_mapping_parse_udp() {
        let pm = PortMapping::parse("5353:53/udp").unwrap();
        assert_eq!(pm.host_port, Some(5353));
        assert_eq!(pm.container_port, 53);
        assert_eq!(pm.protocol, PortProtocol::Udp);
    }

    #[test]
    fn test_port_mapping_parse_explicit_tcp() {
        let pm = PortMapping::parse("8080:80/tcp").unwrap();
        assert_eq!(pm.host_port, Some(8080));
        assert_eq!(pm.container_port, 80);
        assert_eq!(pm.protocol, PortProtocol::Tcp);
    }

    #[test]
    fn test_port_mapping_parse_invalid_host() {
        assert!(PortMapping::parse("abc:80").is_err());
    }

    #[test]
    fn test_port_mapping_parse_invalid_container() {
        assert!(PortMapping::parse("8080:abc").is_err());
    }

    #[test]
    fn test_port_mapping_parse_invalid_single() {
        assert!(PortMapping::parse("not-a-port").is_err());
    }

    #[test]
    fn test_port_mapping_display() {
        assert_eq!(
            format!(
                "{}",
                PortMapping {
                    host_port: Some(8080),
                    container_port: 80,
                    protocol: PortProtocol::Tcp
                }
            ),
            "8080:80"
        );
        assert_eq!(
            format!(
                "{}",
                PortMapping {
                    host_port: None,
                    container_port: 3000,
                    protocol: PortProtocol::Tcp
                }
            ),
            "3000"
        );
        assert_eq!(
            format!(
                "{}",
                PortMapping {
                    host_port: Some(5353),
                    container_port: 53,
                    protocol: PortProtocol::Udp
                }
            ),
            "5353:53/udp"
        );
    }

    #[test]
    fn test_port_mapping_serialize_roundtrip() {
        let pm = PortMapping::parse("8080:80").unwrap();
        let json = serde_json::to_string(&pm).unwrap();
        let pm2: PortMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(pm, pm2);
    }

    #[test]
    fn test_sandbox_config_with_ports() {
        let ports = vec![
            PortMapping::parse("8080:80").unwrap(),
            PortMapping::parse("3000").unwrap(),
        ];
        let config = SandboxConfig::default().with_ports(ports);
        assert_eq!(config.ports.len(), 2);
        assert_eq!(config.ports[0].container_port, 80);
        assert_eq!(config.ports[1].container_port, 3000);
    }
}
