//! App-owned SSH forwarding for remote AgentKernel servers.
//!
//! A tunnel is deliberately opt-in. The manager stores the exact `Child` it
//! spawned and only ever terminates that child; it does not inspect or kill
//! unrelated SSH processes and it never writes the user's SSH configuration.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{Manager, State};

use crate::api_client::ApiClient;
use crate::state::{AppState, ServerEntry, Settings};

const DEFAULT_AGENTKERNEL_PORT: u16 = 18_888;
const DEFAULT_REMOTE_HOST: &str = "127.0.0.1";
const SERVER_ALIVE_INTERVAL: &str = "15";
const SERVER_ALIVE_COUNT_MAX: &str = "3";

/// Public state shown by the desktop while an optional tunnel is starting or
/// failing. Strings keep this compatible with older frontend builds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TunnelStatus {
    pub state: String,
    pub server_name: Option<String>,
    pub local_url: Option<String>,
    pub error: Option<String>,
}

impl Default for TunnelStatus {
    fn default() -> Self {
        Self {
            state: "disabled".to_string(),
            server_name: None,
            local_url: None,
            error: None,
        }
    }
}

struct ManagedTunnel {
    child: Child,
    server_name: String,
    local_url: String,
    identity: Vec<String>,
}

/// Owns at most one SSH process for the active desktop server.
pub struct TunnelManager {
    /// Serializes connect/disconnect transitions so startup, settings saves,
    /// and a manual retry cannot replace one another mid-health-check.
    operation: tokio::sync::Mutex<()>,
    process: Mutex<Option<ManagedTunnel>>,
    /// A replacement remains separate from the current healthy tunnel until
    /// its API health check succeeds. This makes A -> B failures recoverable.
    pending: Mutex<Option<ManagedTunnel>>,
    status: Mutex<TunnelStatus>,
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self {
            operation: tokio::sync::Mutex::new(()),
            process: Mutex::new(None),
            pending: Mutex::new(None),
            status: Mutex::new(TunnelStatus::default()),
        }
    }
}

