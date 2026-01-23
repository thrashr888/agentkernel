//! Daemon client for CLI to connect to the daemon.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::protocol::{DaemonCompatibilityMode, DaemonRequest, DaemonResponse};
use super::server::DaemonServer;

/// Client for connecting to the daemon
pub struct DaemonClient {
    socket_path: PathBuf,
}

/// VM handle returned from acquire
#[allow(dead_code)]
pub struct VmHandle {
    pub id: String,
    pub cid: u32,
    pub vsock_path: String,
}

/// Result of running a command in a pooled VM
pub struct RunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl DaemonClient {
    /// Create a new client with default socket path
    pub fn new() -> Self {
        Self {
            socket_path: DaemonServer::default_socket_path(),
        }
    }

    /// Create a client with custom socket path
    #[allow(dead_code)]
    pub fn with_socket_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Check if daemon is available
    pub fn is_available(&self) -> bool {
        self.socket_path.exists() && DaemonServer::is_running(&self.socket_path)
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Send a request and get a response
    async fn send_request(&self, request: &DaemonRequest) -> Result<DaemonResponse> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send request
        let json = serde_json::to_string(request)? + "\n";
        writer.write_all(json.as_bytes()).await?;

        // Read response
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        let response: DaemonResponse = serde_json::from_str(&line)?;
        Ok(response)
    }

    /// Acquire a VM from the pool
    #[allow(dead_code)]
    pub async fn acquire(&self, runtime: &str) -> Result<VmHandle> {
        self.acquire_with_mode(runtime, DaemonCompatibilityMode::Native)
            .await
    }

    /// Acquire a VM from the pool with a specific compatibility mode
    #[allow(dead_code)]
    pub async fn acquire_with_mode(
        &self,
        runtime: &str,
        compatibility_mode: DaemonCompatibilityMode,
    ) -> Result<VmHandle> {
        use super::protocol::DaemonBackend;
        let request = DaemonRequest::Acquire {
            runtime: runtime.to_string(),
            backend: DaemonBackend::default(),
            compatibility_mode,
        };

        match self.send_request(&request).await? {
            DaemonResponse::Acquired {
                id,
                cid,
                vsock_path,
                backend: _,
            } => Ok(VmHandle {
                id,
                cid: cid.unwrap_or(0),
                vsock_path: vsock_path.unwrap_or_default(),
            }),
            DaemonResponse::Error { message } => {
                bail!("Daemon error: {}", message)
            }
            other => {
                bail!("Unexpected response: {:?}", other)
            }
        }
    }

    /// Release a VM back to the pool
    #[allow(dead_code)]
    pub async fn release(&self, id: &str) -> Result<()> {
        let request = DaemonRequest::Release { id: id.to_string() };

        match self.send_request(&request).await? {
            DaemonResponse::Released => Ok(()),
            DaemonResponse::Error { message } => {
                bail!("Daemon error: {}", message)
            }
            other => {
                bail!("Unexpected response: {:?}", other)
            }
        }
    }

    /// Get daemon status
    pub async fn status(&self) -> Result<(usize, usize, usize, usize)> {
        let request = DaemonRequest::Status;

        match self.send_request(&request).await? {
            DaemonResponse::Status {
                warm,
                in_use,
                min_warm,
                max_warm,
                ..
            } => Ok((warm, in_use, min_warm, max_warm)),
            DaemonResponse::Error { message } => {
                bail!("Daemon error: {}", message)
            }
            other => {
                bail!("Unexpected response: {:?}", other)
            }
        }
    }

    /// Request daemon shutdown
    pub async fn shutdown(&self) -> Result<()> {
        let request = DaemonRequest::Shutdown;

        match self.send_request(&request).await? {
            DaemonResponse::ShuttingDown => Ok(()),
            DaemonResponse::Error { message } => {
                bail!("Daemon error: {}", message)
            }
            other => {
                bail!("Unexpected response: {:?}", other)
            }
        }
    }

    /// Run a command in a pooled VM (single round-trip: acquire + run + release)
    pub async fn run_in_pool(&self, runtime: &str, command: &[String]) -> Result<RunResult> {
        self.run_in_pool_with_mode(runtime, command, DaemonCompatibilityMode::Native)
            .await
    }

    /// Run a command in a pooled VM with a specific compatibility mode
    pub async fn run_in_pool_with_mode(
        &self,
        runtime: &str,
        command: &[String],
        compatibility_mode: DaemonCompatibilityMode,
    ) -> Result<RunResult> {
        use super::protocol::DaemonBackend;
        let request = DaemonRequest::Exec {
            runtime: runtime.to_string(),
            command: command.to_vec(),
            backend: DaemonBackend::default(),
            compatibility_mode,
        };

        match self.send_request(&request).await? {
            DaemonResponse::Executed {
                exit_code,
                stdout,
                stderr,
            } => Ok(RunResult {
                exit_code,
                stdout,
                stderr,
            }),
            DaemonResponse::Error { message } => {
                bail!("Daemon error: {}", message)
            }
            other => {
                bail!("Unexpected response: {:?}", other)
            }
        }
    }
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new()
    }
}
