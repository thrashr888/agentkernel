use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use std::time::Duration;

use crate::error::{Error, Result, error_from_status};
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

    /// Create a new sandbox with optional configuration.
    pub async fn create_sandbox(
        &self,
        name: &str,
        opts: Option<CreateSandboxOptions>,
    ) -> Result<SandboxInfo> {
        let opts = opts.unwrap_or_default();
        let body = CreateRequest {
            name: name.to_string(),
            image: opts.image,
            vcpus: opts.vcpus,
            memory_mb: opts.memory_mb,
            profile: opts.profile,
            source_url: opts.source_url,
            source_ref: opts.source_ref,
            volumes: opts.volumes,
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
    pub async fn exec_in_sandbox(
        &self,
        name: &str,
        command: &[&str],
        opts: Option<ExecOptions>,
    ) -> Result<RunOutput> {
        let opts = opts.unwrap_or_default();
        let body = ExecRequest {
            command: command.iter().map(|s| s.to_string()).collect(),
            env: opts.env,
            workdir: opts.workdir,
            sudo: opts.sudo,
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
    pub async fn with_sandbox<F, Fut, T>(
        &self,
        name: &str,
        opts: Option<CreateSandboxOptions>,
        f: F,
    ) -> Result<T>
    where
        F: FnOnce(SandboxHandle) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.create_sandbox(name, opts).await?;
        let handle = SandboxHandle {
            name: name.to_string(),
            client: self.clone(),
        };
        let result = f(handle).await;
        // Always clean up
        let _ = self.remove_sandbox(name).await;
        result
    }

    /// Write multiple files to a sandbox in one request.
    pub async fn write_files(
        &self,
        name: &str,
        files: std::collections::HashMap<String, String>,
    ) -> Result<BatchFileWriteResponse> {
        let body = BatchFileWriteRequest { files };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/files"),
            Some(&body),
        )
        .await
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

    /// Start a detached (background) command in a sandbox.
    pub async fn exec_detached(
        &self,
        name: &str,
        command: &[&str],
        opts: Option<ExecOptions>,
    ) -> Result<DetachedCommand> {
        let opts = opts.unwrap_or_default();
        let body = ExecRequest {
            command: command.iter().map(|s| s.to_string()).collect(),
            env: opts.env,
            workdir: opts.workdir,
            sudo: opts.sudo,
        };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/exec/detach"),
            Some(&body),
        )
        .await
    }

    /// Get the status of a detached command.
    pub async fn detached_status(&self, name: &str, cmd_id: &str) -> Result<DetachedCommand> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/exec/detached/{cmd_id}"),
            None::<&()>,
        )
        .await
    }

    /// Get logs from a detached command.
    pub async fn detached_logs(
        &self,
        name: &str,
        cmd_id: &str,
        stream: Option<&str>,
    ) -> Result<DetachedLogsResponse> {
        let query = match stream {
            Some(s) => format!("?stream={s}"),
            None => String::new(),
        };
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/exec/detached/{cmd_id}/logs{query}"),
            None::<&()>,
        )
        .await
    }

    /// Kill a detached command.
    pub async fn detached_kill(&self, name: &str, cmd_id: &str) -> Result<String> {
        self.request(
            reqwest::Method::DELETE,
            &format!("/sandboxes/{name}/exec/detached/{cmd_id}"),
            None::<&()>,
        )
        .await
    }

    /// List detached commands in a sandbox.
    pub async fn detached_list(&self, name: &str) -> Result<Vec<DetachedCommand>> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/exec/detached"),
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

    /// Extend a sandbox's time-to-live.
    pub async fn extend_ttl(&self, name: &str, by: &str) -> Result<ExtendTtlResponse> {
        let body = ExtendTtlRequest { by: by.to_string() };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/extend"),
            Some(&body),
        )
        .await
    }

    /// List all snapshots.
    pub async fn list_snapshots(&self) -> Result<Vec<SnapshotMeta>> {
        self.request(reqwest::Method::GET, "/snapshots", None::<&()>)
            .await
    }

    /// Take a snapshot of a sandbox.
    pub async fn take_snapshot(&self, opts: TakeSnapshotOptions) -> Result<SnapshotMeta> {
        self.request(reqwest::Method::POST, "/snapshots", Some(&opts))
            .await
    }

    /// Get info about a snapshot.
    pub async fn get_snapshot(&self, name: &str) -> Result<SnapshotMeta> {
        self.request(
            reqwest::Method::GET,
            &format!("/snapshots/{name}"),
            None::<&()>,
        )
        .await
    }

    /// Delete a snapshot.
    pub async fn delete_snapshot(&self, name: &str) -> Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/snapshots/{name}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    /// Restore a sandbox from a snapshot.
    pub async fn restore_snapshot(&self, name: &str) -> Result<SandboxInfo> {
        self.request(
            reqwest::Method::POST,
            &format!("/snapshots/{name}/restore"),
            None::<&()>,
        )
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
        self.client.exec_in_sandbox(&self.name, command, None).await
    }

    /// Run a command with options (workdir, env, sudo).
    pub async fn run_with_options(&self, command: &[&str], opts: ExecOptions) -> Result<RunOutput> {
        self.client
            .exec_in_sandbox(&self.name, command, Some(opts))
            .await
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
    pub async fn write_file(
        &self,
        path: &str,
        content: &str,
        encoding: Option<&str>,
    ) -> Result<String> {
        self.client
            .write_file(&self.name, path, content, encoding)
            .await
    }

    /// Write multiple files to this sandbox.
    pub async fn write_files(
        &self,
        files: std::collections::HashMap<String, String>,
    ) -> Result<BatchFileWriteResponse> {
        self.client.write_files(&self.name, files).await
    }

    /// Delete a file from this sandbox.
    pub async fn delete_file(&self, path: &str) -> Result<String> {
        self.client.delete_file(&self.name, path).await
    }
}
