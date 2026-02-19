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
                Err(e) => AgentResponse::error(&request.id, &format!("Failed to start shell: {}", e)),
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
                Err(e) => AgentResponse::error(&request.id, &format!("Failed to write to session: {}", e)),
            }
        }

        RequestType::ShellResize => {
            // Resize a shell session's terminal
            let Some(session_id) = request.session_id else {
                return AgentResponse::error(&request.id, "No session_id specified");
            };
            let rows = request.rows.unwrap_or(24);
            let cols = request.cols.unwrap_or(80);

            match session_manager.resize_session(&session_id, rows, cols).await {
                Ok(()) => AgentResponse::success(&request.id),
                Err(e) => AgentResponse::error(&request.id, &format!("Failed to resize session: {}", e)),
            }
        }

        RequestType::ShellClose => {
            // Close a shell session
            let Some(session_id) = request.session_id else {
                return AgentResponse::error(&request.id, "No session_id specified");
            };

            match session_manager.close_session(&session_id).await {
                Ok(exit_code) => {
                    eprintln!("Shell session closed: {} (exit: {:?})", session_id, exit_code);
                    AgentResponse::shell_exited(&request.id, &session_id, exit_code.unwrap_or(-1))
                }
                Err(e) => AgentResponse::error(&request.id, &format!("Failed to close session: {}", e)),
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
            eprintln!("Request too large: {} bytes", len);
            continue;
        }

        // Read request body
        let mut request_bytes = vec![0u8; len];
        stream
            .read_exact(&mut request_bytes)
            .await
            .context("Failed to read request")?;

        // Parse request
        let request: AgentRequest = match serde_json::from_slice(&request_bytes) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to parse request: {}", e);
                continue;
            }
        };

        // Handle request
        let response = handle_request(request, session_manager.clone()).await;

        // Serialize response
        let response_bytes = serde_json::to_vec(&response)?;

        // Send length-prefixed response
        let len = response_bytes.len() as u32;
        stream.write_all(&len.to_le_bytes()).await?;
        stream.write_all(&response_bytes).await?;
        stream.flush().await?;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    eprintln!("Agentkernel guest agent starting...");
    eprintln!("Listening on vsock port {}", AGENT_PORT);

    // Create the session manager for PTY sessions
    let session_manager = Arc::new(SessionManager::new());

    let addr = VsockAddr::new(VMADDR_CID_ANY, AGENT_PORT);
    let mut listener = VsockListener::bind(addr).context("Failed to bind vsock listener")?;

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
