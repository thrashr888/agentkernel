//! Firecracker microVM backend implementing the Sandbox trait.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use super::{
    BackendType, ExecResult, FullStatePauseError, FullStateSnapshot, FullStateTerminationError,
    Sandbox, SandboxConfig,
};
use crate::cow::{RootfsCow, RootfsCowStore};
use crate::firecracker_client::{
    BootSource, Drive, FirecrackerClient, MachineConfig, MemoryBackend, SnapshotCreateParams,
    SnapshotLoadParams, VsockDevice, VsockOverride,
};
use crate::full_state::{MEMORY_FILE, ROOTFS_FILE, VMSTATE_FILE};
use crate::languages::docker_image_to_firecracker_runtime;
use crate::vsock::VsockClient;

const SUPPORTED_FIRECRACKER_VERSION: &str = "1.16.1";
const SUPPORTED_GUEST_KERNEL_RELEASE: &str = "6.18.45-agentkernel";

fn canonical_firecracker_binary(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    fs::canonicalize(path)
        .with_context(|| format!("failed to resolve Firecracker binary {}", path.display()))
}

fn ensure_full_state_host_supported() -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        bail!(
            "Firecracker full-state pause/resume requires Linux x86_64 with KVM (host is {} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    Ok(())
}

fn ensure_clock_realtime_kernel_supported(release: &str) -> Result<()> {
    let mut parts = release.split(['.', '-']);
    let major = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or_else(|| anyhow::anyhow!("unrecognized host kernel release '{release}'"))?;
    let minor = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or_else(|| anyhow::anyhow!("unrecognized host kernel release '{release}'"))?;
    if (major, minor) < (5, 16) {
        bail!(
            "Firecracker full-state restore with clock_realtime requires host Linux >= 5.16 (host is {release})"
        );
    }
    Ok(())
}

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
            return canonical_firecracker_binary(path);
        }
    }

    // Check user's local bin directories
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);

        // ~/.local/bin/firecracker (common user install location)
        let local_bin = home.join(".local/bin/firecracker");
        if local_bin.exists() {
            return canonical_firecracker_binary(local_bin);
        }

        // ~/.local/share/agentkernel/bin/firecracker (agentkernel managed)
        let agentkernel_bin = home.join(".local/share/agentkernel/bin/firecracker");
        if agentkernel_bin.exists() {
            return canonical_firecracker_binary(agentkernel_bin);
        }
    }

    // Check common system locations
    let locations = [
        "/usr/local/bin/firecracker",
        "/usr/bin/firecracker",
        "./firecracker",
    ];

    for loc in locations {
        let path = PathBuf::from(loc);
        if path.exists() {
            return canonical_firecracker_binary(path);
        }
    }

    // Try PATH
    if let Ok(output) = Command::new("which").arg("firecracker").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return canonical_firecracker_binary(path);
        }
    }

    bail!("Firecracker binary not found")
}

fn host_kernel_release() -> Result<String> {
    let output = Command::new("uname")
        .arg("-r")
        .output()
        .context("failed to inspect host kernel release")?;
    if !output.status.success() {
        bail!(
            "failed to inspect host kernel release: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let release = String::from_utf8(output.stdout)
        .context("host kernel release is not valid UTF-8")?
        .trim()
        .to_string();
    if release.is_empty() {
        bail!("host kernel release is empty");
    }
    Ok(release)
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn host_identity_sha256() -> Result<String> {
    let machine_id = fs::read_to_string("/etc/machine-id")
        .context("failed to read host machine identity from /etc/machine-id")?;
    let machine_id = machine_id.trim();
    if machine_id.is_empty() {
        bail!("host machine identity is empty");
    }
    Ok(sha256_hex(machine_id.as_bytes()))
}

fn canonical_cpu_identity(cpuinfo: &str) -> Result<String> {
    let stanza = cpuinfo
        .split("\n\n")
        .find(|stanza| stanza.lines().any(|line| line.starts_with("processor")))
        .ok_or_else(|| anyhow::anyhow!("/proc/cpuinfo contains no processor stanza"))?;
    let fields = [
        "vendor_id",
        "cpu family",
        "model",
        "model name",
        "stepping",
        "microcode",
        "flags",
    ];
    let mut canonical = Vec::new();
    for field in fields {
        let Some((_, raw_value)) = stanza.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == field).then_some((key, value))
        }) else {
            if matches!(
                field,
                "vendor_id" | "cpu family" | "model" | "stepping" | "flags"
            ) {
                bail!("/proc/cpuinfo is missing required CPU field '{field}'");
            }
            continue;
        };
        let value = if field == "flags" {
            let mut flags: Vec<&str> = raw_value.split_whitespace().collect();
            flags.sort_unstable();
            flags.dedup();
            flags.join(" ")
        } else {
            raw_value.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        canonical.push(format!("{field}={value}"));
    }
    Ok(canonical.join("\n"))
}

fn cpu_fingerprint_sha256() -> Result<String> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").context("failed to read /proc/cpuinfo")?;
    Ok(sha256_hex(canonical_cpu_identity(&cpuinfo)?.as_bytes()))
}

fn ensure_guest_kernel_supported(release: &str) -> Result<()> {
    if release != SUPPORTED_GUEST_KERNEL_RELEASE {
        bail!(
            "Firecracker full-state requires guest kernel {}, guest is {}",
            SUPPORTED_GUEST_KERNEL_RELEASE,
            release
        );
    }
    Ok(())
}

