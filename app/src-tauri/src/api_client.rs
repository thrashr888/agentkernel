use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use std::time::Duration;

use crate::types::*;

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// HTTP client for the agentkernel API.
#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    http: reqwest::Client,
}

impl ApiClient {
    /// Create a new API client pointed at `base_url`.
    ///
    /// If `api_key` is provided it is sent as a Bearer token on every request.
    pub fn new(base_url: &str, api_key: Option<&str>) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("agentkernel-desktop/0.1.0"),
        );
        if let Some(key) = api_key {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {key}")) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("failed to build reqwest client");

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    // -----------------------------------------------------------------
    // Health
    // -----------------------------------------------------------------

    /// Health check. Returns `"ok"` on success.
    pub async fn health(&self) -> anyhow::Result<String> {
        self.request::<String>(reqwest::Method::GET, "/health", None::<&()>)
            .await
    }

    // -----------------------------------------------------------------
    // Sandboxes
    // -----------------------------------------------------------------

    /// List all sandboxes.
    pub async fn list_sandboxes(&self) -> anyhow::Result<Vec<SandboxInfo>> {
        self.request(reqwest::Method::GET, "/sandboxes", None::<&()>)
            .await
    }

    /// Get info about a single sandbox.
    pub async fn get_sandbox(&self, name: &str) -> anyhow::Result<SandboxInfo> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}"),
            None::<&()>,
        )
        .await
    }

    /// Create a new sandbox.
    pub async fn create_sandbox(&self, req: &CreateSandboxRequest) -> anyhow::Result<SandboxInfo> {
        self.request(reqwest::Method::POST, "/sandboxes", Some(req))
            .await
    }

    /// Remove a sandbox.
    pub async fn remove_sandbox(&self, name: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/sandboxes/{name}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    /// Extend a sandbox's time-to-live.
    pub async fn extend_ttl(
        &self,
        name: &str,
        by: &str,
    ) -> anyhow::Result<ExtendTtlResponse> {
        let body = ExtendTtlRequest {
            by: by.to_string(),
        };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/extend"),
            Some(&body),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Exec
    // -----------------------------------------------------------------

    /// Execute a command inside an existing sandbox.
    pub async fn exec_in_sandbox(
        &self,
        name: &str,
        command: Vec<String>,
        env: Vec<String>,
        workdir: Option<String>,
    ) -> anyhow::Result<RunOutput> {
        let body = ExecRequest {
            command,
            env,
            workdir,
            sudo: None,
        };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/exec"),
            Some(&body),
        )
        .await
    }

    /// Start a detached (background) command.
    pub async fn exec_detached(
        &self,
        name: &str,
        command: Vec<String>,
    ) -> anyhow::Result<DetachedCommand> {
        let body = ExecRequest {
            command,
            env: Vec::new(),
            workdir: None,
            sudo: None,
        };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/exec/detach"),
            Some(&body),
        )
        .await
    }

    /// List detached commands in a sandbox.
    pub async fn list_detached(&self, name: &str) -> anyhow::Result<Vec<DetachedCommand>> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/exec/detached"),
            None::<&()>,
        )
        .await
    }

    /// Get logs from a detached command.
    pub async fn detached_logs(
        &self,
        name: &str,
        cmd_id: &str,
    ) -> anyhow::Result<DetachedLogsResponse> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/exec/detached/{cmd_id}/logs"),
            None::<&()>,
        )
        .await
    }

    /// Kill a detached command.
    pub async fn kill_detached(
        &self,
        name: &str,
        cmd_id: &str,
    ) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/sandboxes/{name}/exec/detached/{cmd_id}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Snapshots
    // -----------------------------------------------------------------

    /// List all snapshots.
    pub async fn list_snapshots(&self) -> anyhow::Result<Vec<SnapshotMeta>> {
        self.request(reqwest::Method::GET, "/snapshots", None::<&()>)
            .await
    }

    /// Take a snapshot of a sandbox.
    pub async fn take_snapshot(
        &self,
        sandbox: &str,
        name: Option<&str>,
    ) -> anyhow::Result<SnapshotMeta> {
        let body = TakeSnapshotRequest {
            sandbox: sandbox.to_string(),
            name: name.map(String::from),
        };
        self.request(reqwest::Method::POST, "/snapshots", Some(&body))
            .await
    }

    /// Delete a snapshot.
    pub async fn delete_snapshot(&self, name: &str) -> anyhow::Result<()> {
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
    pub async fn restore_snapshot(&self, name: &str) -> anyhow::Result<SandboxInfo> {
        self.request(
            reqwest::Method::POST,
            &format!("/snapshots/{name}/restore"),
            None::<&()>,
        )
        .await
    }

    // -----------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&(impl serde::Serialize + ?Sized)>,
    ) -> anyhow::Result<T> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.request(method, &url);
        if let Some(b) = body {
            req = req.header(CONTENT_TYPE, "application/json").json(b);
        }

        let response = req.send().await?;
        let status = response.status().as_u16();
        let text = response.text().await?;

        if status >= 400 {
            anyhow::bail!("API error (HTTP {status}): {text}");
        }

        let parsed: ApiResponse<T> = serde_json::from_str(&text)?;
        if !parsed.success {
            anyhow::bail!(
                "{}",
                parsed.error.unwrap_or_else(|| "Unknown error".to_string())
            );
        }
        parsed
            .data
            .ok_or_else(|| anyhow::anyhow!("Missing data field in response"))
    }
}