impl TunnelManager {
    pub fn status(&self) -> Result<TunnelStatus, String> {
        // Always acquire process before pending. Keep the guards limited to
        // non-blocking child inspection and removal; stderr reads and waits
        // happen after releasing both locks.
        let (current_exit, pending_exit) = {
            let mut process = self.process.lock().map_err(|e| e.to_string())?;
            let mut pending = self.pending.lock().map_err(|e| e.to_string())?;
            let current_exit = match process.as_mut().map(|tunnel| tunnel.child.try_wait()) {
                Some(Ok(None)) | None => None,
                Some(Ok(Some(_))) => {
                    let tunnel = process.take().expect("tunnel exists");
                    Some(Ok(tunnel))
                }
                Some(Err(error)) => {
                    let tunnel = process.take().expect("tunnel exists");
                    Some(Err((tunnel, error.to_string())))
                }
            };
            let pending_exit = match pending.as_mut().map(|tunnel| tunnel.child.try_wait()) {
                Some(Ok(None)) | None => None,
                Some(Ok(Some(exit))) => {
                    let tunnel = pending.take().expect("pending tunnel exists");
                    Some(Ok((tunnel, exit.code())))
                }
                Some(Err(error)) => {
                    let tunnel = pending.take().expect("pending tunnel exists");
                    Some(Err((tunnel, error.to_string())))
                }
            };
            (current_exit, pending_exit)
        };

        if let Some(current_exit) = current_exit {
            match current_exit {
                Ok(tunnel) => {
                    let name = tunnel.server_name.clone();
                    terminate_managed(tunnel.child);
                    self.set_status(TunnelStatus {
                        state: "error".to_string(),
                        server_name: Some(name),
                        local_url: None,
                        error: Some("The SSH tunnel exited unexpectedly".to_string()),
                    })?;
                }
                Err((tunnel, error)) => {
                    terminate_managed(tunnel.child);
                    self.set_status(TunnelStatus {
                        state: "error".to_string(),
                        server_name: None,
                        local_url: None,
                        error: Some(format!("Unable to inspect the SSH tunnel: {error}")),
                    })?;
                }
            }
        }
        if let Some(pending_exit) = pending_exit {
            match pending_exit {
                Ok((mut tunnel, code)) => {
                    let name = tunnel.server_name.clone();
                    let detail = read_stderr(&mut tunnel.child);
                    terminate_managed(tunnel.child);
                    self.set_status(TunnelStatus {
                        state: "error".to_string(),
                        server_name: Some(name),
                        local_url: None,
                        error: Some(format_ssh_failure(code, detail.as_deref())),
                    })?;
                }
                Err((tunnel, error)) => {
                    terminate_managed(tunnel.child);
                    self.set_status(TunnelStatus {
                        state: "error".to_string(),
                        server_name: None,
                        local_url: None,
                        error: Some(format!("Unable to inspect the SSH tunnel process: {error}")),
                    })?;
                }
            }
        }
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|e| e.to_string())
    }

    fn set_status(&self, status: TunnelStatus) -> Result<(), String> {
        let mut current = self.status.lock().map_err(|e| e.to_string())?;
        *current = status;
        Ok(())
    }

    /// Start an app-owned tunnel and return the local URL that should be used
    /// only after it passes an API health check.
    pub fn start(&self, entry: &ServerEntry) -> Result<String, String> {
        let validated = validate_tunnel_entry(entry)?;
        let local_url = validated.local_url.clone();
        let command_args = build_ssh_args(&validated);

        let stale_current = {
            let mut process = self.process.lock().map_err(|e| e.to_string())?;
            let existing = process.as_mut();
            if let Some(existing) = existing.filter(|existing| {
                existing.server_name == entry.name
                    && existing.local_url == local_url
                    && existing.identity == command_args
            }) {
                match existing.child.try_wait() {
                    Ok(None) => return Ok(local_url),
                    Ok(Some(_)) | Err(_) => process.take(),
                }
            } else {
                None
            }
        };
        if let Some(stale_current) = stale_current {
            terminate_managed(stale_current.child);
        }

        // A previous replacement that has not yet been health-checked is no
        // longer useful. It is app-owned, so it is safe to terminate it.
        let old_pending = {
            let _process = self.process.lock().map_err(|e| e.to_string())?;
            self.pending.lock().map_err(|e| e.to_string())?.take()
        };
        if let Some(old_pending) = old_pending {
            terminate_managed(old_pending.child);
        }

        self.set_status(TunnelStatus {
            state: "starting".to_string(),
            server_name: Some(entry.name.clone()),
            local_url: Some(local_url.clone()),
            error: None,
        })?;

        let mut command = Command::new("ssh");
        command
            .args(&command_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let child = command.spawn().map_err(|error| {
            let message = format!(
                "Unable to start SSH tunnel: {error}. Install OpenSSH or check that `ssh` is on PATH."
            );
            let _ = self.set_status(TunnelStatus {
                state: "error".to_string(),
                server_name: Some(entry.name.clone()),
                local_url: None,
                error: Some(message.clone()),
            });
            message
        })?;

        let mut tunnel = ManagedTunnel {
            child,
            server_name: entry.name.clone(),
            local_url: local_url.clone(),
            identity: command_args,
        };
        if let Some(exit) = tunnel
            .child
            .try_wait()
            .map_err(|error| format!("Unable to inspect the SSH tunnel process: {error}"))?
        {
            let detail = read_stderr(&mut tunnel.child);
            let message = format_ssh_failure(exit.code(), detail.as_deref());
            self.set_status(TunnelStatus {
                state: "error".to_string(),
                server_name: Some(entry.name.clone()),
                local_url: None,
                error: Some(message.clone()),
            })?;
            return Err(message);
        }

        let _process = self.process.lock().map_err(|e| e.to_string())?;
        self.pending
            .lock()
            .map_err(|e| e.to_string())?
            .replace(tunnel);
        Ok(local_url)
    }

    /// Mark a tunnel healthy only after the API client has checked its local
    /// endpoint. This prevents a client URL switch on a half-open tunnel.
    pub fn mark_connected(&self, server_name: &str, local_url: &str) -> Result<(), String> {
        let old = {
            let mut process = self.process.lock().map_err(|e| e.to_string())?;
            let candidate = self.pending.lock().map_err(|e| e.to_string())?.take();
            candidate.and_then(|candidate| process.replace(candidate))
        };
        if let Some(old) = old {
            terminate_managed(old.child);
        }
        self.set_status(TunnelStatus {
            state: "connected".to_string(),
            server_name: Some(server_name.to_string()),
            local_url: Some(local_url.to_string()),
            error: None,
        })
    }

    pub fn mark_error(&self, server_name: &str, error: String) -> Result<(), String> {
        self.set_status(TunnelStatus {
            state: "error".to_string(),
            server_name: Some(server_name.to_string()),
            local_url: None,
            error: Some(error),
        })
    }

    /// Drop an unverified replacement while retaining the current healthy
    /// tunnel, if any.
    pub fn abort_pending(&self) -> Result<(), String> {
        let pending = {
            let _process = self.process.lock().map_err(|e| e.to_string())?;
            self.pending.lock().map_err(|e| e.to_string())?.take()
        };
        if let Some(pending) = pending {
            terminate_managed(pending.child);
        }
        Ok(())
    }

    fn has_pending(&self) -> Result<bool, String> {
        let _process = self.process.lock().map_err(|e| e.to_string())?;
        self.pending
            .lock()
            .map(|pending| pending.is_some())
            .map_err(|e| e.to_string())
    }

    fn stop_unlocked(&self) -> Result<bool, String> {
        let (pending, tunnel) = {
            let mut process = self.process.lock().map_err(|e| e.to_string())?;
            let pending = self.pending.lock().map_err(|e| e.to_string())?.take();
            let tunnel = process.take();
            (pending, tunnel)
        };
        let had_pending = pending.is_some();
        if let Some(pending) = pending {
            terminate_managed(pending.child);
        }
        let Some(tunnel) = tunnel else {
            self.set_status(TunnelStatus::default())?;
            return Ok(had_pending);
        };
        terminate_managed(tunnel.child);
        self.set_status(TunnelStatus::default())?;
        Ok(true)
    }

    /// Stop only the SSH child created by this manager, serializing against an
    /// in-flight health-checked connect operation.
    pub async fn stop(&self) -> Result<bool, String> {
        let _operation = self.operation.lock().await;
        self.stop_unlocked()
    }

    /// Synchronous shutdown path used by Tauri's exit callback. Tauri has
    /// already stopped dispatching new work at this point.
    pub fn stop_now(&self) -> Result<bool, String> {
        self.stop_unlocked()
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.lock() {
            if let Ok(mut pending) = self.pending.lock() {
                let tunnel = process.take();
                let replacement = pending.take();
                drop(pending);
                drop(process);
                if let Some(tunnel) = tunnel {
                    terminate_managed(tunnel.child);
                }
                if let Some(replacement) = replacement {
                    terminate_managed(replacement.child);
                }
            }
        }
    }
}

