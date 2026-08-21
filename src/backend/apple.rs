//! Apple Containers backend implementing the Sandbox trait (macOS 26+ only).

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};

/// Cached flag indicating if system is already verified running
static SYSTEM_VERIFIED: AtomicBool = AtomicBool::new(false);

/// Check if Apple container system service is running
pub fn apple_system_running() -> bool {
    // Fast path: if we've already verified, skip the command
    if SYSTEM_VERIFIED.load(Ordering::Relaxed) {
        return true;
    }

    let structured_status = Command::new("container")
        .args(["system", "status", "--format", "json"])
        .output();
    let running = match structured_status {
        Ok(output) if output.status.success() => {
            apple_system_status_is_running(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Command::new("container")
            .args(["system", "status"])
            .output()
            .map(|output| {
                output.status.success()
                    && apple_system_status_is_running(&String::from_utf8_lossy(&output.stdout))
            })
            .unwrap_or(false),
    };

    if running {
        SYSTEM_VERIFIED.store(true, Ordering::Relaxed);
    }

    running
}

/// Parse structured status output from current Apple container releases while
/// retaining compatibility with the human-readable output used by older CLIs.
fn apple_system_status_is_running(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output.trim())
        .ok()
        .and_then(|value| value.get("status")?.as_str().map(str::to_owned))
        .is_some_and(|status| status.eq_ignore_ascii_case("running"))
        || output.contains("is running")
}

/// Start the Apple container system service
pub fn start_apple_system() -> Result<()> {
    // Fast path: if already verified running, skip everything
    if SYSTEM_VERIFIED.load(Ordering::Relaxed) {
        return Ok(());
    }

    if apple_system_running() {
        return Ok(());
    }

    eprintln!("Starting Apple container system...");

    let output = Command::new("container")
        .args(["system", "start", "--enable-kernel-install"])
        .output()
        .context("Failed to start Apple container system")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already") {
            bail!("Failed to start Apple container system: {}", stderr);
        }
    }

    // Only sleep on first start, not when already running
    std::thread::sleep(std::time::Duration::from_millis(500));
    SYSTEM_VERIFIED.store(true, Ordering::Relaxed);
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

/// Check whether an image tag refers to a local-only image that should never be
/// pulled from a remote registry (e.g. snapshot images created via `docker commit`).
fn is_local_image(image: &str) -> bool {
    image.starts_with("agentkernel-snap:")
        || (image.starts_with("agentkernel-")
            && !image.contains('/')
            && !image.contains(".io")
            && !image.contains(".com"))
}

