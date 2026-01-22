//! Apple Containers backend implementing the Sandbox trait (macOS 26+ only).

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::process::Command;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};

/// Check if Apple container system service is running
pub fn apple_system_running() -> bool {
    Command::new("container")
        .args(["system", "status"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("is running"))
        .unwrap_or(false)
}

/// Start the Apple container system service
pub fn start_apple_system() -> Result<()> {
    if apple_system_running() {
        return Ok(());
    }

    eprintln!("Starting Apple container system...");

    // Use echo "Y" to auto-accept kernel download prompt
    let output = Command::new("sh")
        .args(["-c", "echo 'Y' | container system start"])
        .output()
        .context("Failed to start Apple container system")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already") {
            bail!("Failed to start Apple container system: {}", stderr);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    Ok(())
}

/// Check if Apple containers is available
pub fn apple_containers_available() -> bool {
    Command::new("container")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check macOS version (needs 26+)
pub fn macos_version_supported() -> bool {
    let output = Command::new("sw_vers").arg("-productVersion").output().ok();

    if let Some(output) = output
        && let Ok(version) = String::from_utf8(output.stdout)
        && let Some(major) = version.trim().split('.').next()
        && let Ok(major_num) = major.parse::<u32>()
    {
        return major_num >= 26;
    }

    false
}

/// Apple Containers sandbox
pub struct AppleSandbox {
    name: String,
    container_id: Option<String>,
    running: bool,
}

impl AppleSandbox {
    /// Create a new Apple sandbox
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            container_id: None,
            running: false,
        }
    }

    /// Get the container name
    fn container_name(&self) -> String {
        format!("agentkernel-{}", self.name)
    }
}

#[async_trait]
impl Sandbox for AppleSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        // Ensure system is running
        start_apple_system()?;

        let container_name = self.container_name();

        // Remove any existing container
        let _ = Command::new("container")
            .args(["delete", "-f", &container_name])
            .output();

        // Build container arguments
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            container_name.clone(),
        ];

        // Resource limits
        args.push("--cpus".to_string());
        args.push(config.vcpus.to_string());
        args.push("--memory".to_string());
        args.push(format!("{}M", config.memory_mb));

        // Mount working directory if requested
        if config.mount_cwd
            && let Some(ref work_dir) = config.work_dir
        {
            args.push("-v".to_string());
            args.push(format!("{}:/workspace", work_dir));
            args.push("-w".to_string());
            args.push("/workspace".to_string());
        }

        // Mount home directory if requested
        if config.mount_home
            && let Some(home) = std::env::var_os("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user:ro", home.to_string_lossy()));
        }

        // Add environment variables
        for (key, value) in &config.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Note: Apple containers don't support --read-only flag directly
        // Image and command to keep container running
        args.push(config.image.clone());
        args.push("sleep".to_string());
        args.push("infinity".to_string());

        // Run the container
        let output = Command::new("container")
            .args(&args)
            .output()
            .context("Failed to start Apple container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to start container: {}", stderr);
        }

        self.container_id = Some(container_name);
        self.running = true;
        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        let container_id = self
            .container_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Container not started"))?;

        let mut args = vec!["exec", container_id.as_str()];
        args.extend(cmd);

        let output = Command::new("container")
            .args(&args)
            .output()
            .context("Failed to run command in Apple container")?;

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
        if let Some(container_id) = &self.container_id {
            // Stop with short timeout
            let _ = Command::new("container")
                .args(["stop", "-t", "1", container_id])
                .output();

            // Force delete
            let _ = Command::new("container")
                .args(["delete", "-f", container_id])
                .output();
        }

        self.container_id = None;
        self.running = false;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Apple
    }

    fn is_running(&self) -> bool {
        if !self.running {
            return false;
        }

        if let Some(container_id) = &self.container_id {
            Command::new("container")
                .args(["ls", "--filter", &format!("name={}", container_id)])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(container_id))
                .unwrap_or(false)
        } else {
            false
        }
    }
}

impl Drop for AppleSandbox {
    fn drop(&mut self) {
        if let Some(container_id) = &self.container_id {
            let _ = Command::new("container")
                .args(["delete", "-f", container_id])
                .output();
        }
    }
}
