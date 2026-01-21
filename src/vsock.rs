//! Vsock Communication Layer
//!
//! Host-to-guest communication via virtio-vsock for Firecracker microVMs.
//! Uses a simple JSON-RPC protocol over length-prefixed messages.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

#[cfg(unix)]
use tokio_vsock::{VsockAddr, VsockStream};

/// Default port for the guest agent
#[allow(dead_code)]
pub const AGENT_PORT: u32 = 52000;

/// Host CID (always 2 for the host)
#[allow(dead_code)]
pub const HOST_CID: u32 = 2;

/// Request types supported by the guest agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestType {
    /// Run a command and return output
    Run,
    /// Start an interactive shell (PTY)
    Shell,
    /// Health check
    Ping,
    /// Graceful shutdown
    Shutdown,
}

/// Request sent from host to guest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    /// Unique request ID
    pub id: String,
    /// Request type
    #[serde(rename = "type")]
    pub request_type: RequestType,
    /// Command to run (for Run type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// Working directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Environment variables
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

/// Response from guest to host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// Request ID this is responding to
    pub id: String,
    /// Exit code (for Run type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Standard output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    /// Standard error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    /// Error message if request failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of running a command in the guest
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A persistent vsock connection that can be reused for multiple commands.
/// This saves the overhead of reconnecting and re-handshaking for each command.
#[cfg(unix)]
pub struct VsockConnection {
    stream: tokio::net::UnixStream,
    timeout_secs: u64,
}

#[cfg(unix)]
impl VsockConnection {
    /// Establish a new vsock connection to a Firecracker VM.
    /// Performs the CONNECT handshake so the connection is ready for commands.
    pub async fn connect(uds_path: impl AsRef<std::path::Path>, port: u32) -> Result<Self> {
        use tokio::net::UnixStream;

        let mut stream = timeout(
            Duration::from_secs(30),
            UnixStream::connect(uds_path.as_ref()),
        )
        .await
        .context("Connection timeout")?
        .context("Failed to connect to Firecracker vsock socket")?;

        // Firecracker vsock protocol: send CONNECT <port>\n
        let connect_cmd = format!("CONNECT {}\n", port);
        stream
            .write_all(connect_cmd.as_bytes())
            .await
            .context("Failed to send CONNECT")?;
        stream.flush().await?;

        // Read response: OK <host_port>\n
        let mut response_buf = [0u8; 32];
        let n = timeout(Duration::from_secs(5), stream.read(&mut response_buf))
            .await
            .context("Timeout waiting for CONNECT response")?
            .context("Failed to read CONNECT response")?;

        let response_str = std::str::from_utf8(&response_buf[..n])
            .context("Invalid CONNECT response")?
            .trim();

        if !response_str.starts_with("OK ") {
            bail!("Firecracker vsock CONNECT failed: {}", response_str);
        }

        Ok(Self {
            stream,
            timeout_secs: 30,
        })
    }

    /// Run a command using this established connection.
    pub async fn run_command(&mut self, command: &[String]) -> Result<RunResult> {
        let request = AgentRequest {
            id: uuid::Uuid::new_v4().to_string(),
            request_type: RequestType::Run,
            command: Some(command.to_vec()),
            cwd: None,
            env: None,
        };

        let response = self.send_request(&request).await?;

        if let Some(error) = response.error {
            bail!("Guest agent error: {}", error);
        }

        Ok(RunResult {
            exit_code: response.exit_code.unwrap_or(-1),
            stdout: response.stdout.unwrap_or_default(),
            stderr: response.stderr.unwrap_or_default(),
        })
    }

    /// Send a request and receive response over the established connection.
    async fn send_request(&mut self, request: &AgentRequest) -> Result<AgentResponse> {
        // Serialize request
        let request_bytes = serde_json::to_vec(request)?;

        // Send length-prefixed request
        let len = request_bytes.len() as u32;
        self.stream.write_all(&len.to_le_bytes()).await?;
        self.stream.write_all(&request_bytes).await?;
        self.stream.flush().await?;

        // Read length-prefixed response
        let mut len_bytes = [0u8; 4];
        timeout(
            Duration::from_secs(self.timeout_secs),
            self.stream.read_exact(&mut len_bytes),
        )
        .await
        .context("Read timeout")?
        .context("Failed to read response length")?;

        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > 10 * 1024 * 1024 {
            bail!("Response too large: {} bytes", len);
        }

        let mut response_bytes = vec![0u8; len];
        timeout(
            Duration::from_secs(self.timeout_secs),
            self.stream.read_exact(&mut response_bytes),
        )
        .await
        .context("Read timeout")?
        .context("Failed to read response body")?;

        let response: AgentResponse =
            serde_json::from_slice(&response_bytes).context("Failed to parse response")?;

        Ok(response)
    }

    /// Check if the connection is still alive by sending a ping.
    #[allow(dead_code)]
    pub async fn ping(&mut self) -> bool {
        let request = AgentRequest {
            id: "ping".to_string(),
            request_type: RequestType::Ping,
            command: None,
            cwd: None,
            env: None,
        };

        self.send_request(&request).await.is_ok()
    }
}

