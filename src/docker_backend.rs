//! Container backend for running sandboxes when KVM is not available.
//!
//! This provides a fallback for macOS and other systems without KVM support.
//! Uses Docker or Podman containers instead of Firecracker microVMs.

use anyhow::{Context, Result, bail};
use std::process::Command;

use crate::permissions::Permissions;

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
    #[allow(dead_code)]
    pub async fn start(&mut self, image: &str) -> Result<()> {
        self.start_with_permissions(image, &Permissions::default())
            .await
    }

    /// Start the container with the specified image and permissions
    pub async fn start_with_permissions(&mut self, image: &str, perms: &Permissions) -> Result<()> {
        let cmd = self.runtime.cmd();

        // Optimized: Use --rm to auto-remove on stop, avoiding separate cleanup
        // Also use --force-rm style by directly replacing any existing container
        let container_name = format!("agentkernel-{}", self.name);

        // Fast-path: remove any existing container (no check, just force remove)
        let _ = Command::new(cmd)
            .args(["rm", "-f", &container_name])
            .output();

        // Build container arguments
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--rm".to_string(), // Auto-remove on stop for faster cleanup
            "--name".to_string(),
            format!("agentkernel-{}", self.name),
            "--hostname".to_string(),
            "agentkernel".to_string(),
        ];

        // Add permission-based security args
        args.extend(perms.to_docker_args());
        args.extend(perms.get_env_args());
        args.extend(perms.get_mount_args(None));

        // Add entrypoint override for tool images
        args.extend([
            "--entrypoint".to_string(),
            "sh".to_string(),
            image.to_string(),
            "-c".to_string(),
            "while true; do sleep 3600; done".to_string(),
        ]);

        // Start new container
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

    /// Stop the container (uses rm -f to kill and remove in one operation)
    #[allow(dead_code)]
    pub async fn stop(&mut self) -> Result<()> {
        let container_name = format!("agentkernel-{}", self.name);

        // Use rm -f instead of stop - kills and removes in one CLI call
        // This is faster than stop + remove for ephemeral containers
        let _ = Command::new(self.runtime.cmd())
            .args(["rm", "-f", &container_name])
            .output();

        self.container_id = None;
        Ok(())
    }

    /// Remove the container (no-op if already stopped with rm -f)
    #[allow(dead_code)]
    pub async fn remove(&mut self) -> Result<()> {
        let container_name = format!("agentkernel-{}", self.name);

        // Safe to call even if container was already removed by stop()
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

    /// Run a command in a temporary container using `docker run --rm`
    /// This is faster than create→start→exec→stop for one-shot commands
    pub fn run_ephemeral_cmd(
        runtime: ContainerRuntime,
        image: &str,
        cmd: &[String],
        perms: &Permissions,
    ) -> Result<(i32, String, String)> {
        let runtime_cmd = runtime.cmd();

        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(), // auto-remove after exit
        ];

        // Resource limits
        if let Some(cpu) = perms.max_cpu_percent {
            args.push(format!("--cpus={}", cpu as f32 / 100.0));
        }
        if let Some(mem) = perms.max_memory_mb {
            args.push(format!("--memory={}m", mem));
        }

        // Network configuration
        if !perms.network {
            args.push("--network=none".to_string());
        }

        // Mount working directory if requested
        if perms.mount_cwd
            && let Ok(cwd) = std::env::current_dir()
        {
            args.push("-v".to_string());
            args.push(format!("{}:/workspace", cwd.display()));
            args.push("-w".to_string());
            args.push("/workspace".to_string());
        }

        // Mount home directory if requested (read-only)
        if perms.mount_home
            && let Some(home) = std::env::var_os("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user:ro", home.to_string_lossy()));
        }

        // Read-only root filesystem
        if perms.read_only_root {
            args.push("--read-only".to_string());
        }

        // Environment variables
        if perms.pass_env {
            for var in ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"] {
                if let Ok(val) = std::env::var(var) {
                    args.push("-e".to_string());
                    args.push(format!("{}={}", var, val));
                }
            }
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

        Ok((exit_code, stdout, stderr))
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
