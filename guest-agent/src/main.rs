//! Agentkernel Guest Agent
//!
//! Lightweight agent that runs inside microVMs to handle commands from the host.
//! Communicates over virtio-vsock using a JSON-RPC protocol.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
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
    /// Health check
    Ping,
    /// Graceful shutdown
    Shutdown,
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
}

impl AgentResponse {
    fn success(id: &str) -> Self {
        Self {
            id: id.to_string(),
            exit_code: Some(0),
            stdout: None,
            stderr: None,
            error: None,
        }
    }

    fn error(id: &str, msg: &str) -> Self {
        Self {
            id: id.to_string(),
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some(msg.to_string()),
        }
    }

    fn from_output(id: &str, exit_code: i32, stdout: String, stderr: String) -> Self {
        Self {
            id: id.to_string(),
            exit_code: Some(exit_code),
            stdout: Some(stdout),
            stderr: Some(stderr),
            error: None,
        }
    }
}

/// Handle a single request
async fn handle_request(request: AgentRequest) -> AgentResponse {
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
    }
}

/// Handle a single connection
async fn handle_connection(mut stream: tokio_vsock::VsockStream) -> Result<()> {
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
        let response = handle_request(request).await;

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

    let addr = VsockAddr::new(VMADDR_CID_ANY, AGENT_PORT);
    let mut listener = VsockListener::bind(addr).context("Failed to bind vsock listener")?;

    eprintln!("Agent ready");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                eprintln!("Connection from CID {}", peer.cid());
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream).await {
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
