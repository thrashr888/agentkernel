//! Cloud Hypervisor backend implementing the Sandbox trait.
//!
//! This is a modern, Rust-based VMM optimized for cloud workloads.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tokio::time::{Duration, sleep};

use super::{BackendReadiness, BackendType, ExecOptions, ExecResult, Sandbox, SandboxConfig};
use crate::cloud_hypervisor_client::{
    CloudHypervisorClient, CpusConfig, DiskConfig, MemoryConfig, PayloadConfig, VmConfig,
    VsockConfig,
};
use crate::cow::{RootfsCow, RootfsCowStore};
use crate::vsock::VsockClient;

/// This backend currently supports Linux x86_64/KVM only, not MSHV or macOS.
pub fn cloud_hypervisor_available() -> bool {
    cloud_hypervisor_readiness().usable
}

pub fn cloud_hypervisor_readiness() -> BackendReadiness {
    let configured = find_cloud_hypervisor().is_ok();
    let reason = if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("Cloud Hypervisor requires Linux x86_64 with KVM")
    } else if !configured {
        Some("Cloud Hypervisor binary is not installed or executable")
    } else if fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_err()
    {
        Some("Cloud Hypervisor requires read/write access to /dev/kvm")
    } else if find_kernel().is_err() {
        Some(
            "Set CLOUD_HYPERVISOR_KERNEL to a compatible PVH ELF or bzImage kernel with virtio PCI support",
        )
    } else if find_rootfs("base").is_err() {
        Some("Cloud Hypervisor base rootfs is missing; run agentkernel setup")
    } else {
        None
    };
    BackendReadiness {
        configured,
        usable: reason.is_none(),
        reason: reason.unwrap_or("ready").to_string(),
    }
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn find_cloud_hypervisor() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CLOUD_HYPERVISOR_BIN") {
        let path = PathBuf::from(path);
        if !executable(&path) {
            bail!(
                "CLOUD_HYPERVISOR_BIN is not an executable file: {}",
                path.display()
            );
        }
        return fs::canonicalize(path).context("Failed to resolve Cloud Hypervisor binary");
    }
    let mut locations = vec![data_dir().join("bin/cloud-hypervisor")];
    if let Some(path) = std::env::var_os("PATH") {
        locations.extend(std::env::split_paths(&path).map(|p| p.join("cloud-hypervisor")));
    }
    locations.extend([
        PathBuf::from("/usr/local/bin/cloud-hypervisor"),
        PathBuf::from("/usr/bin/cloud-hypervisor"),
    ]);
    for path in locations {
        if executable(&path) {
            return fs::canonicalize(path).context("Failed to resolve Cloud Hypervisor binary");
        }
    }
    bail!("Cloud Hypervisor binary not found")
}

fn data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share/agentkernel"))
        .unwrap_or_else(|| PathBuf::from("/usr/local/share/agentkernel"))
}

fn find_kernel() -> Result<PathBuf> {
    // The Firecracker kernel is MMIO-only, and may lack PVH/PCI support. Never
    // silently reuse it or download/relabel a different kernel on the user's behalf.
    let path = std::env::var_os("CLOUD_HYPERVISOR_KERNEL")
        .map(PathBuf::from)
        .context("Set CLOUD_HYPERVISOR_KERNEL to a compatible PVH ELF or bzImage kernel")?;
    if !path.is_file() {
        bail!("CLOUD_HYPERVISOR_KERNEL is not a file: {}", path.display());
    }
    fs::canonicalize(path).context("Failed to resolve Cloud Hypervisor kernel")
}

fn rootfs_name(image: &str) -> Result<&'static str> {
    match image {
        "python:3.12" | "python" => Ok("python.ext4"),
        "node:20" | "node" => Ok("node.ext4"),
        "rust:1.85" | "rust" => Ok("rust.ext4"),
        "golang:1.24" | "go" => Ok("go.ext4"),
        "alpine:3.24" | "alpine" | "base" => Ok("base.ext4"),
        _ => bail!(
            "Cloud Hypervisor does not support image '{image}'; use a setup-provided base, python, node, rust, or go runtime"
        ),
    }
}

