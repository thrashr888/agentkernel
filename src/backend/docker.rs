//! Docker/Podman container backend implementing the Sandbox trait.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

use super::{
    BackendType, ExecOptions, ExecResult, ManagedNetworkConfig, ManagedNetworkLease,
    NetworkAllocator, Sandbox, SandboxConfig,
};

const MANAGED_NETWORK_LABEL: &str = "com.agentkernel.managed";
const MANAGED_NETWORK_NAME_LABEL: &str = "com.agentkernel.network";

/// Container runtime to use
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntime {
    Docker,
    Podman,
}

impl ContainerRuntime {
    /// Get the command name for this runtime
    pub fn cmd(&self) -> &'static str {
        match self {
            ContainerRuntime::Docker => "docker",
            ContainerRuntime::Podman => "podman",
        }
    }

    /// Convert to BackendType
    pub fn to_backend_type(self) -> BackendType {
        match self {
            ContainerRuntime::Docker => BackendType::Docker,
            ContainerRuntime::Podman => BackendType::Podman,
        }
    }
}

/// Check if Docker is available
pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if Podman is available
pub fn podman_available() -> bool {
    Command::new("podman")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the IP address of a running container by name, using the selected
/// runtime's inspect format.
pub fn get_container_ip_with_runtime(
    runtime: ContainerRuntime,
    container_name: &str,
) -> Option<String> {
    let output = Command::new(runtime.cmd())
        .args([
            "inspect",
            "-f",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
            container_name,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if ip.is_empty() { None } else { Some(ip) }
}

/// Get the IP address of a running Docker container by name.
pub fn get_container_ip(container_name: &str) -> Option<String> {
    get_container_ip_with_runtime(ContainerRuntime::Docker, container_name)
}

fn allocation_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share/agentkernel"))
        .unwrap_or_else(|| PathBuf::from("/tmp/agentkernel"))
}

#[derive(Debug, PartialEq, Eq)]
struct InspectedNetwork {
    driver: String,
    subnet: Option<String>,
    gateway: Option<String>,
    managed: bool,
    managed_name: Option<String>,
    container_ips: Vec<String>,
}

fn first_string(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| {
            value
                .and_then(|value| value.get(*key))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

fn first_object(value: &Value) -> Option<&Value> {
    value
        .as_array()
        .and_then(|values| values.first())
        .or_else(|| value.as_object().map(|_| value))
}

fn collect_container_ips(value: &Value, ips: &mut Vec<String>) {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                let key = key.to_ascii_lowercase();
                if matches!(
                    key.as_str(),
                    "ip" | "ip_address" | "ipaddress" | "ipv4address"
                ) && let Some(ip) = value.as_str()
                {
                    let ip = ip.split('/').next().unwrap_or(ip).to_string();
                    if !ip.is_empty() && !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
                collect_container_ips(value, ips);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_container_ips(value, ips);
            }
        }
        _ => {}
    }
}

fn parse_network_inspection(runtime: ContainerRuntime, output: &[u8]) -> Result<InspectedNetwork> {
    let networks: Vec<Value> = serde_json::from_slice(output)
        .context("container runtime returned invalid network inspection JSON")?;
    let network = networks
        .first()
        .ok_or_else(|| anyhow::anyhow!("container runtime returned no network details"))?;

    let (driver, ipam, containers, labels) = match runtime {
        ContainerRuntime::Docker => (
            first_string(Some(network), &["Driver"]),
            network
                .get("IPAM")
                .and_then(|value| value.get("Config"))
                .and_then(first_object),
            network.get("Containers"),
            network.get("Labels"),
        ),
        ContainerRuntime::Podman => (
            first_string(Some(network), &["driver", "Driver"]),
            network
                .get("subnets")
                .or_else(|| network.get("Subnets"))
                .and_then(first_object),
            network
                .get("containers")
                .or_else(|| network.get("Containers")),
            network.get("labels").or_else(|| network.get("Labels")),
        ),
    };
    let mut container_ips = Vec::new();
    if let Some(containers) = containers {
        collect_container_ips(containers, &mut container_ips);
    }
    Ok(InspectedNetwork {
        driver: driver.unwrap_or_default(),
        subnet: first_string(ipam, &["Subnet", "subnet"]),
        gateway: first_string(ipam, &["Gateway", "gateway"]),
        managed: first_string(labels, &[MANAGED_NETWORK_LABEL]).as_deref() == Some("true"),
        managed_name: first_string(labels, &[MANAGED_NETWORK_NAME_LABEL]),
        container_ips,
    })
}

fn validate_inspected_network(
    config: &ManagedNetworkConfig,
    inspected: &InspectedNetwork,
) -> Result<()> {
    if !inspected.managed || inspected.managed_name.as_deref() != Some(config.name.as_str()) {
        bail!(
            "managed network '{}' exists but is not owned by AgentKernel",
            config.name
        );
    }
    if inspected.driver != "bridge" {
        bail!(
            "managed network '{}' uses driver '{}', expected bridge",
            config.name,
            inspected.driver
        );
    }
    if inspected.subnet.as_deref() != Some(config.subnet.as_str()) {
        bail!(
            "managed network '{}' has subnet {:?}, expected {}",
            config.name,
            inspected.subnet,
            config.subnet
        );
    }
    if inspected.gateway.as_deref() != Some(config.effective_gateway()?.as_str()) {
        bail!("managed network '{}' has a different gateway", config.name);
    }
    Ok(())
}

fn managed_network_create_args(config: &ManagedNetworkConfig) -> Result<Vec<String>> {
    config.validate()?;
    let args = vec![
        "network".to_string(),
        "create".to_string(),
        "--driver".to_string(),
        "bridge".to_string(),
        "--subnet".to_string(),
        config.subnet.clone(),
        "--label".to_string(),
        format!("{}=true", MANAGED_NETWORK_LABEL),
        "--label".to_string(),
        format!("{}={}", MANAGED_NETWORK_NAME_LABEL, config.name),
        "--gateway".to_string(),
        config.effective_gateway()?,
        config.name.clone(),
    ];
    Ok(args)
}

fn managed_network_container_args(config: &ManagedNetworkConfig, ip: &str) -> Vec<String> {
    let mut args = vec![
        "--network".to_string(),
        config.name.clone(),
        "--ip".to_string(),
        ip.to_string(),
    ];
    for dns in &config.dns {
        args.extend(["--dns".to_string(), dns.clone()]);
    }
    args
}

fn ensure_managed_network(runtime: ContainerRuntime, config: &ManagedNetworkConfig) -> Result<()> {
    config.validate()?;
    let cmd = runtime.cmd();
    let inspect = Command::new(cmd)
        .args(["network", "inspect", &config.name])
        .output()
        .context("failed to inspect managed container network")?;
    if inspect.status.success() {
        let inspected = parse_network_inspection(runtime, &inspect.stdout)?;
        validate_inspected_network(config, &inspected)?;
        return Ok(());
    }

    let args = managed_network_create_args(config)?;
    let output = Command::new(cmd)
        .args(&args)
        .output()
        .context("failed to create managed container network")?;
    if output.status.success() {
        return Ok(());
    }
    // Another process may have won the create race. Re-inspect and apply the
    // same compatibility checks instead of blindly using a conflicting bridge.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let retry = Command::new(cmd)
        .args(["network", "inspect", &config.name])
        .output()
        .context("failed to inspect network after create race")?;
    if retry.status.success() {
        let inspected = parse_network_inspection(runtime, &retry.stdout)?;
        if validate_inspected_network(config, &inspected).is_ok() {
            return Ok(());
        }
    }
    bail!(
        "failed to create managed network '{}': {}",
        config.name,
        stderr.trim()
    )
}

fn network_ip_is_in_use(
    runtime: ContainerRuntime,
    network: &ManagedNetworkConfig,
    ip: &str,
) -> Result<bool> {
    let output = Command::new(runtime.cmd())
        .args(["network", "inspect", &network.name])
        .output()
        .context("failed to inspect managed network leases")?;
    if !output.status.success() {
        bail!(
            "failed to inspect managed network '{}' while checking address ownership",
            network.name
        );
    }
    let inspected = parse_network_inspection(runtime, &output.stdout)?;
    Ok(inspected.container_ips.iter().any(|address| address == ip))
}

/// Detect the best available container runtime
pub fn detect_container_runtime() -> Option<ContainerRuntime> {
    if podman_available() {
        Some(ContainerRuntime::Podman)
    } else if docker_available() {
        Some(ContainerRuntime::Docker)
    } else {
        None
    }
}

/// Docker/Podman container sandbox
pub struct DockerSandbox {
    name: String,
    runtime: ContainerRuntime,
    container_id: Option<String>,
    running: bool,
    /// If true, don't clean up container in Drop (for persistent sandboxes)
    persistent: bool,
    managed_network_lease: Option<ManagedNetworkLease>,
}

struct ManagedNetworkLeaseGuard {
    sandbox: String,
    lease: Option<ManagedNetworkLease>,
    allocator: NetworkAllocator,
    committed: bool,
}

impl ManagedNetworkLeaseGuard {
    fn new(sandbox: &str, lease: ManagedNetworkLease) -> Self {
        Self::new_with_allocator(sandbox, lease, NetworkAllocator::new(allocation_data_dir()))
    }

    fn new_with_allocator(
        sandbox: &str,
        lease: ManagedNetworkLease,
        allocator: NetworkAllocator,
    ) -> Self {
        Self {
            sandbox: sandbox.to_string(),
            lease: Some(lease),
            allocator,
            committed: false,
        }
    }

    fn lease(&self) -> &ManagedNetworkLease {
        self.lease
            .as_ref()
            .expect("managed network lease guard always owns its lease")
    }

    fn commit(mut self) -> ManagedNetworkLease {
        self.committed = true;
        self.lease
            .take()
            .expect("managed network lease guard always owns its lease")
    }
}

impl Drop for ManagedNetworkLeaseGuard {
    fn drop(&mut self) {
        if !self.committed
            && let Some(lease) = self.lease.take()
        {
            let _ = self.allocator.release(&self.sandbox, &lease.network);
        }
    }
}

impl DockerSandbox {
    /// Create a new Docker sandbox with the specified runtime
    pub fn new(name: &str, runtime: ContainerRuntime) -> Self {
        Self {
            name: name.to_string(),
            runtime,
            container_id: None,
            running: false,
            persistent: false,
            managed_network_lease: None,
        }
    }

    /// Create a persistent Docker sandbox (won't be cleaned up in Drop)
    pub fn new_persistent(name: &str, runtime: ContainerRuntime) -> Self {
        Self {
            name: name.to_string(),
            runtime,
            container_id: None,
            running: false,
            persistent: true,
            managed_network_lease: None,
        }
    }

    /// Mark this sandbox as persistent (won't be cleaned up in Drop)
    pub fn set_persistent(&mut self, persistent: bool) {
        self.persistent = persistent;
    }

    /// Create a new Docker sandbox with auto-detected runtime
    pub fn with_detected_runtime(name: &str) -> Result<Self> {
        let runtime = detect_container_runtime()
            .ok_or_else(|| anyhow::anyhow!("No container runtime available"))?;
        Ok(Self::new(name, runtime))
    }

    /// Get the container name
    fn container_name(&self) -> String {
        format!("agentkernel-{}", self.name)
    }
}

impl DockerSandbox {
    /// Write a file to the container using docker cp
    async fn write_file_impl(&self, path: &str, content: &[u8]) -> Result<()> {
        let container_name = self.container_name();
        let cmd = self.runtime.cmd();

        // Create a temporary file to copy
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("agentkernel-upload-{}", uuid::Uuid::new_v4()));
        std::fs::write(&temp_file, content).context("Failed to write temp file")?;

        // Ensure parent directory exists in container
        let parent = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let _ = Command::new(cmd)
            .args(["exec", &container_name, "mkdir", "-p", &parent])
            .output();

        // Copy file into container
        let dest = format!("{}:{}", container_name, path);
        let output = Command::new(cmd)
            .args(["cp", temp_file.to_str().unwrap(), &dest])
            .output()
            .context("Failed to copy file to container")?;

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_file);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("docker cp failed: {}", stderr);
        }

        Ok(())
    }

    /// Read a file from the container using docker cp
    async fn read_file_impl(&self, path: &str) -> Result<Vec<u8>> {
        let container_name = self.container_name();
        let cmd = self.runtime.cmd();

        // Create temp file for output
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("agentkernel-download-{}", uuid::Uuid::new_v4()));

        // Copy file from container
        let src = format!("{}:{}", container_name, path);
        let output = Command::new(cmd)
            .args(["cp", &src, temp_file.to_str().unwrap()])
            .output()
            .context("Failed to copy file from container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("docker cp failed: {}", stderr);
        }

        // Read and return content
        let content = std::fs::read(&temp_file).context("Failed to read temp file")?;

        // Clean up
        let _ = std::fs::remove_file(&temp_file);

        Ok(content)
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    fn restore_managed_network(&mut self, config: &ManagedNetworkConfig) -> Result<()> {
        ensure_managed_network(self.runtime, config)?;
        let lease = NetworkAllocator::new(allocation_data_dir()).reserve(&self.name, config)?;
        let actual_ip = get_container_ip_with_runtime(self.runtime, &self.container_name())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "running container '{}' has no inspectable managed-network address",
                    self.container_name()
                )
            })?;
        if actual_ip != lease.ip {
            bail!(
                "running container '{}' uses managed-network address '{}', but its durable lease is '{}'",
                self.container_name(),
                actual_ip,
                lease.ip
            );
        }
        self.managed_network_lease = Some(lease);
        Ok(())
    }

    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        let cmd = self.runtime.cmd();
        let container_name = self.container_name();

        // Remove any existing container with this name
        let _ = Command::new(cmd)
            .args(["rm", "-f", &container_name])
            .output();

        // Build container arguments
        // Note: We use --rm for ephemeral containers but persistent sandboxes
        // will have their containers survive because Drop cleanup is skipped
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            container_name.clone(),
            "--hostname".to_string(),
            "agentkernel".to_string(),
        ];

        // Add resource limits
        args.push(format!("--cpus={}", config.vcpus));
        args.push(format!("--memory={}m", config.memory_mb));

        // Network configuration
        if !config.network {
            args.push("--network=none".to_string());
        }

        let mut pending_network_lease = None;
        if let Some(network) = config.managed_network.as_ref() {
            if !config.network {
                bail!("managed bridge networking requires network access to be enabled");
            }
            ensure_managed_network(self.runtime, network)?;
            let lease =
                NetworkAllocator::new(allocation_data_dir()).reserve(&self.name, network)?;
            let lease = ManagedNetworkLeaseGuard::new(&self.name, lease);
            if network_ip_is_in_use(self.runtime, network, &lease.lease().ip)? {
                bail!(
                    "managed network '{}' address '{}' is already in use",
                    network.name,
                    lease.lease().ip
                );
            }
            args.extend(managed_network_container_args(network, &lease.lease().ip));
            pending_network_lease = Some(lease);
        }

        // Port mappings (-p host:container[/udp])
        for pm in &config.ports {
            args.push("-p".to_string());
            args.push(pm.to_string());
        }

        // Mount working directory if requested
        if config.mount_cwd
            && let Some(ref work_dir) = config.work_dir
        {
            args.push("-v".to_string());
            let container_work_dir = config.container_work_dir.as_deref().unwrap_or("/workspace");
            args.push(format!("{}:{}", work_dir, container_work_dir));
            args.push("-w".to_string());
            args.push(container_work_dir.to_string());
        }

        // Mount home directory if requested
        if config.mount_home
            && let Some(home) = std::env::var_os("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user:ro", home.to_string_lossy()));
        }

        // Mount persistent volumes
        for volume_spec in &config.volumes {
            args.push("-v".to_string());
            args.push(volume_spec.clone());
        }

        // Read-only root filesystem
        if config.read_only {
            args.push("--read-only".to_string());
        }

        // Add environment variables
        for (key, value) in &config.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Add entrypoint override to keep container running
        args.extend([
            "--entrypoint".to_string(),
            "sh".to_string(),
            config.image.clone(),
            "-c".to_string(),
            "while true; do sleep 3600; done".to_string(),
        ]);

        // Start container
        let output = Command::new(cmd)
            .args(&args)
            .output()
            .context("Failed to start container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Some(lease) = self.managed_network_lease.take() {
                let _ = NetworkAllocator::new(allocation_data_dir())
                    .release(&self.name, &lease.network);
            }
            bail!("Failed to start container: {}", stderr);
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.container_id = Some(container_id);
        self.running = true;
        if let Some(lease) = pending_network_lease {
            self.managed_network_lease = Some(lease.commit());
        }

        Ok(())
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
        let runtime_cmd = self.runtime.cmd();
        let container_name = self.container_name();

        let mut args = vec!["exec".to_string()];

        if let Some(ref workdir) = opts.workdir {
            args.push("-w".to_string());
            args.push(workdir.clone());
        }

        if let Some(ref user) = opts.user {
            args.push("-u".to_string());
            args.push(user.clone());
        }

        for e in &opts.env {
            args.push("-e".to_string());
            args.push(e.clone());
        }

        args.push(container_name);
        args.extend(cmd.iter().map(|s| s.to_string()));

        let mut command = tokio::process::Command::new(runtime_cmd);
        command.args(&args).kill_on_drop(true);
        let output = command
            .output()
            .await
            .context("Failed to run command in container")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    async fn stop(&mut self) -> Result<()> {
        let container_name = self.container_name();

        // Use rm -f to kill and remove in one operation
        let removed = Command::new(self.runtime.cmd())
            .args(["rm", "-f", &container_name])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        if !removed {
            bail!("Failed to remove container '{}'", container_name);
        }

        if let Some(lease) = self.managed_network_lease.take() {
            NetworkAllocator::new(allocation_data_dir()).release(&self.name, &lease.network)?;
        }

        self.container_id = None;
        self.running = false;
        Ok(())
    }

    async fn resize(&mut self, vcpus: u32, memory_mb: u64) -> Result<bool> {
        let container_name = self.container_name();
        let output = Command::new(self.runtime.cmd())
            .args([
                "update",
                "--cpus",
                &vcpus.to_string(),
                "--memory",
                &format!("{}m", memory_mb),
                &container_name,
            ])
            .output()
            .context("Failed to resize container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "Warning: in-place resize not supported for '{}': {}",
                container_name,
                stderr.trim()
            );
            return Ok(false);
        }

        Ok(true)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        self.runtime.to_backend_type()
    }

    fn is_running(&self) -> bool {
        // Check Docker directly - don't rely on internal state since
        // we might be reconnecting to an existing container
        let container_name = self.container_name();
        Command::new(self.runtime.cmd())
            .args(["ps", "-q", "-f", &format!("name={}", container_name)])
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false)
    }

    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> Result<()> {
        self.write_file_impl(path, content).await
    }

    async fn read_file_unchecked(&mut self, path: &str) -> Result<Vec<u8>> {
        self.read_file_impl(path).await
    }

    async fn remove_file_unchecked(&mut self, path: &str) -> Result<()> {
        let container_name = self.container_name();
        let output = Command::new(self.runtime.cmd())
            .args(["exec", &container_name, "rm", "-f", path])
            .output()
            .context("Failed to remove file in container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("rm failed: {}", stderr);
        }

        Ok(())
    }

    async fn mkdir_unchecked(&mut self, path: &str, recursive: bool) -> Result<()> {
        let container_name = self.container_name();
        let mut args = vec!["exec", &container_name, "mkdir"];
        if recursive {
            args.push("-p");
        }
        args.push(path);

        let output = Command::new(self.runtime.cmd())
            .args(&args)
            .output()
            .context("Failed to create directory in container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("mkdir failed: {}", stderr);
        }

        Ok(())
    }

    async fn attach(&mut self, shell: Option<&str>) -> Result<i32> {
        self.attach_with_env(shell, &[]).await
    }

    async fn attach_with_env(&mut self, shell: Option<&str>, env: &[String]) -> Result<i32> {
        // Check Docker directly since we might be reconnecting to an existing container
        if !self.is_running() {
            bail!("Container is not running");
        }

        let container_name = self.container_name();
        let shell_cmd = shell.unwrap_or("/bin/sh");

        // Build the docker exec command
        let mut docker_args = vec!["exec".to_string(), "-it".to_string()];
        for e in env {
            docker_args.push("-e".to_string());
            docker_args.push(e.clone());
        }
        docker_args.push(container_name);
        docker_args.push(shell_cmd.to_string());

        let runtime_cmd = self.runtime.cmd();

        // Check if recording was requested via AGENTKERNEL_RECORD env var
        // (set by the attach command handler when --record is passed)
        let record_path = std::env::var("AGENTKERNEL_RECORD").ok();

        let status = if let Some(ref cast_path) = record_path {
            // Wrap with `script` to capture PTY I/O for session recording.
            // macOS: script -q <file> <cmd> [args...]
            // Linux: script -qc "<cmd> [args...]" <file>
            let full_cmd = std::iter::once(runtime_cmd.to_string())
                .chain(docker_args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" ");

            let mut script_args = if cfg!(target_os = "macos") {
                vec!["-q".to_string(), cast_path.clone(), runtime_cmd.to_string()]
            } else {
                vec![
                    "-q".to_string(),
                    "-c".to_string(),
                    full_cmd,
                    cast_path.clone(),
                ]
            };

            if cfg!(target_os = "macos") {
                script_args.extend(docker_args);
            }

            std::process::Command::new("script")
                .args(&script_args)
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .context("Failed to record session with script")?
        } else {
            std::process::Command::new(runtime_cmd)
                .args(&docker_args)
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .context("Failed to attach to container")?
        };

        Ok(status.code().unwrap_or(-1))
    }
}

