use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use std::time::Duration;

use crate::error::{error_from_status, Error, Result};
use crate::types::*;

const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_BASE_URL: &str = "http://localhost:18888";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Builder for constructing an [`AgentKernel`] client.
pub struct AgentKernelBuilder {
    base_url: String,
    api_key: Option<String>,
    timeout: Duration,
}

impl AgentKernelBuilder {
    /// Set the base URL.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set the API key for Bearer authentication.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build the client.
    pub fn build(self) -> Result<AgentKernel> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("agentkernel-rust-sdk/{SDK_VERSION}")).unwrap(),
        );
        if let Some(ref key) = self.api_key {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {key}"))
                    .map_err(|e| Error::Auth(e.to_string()))?,
            );
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(self.timeout)
            .build()?;

        Ok(AgentKernel {
            base_url: self.base_url.trim_end_matches('/').to_string(),
            http,
        })
    }
}

/// Client for the agentkernel HTTP API.
///
/// # Example
/// ```no_run
/// # async fn example() -> agentkernel_sdk::Result<()> {
/// let client = agentkernel_sdk::AgentKernel::builder().build()?;
/// let output = client.run(&["echo", "hello"], None).await?;
/// println!("{}", output.output);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AgentKernel {
    base_url: String,
    http: reqwest::Client,
}

impl AgentKernel {
    /// Create a new builder with defaults resolved from env vars.
    pub fn builder() -> AgentKernelBuilder {
        AgentKernelBuilder {
            base_url: std::env::var("AGENTKERNEL_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string()),
            api_key: std::env::var("AGENTKERNEL_API_KEY").ok(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Health check. Returns `"ok"`.
    pub async fn health(&self) -> Result<String> {
        self.request::<String>(reqwest::Method::GET, "/health", None::<&()>)
            .await
    }

    /// Run a command in a temporary sandbox.
    pub async fn run(&self, command: &[&str], opts: Option<RunOptions>) -> Result<RunOutput> {
        let opts = opts.unwrap_or_default();
        let body = RunRequest {
            command: command.iter().map(|s| s.to_string()).collect(),
            image: opts.image,
            profile: opts.profile,
            fast: opts.fast.unwrap_or(true),
        };
        self.request(reqwest::Method::POST, "/run", Some(&body))
            .await
    }

    /// List all sandboxes.
    pub async fn list_sandboxes(&self) -> Result<Vec<SandboxInfo>> {
        self.request(reqwest::Method::GET, "/sandboxes", None::<&()>)
            .await
    }

    /// Create a new sandbox with optional resource limits.
    pub async fn create_sandbox(
        &self,
        name: &str,
        image: Option<&str>,
        vcpus: Option<u32>,
        memory_mb: Option<u64>,
        profile: Option<SecurityProfile>,
    ) -> Result<SandboxInfo> {
        let body = CreateRequest {
            name: name.to_string(),
            image: image.map(String::from),
            vcpus,
            memory_mb,
            profile,
        };
        self.request(reqwest::Method::POST, "/sandboxes", Some(&body))
            .await
    }

    /// Get info about a sandbox.
    pub async fn get_sandbox(&self, name: &str) -> Result<SandboxInfo> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}"),
            None::<&()>,
        )
        .await
    }

    /// Remove a sandbox.
    pub async fn remove_sandbox(&self, name: &str) -> Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/sandboxes/{name}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    /// Run a command in an existing sandbox.
    pub async fn exec_in_sandbox(&self, name: &str, command: &[&str]) -> Result<RunOutput> {
        let body = ExecRequest {
            command: command.iter().map(|s| s.to_string()).collect(),
        };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/exec"),
            Some(&body),
        )
        .await
    }

    /// Create a sandbox and return a guard that removes it on drop.
    ///
    /// Use `with_sandbox` for guaranteed cleanup via a closure.
    pub async fn with_sandbox<F, Fut, T>(&self, name: &str, image: Option<&str>, f: F) -> Result<T>
    where
        F: FnOnce(SandboxHandle) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.create_sandbox(name, image, None, None, None).await?;
        let handle = SandboxHandle {
            name: name.to_string(),
            client: self.clone(),
        };
        let result = f(handle).await;
        // Always clean up
        let _ = self.remove_sandbox(name).await;
        result
    }

    /// Read a file from a sandbox.
    pub async fn read_file(&self, name: &str, path: &str) -> Result<FileReadResponse> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/files/{path}"),
            None::<&()>,
        )
        .await
    }

    /// Write a file to a sandbox.
    pub async fn write_file(
        &self,
        name: &str,
        path: &str,
        content: &str,
        encoding: Option<&str>,
    ) -> Result<String> {
        let body = FileWriteRequest {
            content: content.to_string(),
            encoding: encoding.map(String::from),
        };
        self.request(
            reqwest::Method::PUT,
            &format!("/sandboxes/{name}/files/{path}"),
            Some(&body),
        )
        .await
    }

    /// Delete a file from a sandbox.
    pub async fn delete_file(&self, name: &str, path: &str) -> Result<String> {
        self.request(
            reqwest::Method::DELETE,
            &format!("/sandboxes/{name}/files/{path}"),
            None::<&()>,
        )
        .await
    }

    /// Get audit log entries for a sandbox.
    pub async fn get_sandbox_logs(&self, name: &str) -> Result<Vec<serde_json::Value>> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/logs"),
            None::<&()>,
        )
        .await
    }

    /// Run multiple commands in parallel.
    pub async fn batch_run(&self, commands: Vec<BatchCommand>) -> Result<BatchRunResponse> {
        let body = BatchRunRequest { commands };
        self.request(reqwest::Method::POST, "/batch/run", Some(&body))
            .await
    }

    // -- Internal --

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&(impl serde::Serialize + ?Sized)>,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.request(method, &url);
        if let Some(b) = body {
            req = req.header(CONTENT_TYPE, "application/json").json(b);
        }

        let response = req.send().await?;
        let status = response.status().as_u16();
        let text = response.text().await?;

        if status >= 400 {
            return Err(error_from_status(status, &text));
        }

        let parsed: ApiResponse<T> = serde_json::from_str(&text)?;
        if !parsed.success {
            return Err(Error::Server(
                parsed.error.unwrap_or_else(|| "Unknown error".to_string()),
            ));
        }
        parsed
            .data
            .ok_or_else(|| Error::Server("Missing data field".to_string()))
    }
}

/// Handle to a sandbox within a `with_sandbox` closure.
///
/// Owns a clone of the client (cheap — `reqwest::Client` is `Arc`-backed).
pub struct SandboxHandle {
    name: String,
    client: AgentKernel,
}

impl SandboxHandle {
    /// The sandbox name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Run a command in this sandbox.
    pub async fn run(&self, command: &[&str]) -> Result<RunOutput> {
        self.client.exec_in_sandbox(&self.name, command).await
    }

    /// Get sandbox info.
    pub async fn info(&self) -> Result<SandboxInfo> {
        self.client.get_sandbox(&self.name).await
    }

    /// Read a file from this sandbox.
    pub async fn read_file(&self, path: &str) -> Result<FileReadResponse> {
        self.client.read_file(&self.name, path).await
    }

    /// Write a file to this sandbox.
    pub async fn write_file(&self, path: &str, content: &str, encoding: Option<&str>) -> Result<String> {
        self.client.write_file(&self.name, path, content, encoding).await
    }

    /// Delete a file from this sandbox.
    pub async fn delete_file(&self, path: &str) -> Result<String> {
        self.client.delete_file(&self.name, path).await
    }
}
