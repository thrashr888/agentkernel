//! Firecracker API Client
//!
//! Native Rust HTTP client for Firecracker's REST API over Unix sockets.

use anyhow::{Context, Result, bail};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;

const FIRECRACKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FIRECRACKER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Firecracker API client
pub struct FirecrackerClient {
    socket_path: String,
    connect_timeout: Duration,
    request_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
#[error("Firecracker API {stage} timed out after {timeout:?} for {socket_path}")]
pub struct FirecrackerApiTimeout {
    stage: &'static str,
    timeout: Duration,
    socket_path: String,
}

/// Firecracker API error response
#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub fault_message: Option<String>,
}

/// Boot source configuration
#[derive(Debug, Serialize)]
pub struct BootSource {
    pub kernel_image_path: String,
    pub boot_args: String,
}

/// Drive configuration
#[derive(Debug, Serialize)]
pub struct Drive {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

/// Machine configuration
#[derive(Debug, Serialize)]
pub struct MachineConfig {
    pub vcpu_count: u32,
    pub mem_size_mib: u64,
}

/// Vsock device configuration
#[derive(Debug, Serialize)]
pub struct VsockDevice {
    pub guest_cid: u32,
    pub uds_path: String,
}

/// Instance action (start, stop, etc.)
#[derive(Debug, Serialize)]
pub struct InstanceAction {
    pub action_type: String,
}

/// Runtime state accepted by Firecracker's `PATCH /vm` endpoint.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum VmState {
    Paused,
    Resumed,
}

#[derive(Debug, Serialize)]
struct VmStatePatch {
    state: VmState,
}

/// Network interface configuration
#[derive(Debug, Serialize)]
pub struct NetworkInterface {
    pub iface_id: String,
    pub guest_mac: Option<String>,
    pub host_dev_name: String,
}

/// Full snapshot creation parameters.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct SnapshotCreateParams {
    pub mem_file_path: String,
    pub snapshot_path: String,
    pub snapshot_type: String,
}

/// Override the host-side UDS when restoring a VM snapshot.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct VsockOverride {
    pub uds_path: String,
}

/// Snapshot memory backend supported by Firecracker 1.16.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum MemoryBackendType {
    File,
}

/// Memory source used while lazily restoring a Firecracker snapshot.
#[derive(Debug, Serialize)]
pub struct MemoryBackend {
    pub backend_type: MemoryBackendType,
    pub backend_path: String,
}

impl MemoryBackend {
    pub fn file(path: impl AsRef<Path>) -> Self {
        Self {
            backend_type: MemoryBackendType::File,
            backend_path: path.as_ref().to_string_lossy().into_owned(),
        }
    }
}

/// Snapshot restoration parameters supported by Firecracker 1.16.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct SnapshotLoadParams {
    pub mem_backend: MemoryBackend,
    pub snapshot_path: String,
    pub resume_vm: bool,
    pub vsock_override: VsockOverride,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_realtime: Option<bool>,
}

/// Instance info response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct InstanceInfo {
    pub id: Option<String>,
    pub state: String,
    pub vmm_version: String,
}

/// Firecracker version response.
#[derive(Debug, Deserialize)]
pub struct FirecrackerVersion {
    pub firecracker_version: String,
}

impl FirecrackerClient {
    /// Create a new Firecracker API client
    pub fn new(socket_path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_string_lossy().to_string(),
            connect_timeout: FIRECRACKER_CONNECT_TIMEOUT,
            request_timeout: FIRECRACKER_REQUEST_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeouts(
        socket_path: impl AsRef<Path>,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_string_lossy().to_string(),
            connect_timeout,
            request_timeout,
        }
    }

    /// Check if the socket is available
    #[allow(dead_code)]
    pub async fn is_available(&self) -> bool {
        std::path::Path::new(&self.socket_path).exists()
    }

    /// Get instance info
    #[allow(dead_code)]
    pub async fn get_instance_info(&self) -> Result<InstanceInfo> {
        let response = self.request(Method::GET, "/", None::<&()>).await?;
        serde_json::from_slice(&response).context("Failed to parse instance info")
    }

    /// Get the running Firecracker VMM version.
    pub async fn get_version(&self) -> Result<FirecrackerVersion> {
        let response = self.request(Method::GET, "/version", None::<&()>).await?;
        serde_json::from_slice(&response).context("Failed to parse Firecracker version")
    }