fn terminate_managed(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_stderr(child: &mut Child) -> Option<String> {
    let mut output = String::new();
    child
        .stderr
        .as_mut()
        .and_then(|stderr| stderr.read_to_string(&mut output).ok())?;
    let output = output.trim().to_string();
    (!output.is_empty()).then_some(output)
}

fn format_ssh_failure(code: Option<i32>, detail: Option<&str>) -> String {
    let status = code.map_or_else(
        || "terminated by signal".to_string(),
        |code| format!("exited with code {code}"),
    );
    match detail {
        Some(detail) => format!("SSH tunnel {status}: {detail}"),
        None => format!(
            "SSH tunnel {status}. Check the SSH host alias, key, and remote AgentKernel port."
        ),
    }
}

pub(crate) struct ValidatedTunnel {
    ssh_host: String,
    ssh_user: Option<String>,
    ssh_port: Option<u16>,
    remote_host: String,
    remote_port: u16,
    local_port: u16,
    local_url: String,
}

/// Validate every value that becomes an argument to `ssh`, and require a
/// remote AgentKernel URL that is actually remote when tunnelling is enabled.
pub fn validate_tunnel_entry(entry: &ServerEntry) -> Result<ValidatedTunnel, String> {
    let config = entry
        .ssh_tunnel
        .as_ref()
        .filter(|config| config.enabled)
        .ok_or_else(|| "SSH tunnel management is not enabled for this server".to_string())?;
    if entry.name.trim().is_empty() {
        return Err("Server name cannot be empty".to_string());
    }

    let url =
        url::Url::parse(&entry.url).map_err(|error| format!("Invalid server URL: {error}"))?;
    if url.scheme() != "http" {
        if url.scheme() == "https" {
            return Err(
                "SSH tunnels currently require an HTTP AgentKernel URL; HTTPS certificates are tied to the remote hostname"
                    .to_string(),
            );
        }
        return Err("Server URL must use http or https".to_string());
    }
    if url.host_str().is_none() || is_loopback_host(url.host_str().unwrap_or_default()) {
        return Err("SSH tunnels require a non-loopback server URL".to_string());
    }
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Server URL cannot contain credentials, a query, or a fragment".to_string());
    }
    if !url.path().is_empty() && url.path() != "/" {
        return Err("Server URL must point to the AgentKernel server root".to_string());
    }

    validate_ssh_host(&config.ssh_host)?;
    if let Some(user) = config.ssh_user.as_deref() {
        validate_ssh_user(user)?;
    }
    validate_optional_port(config.ssh_port, "SSH")?;

    let remote_host = config
        .remote_host
        .as_deref()
        .unwrap_or(DEFAULT_REMOTE_HOST)
        .to_string();
    validate_remote_host(&remote_host)?;

    let remote_port = config
        .remote_port
        .or_else(|| url.port())
        .unwrap_or(DEFAULT_AGENTKERNEL_PORT);
    validate_port(remote_port, "remote AgentKernel")?;

    let local_port = match config.local_port {
        Some(port) => {
            validate_port(port, "local")?;
            // Do not hold a probe socket here: releasing it before OpenSSH
            // binds would introduce a race. ExitOnForwardFailure and the
            // health-check path report an occupied port without touching the
            // process that currently owns it.
            port
        }
        None => TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("Unable to choose a local loopback port: {error}"))?
            .local_addr()
            .map_err(|error| format!("Unable to inspect the chosen local port: {error}"))?
            .port(),
    };

    Ok(ValidatedTunnel {
        ssh_host: config.ssh_host.clone(),
        ssh_user: config.ssh_user.clone(),
        ssh_port: config.ssh_port,
        remote_host,
        remote_port,
        local_port,
        local_url: format!("{}://127.0.0.1:{local_port}", url.scheme()),
    })
}

