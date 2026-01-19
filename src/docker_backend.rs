//! Container backend for running sandboxes when KVM is not available.
//!
//! This provides a fallback for macOS and other systems without KVM support.
//! Uses Docker or Podman containers instead of Firecracker microVMs.

use anyhow::{Context, Result, bail};
use std::process::Command;

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
}

/// Detect the best available container runtime
pub fn detect_container_runtime() -> Option<ContainerRuntime> {
    // Prefer Podman (rootless, daemonless) over Docker
    if podman_available() {
        Some(ContainerRuntime::Podman)
    } else if docker_available() {
        Some(ContainerRuntime::Docker)
    } else {
        None
    }
}

/// Container-based sandbox (Docker or Podman)
pub struct ContainerSandbox {
    pub name: String,
    runtime: ContainerRuntime,
    container_id: Option<String>,
}

// Keep the old name as an alias for compatibility
#[allow(dead_code)]
pub type DockerSandbox = ContainerSandbox;

impl ContainerSandbox {
    /// Create a new container sandbox with the detected runtime
    #[allow(dead_code)]
    pub fn new(name: &str) -> Self {
        let runtime = detect_container_runtime().unwrap_or(ContainerRuntime::Docker);
        Self {
            name: name.to_string(),
            runtime,
            container_id: None,
        }
    }

    /// Create a new container sandbox with a specific runtime
    pub fn with_runtime(name: &str, runtime: ContainerRuntime) -> Self {
        Self {
            name: name.to_string(),
            runtime,
            container_id: None,
        }
    }

    /// Get the runtime being used
    #[allow(dead_code)]
    pub fn runtime(&self) -> ContainerRuntime {
        self.runtime
    }

    /// Start the container with the specified image
    pub async fn start(&mut self, image: &str) -> Result<()> {
        let cmd = self.runtime.cmd();

        // Check if container already exists
        let existing = Command::new(cmd)
            .args([
                "ps",
                "-aq",
                "-f",
                &format!("name=agentkernel-{}", self.name),
            ])
            .output()
            .context("Failed to check for existing container")?;

        let existing_id = String::from_utf8_lossy(&existing.stdout).trim().to_string();
        if !existing_id.is_empty() {
            // Remove existing container
            let _ = Command::new(cmd).args(["rm", "-f", &existing_id]).output();
        }

        // Start new container with overridden entrypoint to handle tool images
        let output = Command::new(cmd)
            .args([
                "run",
                "-d",
                "--name",
                &format!("agentkernel-{}", self.name),
                "--hostname",
                "agentkernel",
                "--entrypoint",
                "sh",
                image,
                "-c",
                "while true; do sleep 3600; done",
            ])
            .output()
            .context("Failed to start container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to start container: {}", stderr);
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.container_id = Some(container_id);

        Ok(())
    }

    /// Execute a command in the container
    #[allow(dead_code)]
    pub async fn exec(&self, cmd: &[String]) -> Result<String> {
        let runtime_cmd = self.runtime.cmd();
        let container_name = format!("agentkernel-{}", self.name);

        let mut args = vec!["exec", &container_name];
        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        args.extend(cmd_refs);

        let output = Command::new(runtime_cmd)
            .args(&args)
            .output()
            .context("Failed to execute command in container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Command failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Stop the container
    #[allow(dead_code)]
    pub async fn stop(&mut self) -> Result<()> {
        let container_name = format!("agentkernel-{}", self.name);

        let _ = Command::new(self.runtime.cmd())
            .args(["stop", &container_name])
            .output();

        self.container_id = None;
        Ok(())
    }

    /// Remove the container
    #[allow(dead_code)]
    pub async fn remove(&mut self) -> Result<()> {
        let container_name = format!("agentkernel-{}", self.name);

        let _ = Command::new(self.runtime.cmd())
            .args(["rm", "-f", &container_name])
            .output();

        self.container_id = None;
        Ok(())
    }

    /// Check if container is running
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        let container_name = format!("agentkernel-{}", self.name);

        if let Ok(output) = Command::new(self.runtime.cmd())
            .args(["ps", "-q", "-f", &format!("name={}", container_name)])
            .output()
        {
            !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        } else {
            false
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

/// Check if any container runtime is available
#[allow(dead_code)]
pub fn container_runtime_available() -> bool {
    docker_available() || podman_available()
}
