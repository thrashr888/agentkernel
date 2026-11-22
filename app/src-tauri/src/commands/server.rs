use std::net::IpAddr;
use std::path::{Path, PathBuf};
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
    #[cfg(target_os = "macos")]
    owned_control: Mutex<Option<OwnedControl>>,
}

#[cfg(target_os = "macos")]
struct OwnedControl {
    dir: tempfile::TempDir,
    selection: (String, String, Option<String>),
}

#[cfg(target_os = "macos")]
fn owned_selection(entry: &ServerEntry) -> (String, String, Option<String>) {
    (
        entry.name.clone(),
        entry.url.clone(),
        entry.config_path.clone(),
    )
}

impl Default for ServerProcess {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
            #[cfg(target_os = "macos")]
            owned_control: Mutex::new(None),
        }
    }
}

impl ServerProcess {
    #[cfg(target_os = "macos")]
    pub(super) fn managed_ssh_client(
        &self,
        app_state: &AppState,
    ) -> Option<crate::api_client::ApiClient> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};

        // Maintain the same lock order as child startup.
        let mut child = self.child.lock().ok()?;
        let current = child.as_mut()?;
        if current.try_wait().ok()?.is_some() {
            return None;
        }
        let settings = app_state.settings.lock().ok()?;
        let entry = active_owned_server(&settings)?;
        if entry
            .ssh_tunnel
            .as_ref()
            .is_some_and(|tunnel| tunnel.enabled)
        {
            return None;
        }
        let control = self.owned_control.lock().ok()?;
        let owned = control.as_ref()?;
        if owned.selection != owned_selection(entry) {
            return None;
        }
        let socket = owned.dir.path().join("ctl.sock");
        let meta = std::fs::symlink_metadata(&socket).ok()?;
        let dir_meta = std::fs::metadata(owned.dir.path()).ok()?;
        if !meta.file_type().is_socket()
            || meta.mode() & 0o777 != 0o600
            || dir_meta.mode() & 0o777 != 0o700
            || meta.uid() != dir_meta.uid()
        {
            return None;
        }
        crate::api_client::ApiClient::new_owned_unix(socket, entry.api_key.as_deref()).ok()
    }

    /// Stop the child owned by this app, if one is still running.
    pub fn stop(&self) -> Result<bool, String> {
        let mut child_lock = self.child.lock().map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let _owned = self.owned_control.lock().map_err(|e| e.to_string())?.take();
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

/// Stable configuration location owned by the desktop app. A sidecar always
/// receives this absolute path, so policy cannot change when the app is
/// launched from a different working directory.
pub fn default_local_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agentkernel-desktop")
        .join("agentkernel.toml")
}

pub fn config_path_for_server(entry: &ServerEntry) -> Result<PathBuf, String> {
    let configured = entry
        .config_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(default_local_config_path);
    if configured.exists() {
        std::fs::canonicalize(&configured)
            .map_err(|e| format!("Failed to canonicalize server config: {e}"))
    } else {
        let parent = configured
            .parent()
            .ok_or_else(|| "Server config has no parent directory".to_string())?;
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create server config directory: {e}"))?;
        }
        let parent = std::fs::canonicalize(parent)
            .map_err(|e| format!("Failed to resolve server config directory: {e}"))?;
        Ok(parent.join(
            configured
                .file_name()
                .ok_or_else(|| "Server config has no file name".to_string())?,
        ))
    }
}

/// The server schema requires [sandbox]. Use its defaults for every other
/// field; in particular, do not enable or synthesize a governance policy.
const DEFAULT_DESKTOP_SERVER_CONFIG: &str = "[sandbox]\nname = \"agentkernel-desktop\"\n";

fn startup_config_path(entry: &ServerEntry, default: &Path) -> Result<PathBuf, String> {
    use std::io::{ErrorKind, Write};

    let configured = match entry.config_path.as_deref() {
        Some(explicit) => PathBuf::from(explicit),
        None => {
            // Only an implicit, missing desktop default can be initialized.
            // Publish complete TOML without overwriting a concurrent writer.
            match std::fs::symlink_metadata(default) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    let parent = default.parent().ok_or_else(|| {
                        "Default desktop server config has no parent directory".to_string()
                    })?;
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!("Failed to create desktop server config directory: {e}")
                    })?;
                    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
                        format!("Failed to stage default desktop server config: {e}")
                    })?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        temp.as_file()
                            .set_permissions(std::fs::Permissions::from_mode(0o600))
                            .map_err(|e| {
                                format!("Failed to protect default desktop server config: {e}")
                            })?;
                    }
                    temp.write_all(DEFAULT_DESKTOP_SERVER_CONFIG.as_bytes())
                        .and_then(|()| temp.as_file().sync_all())
                        .map_err(|e| {
                            format!("Failed to write default desktop server config: {e}")
                        })?;
                    match temp.persist_noclobber(default) {
                        Ok(_) => {}
                        Err(e) if e.error.kind() == ErrorKind::AlreadyExists => {}
                        Err(e) => {
                            return Err(format!(
                                "Failed to publish default desktop server config: {}",
                                e.error
                            ))
                        }
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to inspect default desktop server config: {e}"
                    ))
                }
            }
            default.to_path_buf()
        }
    };
    let path = std::fs::canonicalize(&configured).map_err(|e| {
        format!(
            "Failed to open server config '{}': {e}",
            configured.display()
        )
    })?;
    if !path.is_file() {
        return Err(format!(
            "Server config '{}' is not a regular file",
            path.display()
        ));
    }
    Ok(path)
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