fn find_rootfs(image: &str) -> Result<PathBuf> {
    let rootfs = data_dir().join("images/rootfs").join(rootfs_name(image)?);
    if rootfs.is_file() {
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
    runtime_dir: tempfile::TempDir,
    env: HashMap<String, String>,
    work_dir: Option<String>,
    sandbox_rootfs: Option<RootfsCow>,
}

impl CloudHypervisorSandbox {
    /// Create a new Cloud Hypervisor sandbox
    pub fn new(name: &str) -> Result<Self> {
        // Request 0700 at creation time, before any sockets or logs exist.
        // Use /tmp explicitly to keep Unix socket paths below sun_path limits.
        let mut builder = tempfile::Builder::new();
        builder.prefix("ak-ch-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let runtime_dir = builder
            .tempdir_in("/tmp")
            .context("Failed to create private Cloud Hypervisor runtime directory")?;
        let socket_path = runtime_dir.path().join("api.sock");
        let vsock_path = runtime_dir.path().join("vsock.sock");
        Ok(Self {
            name: name.to_string(),
            running: false,
            process: None,
            socket_path,
            vsock_path,
            runtime_dir,
            env: HashMap::new(),
            work_dir: None,
            sandbox_rootfs: None,
        })
    }

    fn validate_config(config: &SandboxConfig) -> Result<()> {
        if config.vcpus == 0 || config.memory_mb == 0 {
            bail!("Cloud Hypervisor vcpus and memory_mb must be positive");
        }
        config
            .memory_mb
            .checked_mul(1024 * 1024)
            .context("Cloud Hypervisor memory size overflow")?;
        if config.network || config.managed_network.is_some() || !config.ports.is_empty() {
            bail!(
                "Cloud Hypervisor networking and port mappings are not implemented; disable network access"
            );
        }
        if config.mount_cwd || config.mount_home || !config.volumes.is_empty() {
            bail!(
                "Cloud Hypervisor host mounts are not implemented; disable mount_cwd and mount_home"
            );
        }
        if config.ssh.is_some() {
            bail!("Cloud Hypervisor SSH is not implemented");
        }
        if config.read_only && !config.files.is_empty() {
            bail!("Cannot inject files into a read-only Cloud Hypervisor rootfs");
        }
        Ok(())
    }

    fn vm_config(&self, config: &SandboxConfig, kernel: &Path, rootfs: &Path) -> Result<VmConfig> {
        Self::validate_config(config)?;
        let root_mode = if config.read_only { "ro" } else { "rw" };
        Ok(VmConfig {
            payload: PayloadConfig {
                kernel: Some(kernel.to_string_lossy().into_owned()),
                // Cloud Hypervisor uses virtio PCI, unlike Firecracker's MMIO.
                cmdline: Some(format!(
                    "console=hvc0 reboot=k panic=1 nomodules root=/dev/vda {root_mode} init=/init"
                )),
                initramfs: None,
            },
            cpus: CpusConfig {
                boot_vcpus: config.vcpus,
                max_vcpus: config.vcpus,
                kvm_hyperv: Some(true),
            },
            memory: MemoryConfig {
                size: config
                    .memory_mb
                    .checked_mul(1024 * 1024)
                    .context("Cloud Hypervisor memory size overflow")?,
                shared: None,
            },
            disks: Some(vec![DiskConfig {
                path: rootfs.to_string_lossy().into_owned(),
                readonly: Some(config.read_only),
                direct: Some(false),
            }]),
            net: None,
            vsock: Some(VsockConfig {
                cid: 3,
                socket: self.vsock_path.to_string_lossy().into_owned(),
            }),
        })
    }

    fn check_child(&mut self) -> Result<()> {
        if let Some(child) = &mut self.process
            && let Some(status) = child
                .try_wait()
                .context("Failed to inspect Cloud Hypervisor process")?
        {
            bail!("Cloud Hypervisor exited before becoming ready ({status})");
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if let Some(child) = self.process.as_mut() {
            if child
                .try_wait()
                .context("Failed to inspect Cloud Hypervisor process")?
                .is_none()
            {
                child
                    .kill()
                    .context("Failed to terminate Cloud Hypervisor; runtime remains owned")?;
            }
            child
                .wait()
                .context("Failed to reap Cloud Hypervisor; runtime remains owned")?;
        }
        self.process = None;
        self.running = false;
        self.sandbox_rootfs = None;
        self.env.clear();
        self.work_dir = None;
        for name in ["api.sock", "vsock.sock", "vmm.log"] {
            let path = self.runtime_dir.path().join(name);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("Failed to clean Cloud Hypervisor runtime files"),
            }
        }
        Ok(())
    }

    fn spawn_vmm(&mut self, binary: &Path) -> Result<()> {
        let log_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.runtime_dir.path().join("vmm.log"))
            .context("Failed to create private Cloud Hypervisor log")?;
        let mut command = Command::new(binary);
        command
            .arg("--api-socket")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(log_file.try_clone()?)
            .stderr(log_file);
        let rootfs = self
            .sandbox_rootfs
            .as_ref()
            .context("Cloud Hypervisor rootfs was not prepared")?;
        // Keep the disk leased even if AgentKernel dies before the VMM.
        rootfs.inherit_lease_in_child(&mut command);
        self.process = Some(
            command
                .spawn()
                .context("Failed to spawn Cloud Hypervisor")?,
        );
        self.running = true;
        Ok(())
    }

    fn log_tail(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let mut bytes = Vec::new();
        if let Ok(mut file) = fs::File::open(self.runtime_dir.path().join("vmm.log")) {
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = file.seek(SeekFrom::Start(len.saturating_sub(4096)));
            let _ = file.take(4096).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn start_with_assets(
        &mut self,
        config: &SandboxConfig,
        binary: &Path,
        kernel: &Path,
        base_rootfs: &Path,
        store: &RootfsCowStore,
    ) -> Result<()> {
        let rootfs = store.prepare(base_rootfs)?;
        let vm_config = self.vm_config(config, kernel, rootfs.path())?;
        self.sandbox_rootfs = Some(rootfs);
        let result = async {
            self.spawn_vmm(binary)?;
            self.wait_for_socket().await?;
            let client = CloudHypervisorClient::new(&self.socket_path);
            let version = client.get_version().await?;
            if version.version.is_empty() {
                bail!("Cloud Hypervisor returned an empty version");
            }
            client.create_vm(&vm_config).await?;
            client.boot_vm().await?;
            tokio::time::timeout(Duration::from_secs(15), self.wait_for_agent())
                .await
                .context("Timeout waiting for Cloud Hypervisor guest agent")??;
            self.env = config.env.iter().cloned().collect();
            self.work_dir = config.container_work_dir.clone();
            Ok(())
        }
        .await;
        if let Err(error) = result {
            let tail = self.log_tail();
            let error = if tail.is_empty() {
                error
            } else {
                error.context(format!("Cloud Hypervisor log tail: {tail}"))
            };
            if let Err(cleanup_error) = self.cleanup() {
                return Err(cleanup_error).context(format!(
                    "Cloud Hypervisor startup failed ({error:#}) and cleanup could not finish"
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    async fn wait_for_socket(&mut self) -> Result<()> {
        for _ in 0..100 {
            self.check_child()?;
            if self.socket_path.exists() {
                return Ok(());
            }
            sleep(Duration::from_millis(50)).await;
        }
        bail!("Timeout waiting for Cloud Hypervisor API socket")
    }

    async fn wait_for_agent(&mut self) -> Result<()> {
        for _ in 0..100 {
            self.check_child()?;
            if self.vsock_path.exists() {
                // Try to connect to guest agent
                let client = VsockClient::for_firecracker(&self.vsock_path).with_timeout(1);
                if client.ping().await.unwrap_or(false) {
                    return Ok(());
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
        if self.running || self.process.is_some() {
            bail!("Sandbox already running or still owns a VMM process");
        }
        Self::validate_config(config)?;
        let readiness = cloud_hypervisor_readiness();
        if !readiness.usable {
            bail!("{}", readiness.reason);
        }
        self.start_with_assets(
            config,
            &find_cloud_hypervisor()?,
            &find_kernel()?,
            &find_rootfs(&config.image)?,
            &RootfsCowStore::open_default()?,
        )
        .await
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_options(cmd, &ExecOptions::default()).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        self.exec_with_options(
            cmd,
            &ExecOptions {
                env: env.to_vec(),
                ..Default::default()
            },
        )
        .await
    }

    async fn exec_with_options(&mut self, cmd: &[&str], opts: &ExecOptions) -> Result<ExecResult> {
        if !self.running {
            bail!("Sandbox not running");
        }
        self.check_child()?;
        if opts.user.is_some() {
            bail!("Cloud Hypervisor exec user selection is not implemented");
        }
        let mut env = self.env.clone();
        for entry in &opts.env {
            let (key, value) = entry
                .split_once('=')
                .context("Environment entries must have KEY=VALUE format")?;
            env.insert(key.to_string(), value.to_string());
        }
        let client = VsockClient::for_firecracker(&self.vsock_path);
        let command = cmd.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let result = client
            .run_command_with_env(
                &command,
                opts.workdir.as_deref().or(self.work_dir.as_deref()),
                Some(env),
            )
            .await?;
        Ok(ExecResult {
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }

    async fn stop(&mut self) -> Result<()> {
        // Kill and reap directly: shutdown API failure must not strand the VMM.
        self.cleanup()
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

impl Drop for CloudHypervisorSandbox {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn config() -> SandboxConfig {
        SandboxConfig {
            network: false,
            ..Default::default()
        }
    }

    #[test]
    fn vm_config_preserves_resources_and_read_only_policy() {
        let sandbox = CloudHypervisorSandbox::new("config-test").unwrap();
        for read_only in [false, true] {
            let config = SandboxConfig {
                memory_mb: 128,
                vcpus: 2,
                read_only,
                ..config()
            };
            let vm = sandbox
                .vm_config(&config, Path::new("/kernel"), Path::new("/rootfs"))
                .unwrap();
            assert_eq!(vm.memory.size, 128 * 1024 * 1024);
            assert_eq!(vm.cpus.boot_vcpus, 2);
            assert_eq!(vm.cpus.max_vcpus, 2);
            assert_eq!(vm.cpus.kvm_hyperv, Some(true));
            assert_eq!(vm.disks.unwrap()[0].readonly, Some(read_only));
            let cmdline = vm.payload.cmdline.unwrap();
            assert!(cmdline.contains("init=/init"));
            assert!(!cmdline.contains("pci=off"));
            assert!(cmdline.contains(if read_only { " ro " } else { " rw " }));
            assert!(vm.net.is_none());
        }
    }

    #[test]
    fn unsupported_requests_fail_closed() {
        let mut c = config();
        c.network = true;
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.mount_cwd = true;
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.mount_home = true;
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.volumes.push("/tmp:/tmp".into());
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.ssh = Some(Default::default());
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.memory_mb = u64::MAX;
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.vcpus = 0;
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        c = config();
        c.memory_mb = 0;
        assert!(CloudHypervisorSandbox::validate_config(&c).is_err());
        assert!(rootfs_name("my-custom-app:latest").is_err());
        assert_eq!(rootfs_name("alpine:3.24").unwrap(), "base.ext4");
    }

    #[test]
    fn same_names_get_private_unique_runtime_directories() {
        use std::os::unix::fs::PermissionsExt;
        let a = CloudHypervisorSandbox::new("../../same-name").unwrap();
        let b = CloudHypervisorSandbox::new("../../same-name").unwrap();
        assert_ne!(a.socket_path, b.socket_path);
        assert_ne!(a.vsock_path, b.vsock_path);
        assert_eq!(
            fs::metadata(a.runtime_dir.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(a.socket_path.parent(), Some(a.runtime_dir.path()));
        let path = a.runtime_dir.path().to_path_buf();
        drop(a);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn failed_spawn_cleans_cow_disk_and_runtime_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, vec![0u8; 4096]).unwrap();
        let store = RootfsCowStore::new(tmp.path().join("cow")).unwrap();
        let mut sandbox = CloudHypervisorSandbox::new("spawn-failure").unwrap();
        let error = sandbox
            .start_with_assets(
                &config(),
                Path::new("/nonexistent/ak-ch-test"),
                Path::new("/kernel"),
                &base,
                &store,
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("spawn"));
        assert!(!sandbox.is_running());
        assert!(sandbox.process.is_none());
        assert!(sandbox.sandbox_rootfs.is_none());
        assert!(!sandbox.runtime_dir.path().join("vmm.log").exists());
        // A retry does not fail because of a stale create_new log file.
        assert!(
            sandbox
                .start_with_assets(
                    &config(),
                    Path::new("/nonexistent/ak-ch-test"),
                    Path::new("/kernel"),
                    &base,
                    &store
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn early_process_exit_is_reported_and_reaped() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, vec![0u8; 4096]).unwrap();
        let store = RootfsCowStore::new(tmp.path().join("cow")).unwrap();
        let mut sandbox = CloudHypervisorSandbox::new("early-exit").unwrap();
        let error = sandbox
            .start_with_assets(
                &config(),
                Path::new("false"),
                Path::new("/kernel"),
                &base,
                &store,
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("exited before becoming ready"));
        assert!(!sandbox.is_running());
        assert!(sandbox.process.is_none());
        assert!(sandbox.sandbox_rootfs.is_none());
    }

    #[tokio::test]
    async fn stop_and_drop_kill_and_reap_owned_processes() {
        let mut sandbox = CloudHypervisorSandbox::new("stop").unwrap();
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        sandbox.process = Some(child);
        sandbox.running = true;
        sandbox.stop().await.unwrap();
        assert!(!sandbox.is_running());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        sandbox.stop().await.unwrap();
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        sandbox.process = Some(child);
        sandbox.running = true;
        drop(sandbox);
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }

    #[tokio::test]
    async fn rejecting_config_happens_before_process_creation() {
        let mut sandbox = CloudHypervisorSandbox::new("reject").unwrap();
        assert!(
            sandbox
                .start(&SandboxConfig::default())
                .await
                .unwrap_err()
                .to_string()
                .contains("network")
        );
        assert!(sandbox.process.is_none());
        assert!(sandbox.sandbox_rootfs.is_none());
    }

    #[test]
    fn discovery_advertises_only_implemented_capabilities() {
        let caps = super::super::backend_capabilities(BackendType::CloudHypervisor);
        assert!(!caps.mount_cwd && !caps.mount_home && !caps.host_volumes);
        assert!(!caps.ssh && !caps.endpoints && !caps.attach);
        assert!(!caps.snapshots && !caps.resume && !caps.full_state_pause_resume);
        assert!(caps.secret_files);
        assert_eq!(
            super::super::backend_available(BackendType::CloudHypervisor),
            cloud_hypervisor_readiness().usable
        );
    }

    #[tokio::test]
    async fn api_rejection_kills_process_and_cleans_disk() {
        use http_body_util::Full;
        use hyper::body::Bytes;
        use hyper::{Response, service::service_fn};
        use hyper_util::rt::TokioIo;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{Arc, Mutex};
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("fake-vmm");
        fs::write(&binary, "#!/bin/sh\nexec sleep 60\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, vec![0u8; 4096]).unwrap();
        let store = RootfsCowStore::new(tmp.path().join("cow")).unwrap();
        let mut sandbox = CloudHypervisorSandbox::new("api-rejection").unwrap();
        let listener = tokio::net::UnixListener::bind(&sandbox.socket_path).unwrap();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let captured = paths.clone();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let paths = captured.clone();
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(stream),
                        service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                            let path = req.uri().path().to_string();
                            paths.lock().unwrap().push(path.clone());
                            async move {
                                let (status, body) = if path == "/api/v1/vmm.ping" {
                                    (200, r#"{"version":"v53.0"}"#)
                                } else {
                                    (400, r#"{"error":"reject test VM"}"#)
                                };
                                Ok::<_, std::convert::Infallible>(
                                    Response::builder()
                                        .status(status)
                                        .body(Full::new(Bytes::from(body)))
                                        .unwrap(),
                                )
                            }
                        }),
                    )
                    .await
                    .unwrap();
            }
        });
        let result = sandbox
            .start_with_assets(&config(), &binary, Path::new("/kernel"), &base, &store)
            .await;
        assert!(result.unwrap_err().to_string().contains("reject test VM"));
        server.await.unwrap();
        assert_eq!(
            *paths.lock().unwrap(),
            ["/api/v1/vmm.ping", "/api/v1/vm.create"]
        );
        assert!(!sandbox.running);
        assert!(sandbox.process.is_none());
        assert!(sandbox.sandbox_rootfs.is_none());
        assert!(!sandbox.socket_path.exists());
    }

    #[tokio::test]
    async fn false_agent_ping_does_not_mark_ready() {
        let mut sandbox = CloudHypervisorSandbox::new("false-ping").unwrap();
        let listener = tokio::net::UnixListener::bind(&sandbox.vsock_path).unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                drop(stream); // VsockClient::ping returns Ok(false) on handshake failure.
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(200), sandbox.wait_for_agent())
                .await
                .is_err()
        );
        server.abort();
    }
    #[test]
    fn spawned_vmm_inherits_rootfs_lease() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("fake-vmm");
        fs::write(&binary, "#!/bin/sh\nexec sleep 60\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let base = tmp.path().join("base.ext4");
        fs::write(&base, vec![0u8; 4096]).unwrap();
        let store = RootfsCowStore::new(tmp.path().join("cow")).unwrap();
        let mut rootfs = store.prepare(&base).unwrap();
        rootfs.preserve_for_lifecycle().unwrap();
        let reference = rootfs.reference();
        let mut sandbox = CloudHypervisorSandbox::new("lease-test").unwrap();
        sandbox.sandbox_rootfs = Some(rootfs);
        sandbox.spawn_vmm(&binary).unwrap();
        drop(sandbox.sandbox_rootfs.take());
        assert!(store.adopt(&reference).is_err());
        sandbox.cleanup().unwrap();
        store
            .adopt(&reference)
            .unwrap()
            .discard_persisted()
            .unwrap();
    }
    #[test]
    fn existing_log_symlink_is_not_followed_or_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("do-not-truncate");
        fs::write(&target, "preserve me").unwrap();
        let mut sandbox = CloudHypervisorSandbox::new("log-symlink").unwrap();
        std::os::unix::fs::symlink(&target, sandbox.runtime_dir.path().join("vmm.log")).unwrap();
        assert!(sandbox.spawn_vmm(Path::new("false")).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "preserve me");
        assert!(sandbox.process.is_none());
    }
}
