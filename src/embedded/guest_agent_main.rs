//! Agentkernel Guest Agent
//!
//! Lightweight agent that runs inside microVMs to handle commands from the host.
//! Communicates over virtio-vsock using a JSON-RPC protocol.
//!
//! Supports:
//! - Command execution (Run)
//! - Interactive shell sessions (Shell, ShellInput, ShellResize, ShellClose)
//! - File operations (WriteFile, ReadFile, RemoveFile, Mkdir)
//! - Health check (Ping) and shutdown (Shutdown)

mod pty;

use anyhow::{Context, Result};
use pty::SessionManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_vsock::{VsockAddr, VsockListener};

/// Default port to listen on
const AGENT_PORT: u32 = 52000;

/// Listen on any CID
const VMADDR_CID_ANY: u32 = u32::MAX;

/// Request types supported by the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestType {
    /// Run a command and return output
    Run,
    /// Start an interactive shell session (PTY)
    Shell,
    /// Send input to a shell session
    ShellInput,
    /// Resize a shell session's terminal
    ShellResize,
    /// Close a shell session
    ShellClose,
    /// Health check
    Ping,
    /// Graceful shutdown
    Shutdown,
    /// Write a file to the guest filesystem
    WriteFile,
    /// Read a file from the guest filesystem
    ReadFile,
    /// Remove a file from the guest filesystem
    RemoveFile,
    /// Create a directory in the guest filesystem
    Mkdir,
}

/// Shell event types for async shell communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellEvent {
    /// Shell session started
    Started,
    /// Shell produced output
    Output,
    /// Shell session exited
    Exited,
}

/// Request from host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub request_type: RequestType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// File path (for file operations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// File content as base64 (for WriteFile)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    /// Whether to create parent directories (for Mkdir)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,
    // Shell-specific fields
    /// Session ID (for ShellInput, ShellResize, ShellClose)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Terminal rows (for Shell, ShellResize)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    /// Terminal columns (for Shell, ShellResize)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    /// Input data as base64 (for ShellInput)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_base64: Option<String>,
}

/// Response to host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// File content as base64 (for ReadFile)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    // Shell-specific fields
    /// Session ID (for Shell, ShellOutput, ShellExited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Output data as base64 (for ShellOutput)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_base64: Option<String>,
    /// Response type for shell events (started, output, exited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_event: Option<ShellEvent>,
}

impl AgentResponse {
    fn success(id: &str) -> Self {
        Self {
            id: id.to_string(),
            exit_code: Some(0),
            stdout: None,
            stderr: None,
            error: None,
            content_base64: None,
            session_id: None,
            output_base64: None,
            shell_event: None,
        }
    }

    fn error(id: &str, msg: &str) -> Self {
        Self {
            id: id.to_string(),
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some(msg.to_string()),
            content_base64: None,
            session_id: None,
            output_base64: None,
            shell_event: None,
        }
    }

    fn from_output(id: &str, exit_code: i32, stdout: String, stderr: String) -> Self {
        Self {
            id: id.to_string(),
            exit_code: Some(exit_code),
            stdout: Some(stdout),
            stderr: Some(stderr),
            error: None,
            content_base64: None,
            session_id: None,
            output_base64: None,
            shell_event: None,
        }
    }

    fn with_content(id: &str, content_base64: String) -> Self {
        Self {
            id: id.to_string(),
            exit_code: Some(0),
            stdout: None,
            stderr: None,
            error: None,
            content_base64: Some(content_base64),
            session_id: None,
            output_base64: None,
            shell_event: None,
        }
    }

    fn shell_started(id: &str, session_id: String) -> Self {
        Self {
            id: id.to_string(),
            exit_code: None,
            stdout: None,
            stderr: None,
            error: None,
            content_base64: None,
            session_id: Some(session_id),
            output_base64: None,
            shell_event: Some(ShellEvent::Started),
        }
    }