impl DockerSandbox {
    /// Run a command in a temporary container using `docker run --rm`
    /// This is faster than create→start→exec→stop for one-shot commands
    pub fn run_ephemeral_cmd(
        runtime: ContainerRuntime,
        image: &str,
        cmd: &[String],
        config: &SandboxConfig,
    ) -> Result<ExecResult> {
        let runtime_cmd = runtime.cmd();

        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(), // auto-remove after exit
        ];

        // Add resource limits
        args.push(format!("--cpus={}", config.vcpus));
        args.push(format!("--memory={}m", config.memory_mb));

        // Network configuration
        if !config.network {
            args.push("--network=none".to_string());
        }

        if config.managed_network.is_some() {
            bail!("managed bridge networking is only supported for persisted sandboxes");
        }

        // Port mappings (-p host:container[/udp])
        for pm in &config.ports {
            args.push("-p".to_string());
            args.push(pm.to_string());
        }

        // Mount working directory if requested
        if config.mount_cwd
            && let Some(ref work_dir) = config.work_dir
        {
            args.push("-v".to_string());
            let container_work_dir = config.container_work_dir.as_deref().unwrap_or("/workspace");
            args.push(format!("{}:{}", work_dir, container_work_dir));
            args.push("-w".to_string());
            args.push(container_work_dir.to_string());
        }