fn validate_ssh_host(host: &str) -> Result<(), String> {
    if host.is_empty()
        || host.len() > 255
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err("SSH host must be a host/config alias containing only letters, numbers, '.', '-' or '_'".to_string());
    }
    Ok(())
}

fn validate_ssh_user(user: &str) -> Result<(), String> {
    let valid = !user.is_empty()
        && user.len() <= 64
        && user.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (matches!(byte, b'.' | b'_' | b'-') && index > 0)
        })
        && user.as_bytes()[0].is_ascii_alphanumeric();
    if !valid {
        return Err(
            "SSH user must be a simple account name without shell or host syntax".to_string(),
        );
    }
    Ok(())
}

fn validate_remote_host(host: &str) -> Result<(), String> {
    if is_loopback_host(host) && !host.contains(['/', '\\', ' ', '\t', '\n']) {
        Ok(())
    } else {
        Err("Remote AgentKernel bind must be localhost or a loopback IP".to_string())
    }
}

fn validate_port(port: u16, label: &str) -> Result<(), String> {
    if port == 0 {
        Err(format!("{label} port must be between 1 and 65535"))
    } else {
        Ok(())
    }
}

fn validate_optional_port(port: Option<u16>, label: &str) -> Result<(), String> {
    port.map_or(Ok(()), |port| validate_port(port, &format!("{label} SSH")))
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

fn forward_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// Build argument values for a direct `Command`, without shell expansion.
pub fn build_ssh_args(config: &ValidatedTunnel) -> Vec<String> {
    let mut args = vec![
        "-N".to_string(),
        "-T".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        format!("ServerAliveInterval={SERVER_ALIVE_INTERVAL}"),
        "-o".to_string(),
        format!("ServerAliveCountMax={SERVER_ALIVE_COUNT_MAX}"),
        "-L".to_string(),
        format!(
            "127.0.0.1:{}:{}:{}",
            config.local_port,
            forward_host(&config.remote_host),
            config.remote_port
        ),
    ];
    if let Some(port) = config.ssh_port {
        args.extend(["-p".to_string(), port.to_string()]);
    }
    if let Some(user) = &config.ssh_user {
        args.extend(["-l".to_string(), user.clone()]);
    }
    // End options before the validated host alias. This is defense in depth
    // for OpenSSH implementations that treat a leading '-' destination as an
    // option, even though validation already rejects that form.
    args.extend(["--".to_string(), config.ssh_host.clone()]);
    args
}

fn tunnel_entry(settings: &Settings) -> Option<ServerEntry> {
    settings.active().cloned()
}

pub async fn connect_entry(
    app_state: &AppState,
    manager: &TunnelManager,
    entry: ServerEntry,
) -> Result<(), String> {
    let _operation = manager.operation.lock().await;
    let local_url = manager.start(&entry)?;
    let client = ApiClient::new(&local_url, entry.api_key.as_deref());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_error = None;
    let mut health_ok = false;
    loop {
        let health = tokio::time::timeout(Duration::from_millis(750), client.health()).await;
        match health {
            Ok(Ok(_)) => {
                health_ok = true;
                break;
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => last_error = Some("health check timed out".to_string()),
        }

        if let Ok(status) = manager.status() {
            if status.state == "error"
                && status.server_name.as_deref() == Some(entry.name.as_str())
                && !manager.has_pending()?
            {
                last_error = status.error;
                break;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    if !health_ok {
        let error = last_error.unwrap_or_else(|| "health check failed".to_string());
        let detail = manager
            .status()
            .ok()
            .and_then(|status| status.error)
            .unwrap_or(error);
        // A failed replacement must not tear down the last healthy tunnel.
        // Only the unverified, app-owned pending child is cleaned up.
        let _ = manager.abort_pending();
        let message = format!("SSH tunnel started but AgentKernel health check failed: {detail}");
        let _ = manager.mark_error(&entry.name, message.clone());
        return Err(message);
    }

    manager.mark_connected(&entry.name, &local_url)?;
    let mut shared_client = app_state.client.lock().map_err(|e| e.to_string())?;
    *shared_client = client;
    Ok(())
}

/// Start and health-check the tunnel for the active configured server.
pub async fn connect_active(app_state: &AppState, manager: &TunnelManager) -> Result<(), String> {
    let entry = {
        let settings = app_state.settings.lock().map_err(|e| e.to_string())?;
        tunnel_entry(&settings)
    };
    let Some(entry) = entry else {
        let _ = manager.stop().await;
        return Ok(());
    };
    if entry
        .ssh_tunnel
        .as_ref()
        .is_none_or(|config| !config.enabled)
    {
        let _ = manager.stop().await;
        let settings = app_state
            .settings
            .lock()
            .map_err(|e| e.to_string())?
            .clone();
        let client = AppState::client_from_settings(&settings);
        *app_state.client.lock().map_err(|e| e.to_string())? = client;
        return Ok(());
    }
    connect_entry(app_state, manager, entry).await
}

pub fn auto_start(app_handle: &tauri::AppHandle) {
    let handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = connect_active(
            handle.state::<AppState>().inner(),
            handle.state::<TunnelManager>().inner(),
        )
        .await
        {
            eprintln!("SSH tunnel was not started automatically: {error}");
        }
    });
}

#[tauri::command(rename_all = "snake_case")]
pub async fn start_tunnel(
    state: State<'_, AppState>,
    manager: State<'_, TunnelManager>,
) -> Result<TunnelStatus, String> {
    connect_active(&state, &manager).await?;
    manager.status()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn stop_tunnel(
    state: State<'_, AppState>,
    manager: State<'_, TunnelManager>,
) -> Result<TunnelStatus, String> {
    manager.stop().await?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let client = AppState::client_from_settings(&settings);
    *state.client.lock().map_err(|e| e.to_string())? = client;
    manager.status()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tunnel_status(manager: State<'_, TunnelManager>) -> Result<TunnelStatus, String> {
    manager.status()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SshTunnelConfig;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;

    fn entry(config: SshTunnelConfig) -> ServerEntry {
        ServerEntry {
            name: "Rookery".to_string(),
            url: "http://rookery.example:18888".to_string(),
            api_key: None,
            managed: Some(false),
            ssh_tunnel: Some(config),
            config_path: None,
        }
    }

    fn config() -> SshTunnelConfig {
        SshTunnelConfig {
            enabled: true,
            ssh_host: "rookery".to_string(),
            ssh_user: Some("paul".to_string()),
            ssh_port: Some(22),
            remote_host: Some("127.0.0.1".to_string()),
            remote_port: Some(18888),
            local_port: Some(49152),
        }
    }

    #[test]
    fn command_uses_explicit_safe_arguments() {
        let validated = validate_tunnel_entry(&entry(config())).unwrap();
        assert_eq!(
            build_ssh_args(&validated),
            vec![
                "-N",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-L",
                "127.0.0.1:49152:127.0.0.1:18888",
                "-p",
                "22",
                "-l",
                "paul",
                "--",
                "rookery"
            ]
        );
    }

    #[test]
    fn validation_rejects_shell_syntax_and_non_loopback_targets() {
        let mut bad = config();
        bad.ssh_host = "rookery; touch /tmp/pwned".to_string();
        assert!(validate_tunnel_entry(&entry(bad)).is_err());

        let mut bad = config();
        bad.remote_host = Some("10.0.0.4".to_string());
        assert!(validate_tunnel_entry(&entry(bad)).is_err());

        let mut bad = config();
        bad.local_port = Some(0);
        assert!(validate_tunnel_entry(&entry(bad)).is_err());

        let mut bad = config();
        bad.ssh_host = "-oProxyCommand=evil".to_string();
        assert!(validate_tunnel_entry(&entry(bad)).is_err());
    }

    #[test]
    fn tunnel_requires_explicit_opt_in_and_remote_url() {
        let mut disabled = config();
        disabled.enabled = false;
        assert!(validate_tunnel_entry(&entry(disabled)).is_err());

        let mut local = entry(config());
        local.url = "http://localhost:18888".to_string();
        assert!(validate_tunnel_entry(&local).is_err());

        let mut https = entry(config());
        https.url = "https://rookery.example:18888".to_string();
        assert!(validate_tunnel_entry(&https).is_err());
    }

    #[test]
    fn chooses_loopback_url_without_mutating_remote_entry() {
        let original = entry(config());
        let validated = validate_tunnel_entry(&original).unwrap();
        assert_eq!(validated.local_url, "http://127.0.0.1:49152");
        assert_eq!(original.url, "http://rookery.example:18888");
    }

    #[test]
    fn manager_stop_is_idempotent_without_touching_external_processes() {
        let manager = TunnelManager::default();
        assert!(!manager.stop_now().unwrap());
        assert_eq!(manager.status().unwrap().state, "disabled");
    }

    #[test]
    fn failed_replacement_aborts_only_pending_child_and_keeps_current() {
        let manager = TunnelManager::default();
        let current = ManagedTunnel {
            child: Command::new("sleep").arg("60").spawn().unwrap(),
            server_name: "A".to_string(),
            local_url: "http://127.0.0.1:41001".to_string(),
            identity: vec!["A".to_string()],
        };
        let pending = ManagedTunnel {
            child: Command::new("sleep").arg("60").spawn().unwrap(),
            server_name: "B".to_string(),
            local_url: "http://127.0.0.1:41002".to_string(),
            identity: vec!["B".to_string()],
        };
        manager.process.lock().unwrap().replace(current);
        manager.pending.lock().unwrap().replace(pending);

        manager.abort_pending().unwrap();
        assert!(manager.process.lock().unwrap().is_some());
        assert!(manager.pending.lock().unwrap().is_none());
        manager.stop_now().unwrap();
    }

    #[test]
    fn concurrent_status_and_transitions_do_not_deadlock() {
        let manager = Arc::new(TunnelManager::default());
        let barrier = Arc::new(Barrier::new(8));
        let (done_tx, done_rx) = mpsc::channel();
        let mut handles = Vec::new();

        for worker in 0..8 {
            let manager = Arc::clone(&manager);
            let barrier = Arc::clone(&barrier);
            let done_tx = done_tx.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    match worker % 3 {
                        0 => {
                            let _ = manager.status();
                        }
                        1 => {
                            let _ = manager.mark_connected("test", "http://127.0.0.1:1");
                        }
                        _ => {
                            let _ = manager.stop_now();
                        }
                    }
                }
                done_tx.send(()).unwrap();
            }));
        }
        drop(done_tx);

        for _ in 0..8 {
            assert!(
                done_rx.recv_timeout(Duration::from_secs(2)).is_ok(),
                "concurrent tunnel transition did not complete"
            );
        }
        for handle in handles {
            handle.join().unwrap();
        }
    }
}