    fn shell_output(id: &str, session_id: &str, output_base64: String) -> Self {
        Self {
            id: id.to_string(),
            exit_code: None,
            stdout: None,
            stderr: None,
            error: None,
            content_base64: None,
            session_id: Some(session_id.to_string()),
            output_base64: Some(output_base64),
            shell_event: Some(ShellEvent::Output),
        }
    }

    fn shell_exited(id: &str, session_id: &str, exit_code: i32) -> Self {
        Self {
            id: id.to_string(),
            exit_code: Some(exit_code),
            stdout: None,
            stderr: None,
            error: None,
            content_base64: None,
            session_id: Some(session_id.to_string()),
            output_base64: None,
            shell_event: Some(ShellEvent::Exited),
        }
    }
}

/// Validate a path is safe (no traversal, absolute path)
fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err("Path must be absolute".to_string());
    }
    if path.contains("..") {
        return Err("Path traversal not allowed".to_string());
    }
    // Block sensitive system paths
    let blocked = ["/proc", "/sys", "/dev", "/etc/passwd", "/etc/shadow"];
    for b in blocked {
        if path.starts_with(b) {
            return Err(format!("Cannot access system path: {}", b));
        }
    }
    Ok(())
}

/// Handle a single request
async fn handle_request(
    request: AgentRequest,
    session_manager: Arc<SessionManager>,
) -> AgentResponse {
    use base64::{engine::general_purpose::STANDARD, Engine};

    match request.request_type {
        RequestType::Ping => AgentResponse::success(&request.id),

        RequestType::Shutdown => {
            eprintln!("Shutdown requested, exiting...");
            // Schedule shutdown after response is sent
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                std::process::exit(0);
            });
            AgentResponse::success(&request.id)
        }

        RequestType::Shell => {
            // Start a new PTY shell session
            let command = request
                .command
                .as_ref()
                .and_then(|c| c.first())
                .map(|s| s.as_str())
                .unwrap_or("/bin/sh");
            let args: Vec<String> = request
                .command
                .as_ref()
                .map(|c| c.iter().skip(1).cloned().collect())
                .unwrap_or_default();
            let rows = request.rows.unwrap_or(24);
            let cols = request.cols.unwrap_or(80);

            match session_manager
                .create_session(command, &args, rows, cols, request.env.as_ref())
                .await
            {
                Ok(session_id) => {
                    eprintln!("Shell session started: {}", session_id);
                    AgentResponse::shell_started(&request.id, session_id)
                }
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to start shell: {}", e))
                }
            }
        }

        RequestType::ShellInput => {
            // Send input to an existing shell session
            let Some(session_id) = request.session_id else {
                return AgentResponse::error(&request.id, "No session_id specified");
            };
            let Some(input_base64) = request.input_base64 else {
                return AgentResponse::error(&request.id, "No input specified");
            };

            let input = match STANDARD.decode(&input_base64) {
                Ok(data) => data,
                Err(e) => {
                    return AgentResponse::error(&request.id, &format!("Invalid base64: {}", e));
                }
            };

            match session_manager.write_to_session(&session_id, &input).await {
                Ok(()) => AgentResponse::success(&request.id),
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to write to session: {}", e))
                }
            }
        }

        RequestType::ShellResize => {
            // Resize a shell session's terminal
            let Some(session_id) = request.session_id else {
                return AgentResponse::error(&request.id, "No session_id specified");
            };
            let rows = request.rows.unwrap_or(24);
            let cols = request.cols.unwrap_or(80);

            match session_manager
                .resize_session(&session_id, rows, cols)
                .await
            {
                Ok(()) => AgentResponse::success(&request.id),
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to resize session: {}", e))
                }
            }
        }

        RequestType::ShellClose => {
            // Close a shell session
            let Some(session_id) = request.session_id else {
                return AgentResponse::error(&request.id, "No session_id specified");
            };

            match session_manager.close_session(&session_id).await {
                Ok(exit_code) => {
                    eprintln!(
                        "Shell session closed: {} (exit: {:?})",
                        session_id, exit_code
                    );
                    AgentResponse::shell_exited(&request.id, &session_id, exit_code.unwrap_or(-1))
                }
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to close session: {}", e))
                }
            }
        }

        RequestType::Run => {
            let Some(command) = request.command else {
                return AgentResponse::error(&request.id, "No command specified");
            };

            if command.is_empty() {
                return AgentResponse::error(&request.id, "Empty command");
            }

            let program = &command[0];
            let args = &command[1..];

            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd.stdin(Stdio::null());
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            // Set working directory if specified
            if let Some(ref cwd) = request.cwd {
                cmd.current_dir(cwd);
            }

            // Set environment variables if specified
            if let Some(ref env) = request.env {
                for (key, value) in env {
                    cmd.env(key, value);
                }
            }

            match cmd.output().await {
                Ok(output) => {
                    let exit_code = output.status.code().unwrap_or(-1);
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    AgentResponse::from_output(&request.id, exit_code, stdout, stderr)
                }
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to run command: {}", e))
                }
            }
        }

        RequestType::WriteFile => {
            let Some(path) = request.path else {
                return AgentResponse::error(&request.id, "No path specified");
            };

            if let Err(e) = validate_path(&path) {
                return AgentResponse::error(&request.id, &e);
            }

            let Some(content_base64) = request.content_base64 else {
                return AgentResponse::error(&request.id, "No content specified");
            };

            let content = match STANDARD.decode(&content_base64) {
                Ok(c) => c,
                Err(e) => {
                    return AgentResponse::error(&request.id, &format!("Invalid base64: {}", e));
                }
            };

            // Ensure parent directory exists
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return AgentResponse::error(
                        &request.id,
                        &format!("Failed to create parent directory: {}", e),
                    );
                }
            }

            match tokio::fs::write(&path, &content).await {
                Ok(_) => AgentResponse::success(&request.id),
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to write file: {}", e))
                }
            }
        }

        RequestType::ReadFile => {
            let Some(path) = request.path else {
                return AgentResponse::error(&request.id, "No path specified");
            };

            if let Err(e) = validate_path(&path) {
                return AgentResponse::error(&request.id, &e);
            }

            match tokio::fs::read(&path).await {
                Ok(content) => {
                    let content_base64 = STANDARD.encode(&content);
                    AgentResponse::with_content(&request.id, content_base64)
                }
                Err(e) => AgentResponse::error(&request.id, &format!("Failed to read file: {}", e)),
            }
        }

        RequestType::RemoveFile => {
            let Some(path) = request.path else {
                return AgentResponse::error(&request.id, "No path specified");
            };

            if let Err(e) = validate_path(&path) {
                return AgentResponse::error(&request.id, &e);
            }

            match tokio::fs::remove_file(&path).await {
                Ok(_) => AgentResponse::success(&request.id),
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to remove file: {}", e))
                }
            }
        }

        RequestType::Mkdir => {
            let Some(path) = request.path else {
                return AgentResponse::error(&request.id, "No path specified");
            };

            if let Err(e) = validate_path(&path) {
                return AgentResponse::error(&request.id, &e);
            }

            let recursive = request.recursive.unwrap_or(false);
            let result = if recursive {
                tokio::fs::create_dir_all(&path).await
            } else {
                tokio::fs::create_dir(&path).await
            };

            match result {
                Ok(_) => AgentResponse::success(&request.id),
                Err(e) => {
                    AgentResponse::error(&request.id, &format!("Failed to create directory: {}", e))
                }
            }
        }
    }
}