        // Mount home directory if requested (read-only)
        if config.mount_home
            && let Some(home) = std::env::var_os("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user:ro", home.to_string_lossy()));
        }

        // Read-only root filesystem
        if config.read_only {
            args.push("--read-only".to_string());
        }

        // Add environment variables
        for (key, value) in &config.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Image and command
        args.push(image.to_string());
        args.extend(cmd.iter().cloned());

        // Run the container
        let output = Command::new(runtime_cmd)
            .args(&args)
            .output()
            .context("Failed to run container")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }
}

impl Drop for DockerSandbox {
    fn drop(&mut self) {
        // Only clean up if running and not marked as persistent
        let removed = if self.running && !self.persistent {
            let container_name = self.container_name();
            Command::new(self.runtime.cmd())
                .args(["rm", "-f", &container_name])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        } else {
            false
        };
        if removed && let Some(lease) = self.managed_network_lease.take() {
            let _ =
                NetworkAllocator::new(allocation_data_dir()).release(&self.name, &lease.network);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const DOCKER_NETWORK_INSPECT: &str = r#"[
      {
        "Name": "agentkernel-dev",
        "Driver": "bridge",
        "IPAM": {"Config": [{"Subnet": "172.30.0.0/24", "Gateway": "172.30.0.1"}]},
        "Containers": {
          "abc": {"Name": "agentkernel-one", "IPv4Address": "172.30.0.2/24"}
        },
        "Labels": {
          "com.agentkernel.managed": "true",
          "com.agentkernel.network": "agentkernel-dev"
        }
      }
    ]"#;

    const PODMAN_NETWORK_INSPECT: &str = r#"[
      {
        "name": "agentkernel-dev",
        "driver": "bridge",
        "subnets": [{"subnet": "172.30.0.0/24", "gateway": "172.30.0.1"}],
        "containers": {
          "abc": {
            "name": "agentkernel-one",
            "interfaces": [{"ip_address": "172.30.0.3/24"}]
          }
        },
        "labels": {
          "com.agentkernel.managed": "true",
          "com.agentkernel.network": "agentkernel-dev"
        }
      }
    ]"#;

