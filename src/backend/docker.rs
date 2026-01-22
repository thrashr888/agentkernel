//! Docker/Podman container backend implementing the Sandbox trait.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::process::Command;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};

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
}

impl DockerSandbox {
    /// Create a new Docker sandbox with the specified runtime
    pub fn new(name: &str, runtime: ContainerRuntime) -> Self {
        Self {
            name: name.to_string(),
            runtime,
            container_id: None,
            running: false,
        }
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

#[async_trait]
impl Sandbox for DockerSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        let cmd = self.runtime.cmd();
        let container_name = self.container_name();

        // Remove any existing container with this name
        let _ = Command::new(cmd)
            .args(["rm", "-f", &container_name])
            .output();

        // Build container arguments
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--rm".to_string(),
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

        // Mount working directory if requested
        if config.mount_cwd
            && let Some(ref work_dir) = config.work_dir
        {
            args.push("-v".to_string());
            args.push(format!("{}:/workspace", work_dir));
            args.push("-w".to_string());
            args.push("/workspace".to_string());
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
            bail!("Failed to start container: {}", stderr);
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.container_id = Some(container_id);
        self.running = true;

        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        let runtime_cmd = self.runtime.cmd();
        let container_name = self.container_name();

        let mut args = vec!["exec", &container_name];
        args.extend(cmd);

        let output = Command::new(runtime_cmd)
            .args(&args)
            .output()
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
        let _ = Command::new(self.runtime.cmd())
            .args(["rm", "-f", &container_name])
            .output();

        self.container_id = None;
        self.running = false;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        self.runtime.to_backend_type()
    }

    fn is_running(&self) -> bool {
        if !self.running {
            return false;
        }

        let container_name = self.container_name();
        Command::new(self.runtime.cmd())
            .args(["ps", "-q", "-f", &format!("name={}", container_name)])
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false)
    }
}

impl Drop for DockerSandbox {
    fn drop(&mut self) {
        if self.running {
            let container_name = self.container_name();
            let _ = Command::new(self.runtime.cmd())
                .args(["rm", "-f", &container_name])
                .output();
        }
    }
}
