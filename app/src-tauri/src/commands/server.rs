use std::sync::Mutex;
use tauri::State;

/// Tracks the server child process.
pub struct ServerProcess {
    pub child: Mutex<Option<std::process::Child>>,
}

impl Default for ServerProcess {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }
}

/// Find the `agentkernel` binary. Checks:
/// 1. Next to this executable (release bundle)
/// 2. PATH lookup
fn find_agentkernel_binary() -> Option<std::path::PathBuf> {
    // Check next to our own binary (bundled release)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("agentkernel");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // Fall back to PATH
    if let Ok(path) = which::which("agentkernel") {
        return Some(path);
    }

    [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
    ]
    .iter()
    .map(|dir| std::path::PathBuf::from(dir).join("agentkernel"))
    .find(|path| path.is_file())
}

/// Start the agentkernel server if not already running.
#[tauri::command(rename_all = "snake_case")]
pub async fn start_server(
    server_process: State<'_, ServerProcess>,
    app_state: State<'_, crate::state::AppState>,
) -> Result<String, String> {
    let mut child_lock = server_process.child.lock().map_err(|e| e.to_string())?;

    // Check if already running
    if let Some(ref mut child) = *child_lock {
        match child.try_wait() {
            Ok(None) => return Ok("Server already running".to_string()),
            Ok(Some(_)) => {
                // Exited, clear it
                *child_lock = None;
            }
            Err(_) => {
                *child_lock = None;
            }
        }
    }

    let binary = find_agentkernel_binary().ok_or_else(|| {
        "Could not find 'agentkernel' binary. Install it or add it to PATH.".to_string()
    })?;

    // Read settings to get the port from the active server URL
    let port = {
        let settings = app_state.settings.lock().map_err(|e| e.to_string())?;
        let url = settings
            .active()
            .map(|s| s.url.clone())
            .unwrap_or_else(|| settings.api_url.clone());
        // Parse port from URL, default to 18888
        url::Url::parse(&url)
            .ok()
            .and_then(|u| u.port())
            .unwrap_or(18888)
    };

    let child = std::process::Command::new(&binary)
        .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start server: {e}"))?;

    let pid = child.id();
    *child_lock = Some(child);

    Ok(format!("Server started (PID {pid}) on port {port}"))
}

/// Stop the running agentkernel server.
#[tauri::command(rename_all = "snake_case")]
pub async fn stop_server(server_process: State<'_, ServerProcess>) -> Result<String, String> {
    let mut child_lock = server_process.child.lock().map_err(|e| e.to_string())?;

    if let Some(ref mut child) = *child_lock {
        child
            .kill()
            .map_err(|e| format!("Failed to kill server: {e}"))?;
        let _ = child.wait();
        *child_lock = None;
        Ok("Server stopped".to_string())
    } else {
        Ok("Server not running".to_string())
    }
}

/// Check if the managed server process is alive.
#[tauri::command(rename_all = "snake_case")]
pub async fn server_status(server_process: State<'_, ServerProcess>) -> Result<bool, String> {
    let mut child_lock = server_process.child.lock().map_err(|e| e.to_string())?;

    if let Some(ref mut child) = *child_lock {
        match child.try_wait() {
            Ok(None) => Ok(true), // still running
            _ => {
                *child_lock = None;
                Ok(false)
            }
        }
    } else {
        Ok(false)
    }
}
