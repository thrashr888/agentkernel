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
use tokio::net::UnixStream;

/// Firecracker API client
pub struct FirecrackerClient {
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

/// Network interface configuration
#[derive(Debug, Serialize)]
pub struct NetworkInterface {
    pub iface_id: String,
    pub guest_mac: Option<String>,
    pub host_dev_name: String,
}

/// Instance info response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct InstanceInfo {
    pub id: Option<String>,
    pub state: String,
    pub vmm_version: String,
}

impl FirecrackerClient {
    /// Create a new Firecracker API client
    pub fn new(socket_path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_string_lossy().to_string(),
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
        let action = InstanceAction {
            action_type: "Pause".to_string(),
        };
        self.put("/actions", &action).await
    }

    /// Resume the VM
    #[allow(dead_code)]
    pub async fn resume(&self) -> Result<()> {
        let action = InstanceAction {
            action_type: "Resume".to_string(),
        };
        self.put("/actions", &action).await
    }

    /// Make a PUT request
    async fn put<T: Serialize>(&self, path: &str, body: &T) -> Result<()> {
        let _ = self.request(Method::PUT, path, Some(body)).await?;
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
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to connect to Firecracker socket: {}",
                    self.socket_path
                )
            })?;

        let io = TokioIo::new(stream);

        // Create HTTP connection
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
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
            .uri(format!("http://localhost{}", path))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(body_bytes)))
            .context("Failed to build request")?;

        // Send request
        let response = sender
            .send_request(req)
            .await
            .context("Failed to send request to Firecracker")?;

        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
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
