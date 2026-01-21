//! Daemon server - Unix socket server for VM pool management.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::pool::{FirecrackerPool, PoolConfig};
use super::protocol::{DaemonRequest, DaemonResponse};

/// Daemon server state
pub struct DaemonServer {
    pool: Arc<FirecrackerPool>,
    socket_path: PathBuf,
}

impl DaemonServer {
    /// Create a new daemon server
    pub fn new(config: PoolConfig, kernel_path: PathBuf, rootfs_dir: PathBuf) -> Self {
        let socket_path = Self::default_socket_path();
        let pool = Arc::new(FirecrackerPool::new(config, kernel_path, rootfs_dir));

        Self { pool, socket_path }
    }

    /// Get the default socket path
    pub fn default_socket_path() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            let dir = PathBuf::from(home).join(".agentkernel");
            let _ = std::fs::create_dir_all(&dir);
            dir.join("daemon.sock")
        } else {
            PathBuf::from("/tmp/agentkernel-daemon.sock")
        }
    }

    /// Check if daemon is already running
    pub fn is_running(socket_path: &Path) -> bool {
        // Try to connect to existing socket
        std::os::unix::net::UnixStream::connect(socket_path).is_ok()
    }

    /// Run the daemon server
    pub async fn run(&self) -> Result<()> {
        // Check if already running
        if Self::is_running(&self.socket_path) {
            bail!(
                "Daemon is already running at {}",
                self.socket_path.display()
            );
        }

        // Remove stale socket
        let _ = std::fs::remove_file(&self.socket_path);

        // Create socket directory
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Bind to socket
        let listener = UnixListener::bind(&self.socket_path)?;
        eprintln!("Daemon listening on {}", self.socket_path.display());

        // Warm up the pool
        eprintln!("Warming up pool...");
        self.pool.warm_up().await?;
        let (warm, in_use) = self.pool.stats().await;
        eprintln!("Pool ready: {} warm, {} in use", warm, in_use);

        // Start maintenance task
        let pool_clone = Arc::clone(&self.pool);
        tokio::spawn(async move {
            pool_clone.run_maintenance().await;
        });

        // Accept connections
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let pool = Arc::clone(&self.pool);
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, pool).await {
                            eprintln!("Client error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Accept error: {}", e);
                }
            }
        }
    }

    /// Get pool reference
    #[allow(dead_code)]
    pub fn pool(&self) -> &Arc<FirecrackerPool> {
        &self.pool
    }

    /// Shutdown the daemon
    #[allow(dead_code)]
    pub async fn shutdown(&self) {
        self.pool.shutdown();
        self.pool.destroy_all().await;
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Handle a single client connection
async fn handle_client(stream: UnixStream, pool: Arc<FirecrackerPool>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // Client disconnected
            break;
        }

        let request: DaemonRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let response = DaemonResponse::error(format!("Invalid request: {}", e));
                let json = serde_json::to_string(&response)? + "\n";
                writer.write_all(json.as_bytes()).await?;
                continue;
            }
        };

        let response = handle_request(request, &pool).await;
        let json = serde_json::to_string(&response)? + "\n";
        writer.write_all(json.as_bytes()).await?;

        // Check for shutdown
        if matches!(response, DaemonResponse::ShuttingDown) {
            break;
        }
    }

    Ok(())
}

/// Handle a single request
async fn handle_request(request: DaemonRequest, pool: &FirecrackerPool) -> DaemonResponse {
    match request {
        DaemonRequest::Acquire { runtime } => match pool.acquire(&runtime).await {
            Ok(vm) => DaemonResponse::Acquired {
                id: vm.id,
                cid: vm.cid,
                vsock_path: vm.vsock_path.to_string_lossy().to_string(),
            },
            Err(e) => DaemonResponse::error(format!("Failed to acquire VM: {}", e)),
        },
        DaemonRequest::Release { id } => match pool.release(&id).await {
            Ok(_) => DaemonResponse::Released,
            Err(e) => DaemonResponse::error(format!("Failed to release VM: {}", e)),
        },
        DaemonRequest::Status => {
            let (warm, in_use) = pool.stats().await;
            DaemonResponse::Status {
                warm,
                in_use,
                min_warm: 3, // TODO: get from config
                max_warm: 5,
            }
        }
        DaemonRequest::Shutdown => {
            pool.shutdown();
            DaemonResponse::ShuttingDown
        }
    }
}