/// Vsock client for communicating with guest agent
///
/// Supports two modes:
/// - Native vsock (via kernel AF_VSOCK)
/// - Firecracker vsock (via Unix domain socket with CONNECT protocol)
#[allow(dead_code)]
pub struct VsockClient {
    cid: u32,
    port: u32,
    timeout_secs: u64,
    /// Path to Firecracker vsock UDS (if using Firecracker mode)
    uds_path: Option<std::path::PathBuf>,
}

#[allow(dead_code)]
impl VsockClient {
    /// Create a new vsock client for the given guest CID (native vsock mode)
    pub fn new(cid: u32) -> Self {
        Self {
            cid,
            port: AGENT_PORT,
            timeout_secs: 30,
            uds_path: None,
        }
    }

    /// Create a client for Firecracker vsock (via Unix socket)
    pub fn for_firecracker(uds_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            cid: 0, // Not used in Firecracker mode
            port: AGENT_PORT,
            timeout_secs: 30,
            uds_path: Some(uds_path.into()),
        }
    }

    /// Set the port to connect to
    #[allow(dead_code)]
    pub fn with_port(mut self, port: u32) -> Self {
        self.port = port;
        self
    }

    /// Set the timeout for operations
    #[allow(dead_code)]
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Run a command in the guest
    #[cfg(unix)]
    pub async fn run_command(&self, command: &[String]) -> Result<RunResult> {
        let request = AgentRequest {
            id: uuid::Uuid::new_v4().to_string(),
            request_type: RequestType::Run,
            command: Some(command.to_vec()),
            cwd: None,
            env: None,
        };

        let response = self.send_request(&request).await?;

        if let Some(error) = response.error {
            bail!("Guest agent error: {}", error);
        }

        Ok(RunResult {
            exit_code: response.exit_code.unwrap_or(-1),
            stdout: response.stdout.unwrap_or_default(),
            stderr: response.stderr.unwrap_or_default(),
        })
    }

    /// Run a command with custom working directory and environment
    #[cfg(unix)]
    #[allow(dead_code)]
    pub async fn run_command_with_env(
        &self,
        command: &[String],
        cwd: Option<&str>,
        env: Option<HashMap<String, String>>,
    ) -> Result<RunResult> {
        let request = AgentRequest {
            id: uuid::Uuid::new_v4().to_string(),
            request_type: RequestType::Run,
            command: Some(command.to_vec()),
            cwd: cwd.map(|s| s.to_string()),
            env,
        };

        let response = self.send_request(&request).await?;

        if let Some(error) = response.error {
            bail!("Guest agent error: {}", error);
        }

        Ok(RunResult {
            exit_code: response.exit_code.unwrap_or(-1),
            stdout: response.stdout.unwrap_or_default(),
            stderr: response.stderr.unwrap_or_default(),
        })
    }

    /// Ping the guest agent to check if it's alive
    #[cfg(unix)]
    #[allow(dead_code)]
    pub async fn ping(&self) -> Result<bool> {
        let request = AgentRequest {
            id: uuid::Uuid::new_v4().to_string(),
            request_type: RequestType::Ping,
            command: None,
            cwd: None,
            env: None,
        };

        match self.send_request(&request).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Request graceful shutdown of the guest
    #[cfg(unix)]
    #[allow(dead_code)]
    pub async fn shutdown(&self) -> Result<()> {
        let request = AgentRequest {
            id: uuid::Uuid::new_v4().to_string(),
            request_type: RequestType::Shutdown,
            command: None,
            cwd: None,
            env: None,
        };

        // Shutdown may not get a response if the guest shuts down quickly
        let _ = self.send_request(&request).await;
        Ok(())
    }

    /// Send a request to the guest agent and receive response
    #[cfg(unix)]
    async fn send_request(&self, request: &AgentRequest) -> Result<AgentResponse> {
        // Choose connection method based on whether we have a UDS path (Firecracker mode)
        if let Some(ref uds_path) = self.uds_path {
            self.send_request_via_firecracker(request, uds_path).await
        } else {
            self.send_request_via_native_vsock(request).await
        }
    }

    /// Send request via native kernel vsock
    #[cfg(unix)]
    async fn send_request_via_native_vsock(&self, request: &AgentRequest) -> Result<AgentResponse> {
        // Connect to guest via native vsock
        let addr = VsockAddr::new(self.cid, self.port);
        let mut stream = timeout(
            Duration::from_secs(self.timeout_secs),
            VsockStream::connect(addr),
        )
        .await
        .context("Connection timeout")?
        .context("Failed to connect to guest agent")?;

        self.send_and_receive(&mut stream, request).await
    }

    /// Send request via Firecracker vsock Unix socket
    #[cfg(unix)]
    async fn send_request_via_firecracker(
        &self,
        request: &AgentRequest,
        uds_path: &std::path::Path,
    ) -> Result<AgentResponse> {
        use tokio::net::UnixStream;

        // Connect to Firecracker vsock Unix socket
        let mut stream = timeout(
            Duration::from_secs(self.timeout_secs),
            UnixStream::connect(uds_path),
        )
        .await
        .context("Connection timeout")?
        .context("Failed to connect to Firecracker vsock socket")?;

        // Firecracker vsock protocol: send CONNECT <port>\n
        let connect_cmd = format!("CONNECT {}\n", self.port);
        stream
            .write_all(connect_cmd.as_bytes())
            .await
            .context("Failed to send CONNECT")?;
        stream.flush().await?;

        // Read response: OK <host_port>\n
        let mut response_buf = [0u8; 32];
        let n = timeout(Duration::from_secs(5), stream.read(&mut response_buf))
            .await
            .context("Timeout waiting for CONNECT response")?
            .context("Failed to read CONNECT response")?;

        let response_str = std::str::from_utf8(&response_buf[..n])
            .context("Invalid CONNECT response")?
            .trim();

        if !response_str.starts_with("OK ") {
            bail!("Firecracker vsock CONNECT failed: {}", response_str);
        }

        // Now we can communicate with the guest agent
        self.send_and_receive(&mut stream, request).await
    }

    /// Common send/receive logic for both connection types
    #[cfg(unix)]
    async fn send_and_receive<S>(
        &self,
        stream: &mut S,
        request: &AgentRequest,
    ) -> Result<AgentResponse>
    where
        S: AsyncReadExt + AsyncWriteExt + Unpin,
    {
        // Serialize request
        let request_bytes = serde_json::to_vec(request)?;

        // Send length-prefixed request
        let len = request_bytes.len() as u32;
        stream.write_all(&len.to_le_bytes()).await?;
        stream.write_all(&request_bytes).await?;
        stream.flush().await?;

        // Read length-prefixed response
        let mut len_bytes = [0u8; 4];
        timeout(
            Duration::from_secs(self.timeout_secs),
            stream.read_exact(&mut len_bytes),
        )
        .await
        .context("Read timeout")?
        .context("Failed to read response length")?;

        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > 10 * 1024 * 1024 {
            bail!("Response too large: {} bytes", len);
        }

        let mut response_bytes = vec![0u8; len];
        timeout(
            Duration::from_secs(self.timeout_secs),
            stream.read_exact(&mut response_bytes),
        )
        .await
        .context("Read timeout")?
        .context("Failed to read response body")?;

        let response: AgentResponse =
            serde_json::from_slice(&response_bytes).context("Failed to parse response")?;

        Ok(response)
    }

    /// Stub for non-unix platforms
    #[cfg(not(unix))]
    pub async fn run_command(&self, _command: &[String]) -> Result<RunResult> {
        bail!("Vsock is only supported on Unix platforms");
    }

    /// Stub for non-unix platforms
    #[cfg(not(unix))]
    #[allow(dead_code)]
    pub async fn run_command_with_env(
        &self,
        _command: &[String],
        _cwd: Option<&str>,
        _env: Option<HashMap<String, String>>,
    ) -> Result<RunResult> {
        bail!("Vsock is only supported on Unix platforms");
    }

    /// Stub for non-unix platforms
    #[cfg(not(unix))]
    #[allow(dead_code)]
    pub async fn ping(&self) -> Result<bool> {
        bail!("Vsock is only supported on Unix platforms");
    }

    /// Stub for non-unix platforms
    #[cfg(not(unix))]
    #[allow(dead_code)]
    pub async fn shutdown(&self) -> Result<()> {
        bail!("Vsock is only supported on Unix platforms");
    }
}