    /// Set boot source configuration
    pub async fn set_boot_source(&self, boot_source: &BootSource) -> Result<()> {
        self.put("/boot-source", boot_source).await
    }

    /// Set root drive
    pub async fn set_drive(&self, drive_id: &str, drive: &Drive) -> Result<()> {
        self.put(&format!("/drives/{}", drive_id), drive).await
    }

    /// Set machine configuration
    pub async fn set_machine_config(&self, config: &MachineConfig) -> Result<()> {
        self.put("/machine-config", config).await
    }

    /// Set vsock device
    pub async fn set_vsock(&self, vsock: &VsockDevice) -> Result<()> {
        self.put("/vsock", vsock).await
    }

    /// Set network interface
    #[allow(dead_code)]
    pub async fn set_network_interface(
        &self,
        iface_id: &str,
        iface: &NetworkInterface,
    ) -> Result<()> {
        self.put(&format!("/network-interfaces/{}", iface_id), iface)
            .await
    }

    /// Start the VM instance
    pub async fn start_instance(&self) -> Result<()> {
        let action = InstanceAction {
            action_type: "InstanceStart".to_string(),
        };
        self.put("/actions", &action).await
    }

    /// Send Ctrl+Alt+Del to the VM (graceful shutdown)
    pub async fn send_ctrl_alt_del(&self) -> Result<()> {
        let action = InstanceAction {
            action_type: "SendCtrlAltDel".to_string(),
        };
        self.put("/actions", &action).await
    }

    /// Pause the VM
    #[allow(dead_code)]
    pub async fn pause(&self) -> Result<()> {
        self.set_vm_state(VmState::Paused).await
    }

    /// Resume the VM
    #[allow(dead_code)]
    pub async fn resume(&self) -> Result<()> {
        self.set_vm_state(VmState::Resumed).await
    }

    /// Update the state of a running VM.
    pub async fn set_vm_state(&self, state: VmState) -> Result<()> {
        self.patch("/vm", &VmStatePatch { state }).await
    }

    /// Create a full VM snapshot. The VM must be paused first.
    #[allow(dead_code)]
    pub async fn create_snapshot(&self, snapshot: &SnapshotCreateParams) -> Result<()> {
        self.put("/snapshot/create", snapshot).await
    }

    /// Load and optionally resume a VM from a snapshot.
    #[allow(dead_code)]
    pub async fn load_snapshot(&self, snapshot: &SnapshotLoadParams) -> Result<()> {
        self.put("/snapshot/load", snapshot).await
    }

    /// Make a PUT request
    async fn put<T: Serialize>(&self, path: &str, body: &T) -> Result<()> {
        let _ = self.request(Method::PUT, path, Some(body)).await?;
        Ok(())
    }

    /// Make a PATCH request.
    async fn patch<T: Serialize>(&self, path: &str, body: &T) -> Result<()> {
        let _ = self.request(Method::PATCH, path, Some(body)).await?;
        Ok(())
    }