/// Check if an image exists in the Apple container image store.
fn apple_image_exists(image: &str) -> bool {
    Command::new("container")
        .args(["image", "inspect", image])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Import a Docker-local image into the Apple container image store.
///
/// Uses `docker save <tag> | container image load` to transfer images that
/// exist in Docker (e.g. from `docker commit`) but not in the Apple store.
fn import_image_from_docker(image: &str) -> Result<()> {
    use std::process::Stdio;

    // Verify the image exists in Docker first
    let docker_check = Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .context("Failed to check Docker for image")?;

    if !docker_check.status.success() {
        bail!(
            "Image '{}' not found in Docker or Apple container stores. \
             Was the snapshot created on this machine?",
            image
        );
    }

    eprintln!(
        "Importing image '{}' from Docker into Apple containers...",
        image
    );

    // Pipe: docker save <image> | container image load
    let docker_save = Command::new("docker")
        .args(["save", image])
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run docker save")?;

    let load_output = Command::new("container")
        .args(["image", "load"])
        .stdin(docker_save.stdout.unwrap())
        .output()
        .context("Failed to run container image load")?;

    if !load_output.status.success() {
        let stderr = String::from_utf8_lossy(&load_output.stderr);
        bail!("Failed to import image into Apple containers: {}", stderr);
    }

    Ok(())
}

/// Get the IP address of an Apple container by parsing `container inspect` JSON.
pub fn get_container_ip(container_name: &str) -> Option<String> {
    let output = Command::new("container")
        .args(["inspect", container_name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_container_ip(&text)
}

/// Parse `container inspect` output from current and legacy Apple container
/// releases. Version 1.0 moved network details under `status` and renamed the
/// address field to `ipv4Address`.
fn parse_container_ip(output: &str) -> Option<String> {
    let arr: serde_json::Value = serde_json::from_str(output.trim()).ok()?;
    let container = arr.get(0)?;
    let addr = container
        .pointer("/status/networks/0/ipv4Address")
        .or_else(|| container.pointer("/networks/0/address"))?
        .as_str()?;
    Some(addr.split('/').next().unwrap_or(addr).to_string())
}

/// Apple Containers sandbox
pub struct AppleSandbox {
    name: String,
    /// Whether we started this container (controls Drop cleanup)
    running: bool,
    /// Whether this sandbox should persist after Drop (like Docker)
    persistent: bool,
}

impl AppleSandbox {
    /// Create a new Apple sandbox
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            running: false,
            persistent: false,
        }
    }

    /// Create a new persistent Apple sandbox (won't be cleaned up on Drop)
    pub fn new_persistent(name: &str) -> Self {
        Self {
            name: name.to_string(),
            running: false,
            persistent: true,
        }
    }

    /// Get the container name (always derived from sandbox name, like Docker)
    fn container_name(&self) -> String {
        format!("agentkernel-{}", self.name)
    }

    /// Run a command in a temporary Apple container using `container run --rm`
    pub fn run_ephemeral_cmd(cmd: &[String], config: &SandboxConfig) -> Result<ExecResult> {
        start_apple_system()?;

        if is_local_image(&config.image) && !apple_image_exists(&config.image) {
            import_image_from_docker(&config.image)?;
        }

        let mut args = vec!["run".to_string(), "--rm".to_string()];
        args.push("--cpus".to_string());
        args.push(config.vcpus.to_string());
        args.push("--memory".to_string());
        args.push(format!("{}M", config.memory_mb));

        if config.mount_cwd
            && let Some(ref work_dir) = config.work_dir
        {
            args.push("-v".to_string());
            args.push(format!("{}:/workspace", work_dir));
            args.push("-w".to_string());
            args.push("/workspace".to_string());
        }

        if config.mount_home
            && let Some(home) = std::env::var_os("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user", home.to_string_lossy()));
        }

        for (key, value) in &config.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, value));
        }

        for pm in &config.ports {
            args.push("-p".to_string());
            args.push(pm.to_string());
        }

        args.push(config.image.clone());
        args.extend(cmd.iter().cloned());

        let output = Command::new("container")
            .args(&args)
            .output()
            .context("Failed to run Apple container")?;

        Ok(ExecResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

#[async_trait]
impl Sandbox for AppleSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        // Ensure system is running
        start_apple_system()?;

        let container_name = self.container_name();

        // For local/snapshot images (e.g. "agentkernel-snap:my-snap"), ensure the
        // image is available in the Apple container store. These images are created
        // via `docker commit` and live only in Docker's image store, so Apple's
        // `container run` would try (and fail) to pull them from a registry.
        if is_local_image(&config.image) && !apple_image_exists(&config.image) {
            import_image_from_docker(&config.image)?;
        }

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

        self.running = true;
        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_env(cmd, &[]).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        let container_name = self.container_name();

        let mut args = vec!["exec".to_string()];

        // Add environment variables
        for e in env {
            args.push("-e".to_string());
            args.push(e.clone());
        }

        args.push(container_name);
        args.extend(cmd.iter().map(|s| s.to_string()));

        // Use tokio::process::Command so exec doesn't block the tokio runtime.
        // This is critical for the secret proxy: the proxy runs as a tokio task,
        // and blocking with std::process::Command would starve it when the exec'd
        // process makes requests through the proxy (deadlock).
        let output = tokio::process::Command::new("container")
            .args(&args)
            .output()
            .await
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
        let container_name = self.container_name();

        // Stop with short timeout — use tokio to avoid blocking forever
        // if the VM process is stuck (e.g. spinning at 100%+ CPU).
        let stop_timeout = std::time::Duration::from_secs(10);
        let mut stop_child = tokio::process::Command::new("container")
            .args(["stop", "-t", "1", &container_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
        if let Some(ref mut child) = stop_child {
            match tokio::time::timeout(stop_timeout, child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    // Timed out — kill the stop process itself
                    let _ = child.kill().await;
                }
            }
        }

        // Force delete (also with timeout)
        let mut del_child = tokio::process::Command::new("container")
            .args(["delete", "-f", &container_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
        if let Some(ref mut child) = del_child {
            match tokio::time::timeout(stop_timeout, child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    let _ = child.kill().await;
                }
            }
        }

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
        // Check container directly — don't rely on internal state since
        // we might be reconnecting to an existing container.
        // Note: Apple's `container ls` doesn't support --filter, so we
        // list all containers and check if ours is present.
        let container_name = self.container_name();
        Command::new("container")
            .args(["ls"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&container_name))
            .unwrap_or(false)
    }

    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> Result<()> {
        let container_name = self.container_name();

        // Ensure parent directory exists in container
        let parent = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let _ = Command::new("container")
            .args(["exec", &container_name, "mkdir", "-p", &parent])
            .output();

        // Write file via exec: pipe base64-encoded content through sh -c
        use base64::{Engine, engine::general_purpose::STANDARD};
        let encoded = STANDARD.encode(content);
        let decode_cmd = format!("echo '{}' | base64 -d > '{}'", encoded, path);
        let output = Command::new("container")
            .args(["exec", &container_name, "sh", "-c", &decode_cmd])
            .output()
            .context("Failed to write file in container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to write file: {}", stderr);
        }

        Ok(())
    }

    async fn read_file_unchecked(&mut self, path: &str) -> Result<Vec<u8>> {
        let container_name = self.container_name();

        // Read file via exec: base64-encode the content and decode on host
        let output = Command::new("container")
            .args(["exec", &container_name, "base64", path])
            .output()
            .context("Failed to read file from container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to read file: {}", stderr);
        }

        use base64::{Engine, engine::general_purpose::STANDARD};
        let decoded = STANDARD
            .decode(String::from_utf8_lossy(&output.stdout).trim())
            .context("Failed to decode base64 file content")?;

        Ok(decoded)
    }

    async fn remove_file_unchecked(&mut self, path: &str) -> Result<()> {
        let container_name = self.container_name();

        let output = Command::new("container")
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

        let output = Command::new("container")
            .args(&args)
            .output()
            .context("Failed to create directory in container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("mkdir failed: {}", stderr);
        }

        Ok(())
    }
}