#[cfg(target_os = "macos")]
fn is_packaged_sidecar(binary: &std::path::Path) -> bool {
    // The signed app's sibling sidecar and the assembled developer sidecar
    // are version-matched; a PATH fallback might not implement this option.
    let built_sidecars = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    binary.parent() == Some(built_sidecars.as_path())
        || std::env::current_exe().is_ok_and(|exe| {
            exe.parent().is_some_and(|dir| {
                binary.parent() == Some(dir)
                    || binary.parent() == Some(dir.join("binaries").as_path())
            })
        })
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
    let config_path = startup_config_path(&entry, &default_local_config_path())?;

    let mut command = Command::new(&binary);
    command.args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()]);
    // Always pass the canonical path, including on first launch. This keeps
    // the server's policy/config resolution stable instead of depending on
    // whichever working directory launched the desktop app.
    command.args([
        "--config",
        config_path
            .to_str()
            .ok_or_else(|| "Server config path is not valid UTF-8".to_string())?,
    ]);
    #[cfg(target_os = "macos")]
    let mut control = server_process
        .owned_control
        .lock()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    {
        *control = None;
    }
    // Keep the existing TCP API, but offer managed SSH only through a
    // separately owned, ephemeral local socket. Ordinary startup still works
    // if the private socket directory cannot be created.
    #[cfg(target_os = "macos")]
    let private_control = if entry
        .ssh_tunnel
        .as_ref()
        .is_some_and(|tunnel| tunnel.enabled)
        || !is_packaged_sidecar(&binary)
    {
        None
    } else {
        tempfile::Builder::new()
            .prefix("ak-ui-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/private/tmp")
            .ok()
            .map(|dir| {
                command
                    .arg("--control-socket")
                    .arg(dir.path().join("ctl.sock"));
                OwnedControl {
                    dir,
                    selection: owned_selection(&entry),
                }
            })
    };
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start bundled server: {e}"))?;

    let pid = child.id();
    *child_lock = Some(child);
    #[cfg(target_os = "macos")]
    {
        *control = private_control;
    }
    Ok(format!("Server started (PID {pid}) on port {port}"))
}

/// Restart only the child process spawned by this desktop instance.
///
/// A remote or separately managed server is never represented by
/// `ServerProcess`, so it cannot be stopped or restarted through this path.
pub fn restart_owned_server(
    server_process: &ServerProcess,
    app_state: &AppState,
) -> Result<String, String> {
    let _ = server_process.stop()?;
    start_owned_server(server_process, app_state)
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
            config_path: None,
        }
    }

    #[test]
    fn first_start_creates_an_anchored_default_and_never_overwrites_policy() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("desktop/agentkernel.toml");
        let local = entry("http://localhost:18888", true);
        let path = startup_config_path(&local, &default).unwrap();
        assert_eq!(path, default.canonicalize().unwrap());
        let initial = std::fs::read_to_string(&path).unwrap();
        let toml: toml::Value = toml::from_str(&initial).unwrap();
        assert_eq!(
            toml["sandbox"]["name"].as_str(),
            Some("agentkernel-desktop")
        );
        assert!(toml.get("llm_governance").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let policy = "[sandbox]\nname = \"existing\"\n[llm_governance]\nenabled = true\n";
        std::fs::write(&path, policy).unwrap();
        assert_eq!(startup_config_path(&local, &default).unwrap(), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), policy);
    }

    #[test]
    fn missing_explicit_config_never_falls_back_or_creates_any_file() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("desktop/agentkernel.toml");
        let custom = dir.path().join("explicit/agentkernel.toml");
        let mut local = entry("http://localhost:18888", true);
        local.config_path = Some(custom.to_str().unwrap().to_string());
        assert!(startup_config_path(&local, &default).is_err());
        assert!(!custom.parent().unwrap().exists());
        assert!(!default.exists());

        std::fs::create_dir_all(custom.parent().unwrap()).unwrap();
        std::fs::write(&custom, "explicit").unwrap();
        assert_eq!(
            startup_config_path(&local, &default).unwrap(),
            custom.canonicalize().unwrap()
        );
        assert_eq!(std::fs::read_to_string(&custom).unwrap(), "explicit");
        assert!(!default.exists());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_default_symlink_is_not_replaced_or_followed() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("agentkernel.toml");
        let target = dir.path().join("missing.toml");
        std::os::unix::fs::symlink(&target, &default).unwrap();
        let local = entry("http://localhost:18888", true);
        assert!(startup_config_path(&local, &default).is_err());
        assert_eq!(std::fs::read_link(&default).unwrap(), target);
        assert!(!target.exists());
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