    /// Make an HTTP request to the Firecracker API
    async fn request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<Bytes> {
        // Connect to Unix socket
        let stream =
            tokio::time::timeout(self.connect_timeout, UnixStream::connect(&self.socket_path))
                .await
                .map_err(|_| FirecrackerApiTimeout {
                    stage: "connect",
                    timeout: self.connect_timeout,
                    socket_path: self.socket_path.clone(),
                })?
                .with_context(|| {
                    format!(
                        "Failed to connect to Firecracker socket: {}",
                        self.socket_path
                    )
                })?;

        let io = TokioIo::new(stream);

        // Create HTTP connection
        let (mut sender, conn) = tokio::time::timeout(
            self.connect_timeout,
            hyper::client::conn::http1::handshake(io),
        )
        .await
        .map_err(|_| FirecrackerApiTimeout {
            stage: "HTTP handshake",
            timeout: self.connect_timeout,
            socket_path: self.socket_path.clone(),
        })?
        .context("Failed to create HTTP connection")?;

        // Spawn connection handler
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("Firecracker connection error: {:?}", e);
            }
        });

        // Build request
        let body_bytes = if let Some(b) = body {
            serde_json::to_vec(b)?
        } else {
            Vec::new()
        };

        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "localhost")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(body_bytes)))
            .context("Failed to build request")?;

        // Send request
        let response = tokio::time::timeout(self.request_timeout, sender.send_request(req))
            .await
            .map_err(|_| FirecrackerApiTimeout {
                stage: "response",
                timeout: self.request_timeout,
                socket_path: self.socket_path.clone(),
            })?
            .context("Failed to send request to Firecracker")?;

        let status = response.status();
        let body = tokio::time::timeout(self.request_timeout, response.into_body().collect())
            .await
            .map_err(|_| FirecrackerApiTimeout {
                stage: "response body",
                timeout: self.request_timeout,
                socket_path: self.socket_path.clone(),
            })?
            .context("Failed to read response body")?
            .to_bytes();

        // Handle errors
        if !status.is_success() {
            if let Ok(error) = serde_json::from_slice::<ApiError>(&body)
                && let Some(msg) = error.fault_message
            {
                bail!("Firecracker API error ({}): {}", status, msg);
            }
            let body_str = String::from_utf8_lossy(&body);
            bail!("Firecracker API error ({}): {}", status, body_str);
        }

        // Check for error in success response (some endpoints return 200 with fault_message)
        if !body.is_empty()
            && let Ok(error) = serde_json::from_slice::<ApiError>(&body)
            && let Some(msg) = error.fault_message
        {
            bail!("Firecracker API error: {}", msg);
        }

        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_source_serialize() {
        let boot = BootSource {
            kernel_image_path: "/path/to/kernel".to_string(),
            boot_args: "console=ttyS0".to_string(),
        };
        let json = serde_json::to_string(&boot).unwrap();
        assert!(json.contains("kernel_image_path"));
        assert!(json.contains("boot_args"));
    }

    #[test]
    fn test_snapshot_load_serializes_vsock_override() {
        let snapshot = SnapshotLoadParams {
            mem_backend: MemoryBackend::file("/tmp/vm.mem"),
            snapshot_path: "/tmp/vm.state".to_string(),
            resume_vm: false,
            vsock_override: VsockOverride {
                uds_path: "/tmp/restored-vsock.sock".to_string(),
            },
            clock_realtime: Some(true),
        };

        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value["resume_vm"], false);
        assert_eq!(value["mem_backend"]["backend_type"], "File");
        assert_eq!(value["mem_backend"]["backend_path"], "/tmp/vm.mem");
        assert!(value.get("mem_file_path").is_none());
        assert_eq!(
            value["vsock_override"]["uds_path"],
            "/tmp/restored-vsock.sock"
        );
        assert_eq!(value["clock_realtime"], true);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pause_and_resume_patch_vm_state() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        async fn capture_request(listener: UnixListener) -> String {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let bytes = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            String::from_utf8(request[..bytes].to_vec()).unwrap()
        }

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("firecracker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let request = tokio::spawn(capture_request(listener));
        FirecrackerClient::new(&socket).pause().await.unwrap();
        let request = request.await.unwrap();
        assert!(
            request.starts_with("PATCH /vm HTTP/1.1"),
            "unexpected request: {request:?}"
        );
        assert!(request.contains(r#"{"state":"Paused"}"#));

        std::fs::remove_file(&socket).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let request = tokio::spawn(capture_request(listener));
        FirecrackerClient::new(&socket).resume().await.unwrap();
        let request = request.await.unwrap();
        assert!(request.starts_with("PATCH /vm HTTP/1.1"));
        assert!(request.contains(r#"{"state":"Resumed"}"#));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn request_times_out_when_vmm_accepts_but_never_replies() {
        use tokio::net::UnixListener;

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("wedged-firecracker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        let client = FirecrackerClient::with_timeouts(
            &socket,
            Duration::from_millis(100),
            Duration::from_millis(25),
        );
        let error = client.get_version().await.unwrap_err();
        assert!(
            error.downcast_ref::<FirecrackerApiTimeout>().is_some(),
            "unexpected error: {error:#}"
        );
        server.abort();
    }

    #[test]
    fn test_machine_config_serialize() {
        let config = MachineConfig {
            vcpu_count: 2,
            mem_size_mib: 512,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("vcpu_count"));
        assert!(json.contains("mem_size_mib"));
    }
}
