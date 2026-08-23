use std::net::IpAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use tauri::State;

use crate::state::{AppState, ServerEntry, Settings};

fn terminate_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Tracks the server process owned by this desktop instance.
///
/// A process is only placed here after the app has spawned it. Existing
/// processes listening on a configured endpoint are never killed by the
/// desktop app, which keeps remote and user-managed server entries safe.
pub struct ServerProcess {
    pub child: Mutex<Option<Child>>,
}

impl Default for ServerProcess {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }
}

impl ServerProcess {
    /// Stop the child owned by this app, if one is still running.
    pub fn stop(&self) -> Result<bool, String> {
        let mut child_lock = self.child.lock().map_err(|e| e.to_string())?;
        let Some(mut child) = child_lock.take() else {
            return Ok(false);
        };

        // A server that exited on its own is already stopped. Treat a missing
        // process as success so shutdown remains best-effort and idempotent.
        match child.try_wait() {
            Ok(Some(_)) => Ok(false),
            Ok(None) => {
                child
                    .kill()
                    .map_err(|e| format!("Failed to stop bundled server: {e}"))?;
                let _ = child.wait();
                Ok(true)
            }
            Err(error) => {
                terminate_child(child);
                Err(format!("Failed to inspect bundled server: {error}"))
            }
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Tauri normally emits RunEvent::ExitRequested and calls stop_server
        // below. Drop is a second line of defense for crashes and other
        // teardown paths where the event callback is not reached.
        if let Ok(mut child_lock) = self.child.lock() {
            if let Some(mut child) = child_lock.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Return true only for an app-owned loopback server.
///
/// The `managed` flag is explicit so a user can keep a local server outside
/// the app lifecycle. Requiring a loopback host also prevents a malformed or
/// misconfigured entry from ever causing the desktop app to launch a process
/// intended for a remote endpoint.
pub fn owns_local_server(entry: &ServerEntry) -> bool {
    entry
        .managed
        .unwrap_or_else(|| entry.name.eq_ignore_ascii_case("local"))
        && url::Url::parse(&entry.url)
            .ok()
            .and_then(|url| url.host_str().map(is_loopback_host))
            .unwrap_or(false)
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn active_owned_server(settings: &Settings) -> Option<&ServerEntry> {
    settings.active().filter(|entry| owns_local_server(entry))
}

fn port_for_server(entry: &ServerEntry) -> u16 {
    url::Url::parse(&entry.url)
        .ok()
        .and_then(|url| url.port())
        .unwrap_or(18888)
}

/// Find the CLI binary shipped by Tauri, with development fallbacks.
///
/// Tauri's `externalBin` configuration places the normalized sidecar next to
/// the desktop executable in a release bundle. The normalized name includes
/// the target triple, so try that exact name before the unsuffixed development
/// fallback. The `binaries/` candidate is also useful for local `tauri dev`
/// layouts. PATH remains a deliberate fallback so contributors can run the
/// app without building a sidecar first.
fn find_agentkernel_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    let names = [
        option_env!("TAURI_ENV_TARGET_TRIPLE").map(|target| format!("agentkernel-{target}")),
        Some("agentkernel".to_string()),
    ];

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in names.iter().flatten() {
                candidates.push(dir.join(name));
                candidates.push(dir.join("binaries").join(name));
            }
        }
    }

    // `tauri dev` keeps the source-side binaries directory in place, while
    // release bundles copy the sidecar next to the app executable.
    for name in names.iter().flatten() {
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join(name),
        );
    }

    if let Ok(path) = which::which("agentkernel") {
        candidates.push(path);
    }

    candidates.extend(
        [
            "/opt/homebrew/bin/agentkernel",
            "/opt/homebrew/sbin/agentkernel",
            "/usr/local/bin/agentkernel",
            "/usr/local/sbin/agentkernel",
        ]
        .into_iter()
        .map(PathBuf::from),
    );

    candidates.into_iter().find(|path| path.is_file())
}

/// Start the app-owned local server, if the active entry opts into ownership.
///
/// This is called from Tauri setup as well as the manual start command, so the
/// same ownership and binary-selection rules apply in both paths.
pub fn start_owned_server(
    server_process: &ServerProcess,
    app_state: &AppState,
) -> Result<String, String> {
    let mut child_lock = server_process.child.lock().map_err(|e| e.to_string())?;

    let stale_child = if let Some(ref mut child) = *child_lock {
        match child.try_wait() {
            Ok(None) => return Ok("Server already running".to_string()),
            Ok(Some(_)) => true,
            Err(_) => true,
        }
    } else {
        false
    };
    if stale_child {
        if let Some(child) = child_lock.take() {
            terminate_child(child);
        }
    }

    let entry = {
        let settings = app_state.settings.lock().map_err(|e| e.to_string())?;
        active_owned_server(&settings).cloned()
    };
    let Some(entry) = entry else {
        return Ok("Active server is external; leaving its lifecycle unchanged".to_string());
    };

    let binary = find_agentkernel_binary().ok_or_else(|| {
        "Bundled agentkernel sidecar is unavailable. Build the desktop sidecar or install 'agentkernel' for development.".to_string()
    })?;
    let port = port_for_server(&entry);

    let child = Command::new(&binary)
        .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start bundled server: {e}"))?;

    let pid = child.id();
    *child_lock = Some(child);
    Ok(format!("Server started (PID {pid}) on port {port}"))
}

/// Start the app-owned server when the desktop application launches.
pub fn auto_start_server(server_process: &ServerProcess, app_state: &AppState) {
    match start_owned_server(server_process, app_state) {
        Ok(message) if message != "Active server is external; leaving its lifecycle unchanged" => {
            eprintln!("{message}");
        }
        Ok(_) => {}
        Err(error) => eprintln!("AgentKernel server was not started automatically: {error}"),
    }
}

/// Start the app-owned server on demand. External entries are left untouched
/// rather than spawning a second process against a remote URL.
#[tauri::command(rename_all = "snake_case")]
pub async fn start_server(
    server_process: State<'_, ServerProcess>,
    app_state: State<'_, AppState>,
) -> Result<String, String> {
    start_owned_server(&server_process, &app_state)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn stop_server(server_process: State<'_, ServerProcess>) -> Result<String, String> {
    if server_process.stop()? {
        Ok("Server stopped".to_string())
    } else {
        Ok("Server not running".to_string())
    }
}

/// Check if the process owned by this app is alive. This does not probe or
/// mutate any external server configured in the UI.
#[tauri::command(rename_all = "snake_case")]
pub async fn server_status(server_process: State<'_, ServerProcess>) -> Result<bool, String> {
    let mut child_lock = server_process.child.lock().map_err(|e| e.to_string())?;

    if let Some(ref mut child) = *child_lock {
        match child.try_wait() {
            Ok(None) => Ok(true),
            Ok(Some(_)) => {
                *child_lock = None;
                Ok(false)
            }
            Err(_) => {
                if let Some(child) = child_lock.take() {
                    terminate_child(child);
                }
                Ok(false)
            }
        }
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, managed: bool) -> ServerEntry {
        ServerEntry {
            name: "test".to_string(),
            url: url.to_string(),
            api_key: None,
            managed: Some(managed),
            ssh_tunnel: None,
        }
    }

    #[test]
    fn only_managed_loopback_entries_are_owned() {
        assert!(owns_local_server(&entry("http://localhost:18888", true)));
        assert!(owns_local_server(&entry("http://127.0.0.1:18888", true)));
        assert!(owns_local_server(&entry("http://[::1]:18888", true)));
        assert!(!owns_local_server(&entry("http://localhost:18888", false)));
        assert!(!owns_local_server(&entry("https://example.com", true)));

        let mut legacy_local = entry("http://localhost:18888", false);
        legacy_local.name = "Local".to_string();
        legacy_local.managed = None;
        assert!(owns_local_server(&legacy_local));

        let mut legacy_remote = legacy_local.clone();
        legacy_remote.url = "https://example.com".to_string();
        assert!(!owns_local_server(&legacy_remote));
    }

    #[test]
    fn missing_or_invalid_ports_use_the_server_default() {
        assert_eq!(port_for_server(&entry("http://localhost", true)), 18888);
        assert_eq!(port_for_server(&entry("not a url", true)), 18888);
        assert_eq!(
            port_for_server(&entry("http://localhost:19999", true)),
            19999
        );
    }
}