/// Handle a single connection
async fn handle_connection(
    mut stream: tokio_vsock::VsockStream,
    session_manager: Arc<SessionManager>,
) -> Result<()> {
    loop {
        // Read length prefix
        let mut len_bytes = [0u8; 4];
        match stream.read_exact(&mut len_bytes).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Connection closed
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > 10 * 1024 * 1024 {
            // The body is still in the stream, so the connection can't be
            // resynchronized: answer once and close it.
            eprintln!("Request too large: {} bytes", len);
            let response = AgentResponse::error("", &format!("Request too large: {} bytes", len));
            write_response(&mut stream, &response).await?;
            return Ok(());
        }

        // Read request body
        let mut request_bytes = vec![0u8; len];
        stream
            .read_exact(&mut request_bytes)
            .await
            .context("Failed to read request")?;

        // Parse request. A request this agent can't parse (for example a
        // request type added after this agent was built) still gets an error
        // response, so the host fails fast instead of waiting out its read
        // timeout.
        let response = match parse_request(&request_bytes) {
            Ok(request) => handle_request(request, session_manager.clone()).await,
            Err(response) => *response,
        };

        write_response(&mut stream, &response).await?;
    }
}

/// Parse a request body, or build the error response to send back.
fn parse_request(request_bytes: &[u8]) -> Result<AgentRequest, Box<AgentResponse>> {
    serde_json::from_slice(request_bytes).map_err(|e| {
        eprintln!("Failed to parse request: {}", e);
        let id = serde_json::from_slice::<serde_json::Value>(request_bytes)
            .ok()
            .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
            .unwrap_or_default();
        Box::new(AgentResponse::error(
            &id,
            &format!("Unsupported or invalid request: {}", e),
        ))
    })
}