/// Wait for the guest agent to become available
#[cfg(unix)]
#[allow(dead_code)]
pub async fn wait_for_agent(cid: u32, timeout_secs: u64) -> Result<()> {
    let client = VsockClient::new(cid).with_timeout(5);
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);

    while std::time::Instant::now() < deadline {
        if client.ping().await.unwrap_or(false) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    bail!("Guest agent not available after {} seconds", timeout_secs);
}

/// Wait for agent stub for non-unix
#[cfg(not(unix))]
#[allow(dead_code)]
pub async fn wait_for_agent(_cid: u32, _timeout_secs: u64) -> Result<()> {
    bail!("Vsock is only supported on Unix platforms");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialize() {
        let request = AgentRequest {
            id: "test-123".to_string(),
            request_type: RequestType::Run,
            command: Some(vec!["ls".to_string(), "-la".to_string()]),
            cwd: Some("/app".to_string()),
            env: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"run\""));
        assert!(json.contains("\"command\":[\"ls\",\"-la\"]"));
        assert!(json.contains("\"cwd\":\"/app\""));
    }

    #[test]
    fn test_response_deserialize() {
        let json = r#"{
            "id": "test-123",
            "exit_code": 0,
            "stdout": "hello world\n",
            "stderr": ""
        }"#;

        let response: AgentResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, "test-123");
        assert_eq!(response.exit_code, Some(0));
        assert_eq!(response.stdout, Some("hello world\n".to_string()));
    }
}