    #[test]
    fn parses_docker_network_inspection_fixture() {
        let inspected =
            parse_network_inspection(ContainerRuntime::Docker, DOCKER_NETWORK_INSPECT.as_bytes())
                .unwrap();
        assert_eq!(inspected.driver, "bridge");
        assert_eq!(inspected.subnet.as_deref(), Some("172.30.0.0/24"));
        assert_eq!(inspected.gateway.as_deref(), Some("172.30.0.1"));
        assert!(inspected.managed);
        assert_eq!(inspected.managed_name.as_deref(), Some("agentkernel-dev"));
        assert_eq!(inspected.container_ips, vec!["172.30.0.2"]);
    }

    #[test]
    fn parses_podman_network_inspection_fixture() {
        let inspected =
            parse_network_inspection(ContainerRuntime::Podman, PODMAN_NETWORK_INSPECT.as_bytes())
                .unwrap();
        assert_eq!(inspected.driver, "bridge");
        assert_eq!(inspected.subnet.as_deref(), Some("172.30.0.0/24"));
        assert_eq!(inspected.gateway.as_deref(), Some("172.30.0.1"));
        assert!(inspected.managed);
        assert_eq!(inspected.container_ips, vec!["172.30.0.3"]);
    }

    #[test]
    fn rejects_unowned_network_and_keeps_dns_on_container_command() {
        let config = ManagedNetworkConfig {
            name: "agentkernel-dev".to_string(),
            subnet: "172.30.0.0/24".to_string(),
            gateway: None,
            dns: vec!["1.1.1.1".to_string()],
            static_ip: None,
        };
        let mut external = serde_json::from_str::<Value>(DOCKER_NETWORK_INSPECT).unwrap();
        external[0]["Labels"] = serde_json::json!({});
        let external = serde_json::to_vec(&external).unwrap();
        let inspected = parse_network_inspection(ContainerRuntime::Docker, &external).unwrap();
        assert!(validate_inspected_network(&config, &inspected).is_err());

        let create_args = managed_network_create_args(&config).unwrap();
        assert!(!create_args.contains(&"--dns".to_string()));
        assert_eq!(create_args.last(), Some(&config.name));
        assert_eq!(
            create_args
                .iter()
                .filter(|arg| *arg == &config.name)
                .count(),
            1
        );
        let run_args = managed_network_container_args(&config, "172.30.0.2");
        assert!(
            run_args
                .windows(2)
                .any(|window| window == ["--dns", "1.1.1.1"])
        );
    }

    #[test]
    fn failed_start_guard_releases_new_lease() {
        let temp = TempDir::new().unwrap();
        let allocator = NetworkAllocator::new(temp.path());
        let config = ManagedNetworkConfig::new("agentkernel-dev");
        let lease = allocator.reserve("failed-start", &config).unwrap();
        {
            let guard = ManagedNetworkLeaseGuard::new_with_allocator(
                "failed-start",
                lease,
                allocator.clone(),
            );
            assert_eq!(guard.lease().ip, "172.30.0.2");
        }
        assert_eq!(
            allocator.reserve("retry-start", &config).unwrap().ip,
            "172.30.0.2"
        );
    }
}
