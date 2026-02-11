use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
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
        headers.insert(USER_AGENT, HeaderValue::from_static("agentkernel/0.1.0"));
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
    // Diagnostics
    // -----------------------------------------------------------------

    /// Get system status information.
    pub async fn get_status(&self) -> anyhow::Result<StatusInfo> {
        self.request(reqwest::Method::GET, "/status", None::<&()>)
            .await
    }

    /// Run health checks.
    pub async fn get_doctor(&self) -> anyhow::Result<DoctorResult> {
        self.request(reqwest::Method::GET, "/doctor", None::<&()>)
            .await
    }

    /// Run garbage collection to remove expired sandboxes.
    pub async fn run_gc(&self) -> anyhow::Result<GcResult> {
        self.request(reqwest::Method::POST, "/gc", None::<&()>)
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

    /// Start a stopped sandbox.
    pub async fn start_sandbox(&self, name: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::POST,
                &format!("/sandboxes/{name}/start"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    /// Stop a running sandbox.
    pub async fn stop_sandbox(&self, name: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::POST,
                &format!("/sandboxes/{name}/stop"),
                None::<&()>,
            )
            .await?;
        Ok(())
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

    /// Get audit logs for a sandbox.
    pub async fn get_sandbox_logs(&self, name: &str) -> anyhow::Result<Vec<AuditLogEntry>> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/logs"),
            None::<&()>,
        )
        .await
    }

    /// Extend a sandbox's time-to-live.
    pub async fn extend_ttl(&self, name: &str, by: &str) -> anyhow::Result<ExtendTtlResponse> {
        let body = ExtendTtlRequest { by: by.to_string() };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/extend"),
            Some(&body),
        )
        .await
    }

    pub async fn resize_sandbox(
        &self,
        name: &str,
        vcpus: Option<u32>,
        memory_mb: Option<u64>,
    ) -> anyhow::Result<SandboxInfo> {
        #[derive(serde::Serialize)]
        struct Body {
            #[serde(skip_serializing_if = "Option::is_none")]
            vcpus: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            memory_mb: Option<u64>,
        }
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/resize"),
            Some(&Body { vcpus, memory_mb }),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Quick Run
    // -----------------------------------------------------------------

    /// Run a command in a temporary sandbox (`POST /run`).
    pub async fn quick_run(
        &self,
        command: Vec<String>,
        image: Option<String>,
        profile: Option<String>,
    ) -> anyhow::Result<RunOutput> {
        let body = QuickRunRequest {
            command,
            image,
            profile,
        };
        self.request(reqwest::Method::POST, "/run", Some(&body))
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
    pub async fn kill_detached(&self, name: &str, cmd_id: &str) -> anyhow::Result<()> {
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
    pub async fn restore_snapshot(
        &self,
        name: &str,
        as_name: Option<&str>,
    ) -> anyhow::Result<SandboxInfo> {
        #[derive(serde::Serialize)]
        struct RestoreBody<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            as_name: Option<&'a str>,
        }
        self.request(
            reqwest::Method::POST,
            &format!("/snapshots/{name}/restore"),
            Some(&RestoreBody { as_name }),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Files
    // -----------------------------------------------------------------

    /// List files in a directory (exec `ls -la` in the sandbox).
    pub async fn list_files(&self, name: &str, path: &str) -> anyhow::Result<RunOutput> {
        let body = ExecRequest {
            command: vec!["ls".to_string(), "-la".to_string(), path.to_string()],
            env: Vec::new(),
            workdir: None,
            sudo: None,
        };
        self.request(
            reqwest::Method::POST,
            &format!("/sandboxes/{name}/exec"),
            Some(&body),
        )
        .await
    }

    /// Read a file from a sandbox.
    pub async fn read_file(&self, name: &str, path: &str) -> anyhow::Result<FileReadResponse> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/files/{}", path.trim_start_matches('/')),
            None::<&()>,
        )
        .await
    }

    // -----------------------------------------------------------------
    // Audit
    // -----------------------------------------------------------------

    /// Get the global audit log, optionally limited to the last N entries.
    pub async fn get_audit_log(&self, last: Option<u32>) -> anyhow::Result<Vec<AuditLogEntry>> {
        let path = match last {
            Some(n) => format!("/audit?last={n}"),
            None => "/audit".to_string(),
        };
        self.request(reqwest::Method::GET, &path, None::<&()>).await
    }

    // -----------------------------------------------------------------
    // Secrets
    // -----------------------------------------------------------------

    /// List stored secret names (not values).
    pub async fn list_secrets(&self) -> anyhow::Result<Vec<crate::types::SecretEntry>> {
        self.request(reqwest::Method::GET, "/secrets", None::<&()>)
            .await
    }

    /// Store a new secret.
    pub async fn create_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            name: &'a str,
            value: &'a str,
        }
        let _: String = self
            .request(
                reqwest::Method::POST,
                "/secrets",
                Some(&Body { name, value }),
            )
            .await?;
        Ok(())
    }

    /// Delete a secret by name.
    pub async fn delete_secret(&self, name: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/secrets/{name}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Agents/Plugins
    // -----------------------------------------------------------------

    /// List agent integrations.
    pub async fn list_agents(&self) -> anyhow::Result<Vec<crate::types::AgentInfo>> {
        self.request(reqwest::Method::GET, "/agents", None::<&()>)
            .await
    }

    // -----------------------------------------------------------------
    // Policy (Enterprise)
    // -----------------------------------------------------------------

    /// Get enterprise policy status.
    pub async fn get_policy_status(&self) -> anyhow::Result<crate::types::PolicyStatus> {
        self.request(reqwest::Method::GET, "/policy/status", None::<&()>)
            .await
    }

    /// Run policy check.
    pub async fn check_policy(
        &self,
        action: &str,
        sandbox: &str,
    ) -> anyhow::Result<crate::types::PolicyCheckResult> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            action: &'a str,
            sandbox: &'a str,
        }
        self.request(
            reqwest::Method::POST,
            "/policy/check",
            Some(&Body { action, sandbox }),
        )
        .await
    }

    /// Force policy reload from server.
    pub async fn reload_policy(&self) -> anyhow::Result<crate::types::PolicyReloadResult> {
        self.request(reqwest::Method::POST, "/policy/reload", None::<&()>)
            .await
    }

    /// Get recent policy audit log entries.
    pub async fn get_policy_audit(
        &self,
        last: Option<u32>,
    ) -> anyhow::Result<Vec<crate::types::PolicyAuditEntry>> {
        let path = match last {
            Some(n) => format!("/policy/audit?last={n}"),
            None => "/policy/audit".to_string(),
        };
        self.request(reqwest::Method::GET, &path, None::<&()>).await
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
