//! Cloud Hypervisor API Client
//!
//! Native Rust HTTP client for Cloud Hypervisor's REST API over Unix sockets.

use anyhow::{Context, Result, bail};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;

const CLOUD_HYPERVISOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CLOUD_HYPERVISOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Cloud Hypervisor API client
pub struct CloudHypervisorClient {
    socket_path: String,
    connect_timeout: Duration,
    request_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
#[error("Cloud Hypervisor API {stage} timed out after {timeout:?} for {socket_path}")]
pub struct CloudHypervisorApiTimeout {
    stage: &'static str,
    timeout: Duration,
    socket_path: String,
}

/// Cloud Hypervisor API error response
#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PayloadConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmdline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initramfs: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CpusConfig {
    pub boot_vcpus: u32,
    pub max_vcpus: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kvm_hyperv: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct MemoryConfig {
    pub size: u64, // bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DiskConfig {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct NetConfig {
    pub tap: String,
    pub mac: String,
}

#[derive(Debug, Serialize)]
pub struct VsockConfig {
    pub cid: u32,
    pub socket: String,
}

#[derive(Debug, Serialize)]
pub struct VmConfig {
    pub payload: PayloadConfig,
    pub cpus: CpusConfig,
    pub memory: MemoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disks: Option<Vec<DiskConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Vec<NetConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vsock: Option<VsockConfig>,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            payload: PayloadConfig {
                kernel: None,
                cmdline: None,
                initramfs: None,
            },
            cpus: CpusConfig {
                boot_vcpus: 1,
                max_vcpus: 1,
                kvm_hyperv: None,
            },
            memory: MemoryConfig {
                size: 512 * 1024 * 1024, // 512 MB
                shared: None,
            },
            disks: None,
            net: None,
            vsock: None,
        }
    }
}

/// Cloud Hypervisor version response
#[derive(Debug, Deserialize)]
pub struct Version {
    pub version: String,
}

impl CloudHypervisorClient {
    /// Create a new Cloud Hypervisor API client
    pub fn new(socket_path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_string_lossy().to_string(),
            connect_timeout: CLOUD_HYPERVISOR_CONNECT_TIMEOUT,
            request_timeout: CLOUD_HYPERVISOR_REQUEST_TIMEOUT,
        }
    }

    /// Check if the socket is available
    #[allow(dead_code)]
    pub async fn is_available(&self) -> bool {
        std::path::Path::new(&self.socket_path).exists()
    }

    /// Get the running VMM version.
    #[allow(dead_code)]
    pub async fn get_version(&self) -> Result<Version> {
        let response = self.request(Method::GET, "/api/v1/vmm.ping", None::<&()>).await?;
        serde_json::from_slice(&response).context("Failed to parse Cloud Hypervisor version")
    }

    /// Create the VM with the given configuration
    pub async fn create_vm(&self, config: &VmConfig) -> Result<()> {
        let _ = self.request(Method::PUT, "/api/v1/vm.create", Some(config)).await?;
        Ok(())
    }

    /// Boot the created VM
    pub async fn boot_vm(&self) -> Result<()> {
        let _ = self.request(Method::PUT, "/api/v1/vm.boot", None::<&()>).await?;
        Ok(())
    }

    /// Stop the VM cleanly
    pub async fn shutdown_vm(&self) -> Result<()> {
        let _ = self.request(Method::PUT, "/api/v1/vm.shutdown", None::<&()>).await?;
        Ok(())
    }

    async fn request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<Bytes> {
        let socket_path = self.socket_path.clone();

        // 1. Connect to the socket with a timeout
        let stream = tokio::time::timeout(self.connect_timeout, UnixStream::connect(&socket_path))
            .await
            .map_err(|_| CloudHypervisorApiTimeout {
                stage: "connect",
                timeout: self.connect_timeout,
                socket_path: socket_path.clone(),
            })?
            .with_context(|| format!("Failed to connect to {}", socket_path))?;

        let io = TokioIo::new(stream);

        // 2. Perform the HTTP handshake
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .context("HTTP handshake failed")?;

        tokio::spawn(async move {
            if let Err(_err) = conn.await {
                // ignore errors for now
            }
        });

        // 3. Prepare the request
        let uri = format!("http://localhost{}", path);
        let req_body = match body {
            Some(b) => Full::new(Bytes::from(serde_json::to_vec(b)?)),
            None => Full::new(Bytes::new()),
        };

        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(req_body)
            .context("Failed to build request")?;

        // 4. Send the request and wait for the response with a timeout
        let res = tokio::time::timeout(self.request_timeout, sender.send_request(req))
            .await
            .map_err(|_| CloudHypervisorApiTimeout {
                stage: "request",
                timeout: self.request_timeout,
                socket_path,
            })?
            .context("Failed to send request")?;

        let status = res.status();
        let body = res.into_body();
        let bytes = body.collect().await?.to_bytes();

        if !status.is_success() {
            let err_msg = if let Ok(api_err) = serde_json::from_slice::<ApiError>(&bytes) {
                api_err
                    .error
                    .unwrap_or_else(|| String::from_utf8_lossy(&bytes).to_string())
            } else {
                String::from_utf8_lossy(&bytes).to_string()
            };
            bail!("Cloud Hypervisor API error ({}): {}", status, err_msg);
        }

        Ok(bytes)
    }
}
