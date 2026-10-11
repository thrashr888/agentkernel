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
        let response = self
            .request(Method::GET, "/api/v1/vmm.ping", None::<&()>)
            .await?;
        serde_json::from_slice(&response).context("Failed to parse Cloud Hypervisor version")
    }

    /// Create the VM with the given configuration
    pub async fn create_vm(&self, config: &VmConfig) -> Result<()> {
        let _ = self
            .request(Method::PUT, "/api/v1/vm.create", Some(config))
            .await?;
        Ok(())
    }

    /// Boot the created VM
    pub async fn boot_vm(&self) -> Result<()> {
        let _ = self
            .request(Method::PUT, "/api/v1/vm.boot", None::<&()>)
            .await?;
        Ok(())
    }

    /// Stop the VM cleanly
    #[allow(dead_code)]
    pub async fn shutdown_vm(&self) -> Result<()> {
        let _ = self
            .request(Method::PUT, "/api/v1/vm.shutdown", None::<&()>)
            .await?;
        Ok(())
    }

    async fn request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<Bytes> {
        let stream =
            tokio::time::timeout(self.connect_timeout, UnixStream::connect(&self.socket_path))
                .await
                .map_err(|_| CloudHypervisorApiTimeout {
                    stage: "connect",
                    timeout: self.connect_timeout,
                    socket_path: self.socket_path.clone(),
                })?
                .with_context(|| format!("Failed to connect to {}", self.socket_path))?;
        self.request_on_stream(stream, method, path, body).await
    }

    async fn request_on_stream<T, S>(
        &self,
        stream: S,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<Bytes>
    where
        T: Serialize,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        tokio::time::timeout(
            self.request_timeout,
            self.exchange(stream, method, path, body),
        )
        .await
        .map_err(|_| CloudHypervisorApiTimeout {
            stage: "request (including response body)",
            timeout: self.request_timeout,
            socket_path: self.socket_path.clone(),
        })?
    }

    async fn exchange<T, S>(
        &self,
        stream: S,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<Bytes>
    where
        T: Serialize,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let io = TokioIo::new(stream);

        // 2. Perform the HTTP handshake
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .context("HTTP handshake failed")?;

        // Cancel the connection task as well if the request is cancelled or times out.
        struct ConnectionTask(tokio::task::JoinHandle<()>);
        impl Drop for ConnectionTask {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _connection = ConnectionTask(tokio::spawn(async move {
            let _ = conn.await;
        }));

        // 3. Prepare the request
        let req_body = match body {
            Some(b) => Full::new(Bytes::from(serde_json::to_vec(b)?)),
            None => Full::new(Bytes::new()),
        };

        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "localhost")
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(req_body)
            .context("Failed to build request")?;

        let res = sender
            .send_request(req)
            .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn sends_vm_configuration_and_preserves_api_errors() {
        // In-memory transport exercises the exact HTTP exchange without host sockets.
        let (client_stream, mut stream) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with("PUT /api/v1/vm.create HTTP/1.1"));
            let length = header
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let config: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(config["memory"]["size"], 128 * 1024 * 1024);
            assert_eq!(config["cpus"]["kvm_hyperv"], true);
            let error = r#"{"error":"unsupported test configuration"}"#;
            stream.write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", error.len(), error).as_bytes()).await.unwrap();
        });
        let mut config = VmConfig::default();
        config.memory.size = 128 * 1024 * 1024;
        config.cpus.kvm_hyperv = Some(true);
        let error = CloudHypervisorClient::new("unused-test-path")
            .request_on_stream(
                client_stream,
                Method::PUT,
                "/api/v1/vm.create",
                Some(&config),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unsupported test configuration"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn request_timeout_includes_stalled_response_body() {
        // In-memory transport exercises the exact HTTP exchange without host sockets.
        let (client_stream, mut stream) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let mut client = CloudHypervisorClient::new("unused-test-path");
        client.request_timeout = Duration::from_millis(100);
        let error = client
            .request_on_stream(client_stream, Method::GET, "/api/v1/vmm.ping", None::<&()>)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<CloudHypervisorApiTimeout>().is_some());
        server.abort();
    }
}