impl Drop for AppleSandbox {
    fn drop(&mut self) {
        // Only clean up if running and not marked as persistent
        if self.running && !self.persistent {
            let container_name = self.container_name();
            let _ = Command::new("container")
                .args(["delete", "-f", &container_name])
                .output();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_local_image_snapshot_tags() {
        assert!(is_local_image("agentkernel-snap:test-snap"));
        assert!(is_local_image("agentkernel-snap:my-snapshot"));
        assert!(is_local_image("agentkernel-snap:v1"));
    }

    #[test]
    fn test_is_local_image_other_agentkernel_tags() {
        assert!(is_local_image("agentkernel-my-tools"));
        assert!(is_local_image("agentkernel-custom:latest"));
    }

    #[test]
    fn test_is_local_image_rejects_registry_images() {
        assert!(!is_local_image("alpine:3.20"));
        assert!(!is_local_image("python:3.12-alpine"));
        assert!(!is_local_image("docker.io/library/alpine"));
        assert!(!is_local_image("ghcr.io/user/image:latest"));
        assert!(!is_local_image("registry.example.com/foo"));
    }

    #[test]
    fn parses_current_system_status_json() {
        let status = r#"{"apiServerVersion":"1.2.2","status":"running"}"#;
        assert!(apple_system_status_is_running(status));
        assert!(!apple_system_status_is_running(r#"{"status":"stopped"}"#));
    }

    #[test]
    fn parses_legacy_system_status_text() {
        assert!(apple_system_status_is_running(
            "Apple container system is running"
        ));
        assert!(!apple_system_status_is_running(
            "Apple container system is stopped"
        ));
    }

    #[test]
    fn parses_current_container_ip() {
        let inspect = r#"[{"status":{"networks":[{"ipv4Address":"192.168.64.8/24"}]}}]"#;
        assert_eq!(parse_container_ip(inspect).as_deref(), Some("192.168.64.8"));
    }

    #[test]
    fn parses_legacy_container_ip() {
        let inspect = r#"[{"networks":[{"address":"192.168.64.9/24"}]}]"#;
        assert_eq!(parse_container_ip(inspect).as_deref(), Some("192.168.64.9"));
    }

    #[test]
    fn rejects_missing_or_invalid_container_ip() {
        assert_eq!(parse_container_ip("not json"), None);
        assert_eq!(parse_container_ip(r#"[{"status":{"networks":[]}}]"#), None);
    }
}
