use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "macos")]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::process::Stdio;

/// Find a runtime binary even when the desktop app was launched without the
/// user's interactive shell PATH.
fn find_runtime_binary(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(name) {
        return Some(path);
    }

    [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/Applications/Docker.app/Contents/Resources/bin",
    ]
    .iter()
    .map(|dir| PathBuf::from(dir).join(name))
    .find(|path| path.is_file())
}

fn command_succeeds(binary: &PathBuf, args: &[&str]) -> bool {
    Command::new(binary)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn prepare_backend_sync() -> Result<String, String> {
    if let Some(container) = find_runtime_binary("container") {
        if command_succeeds(&container, &["--version"]) {
            let mut child = Command::new(&container)
                .args(["system", "start"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("Could not start Apple Containers: {error}"))?;

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(b"Y\n");
            }

            let output = child
                .wait_with_output()
                .map_err(|error| format!("Could not start Apple Containers: {error}"))?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.success()
                || stderr.to_ascii_lowercase().contains("already")
                || stderr.to_ascii_lowercase().contains("running")
            {
                return Ok(
                    "Apple Containers is ready. Retry the health check to continue.".to_string(),
                );
            }

            return Err(format!(
                "Apple Containers could not start: {}",
                stderr.trim()
            ));
        }
    }

    if let Some(docker) = find_runtime_binary("docker") {
        if command_succeeds(&docker, &["version"]) {
            return Ok("Docker is ready. Retry the health check to continue.".to_string());
        }

        if PathBuf::from("/Applications/Docker.app").is_dir() {
            Command::new("/usr/bin/open")
                .args(["-a", "Docker"])
                .status()
                .map_err(|error| format!("Could not open Docker Desktop: {error}"))?;
            return Ok(
                "Docker Desktop is starting. Wait for it to finish, then retry the health check."
                    .to_string(),
            );
        }

        return Err(
            "Docker is installed but its daemon is not running. Start Docker Desktop, then retry."
                .to_string(),
        );
    }

    Err(
        "No local sandbox backend was found. Install Docker Desktop or Apple Containers, then retry."
            .to_string(),
    )
}

#[cfg(not(target_os = "macos"))]
fn prepare_backend_sync() -> Result<String, String> {
    if let Some(docker) = find_runtime_binary("docker") {
        if command_succeeds(&docker, &["version"]) {
            return Ok("Docker is ready. Retry the health check to continue.".to_string());
        }
    }

    Err("No local sandbox backend was found. Install and start Docker, then retry.".to_string())
}

/// Prepare an installed local backend and return guidance for the next step.
#[tauri::command(rename_all = "snake_case")]
pub async fn prepare_backend() -> Result<String, String> {
    tokio::task::spawn_blocking(prepare_backend_sync)
        .await
        .map_err(|error| format!("Backend setup failed: {error}"))?
}