/// Send a length-prefixed response.
async fn write_response<S>(stream: &mut S, response: &AgentResponse) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let response_bytes = serde_json::to_vec(response)?;
    let len = response_bytes.len() as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&response_bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// Bring the loopback interface up if it is down.
///
/// Images converted from minimal containers may ship neither `ip` nor
/// `ifconfig`, so `/init` cannot raise `lo` and 127.0.0.1 is unreachable.
/// The agent runs as root under PID 1, so it sets `IFF_UP` on `lo` with
/// `SIOCGIFFLAGS`/`SIOCSIFFLAGS`; the kernel then assigns 127.0.0.1/8.
/// Returns `Ok(true)` when the interface was raised, `Ok(false)` when it was
/// already up.
#[cfg(target_os = "linux")]
fn bring_up_loopback() -> std::io::Result<bool> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: plain socket(2) call; the returned descriptor is owned below.
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a freshly created, valid descriptor that nothing else owns.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };

    // SAFETY: ifreq is plain old data; all-zero is a valid value.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(b"lo\0") {
        *slot = *byte as libc::c_char;
    }

    // SAFETY: SIOCGIFFLAGS reads into the ifreq we own.
    if unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS as _, &mut request) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: SIOCGIFFLAGS filled the flags member of the union.
    let flags = unsafe { request.ifr_ifru.ifru_flags };
    let up = libc::IFF_UP as libc::c_short;
    if flags & up != 0 {
        return Ok(false);
    }
    request.ifr_ifru.ifru_flags = flags | up;
    // SAFETY: SIOCSIFFLAGS reads the ifreq we own.
    if unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCSIFFLAGS as _, &request) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn bring_up_loopback() -> std::io::Result<bool> {
    Ok(false)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    eprintln!("Agentkernel guest agent starting...");
    match bring_up_loopback() {
        Ok(true) => eprintln!("Loopback interface lo was down; brought it up"),
        Ok(false) => {}
        Err(e) => eprintln!("Could not bring up loopback interface lo: {}", e),
    }
    eprintln!("Listening on vsock port {}", AGENT_PORT);

    // Create the session manager for PTY sessions
    let session_manager = Arc::new(SessionManager::new());

    let addr = VsockAddr::new(VMADDR_CID_ANY, AGENT_PORT);
    let listener = VsockListener::bind(addr).context("Failed to bind vsock listener")?;

    eprintln!("Agent ready (with PTY support)");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                eprintln!("Connection from CID {}", peer.cid());
                let session_manager = session_manager.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, session_manager).await {
                        eprintln!("Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_request_type_gets_an_error_response_with_its_id() {
        let body = serde_json::to_vec(&json!({"id": "req-7", "type": "from_the_future"})).unwrap();
        let response = parse_request(&body).expect_err("unknown type must not parse");
        assert_eq!(response.id, "req-7");
        assert!(response
            .error
            .unwrap()
            .contains("Unsupported or invalid request"));
        assert!(response.exit_code.is_none());
    }

    #[test]
    fn malformed_request_gets_an_error_response() {
        let response = parse_request(b"not json").expect_err("garbage must not parse");
        assert_eq!(response.id, "");
        assert!(response.error.is_some());
    }

    #[test]
    fn shell_messages_keep_the_existing_wire_shape() {
        let request: AgentRequest = serde_json::from_value(json!({
            "id": "request-1",
            "type": "shell_resize",
            "session_id": "session-1",
            "rows": 40,
            "cols": 120
        }))
        .expect("shell resize request should deserialize");
        assert!(matches!(request.request_type, RequestType::ShellResize));
        assert_eq!(request.session_id.as_deref(), Some("session-1"));
        assert_eq!(request.rows, Some(40));
        assert_eq!(request.cols, Some(120));

        let response =
            AgentResponse::shell_output("request-1", "session-1", "aGVsbG8=".to_string());
        assert_eq!(
            serde_json::to_value(response).expect("shell output should serialize"),
            json!({
                "id": "request-1",
                "session_id": "session-1",
                "output_base64": "aGVsbG8=",
                "shell_event": "output"
            })
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod loopback_tests {
    use super::bring_up_loopback;

    /// Reads `lo` flags with SIOCGIFFLAGS, which sees the calling thread's
    /// network namespace (sysfs would show the mount namespace's instead).
    fn lo_is_up() -> bool {
        // SAFETY: test-only socket and ioctl on an ifreq we own.
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
            assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
            let mut request: libc::ifreq = std::mem::zeroed();
            for (slot, byte) in request.ifr_name.iter_mut().zip(b"lo\0") {
                *slot = *byte as libc::c_char;
            }
            let rc = libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut request);
            let err = std::io::Error::last_os_error();
            libc::close(fd);
            assert!(rc >= 0, "SIOCGIFFLAGS: {err}");
            request.ifr_ifru.ifru_flags & libc::IFF_UP as libc::c_short != 0
        }
    }

    /// In a fresh network namespace `lo` starts down, like a guest without an
    /// init system. Needs CAP_SYS_ADMIN for unshare; skipped otherwise.
    #[test]
    fn brings_down_loopback_up_in_fresh_network_namespace() {
        std::thread::spawn(|| {
            // SAFETY: unshare only affects this short-lived thread.
            if unsafe { libc::unshare(libc::CLONE_NEWNET) } != 0 {
                eprintln!(
                    "skipping: unshare(CLONE_NEWNET) failed: {}",
                    std::io::Error::last_os_error()
                );
                return;
            }
            assert!(
                !lo_is_up(),
                "lo should start down in a new network namespace"
            );
            // With lo down, nothing on 127.0.0.1 is reachable.
            let reachable = std::net::TcpListener::bind("127.0.0.1:0")
                .and_then(|l| std::net::TcpStream::connect(l.local_addr()?))
                .is_ok();
            assert!(
                !reachable,
                "127.0.0.1 should be unreachable while lo is down"
            );

            assert!(bring_up_loopback().expect("bring lo up"));
            assert!(lo_is_up());
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1");
            std::net::TcpStream::connect(listener.local_addr().unwrap()).expect("connect over lo");

            assert!(
                !bring_up_loopback().expect("second call"),
                "already up is a no-op"
            );
        })
        .join()
        .expect("namespace thread");
    }
}