fn firecracker_binary_version(binary: &Path) -> Result<String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .with_context(|| format!("failed to run {} --version", binary.display()))?;
    if !output.status.success() {
        bail!(
            "failed to inspect Firecracker version: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8(output.stdout)
        .context("Firecracker version output is not valid UTF-8")?;
    parse_firecracker_version(&stdout)
}

fn parse_firecracker_version(output: &str) -> Result<String> {
    output
        .split_whitespace()
        .rev()
        .find_map(|word| {
            let version = word.strip_prefix('v').unwrap_or(word);
            (version.chars().next().is_some_and(|c| c.is_ascii_digit()) && version.contains('.'))
                .then(|| version.to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("unrecognized Firecracker version output: {output:?}"))
}

fn ensure_checkpoint_artifact(checkpoint_dir: &Path, name: &str) -> Result<PathBuf> {
    let path = checkpoint_dir.join(name);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("checkpoint artifact is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        bail!(
            "checkpoint artifact must be a non-empty regular file: {}",
            path.display()
        );
    }
    fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve checkpoint artifact {}", path.display()))
}

fn validate_checkpoint_output_dir(checkpoint_dir: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(checkpoint_dir).with_context(|| {
        format!(
            "full-state checkpoint directory is missing: {}",
            checkpoint_dir.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "full-state checkpoint path is not a directory: {}",
            checkpoint_dir.display()
        );
    }
    for file in [MEMORY_FILE, VMSTATE_FILE, ROOTFS_FILE] {
        let path = checkpoint_dir.join(file);
        if fs::symlink_metadata(&path).is_ok() {
            bail!(
                "refusing to overwrite checkpoint artifact {}",
                path.display()
            );
        }
    }
    fs::canonicalize(checkpoint_dir).with_context(|| {
        format!(
            "failed to resolve full-state checkpoint directory {}",
            checkpoint_dir.display()
        )
    })
}

fn cleanup_checkpoint_artifacts(checkpoint_dir: &Path) {
    for file in [MEMORY_FILE, VMSTATE_FILE, ROOTFS_FILE] {
        let _ = fs::remove_file(checkpoint_dir.join(file));
    }
}

fn transition_cleanup_error(
    phase: &str,
    transition_error: &anyhow::Error,
    cleanup_error: anyhow::Error,
) -> anyhow::Error {
    cleanup_error.context(format!(
        "Firecracker {phase} failed ({transition_error:#}) and cleanup could not prove the VMM exited"
    ))
}

#[cfg(unix)]
fn ensure_private_owned_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create runtime directory {}", path.display()));
        }
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        bail!(
            "Firecracker runtime directory must be a current-user-owned 0700 directory: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_runtime_directory(runtime_id: &str) -> Result<PathBuf> {
    // Keep the rendered path below macOS/Linux `sockaddr_un.sun_path` limits;
    // the current-user-owned 0700 parent is the security boundary.
    let user_root =
        PathBuf::from("/tmp").join(format!("agentkernel-fc-{}", unsafe { libc::geteuid() }));
    ensure_private_owned_directory(&user_root)?;
    let runtime = user_root.join(runtime_id);
    ensure_private_owned_directory(&runtime)?;
    Ok(runtime)
}

#[cfg(not(unix))]
fn create_private_runtime_directory(_runtime_id: &str) -> Result<PathBuf> {
    bail!("Firecracker runtime sockets require a Unix host")
}

#[cfg(unix)]
fn secure_runtime_socket(path: &Path) -> Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect runtime socket {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        bail!(
            "Firecracker runtime endpoint is not a current-user-owned Unix socket: {}",
            path.display()
        );
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to secure runtime socket {}", path.display()))?;
    let secured = fs::symlink_metadata(path)?;
    if secured.file_type().is_symlink()
        || !secured.file_type().is_socket()
        || secured.uid() != unsafe { libc::geteuid() }
        || secured.permissions().mode() & 0o777 != 0o600
    {
        bail!(
            "Firecracker runtime socket permissions are not 0600: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_runtime_socket(_path: &Path) -> Result<()> {
    bail!("Firecracker runtime sockets require a Unix host")
}

fn make_checkpoint_artifacts_readonly(checkpoint_dir: &Path) -> Result<()> {
    for file in [MEMORY_FILE, VMSTATE_FILE, ROOTFS_FILE] {
        let path = checkpoint_dir.join(file);
        let mut permissions = fs::metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions)
            .with_context(|| format!("failed to make {} immutable", path.display()))?;
    }
    Ok(())
}

/// Firecracker microVM sandbox
pub struct FirecrackerSandbox {
    name: String,
    runtime_dir: PathBuf,
    socket_path: PathBuf,
    vsock_path: PathBuf,
    process: Option<Child>,
    vsock_cid: u32,
    kernel_path: Option<PathBuf>,
    rootfs_path: Option<PathBuf>,
    /// Per-sandbox CoW rootfs, cleaned up on stop/drop only when ownership is
    /// proven by the storage helper.
    sandbox_rootfs: Option<RootfsCow>,
    running: bool,
}

impl FirecrackerSandbox {
    fn native_gate_asset(name: &str) -> Option<PathBuf> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64"))
            || std::env::var("AGENTKERNEL_KVM_SMOKE").as_deref() != Ok("1")
        {
            return None;
        }
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_file())
    }

    /// Create a new Firecracker sandbox
    pub fn new(name: &str) -> Result<Self> {
        // Socket paths are runtime identities, not sandbox-name identities.
        // A fresh nonce prevents a restored fork from colliding with its
        // source or with another process that happens to use the same name.
        let runtime_id = Uuid::new_v4().simple().to_string();
        let runtime_dir = create_private_runtime_directory(&runtime_id)?;
        let socket_path = runtime_dir.join("api.sock");
        let vsock_path = runtime_dir.join("vsock.sock");

        // Generate a unique CID (use hash of name + timestamp)
        let vsock_cid = 100
            + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u32
                % 1000);

        let kernel_path = Self::native_gate_asset("AGENTKERNEL_KVM_KERNEL");
        let rootfs_path = Self::native_gate_asset("AGENTKERNEL_KVM_ROOTFS");

        Ok(Self {
            name: name.to_string(),
            runtime_dir,
            socket_path,
            vsock_path,
            process: None,
            vsock_cid,
            // The dedicated native gate supplies these paths explicitly via
            // environment variables. Production installations continue to
            // use the managed image discovery below when they are unset.
            kernel_path,
            rootfs_path,
            sandbox_rootfs: None,
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

    /// Firecracker API socket for this runtime instance.
    pub fn api_socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Host-side vsock UDS for this runtime instance.
    pub fn vsock_socket_path(&self) -> &Path {
        &self.vsock_path
    }

    /// Return the private rootfs image currently attached to this sandbox.
    ///
    /// The path is only available after `start` has prepared the image. Call
    /// [`Self::preserve_prepared_rootfs_for_snapshot`] before stopping when a
    /// Firecracker snapshot will retain this block-device path.
    pub fn prepared_rootfs_path(&self) -> Option<&Path> {
        self.sandbox_rootfs.as_ref().map(RootfsCow::path)
    }

    /// Deliberately retain the prepared rootfs as a durable snapshot input.
    pub fn preserve_prepared_rootfs_for_snapshot(&mut self) -> Result<&Path> {
        let rootfs = self
            .sandbox_rootfs
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("sandbox rootfs has not been prepared"))?;
        rootfs.preserve_for_snapshot()?;
        Ok(rootfs.path())
    }

    /// Find kernel path
    fn find_kernel() -> Result<PathBuf> {
        fn find_vmlinux_in(dir: &Path) -> Option<PathBuf> {
            let managed = dir.join(format!("vmlinux-{SUPPORTED_GUEST_KERNEL_RELEASE}"));
            if managed.is_file() {
                return Some(managed);
            }
            let mut kernels: Vec<PathBuf> = std::fs::read_dir(dir)
                .ok()?
                .flatten()
                .filter_map(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("vmlinux")
                        .then(|| entry.path())
                })
                .collect();
            kernels.sort();
            kernels.into_iter().next()
        }

        // Check local images/kernel/ (development)
        let local_kernel = PathBuf::from("images/kernel");
        if let Some(path) = find_vmlinux_in(&local_kernel) {
            return Ok(path);
        }

        // Check ~/.local/share/agentkernel/kernel (installed)
        if let Some(home) = std::env::var_os("HOME") {
            let kernel_dir = PathBuf::from(home).join(".local/share/agentkernel/images/kernel");
            if let Some(path) = find_vmlinux_in(&kernel_dir) {
                return Ok(path);
            }
        }

        bail!("Kernel not found. Run 'agentkernel setup' to install.")
    }

    /// Find rootfs path for an image
    fn find_rootfs(image: &str) -> Result<PathBuf> {
        // Check for explicit rootfs path (from Dockerfile conversion)
        if let Some(path) = image.strip_prefix("rootfs:") {
            let rootfs_path = PathBuf::from(path);
            if rootfs_path.exists() {
                return Ok(rootfs_path);
            }
            bail!("Converted rootfs not found: {}", path);
        }

        // Map Docker image name to Firecracker runtime
        let runtime = docker_image_to_firecracker_runtime(image);
        let rootfs_name = format!("{}.ext4", runtime);

        // Check local images/rootfs/ (development)
        let local_rootfs = PathBuf::from("images/rootfs").join(&rootfs_name);
        if local_rootfs.exists() {
            return Ok(local_rootfs);
        }

        // Check ~/.local/share/agentkernel/rootfs (installed)
        if let Some(home) = std::env::var_os("HOME") {
            let rootfs_dir = PathBuf::from(home).join(".local/share/agentkernel/images/rootfs");
            let rootfs_path = rootfs_dir.join(&rootfs_name);
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
            match fs::symlink_metadata(&self.socket_path) {
                Ok(_) => {
                    secure_runtime_socket(&self.socket_path)?;
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("failed to inspect Firecracker API socket");
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        bail!("Firecracker API socket not available after 5 seconds")
    }

    fn spawn_firecracker(&mut self, firecracker_bin: &Path) -> Result<()> {
        let working_directory = self
            .sandbox_rootfs
            .as_ref()
            .map(RootfsCow::artifact_dir)
            .ok_or_else(|| anyhow::anyhow!("sandbox rootfs has not been prepared"))?;
        let process = Command::new(firecracker_bin)
            .arg("--api-sock")
            .arg(&self.socket_path)
            .current_dir(working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("Failed to start firecracker: {}", firecracker_bin.display())
            })?;
        self.process = Some(process);
        Ok(())
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
        let kernel_path = fs::canonicalize(&kernel_path)
            .with_context(|| format!("failed to resolve kernel {}", kernel_path.display()))?;

        self.sandbox_rootfs
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Rootfs path not set"))?;

        // Set boot source with optimized boot args
        let boot_source = BootSource {
            kernel_image_path: kernel_path.to_string_lossy().to_string(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw init=/init quiet loglevel=4 i8042.nokbd i8042.noaux".to_string(),
        };
        client.set_boot_source(&boot_source).await?;

        // Set root drive
        let drive = Drive {
            drive_id: "rootfs".to_string(),
            // This relative path is part of the snapshot compatibility
            // contract.  Every restored fork runs from its own RootfsCow
            // artifact directory containing a private `rootfs.ext4`.
            path_on_host: ROOTFS_FILE.to_string(),
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
            match fs::symlink_metadata(&self.vsock_path) {
                Ok(_) => secure_runtime_socket(&self.vsock_path)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("failed to inspect Firecracker vsock endpoint");
                }
            }
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

    async fn guest_kernel_release(&self) -> Result<String> {
        let client = VsockClient::for_firecracker(&self.vsock_path);
        let result = client
            .run_command(&["uname".to_string(), "-r".to_string()])
            .await
            .context("failed to query guest kernel release")?;
        if result.exit_code != 0 {
            bail!(
                "failed to query guest kernel release (exit {}): {}",
                result.exit_code,
                result.stderr.trim()
            );
        }
        let release = result.stdout.trim().to_string();
        if release.is_empty() {
            bail!("guest kernel release is empty");
        }
        Ok(release)
    }

    fn abort_start(&mut self) -> Result<()> {
        // Failed start/restore cleanup is a lifecycle proof boundary too. Do
        // not hide kill/reap uncertainty behind best-effort cleanup: callers
        // must retain this backend whenever the process may still exist.
        self.terminate_paused_runtime()
    }

    /// Terminate a VM after all checkpoint artifacts have been durably
    /// produced.  An error is only returned while the original process is
    /// still available to resume. Cleanup errors after confirmed process exit
    /// are still returned so normal stop callers can report leaked artifacts;
    /// the lifecycle layer may safely publish a complete checkpoint after a
    /// subsequent idempotent stop confirms no process remains.
    fn terminate_paused_runtime(&mut self) -> Result<()> {
        if let Some(mut process) = self.process.take() {
            let status = match process.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    self.process = Some(process);
                    return Err(FullStateTerminationError {
                        process_may_be_running: true,
                        detail: format!("failed to inspect paused Firecracker: {error}"),
                    }
                    .into());
                }
            };
            match status {
                Some(_) => {}
                None => {
                    if let Err(error) = process.kill() {
                        self.process = Some(process);
                        return Err(FullStateTerminationError {
                            process_may_be_running: true,
                            detail: format!("failed to terminate paused Firecracker: {error}"),
                        }
                        .into());
                    }
                    if let Err(error) = process.wait() {
                        self.process = Some(process);
                        return Err(FullStateTerminationError {
                            process_may_be_running: true,
                            detail: format!(
                                "failed to confirm terminated Firecracker process exit: {error}"
                            ),
                        }
                        .into());
                    }
                }
            }
        }

        let _ = fs::remove_file(&self.socket_path);
        let _ = fs::remove_file(&self.vsock_path);
        let _ = fs::remove_dir(&self.runtime_dir);
        let cleanup_result = self
            .sandbox_rootfs
            .take()
            .map_or(Ok(()), |rootfs| rootfs.cleanup());
        self.running = false;
        cleanup_result.map_err(|error| {
            FullStateTerminationError {
                process_may_be_running: false,
                detail: format!(
                    "Firecracker process exited, but runtime rootfs cleanup failed: {error:#}"
                ),
            }
            .into()
        })
    }

    async fn resume_after_snapshot_failure(
        &mut self,
        client: &FirecrackerClient,
        checkpoint_dir: &Path,
        snapshot: &FullStateSnapshot,
        artifacts_complete: bool,
        snapshot_error: anyhow::Error,
    ) -> Result<FullStateSnapshot> {
        match client.resume().await {
            Ok(()) => {
                self.running = true;
                let agent_health = self.wait_for_agent().await;
                self.finish_resumed_snapshot_failure(
                    checkpoint_dir,
                    snapshot,
                    artifacts_complete,
                    snapshot_error,
                    agent_health,
                )
            }
            Err(resume_error) => {
                self.running = false;
                Err(FullStatePauseError::source_resume_failed(
                    snapshot.clone(),
                    artifacts_complete,
                    format!("{snapshot_error:#}"),
                    format!("{resume_error:#}"),
                )
                .into())
            }
        }
    }

    fn finish_resumed_snapshot_failure(
        &mut self,
        checkpoint_dir: &Path,
        snapshot: &FullStateSnapshot,
        artifacts_complete: bool,
        snapshot_error: anyhow::Error,
        agent_health: Result<()>,
    ) -> Result<FullStateSnapshot> {
        match agent_health {
            Ok(()) => {
                // The Resume response alone is not enough: retain complete or
                // partial artifacts until guest health proves the source is
                // usable again.
                cleanup_checkpoint_artifacts(checkpoint_dir);
                Err(snapshot_error.context("full-state checkpoint failed; source VM resumed"))
            }
            Err(agent_error) => Err(FullStatePauseError::source_resume_failed(
                snapshot.clone(),
                artifacts_complete,
                format!("{snapshot_error:#}"),
                format!("Resume was accepted, but guest health was not confirmed: {agent_error:#}"),
            )
            .into()),
        }
    }
}

#[async_trait]
impl Sandbox for FirecrackerSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        if self.running || self.process.is_some() || self.sandbox_rootfs.is_some() {
            bail!(
                "Firecracker sandbox '{}' already has an active runtime",
                self.name
            );
        }
        let firecracker_bin = find_firecracker()?;

        // Create a per-sandbox CoW rootfs for filesystem isolation.  The
        // helper explicitly detects reflink support and falls back to the
        // previous full-copy behavior.  Overlayfs is reported as a host
        // capability but is not suitable for Firecracker's ext4 drive file.
        let base_rootfs = match self.rootfs_path.clone() {
            Some(path) => path,
            None => Self::find_rootfs(&config.image)?,
        };
        let store = RootfsCowStore::open_default()?;
        let rootfs = store
            .prepare(&base_rootfs)
            .with_context(|| format!("failed to prepare writable rootfs for {}", self.name))?;
        eprintln!(
            "[firecracker] rootfs COW strategy={:?} path={}",
            rootfs.strategy(),
            rootfs.path().display()
        );
        self.sandbox_rootfs = Some(rootfs);

        let startup = async {
            // The root drive is configured as `rootfs.ext4`, so this process
            // must run inside the private RootfsCow artifact directory.
            self.spawn_firecracker(&firecracker_bin)?;

            // Wait for socket, configure, start, and wait for the guest agent.
            self.wait_for_socket().await?;
            self.configure(config).await?;
            self.start_instance().await?;
            self.wait_for_agent().await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        match startup {
            Ok(()) => {
                self.running = true;
                Ok(())
            }
            Err(error) => {
                // Do not leave a partially started VM or its rootfs behind if
                // startup fails after the COW artifact was published.
                match self.abort_start() {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => {
                        Err(transition_cleanup_error("startup", &error, cleanup_error))
                    }
                }
            }
        }
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

        // The lifecycle layer relies on a successful stop as proof that a
        // checkpoint cannot coexist with an untracked original VM. Preserve
        // the child handle and return an error if kill/reap is ambiguous.
        self.terminate_paused_runtime()
    }

    async fn pause_to(&mut self, checkpoint_dir: &Path) -> Result<FullStateSnapshot> {
        ensure_full_state_host_supported()?;
        if !self.running || self.process.is_none() {
            bail!("Firecracker sandbox '{}' is not running", self.name);
        }
        let checkpoint_dir = validate_checkpoint_output_dir(checkpoint_dir)?;
        if self.sandbox_rootfs.is_none() {
            bail!("sandbox rootfs has not been prepared");
        }
        let client = FirecrackerClient::new(&self.socket_path);
        let firecracker_version = client
            .get_version()
            .await
            .context("failed to inspect running Firecracker version")?
            .firecracker_version;
        if firecracker_version != SUPPORTED_FIRECRACKER_VERSION {
            bail!(
                "full-state pause requires Firecracker {}, running process is {}",
                SUPPORTED_FIRECRACKER_VERSION,
                firecracker_version
            );
        }
        let host_kernel_release = host_kernel_release()?;
        ensure_clock_realtime_kernel_supported(&host_kernel_release)?;
        let guest_kernel_release = self.guest_kernel_release().await?;
        ensure_guest_kernel_supported(&guest_kernel_release)?;
        let snapshot = FullStateSnapshot {
            firecracker_version,
            architecture: std::env::consts::ARCH.to_string(),
            host_kernel_release,
            host_identity_sha256: host_identity_sha256()?,
            cpu_fingerprint_sha256: cpu_fingerprint_sha256()?,
            guest_kernel_release,
        };

        if let Err(pause_error) = client.pause().await {
            match client.get_instance_info().await {
                Ok(instance) if instance.state == "Paused" => {
                    return self
                        .resume_after_snapshot_failure(
                            &client,
                            &checkpoint_dir,
                            &snapshot,
                            false,
                            pause_error.context(
                                "pause response was lost after Firecracker entered Paused state",
                            ),
                        )
                        .await;
                }
                Ok(instance) => {
                    return Err(pause_error).context(format!(
                        "failed to pause Firecracker VM; observed instance state '{}'",
                        instance.state
                    ));
                }
                Err(inspect_error) => {
                    // A transport failure can happen after Firecracker applied
                    // the state change. Make one best-effort resume attempt so
                    // the caller never silently treats a known-paused VM as
                    // running.
                    return self
                        .resume_after_snapshot_failure(
                            &client,
                            &checkpoint_dir,
                            &snapshot,
                            false,
                            pause_error.context(format!(
                                "failed to confirm pause response because state inspection failed: {inspect_error:#}"
                            )),
                        )
                        .await;
                }
            }
        }

        let rootfs = self
            .sandbox_rootfs
            .as_ref()
            .expect("rootfs presence was validated before pausing");

        let checkpoint_result = async {
            let memory_path = checkpoint_dir.join(MEMORY_FILE);
            let vmstate_path = checkpoint_dir.join(VMSTATE_FILE);
            client
                .create_snapshot(&SnapshotCreateParams {
                    mem_file_path: memory_path.to_string_lossy().into_owned(),
                    snapshot_path: vmstate_path.to_string_lossy().into_owned(),
                    snapshot_type: "Full".to_string(),
                })
                .await
                .context("Firecracker failed to create full VM snapshot")?;

            let rootfs_path = checkpoint_dir.join(ROOTFS_FILE);
            let strategy = rootfs.snapshot_to(&rootfs_path).with_context(|| {
                format!(
                    "failed to preserve paused rootfs in {}",
                    checkpoint_dir.display()
                )
            })?;
            eprintln!(
                "[firecracker] checkpoint rootfs COW strategy={strategy:?} path={}",
                rootfs_path.display()
            );
            make_checkpoint_artifacts_readonly(&checkpoint_dir)?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(error) = checkpoint_result {
            return self
                .resume_after_snapshot_failure(&client, &checkpoint_dir, &snapshot, false, error)
                .await;
        }

        if let Err(error) = self.terminate_paused_runtime() {
            return self
                .resume_after_snapshot_failure(&client, &checkpoint_dir, &snapshot, true, error)
                .await;
        }

        Ok(snapshot)
    }

    fn full_state_reservation_bytes(&self, memory_mb: u64) -> Result<u64> {
        let memory = memory_mb
            .checked_mul(1024 * 1024)
            .ok_or_else(|| anyhow::anyhow!("full-state memory reservation overflow"))?;
        let rootfs = self
            .sandbox_rootfs
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("sandbox rootfs has not been prepared"))?;
        let rootfs_bytes = fs::metadata(rootfs.path())?.len();
        memory
            .checked_add(rootfs_bytes)
            .and_then(|bytes| bytes.checked_add(64 * 1024 * 1024))
            .ok_or_else(|| anyhow::anyhow!("full-state checkpoint reservation overflow"))
    }

    async fn retry_full_state_resume(&mut self) -> Result<()> {
        ensure_full_state_host_supported()?;
        if self.process.is_none() {
            bail!(
                "Firecracker sandbox '{}' has no live source process to resume",
                self.name
            );
        }
        let client = FirecrackerClient::new(&self.socket_path);
        match client.get_instance_info().await {
            Ok(instance) if instance.state == "Running" => {}
            Ok(instance) if instance.state == "Paused" => {
                if let Err(resume_error) = client.resume().await {
                    match client.get_instance_info().await {
                        Ok(observed) if observed.state == "Running" => {}
                        Ok(observed) => {
                            return Err(resume_error).context(format!(
                                "failed to retry source resume; observed instance state '{}'",
                                observed.state
                            ));
                        }
                        Err(inspect_error) => {
                            return Err(resume_error).context(format!(
                                "failed to retry source resume and inspect final state: {inspect_error:#}"
                            ));
                        }
                    }
                }
            }
            Ok(instance) => {
                bail!(
                    "cannot retry Firecracker source resume from instance state '{}'",
                    instance.state
                );
            }
            Err(inspect_error) => {
                client.resume().await.with_context(|| {
                    format!(
                        "failed to inspect source state ({inspect_error:#}) and best-effort resume failed"
                    )
                })?;
            }
        }

        self.running = true;
        self.wait_for_agent()
            .await
            .context("source VM resumed but guest agent did not reconnect")?;
        Ok(())
    }

    async fn restore_from(
        &mut self,
        checkpoint_dir: &Path,
        snapshot: &FullStateSnapshot,
    ) -> Result<()> {
        ensure_full_state_host_supported()?;
        if self.running || self.process.is_some() || self.sandbox_rootfs.is_some() {
            bail!(
                "Firecracker sandbox '{}' already has an active runtime",
                self.name
            );
        }
        if snapshot.architecture != std::env::consts::ARCH {
            bail!(
                "Firecracker snapshot architecture mismatch: checkpoint={}, host={}",
                snapshot.architecture,
                std::env::consts::ARCH
            );
        }
        if snapshot.firecracker_version != SUPPORTED_FIRECRACKER_VERSION {
            bail!(
                "full-state restore requires Firecracker {}, checkpoint uses {}",
                SUPPORTED_FIRECRACKER_VERSION,
                snapshot.firecracker_version
            );
        }
        let kernel_release = host_kernel_release()?;
        ensure_clock_realtime_kernel_supported(&kernel_release)?;
        if snapshot.host_kernel_release != kernel_release {
            bail!(
                "Firecracker snapshot host kernel mismatch: checkpoint={}, host={}",
                snapshot.host_kernel_release,
                kernel_release
            );
        }
        let host_identity = host_identity_sha256()?;
        if snapshot.host_identity_sha256 != host_identity {
            bail!("Firecracker snapshot host identity mismatch");
        }
        let cpu_fingerprint = cpu_fingerprint_sha256()?;
        if snapshot.cpu_fingerprint_sha256 != cpu_fingerprint {
            bail!("Firecracker snapshot CPU fingerprint mismatch");
        }
        ensure_guest_kernel_supported(&snapshot.guest_kernel_release)?;

        let firecracker_bin = find_firecracker()?;
        let binary_version = firecracker_binary_version(&firecracker_bin)?;
        if snapshot.firecracker_version != binary_version {
            bail!(
                "Firecracker snapshot VMM mismatch: checkpoint={}, installed={}",
                snapshot.firecracker_version,
                binary_version
            );
        }

        let memory_path = ensure_checkpoint_artifact(checkpoint_dir, MEMORY_FILE)?;
        let vmstate_path = ensure_checkpoint_artifact(checkpoint_dir, VMSTATE_FILE)?;
        let checkpoint_rootfs = ensure_checkpoint_artifact(checkpoint_dir, ROOTFS_FILE)?;

        let store = RootfsCowStore::open_default()?;
        let rootfs = store.prepare(&checkpoint_rootfs).with_context(|| {
            format!(
                "failed to prepare writable rootfs from checkpoint {}",
                checkpoint_dir.display()
            )
        })?;
        eprintln!(
            "[firecracker] restored rootfs COW strategy={:?} path={}",
            rootfs.strategy(),
            rootfs.path().display()
        );
        self.sandbox_rootfs = Some(rootfs);

        let restore = async {
            self.spawn_firecracker(&firecracker_bin)?;
            self.wait_for_socket().await?;
            let client = FirecrackerClient::new(&self.socket_path);
            let api_version = client
                .get_version()
                .await
                .context("failed to inspect restore Firecracker version")?
                .firecracker_version;
            if api_version != snapshot.firecracker_version {
                bail!(
                    "Firecracker restore process version mismatch: checkpoint={}, process={}",
                    snapshot.firecracker_version,
                    api_version
                );
            }

            client
                .load_snapshot(&SnapshotLoadParams {
                    mem_backend: MemoryBackend::file(&memory_path),
                    snapshot_path: vmstate_path.to_string_lossy().into_owned(),
                    resume_vm: false,
                    vsock_override: VsockOverride {
                        uds_path: self.vsock_path.to_string_lossy().into_owned(),
                    },
                    clock_realtime: cfg!(target_arch = "x86_64").then_some(true),
                })
                .await
                .context("failed to load Firecracker full-state checkpoint")?;

            let instance = client
                .get_instance_info()
                .await
                .context("failed to inspect restored Firecracker VM")?;
            if instance.state != "Paused" {
                bail!(
                    "Firecracker loaded snapshot in unexpected state '{}'",
                    instance.state
                );
            }
            client
                .resume()
                .await
                .context("failed to resume restored Firecracker VM")?;
            self.wait_for_agent().await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        match restore {
            Ok(()) => {
                self.running = true;
                Ok(())
            }
            Err(error) => match self.abort_start() {
                Ok(()) => Err(error),
                Err(cleanup_error) => {
                    Err(transition_cleanup_error("restore", &error, cleanup_error))
                }
            },
        }
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

impl Drop for FirecrackerSandbox {
    fn drop(&mut self) {
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
        }
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.vsock_path);
        let _ = std::fs::remove_dir(&self.runtime_dir);
        self.sandbox_rootfs.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_socket_paths_are_unique_per_instance() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let first = FirecrackerSandbox::new("same-name").unwrap();
        let second = FirecrackerSandbox::new("same-name").unwrap();
        assert_ne!(first.api_socket_path(), second.api_socket_path());
        assert_ne!(first.vsock_socket_path(), second.vsock_socket_path());
        assert!(first.api_socket_path().is_absolute());
        assert!(first.vsock_socket_path().is_absolute());
        assert_eq!(
            first.api_socket_path().parent(),
            first.vsock_socket_path().parent()
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(first.api_socket_path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_socket_is_current_user_private_and_symlinks_are_rejected() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        use std::os::unix::net::UnixListener;

        let sandbox = FirecrackerSandbox::new("private-runtime-socket").unwrap();
        let listener = UnixListener::bind(sandbox.api_socket_path()).unwrap();
        secure_runtime_socket(sandbox.api_socket_path()).unwrap();
        let metadata = fs::symlink_metadata(sandbox.api_socket_path()).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        let outside = sandbox.runtime_dir.join("outside");
        fs::write(&outside, b"not a socket").unwrap();
        symlink(&outside, sandbox.vsock_socket_path()).unwrap();
        assert!(secure_runtime_socket(sandbox.vsock_socket_path()).is_err());
        drop(listener);
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn parses_pinned_firecracker_version_output() {
        assert_eq!(
            parse_firecracker_version("Firecracker v1.16.1\n").unwrap(),
            SUPPORTED_FIRECRACKER_VERSION
        );
        assert!(parse_firecracker_version("Firecracker unknown").is_err());
    }

    #[test]
    fn full_state_host_boundary_matches_linux_x86_64_contract() {
        assert_eq!(
            ensure_full_state_host_supported().is_ok(),
            cfg!(all(target_os = "linux", target_arch = "x86_64"))
        );
    }

    #[test]
    fn clock_realtime_requires_linux_5_16_or_newer() {
        assert!(ensure_clock_realtime_kernel_supported("5.15.99").is_err());
        assert!(ensure_clock_realtime_kernel_supported("5.16.0").is_ok());
        assert!(ensure_clock_realtime_kernel_supported("6.18.45-agentkernel").is_ok());
        assert!(ensure_clock_realtime_kernel_supported("not-a-kernel").is_err());
    }

    #[test]
    fn full_state_requires_the_managed_guest_kernel() {
        assert!(ensure_guest_kernel_supported(SUPPORTED_GUEST_KERNEL_RELEASE).is_ok());
        assert!(ensure_guest_kernel_supported("6.18.45").is_err());
        assert!(ensure_guest_kernel_supported("6.12.0-agentkernel").is_err());
    }

    #[test]
    fn cpu_identity_is_stable_across_flag_order_and_ignores_other_cpus() {
        let first = "processor : 0\nvendor_id : GenuineIntel\ncpu family : 6\nmodel : 143\nmodel name : Test CPU\nstepping : 8\nmicrocode : 0x1\nflags : vmx sse aes\n\nprocessor : 1\nvendor_id : ignored\n";
        let reordered = "processor : 0\nvendor_id : GenuineIntel\ncpu family : 6\nmodel : 143\nmodel name : Test CPU\nstepping : 8\nmicrocode : 0x1\nflags : aes vmx sse\n";
        assert_eq!(
            canonical_cpu_identity(first).unwrap(),
            canonical_cpu_identity(reordered).unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_source_resume_retains_complete_or_partial_recovery_artifacts() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        async fn reject_resume(listener: UnixListener) -> String {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let bytes = stream.read(&mut request).await.unwrap();
            let body = br#"{"fault_message":"resume rejected"}"#;
            let headers = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            String::from_utf8(request[..bytes].to_vec()).unwrap()
        }

        for artifacts_complete in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let socket = temp.path().join("firecracker.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let request = tokio::spawn(reject_resume(listener));
            let checkpoint = temp.path().join("checkpoint");
            fs::create_dir(&checkpoint).unwrap();
            fs::write(checkpoint.join(MEMORY_FILE), b"partial memory").unwrap();
            if artifacts_complete {
                fs::write(checkpoint.join(VMSTATE_FILE), b"vmstate").unwrap();
                fs::write(checkpoint.join(ROOTFS_FILE), b"rootfs").unwrap();
            }

            let snapshot = FullStateSnapshot {
                firecracker_version: SUPPORTED_FIRECRACKER_VERSION.to_string(),
                architecture: "x86_64".to_string(),
                host_kernel_release: "6.18.45-agentkernel".to_string(),
                host_identity_sha256: "host-id".to_string(),
                cpu_fingerprint_sha256: "cpu-id".to_string(),
                guest_kernel_release: SUPPORTED_GUEST_KERNEL_RELEASE.to_string(),
            };
            let mut sandbox = FirecrackerSandbox::new("resume-recovery-test").unwrap();
            let error = sandbox
                .resume_after_snapshot_failure(
                    &FirecrackerClient::new(&socket),
                    &checkpoint,
                    &snapshot,
                    artifacts_complete,
                    anyhow::anyhow!("checkpoint operation failed"),
                )
                .await
                .unwrap_err();
            let request = request.await.unwrap();
            assert!(request.starts_with("PATCH /vm HTTP/1.1"));
            assert!(request.contains(r#"{"state":"Resumed"}"#));

            let recovery = error
                .downcast_ref::<FullStatePauseError>()
                .expect("resume failure is downcastable");
            assert!(recovery.source_resume_failed);
            assert_eq!(recovery.artifacts_complete, artifacts_complete);
            assert_eq!(recovery.snapshot, snapshot);
            assert!(
                recovery
                    .operation_error
                    .contains("checkpoint operation failed")
            );
            assert!(
                recovery
                    .resume_error
                    .as_deref()
                    .unwrap()
                    .contains("resume rejected")
            );
            assert!(checkpoint.join(MEMORY_FILE).is_file());
            assert_eq!(checkpoint.join(VMSTATE_FILE).is_file(), artifacts_complete);
            assert_eq!(checkpoint.join(ROOTFS_FILE).is_file(), artifacts_complete);
        }
    }

    #[test]
    fn accepted_resume_without_guest_health_retains_complete_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let checkpoint = temp.path().join("checkpoint");
        fs::create_dir(&checkpoint).unwrap();
        fs::write(checkpoint.join(MEMORY_FILE), b"memory").unwrap();
        fs::write(checkpoint.join(VMSTATE_FILE), b"vmstate").unwrap();
        fs::write(checkpoint.join(ROOTFS_FILE), b"rootfs").unwrap();
        let snapshot = FullStateSnapshot {
            firecracker_version: SUPPORTED_FIRECRACKER_VERSION.to_string(),
            architecture: "x86_64".to_string(),
            host_kernel_release: "6.18.45-agentkernel".to_string(),
            host_identity_sha256: "host-id".to_string(),
            cpu_fingerprint_sha256: "cpu-id".to_string(),
            guest_kernel_release: SUPPORTED_GUEST_KERNEL_RELEASE.to_string(),
        };
        let mut sandbox = FirecrackerSandbox::new("accepted-resume-unhealthy").unwrap();
        sandbox.running = true;

        let error = sandbox
            .finish_resumed_snapshot_failure(
                &checkpoint,
                &snapshot,
                true,
                anyhow::anyhow!("snapshot failed after pause"),
                Err(anyhow::anyhow!("guest agent did not reconnect")),
            )
            .unwrap_err();

        let recovery = error
            .downcast_ref::<FullStatePauseError>()
            .expect("failed health confirmation remains a typed pause recovery");
        assert!(recovery.source_resume_failed);
        assert!(recovery.artifacts_complete);
        assert!(
            recovery
                .resume_error
                .as_deref()
                .unwrap()
                .contains("health was not confirmed")
        );
        assert!(checkpoint.join(MEMORY_FILE).is_file());
        assert!(checkpoint.join(VMSTATE_FILE).is_file());
        assert!(checkpoint.join(ROOTFS_FILE).is_file());
    }

    #[test]
    fn restore_cleanup_preserves_typed_termination_uncertainty() {
        let restore_error = anyhow::anyhow!("guest agent failed after snapshot load and resume");
        let error = transition_cleanup_error(
            "restore",
            &restore_error,
            FullStateTerminationError {
                process_may_be_running: true,
                detail: "kill succeeded but wait/reap was not confirmed".to_string(),
            }
            .into(),
        );

        let termination = error
            .downcast_ref::<FullStateTerminationError>()
            .expect("restore cleanup uncertainty remains downcastable");
        assert!(termination.process_may_be_running);
        assert!(error.to_string().contains("cleanup could not prove"));
    }

    #[tokio::test]
    async fn stop_finalizes_state_when_rootfs_cleanup_fails() {
        let temp = tempfile::tempdir().unwrap();
        let store = RootfsCowStore::with_capabilities(
            temp.path().join("cow"),
            crate::cow::RootfsCowCapabilities {
                reflink_copy: false,
                overlayfs_available: false,
            },
        )
        .unwrap();
        let base = store.root().join("base.ext4");
        std::fs::write(&base, b"rootfs contents").unwrap();
        let rootfs = store.prepare(&base).unwrap();
        let artifact_dir = rootfs.path().parent().unwrap().to_path_buf();
        std::fs::remove_dir_all(&artifact_dir).unwrap();

        let name = format!("stop-cleanup-test-{}", std::process::id());
        let mut sandbox = FirecrackerSandbox::new(&name).unwrap();
        sandbox.sandbox_rootfs = Some(rootfs);
        sandbox.running = true;

        let error = sandbox.stop().await.unwrap_err();
        let termination = error
            .downcast_ref::<FullStateTerminationError>()
            .expect("cleanup-only stop errors retain a typed termination outcome");
        assert!(!termination.process_may_be_running);
        assert!(!sandbox.running);
        assert!(sandbox.sandbox_rootfs.is_none());
        assert!(sandbox.process.is_none());
    }
}
