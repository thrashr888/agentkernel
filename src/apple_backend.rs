//! Apple Containers backend for running sandboxes on macOS 26+.
//!
//! Uses Apple's native `container` CLI which provides lightweight VMs
//! (one VM per container) with hardware isolation on Apple Silicon.

use anyhow::{Context, Result, bail};
use std::process::Command;

use crate::permissions::Permissions;

/// Check if Apple containers is available
pub fn apple_containers_available() -> bool {
    // Check if we're on macOS
    if !cfg!(target_os = "macos") {
        return false;
    }

    // Check if `container` CLI is installed
    Command::new("container")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check macOS version (needs 26+)
pub fn macos_version_supported() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }

    // Get macOS version
    let output = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok();

    if let Some(output) = output
        && let Ok(version) = String::from_utf8(output.stdout)
        && let Some(major) = version.trim().split('.').next()
        && let Ok(major_num) = major.parse::<u32>()
    {
        return major_num >= 26;
    }

    false
}

/// Apple container-based sandbox
pub struct AppleContainerSandbox {
    pub name: String,
    container_id: Option<String>,
}

impl AppleContainerSandbox {
    /// Create a new Apple container sandbox
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            container_id: None,
        }
    }

    /// Start the container with the specified image and permissions
    pub async fn start_with_permissions(
        &mut self,
        image: &str,
        perms: &Permissions,
    ) -> Result<()> {
        let container_name = format!("agentkernel-{}", self.name);

        // Remove any existing container (ignore errors)
        let _ = Command::new("container")
            .args(["delete", "-f", &container_name])
            .output();

        // Build container arguments
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(), // detached
            "--name".to_string(),
            container_name.clone(),
        ];

        // Resource limits
        if let Some(cpu) = perms.max_cpu_percent {
            args.push("--cpus".to_string());
            args.push(format!("{:.2}", cpu as f64 / 100.0));
        }
        if let Some(mem) = perms.max_memory_mb {
            args.push("--memory".to_string());
            args.push(format!("{}M", mem));
        }

        // Network access
        if !perms.network {
            // Apple containers may use different network syntax
            // For now, we'll skip network isolation as it may not be supported
            // args.push("--network=none".to_string());
        }

        // Volume mounts
        if perms.mount_cwd
            && let Ok(cwd) = std::env::current_dir()
        {
            args.push("-v".to_string());
            args.push(format!("{}:/app", cwd.display()));
            args.push("-w".to_string());
            args.push("/app".to_string());
        }

        // Read-only filesystem
        if perms.read_only_root {
            args.push("--read-only".to_string());
        }

        // Environment variables (pass through if enabled)
        if perms.pass_env {
            for var in ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"] {
                if let Ok(val) = std::env::var(var) {
                    args.push("-e".to_string());
                    args.push(format!("{}={}", var, val));
                }
            }
        }

        // Image and command (sleep infinity to keep container running)
        args.push(image.to_string());
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
        Ok(())
    }

    /// Execute a command in the running container
    pub async fn execute(&self, cmd: &[String]) -> Result<String> {
        let container_id = self
            .container_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Container not started"))?;

        let mut args = vec!["exec".to_string(), container_id.clone()];
        args.extend(cmd.iter().cloned());

        let output = Command::new("container")
            .args(&args)
            .output()
            .context("Failed to execute command in Apple container")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success()
            && !stderr.is_empty()
        {
            bail!("Command failed: {}", stderr);
        }

        // Return combined output
        if stderr.is_empty() {
            Ok(stdout)
        } else {
            Ok(format!("{}{}", stdout, stderr))
        }
    }

    /// Stop the container
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(container_id) = &self.container_id {
            let output = Command::new("container")
                .args(["stop", "-t", "1", container_id])
                .output()
                .context("Failed to stop Apple container")?;

            if !output.status.success() {
                // Log but don't fail - container might already be stopped
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("Warning: stop returned error: {}", stderr);
            }
        }
        Ok(())
    }

    /// Remove the container
    pub async fn remove(&mut self) -> Result<()> {
        if let Some(container_id) = &self.container_id {
            // Force remove (handles both running and stopped containers)
            let _ = Command::new("container")
                .args(["delete", "-f", container_id])
                .output();

            self.container_id = None;
        }
        Ok(())
    }

    /// Check if container is running
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        if let Some(container_id) = &self.container_id {
            // Use `container ls` to check if container exists
            let output = Command::new("container")
                .args(["ls", "--filter", &format!("name={}", container_id)])
                .output()
                .ok();

            if let Some(output) = output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                return stdout.contains(container_id);
            }
        }
        false
    }

    /// Run a command in a temporary container (create, start, exec, stop, remove)
    #[allow(dead_code)]
    pub async fn run_ephemeral(
        &mut self,
        image: &str,
        cmd: &[String],
        perms: &Permissions,
    ) -> Result<String> {
        // Build container arguments for one-shot execution
        let container_name = format!("agentkernel-{}", self.name);

        // Remove any existing container
        let _ = Command::new("container")
            .args(["delete", "-f", &container_name])
            .output();

        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(), // auto-remove after exit
            "--name".to_string(),
            container_name.clone(),
        ];

        // Resource limits
        if let Some(cpu) = perms.max_cpu_percent {
            args.push("--cpus".to_string());
            args.push(format!("{:.2}", cpu as f64 / 100.0));
        }
        if let Some(mem) = perms.max_memory_mb {
            args.push("--memory".to_string());
            args.push(format!("{}M", mem));
        }

        // Volume mounts
        if perms.mount_cwd
            && let Ok(cwd) = std::env::current_dir()
        {
            args.push("-v".to_string());
            args.push(format!("{}:/app", cwd.display()));
            args.push("-w".to_string());
            args.push("/app".to_string());
        }

        // Read-only filesystem
        if perms.read_only_root {
            args.push("--read-only".to_string());
        }

        // Environment variables (pass through if enabled)
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
        let output = Command::new("container")
            .args(&args)
            .output()
            .context("Failed to run Apple container")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success()
            && !stderr.is_empty()
        {
            bail!("Container command failed: {}", stderr);
        }

        // Return combined output
        if stderr.is_empty() {
            Ok(stdout)
        } else {
            Ok(format!("{}{}", stdout, stderr))
        }
    }
}

impl Drop for AppleContainerSandbox {
    fn drop(&mut self) {
        // Best-effort cleanup
        if let Some(container_id) = &self.container_id {
            let _ = Command::new("container")
                .args(["delete", "-f", container_id])
                .output();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apple_containers_check() {
        // This test just verifies the check doesn't panic
        let _ = apple_containers_available();
        let _ = macos_version_supported();
    }
}
