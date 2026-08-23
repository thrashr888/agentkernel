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

    /// Discover server-supported and ready sandbox backends.
    pub async fn get_backends(&self) -> anyhow::Result<BackendDiscovery> {
        self.request(reqwest::Method::GET, "/backends", None::<&()>)
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

    pub async fn update_sandbox(
        &self,
        name: &str,
        labels: Option<std::collections::HashMap<String, String>>,
        description: Option<String>,
    ) -> anyhow::Result<SandboxInfo> {
        #[derive(serde::Serialize)]
        struct Body {
            #[serde(skip_serializing_if = "Option::is_none")]
            labels: Option<std::collections::HashMap<String, String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }
        self.request(
            reqwest::Method::PATCH,
            &format!("/sandboxes/{name}"),
            Some(&Body {
                labels,
                description,
            }),
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

    /// Run multiple commands in parallel using the batch endpoint.
    pub async fn batch_run(&self, commands: Vec<BatchCommand>) -> anyhow::Result<BatchRunResponse> {
        #[derive(serde::Serialize)]
        struct BatchRunRequest {
            commands: Vec<BatchCommand>,
        }

        self.request(
            reqwest::Method::POST,
            "/batch/run",
            Some(&BatchRunRequest { commands }),
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

    /// Preview or confirm an AgentKernel integration install on the server host.
    pub async fn install_agent_integration(
        &self,
        name: &str,
        scope: &str,
        confirm: bool,
    ) -> anyhow::Result<crate::types::AgentIntegrationResult> {
        #[derive(serde::Serialize)]
        struct Request<'a> {
            scope: &'a str,
            confirm: bool,
        }
        self.request(
            reqwest::Method::POST,
            &format!("/agents/{name}/integration"),
            Some(&Request { scope, confirm }),
        )
        .await
    }

    // -----------------------------------------------------------------
    // LLM Usage
    // -----------------------------------------------------------------

    /// Get LLM usage across all sandboxes.
    pub async fn get_llm_usage(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<crate::types::LlmUsageEntry>>> {
        self.request(reqwest::Method::GET, "/llm/usage", None::<&()>)
            .await
    }

    /// Get LLM usage for a specific sandbox.
    pub async fn get_llm_usage_by_sandbox(
        &self,
        sandbox: &str,
    ) -> anyhow::Result<Vec<crate::types::LlmUsageEntry>> {
        self.request(
            reqwest::Method::GET,
            &format!("/llm/usage/{sandbox}"),
            None::<&()>,
        )
        .await
    }

    /// Get identity-aware daily LLM spend aggregates.
    pub async fn get_llm_spend(&self) -> anyhow::Result<crate::types::LlmSpendReport> {
        self.request(reqwest::Method::GET, "/llm/spend", None::<&()>)
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

    /// Get the authenticated user's and organization's sandbox quotas.
    pub async fn get_quotas(&self) -> anyhow::Result<crate::types::QuotaStatus> {
        self.request(reqwest::Method::GET, "/quotas", None::<&()>)
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
    // Durable Objects
    // -----------------------------------------------------------------

    pub async fn list_objects(&self) -> anyhow::Result<Vec<crate::types::DurableObjectInfo>> {
        self.request(reqwest::Method::GET, "/objects", None::<&()>)
            .await
    }

    pub async fn get_object(&self, id: &str) -> anyhow::Result<crate::types::DurableObjectInfo> {
        self.request(reqwest::Method::GET, &format!("/objects/{id}"), None::<&()>)
            .await
    }

    pub async fn create_object(
        &self,
        req: &crate::types::CreateObjectRequest,
    ) -> anyhow::Result<crate::types::DurableObjectInfo> {
        self.request(reqwest::Method::POST, "/objects", Some(req))
            .await
    }

    pub async fn delete_object(&self, id: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/objects/{id}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    pub async fn patch_object(
        &self,
        id: &str,
        storage: Option<serde_json::Value>,
        status: Option<String>,
    ) -> anyhow::Result<crate::types::DurableObjectInfo> {
        #[derive(serde::Serialize)]
        struct PatchBody {
            #[serde(skip_serializing_if = "Option::is_none")]
            storage: Option<serde_json::Value>,
            #[serde(skip_serializing_if = "Option::is_none")]
            status: Option<String>,
        }
        self.request(
            reqwest::Method::PATCH,
            &format!("/objects/{id}"),
            Some(&PatchBody { storage, status }),
        )
        .await
    }

    pub async fn call_object(
        &self,
        class: &str,
        object_id: &str,
        method: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.request(
            reqwest::Method::POST,
            &format!("/objects/{class}/{object_id}/call/{method}"),
            Some(&args),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Schedules
    // -----------------------------------------------------------------

    pub async fn list_schedules(&self) -> anyhow::Result<Vec<crate::types::ScheduleInfo>> {
        self.request(reqwest::Method::GET, "/schedules", None::<&()>)
            .await
    }

    pub async fn get_schedule(&self, id: &str) -> anyhow::Result<crate::types::ScheduleInfo> {
        self.request(
            reqwest::Method::GET,
            &format!("/schedules/{id}"),
            None::<&()>,
        )
        .await
    }

    pub async fn create_schedule(
        &self,
        req: &crate::types::CreateScheduleRequest,
    ) -> anyhow::Result<crate::types::ScheduleInfo> {
        self.request(reqwest::Method::POST, "/schedules", Some(req))
            .await
    }

    pub async fn delete_schedule(&self, id: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/schedules/{id}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    pub async fn trigger_schedule(&self, id: &str) -> anyhow::Result<ScheduleInfo> {
        self.request(
            reqwest::Method::POST,
            &format!("/schedules/{id}/trigger"),
            None::<&()>,
        )
        .await
    }

    // -----------------------------------------------------------------
    // Durable Stores
    // -----------------------------------------------------------------

    pub async fn list_stores(&self) -> anyhow::Result<Vec<crate::types::DurableStoreInfo>> {
        self.request(reqwest::Method::GET, "/stores", None::<&()>)
            .await
    }

    pub async fn get_store(&self, id: &str) -> anyhow::Result<crate::types::DurableStoreInfo> {
        self.request(reqwest::Method::GET, &format!("/stores/{id}"), None::<&()>)
            .await
    }

    pub async fn create_store(
        &self,
        req: &crate::types::CreateStoreRequest,
    ) -> anyhow::Result<crate::types::DurableStoreInfo> {
        self.request(reqwest::Method::POST, "/stores", Some(req))
            .await
    }

    pub async fn delete_store(&self, id: &str) -> anyhow::Result<()> {
        let _: serde_json::Value = self
            .request(
                reqwest::Method::DELETE,
                &format!("/stores/{id}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    pub async fn query_store(
        &self,
        id: &str,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> anyhow::Result<crate::types::StoreQueryResult> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            sql: &'a str,
            params: Vec<serde_json::Value>,
        }
        self.request(
            reqwest::Method::POST,
            &format!("/stores/{id}/query"),
            Some(&Body { sql, params }),
        )
        .await
    }

    pub async fn execute_store(
        &self,
        id: &str,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> anyhow::Result<crate::types::StoreExecuteResult> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            sql: &'a str,
            params: Vec<serde_json::Value>,
        }
        self.request(
            reqwest::Method::POST,
            &format!("/stores/{id}/execute"),
            Some(&Body { sql, params }),
        )
        .await
    }

    pub async fn command_store(
        &self,
        id: &str,
        command: Vec<String>,
    ) -> anyhow::Result<crate::types::StoreCommandResult> {
        #[derive(serde::Serialize)]
        struct Body {
            command: Vec<String>,
        }
        self.request(
            reqwest::Method::POST,
            &format!("/stores/{id}/command"),
            Some(&Body { command }),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Docker Images
    // -----------------------------------------------------------------

    /// List cached Docker images.
    pub async fn list_images(&self) -> anyhow::Result<Vec<crate::types::DockerImage>> {
        self.request(reqwest::Method::GET, "/images", None::<&()>)
            .await
    }

    /// Remove a Docker image by ID.
    pub async fn remove_image(&self, id: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/images/{id}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    /// Show container-runtime disk usage by resource type.
    pub async fn image_disk_usage(
        &self,
    ) -> anyhow::Result<Vec<crate::types::DockerImageDiskUsage>> {
        self.request(reqwest::Method::GET, "/images/usage", None::<&()>)
            .await
    }

    /// Pull an image into the local container-runtime cache.
    pub async fn pull_image(&self, image: &str) -> anyhow::Result<String> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            image: &'a str,
        }
        self.request(reqwest::Method::POST, "/images/pull", Some(&Body { image }))
            .await
    }

    /// Remove unused images. AgentKernel-only pruning leaves unrelated images alone.
    pub async fn prune_images(&self, agentkernel_only: bool) -> anyhow::Result<String> {
        #[derive(serde::Serialize)]
        struct Body {
            agentkernel_only: bool,
        }
        self.request(
            reqwest::Method::POST,
            "/images/prune",
            Some(&Body { agentkernel_only }),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Benchmark
    // -----------------------------------------------------------------

    /// Run a hardware benchmark (create, exec, destroy cycle).
    pub async fn run_benchmark(&self) -> anyhow::Result<crate::types::BenchmarkResult> {
        self.request(reqwest::Method::POST, "/benchmark", None::<&()>)
            .await
    }

    // -----------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------

    /// List all asciicast session recordings.
    pub async fn list_sessions(&self) -> anyhow::Result<Vec<crate::types::SessionSummary>> {
        self.request(reqwest::Method::GET, "/sessions", None::<&()>)
            .await
    }

    /// Get session recording metadata and parsed events.
    pub async fn get_session(&self, id: &str) -> anyhow::Result<crate::types::SessionRecording> {
        self.request(
            reqwest::Method::GET,
            &format!("/sessions/{id}"),
            None::<&()>,
        )
        .await
    }

    /// Get the original asciicast v2 artifact for a session.
    pub async fn get_session_cast(&self, id: &str) -> anyhow::Result<String> {
        let url = format!("{}/sessions/{id}/cast", self.base_url);
        let response = self.http.get(url).send().await?;
        let status = response.status().as_u16();
        let text = response.text().await?;
        if status >= 400 {
            anyhow::bail!("API error (HTTP {status}): {text}");
        }
        Ok(text)
    }

    // -----------------------------------------------------------------
    // Export/Import Config
    // -----------------------------------------------------------------

    /// Export sandbox configuration as TOML.
    pub async fn export_sandbox_config(&self, name: &str) -> anyhow::Result<String> {
        self.request(
            reqwest::Method::GET,
            &format!("/sandboxes/{name}/config"),
            None::<&()>,
        )
        .await
    }

    /// Import sandbox configuration from TOML.
    pub async fn import_sandbox_config(
        &self,
        name: Option<&str>,
        config: &str,
    ) -> anyhow::Result<SandboxInfo> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
            config: &'a str,
        }
        self.request(
            reqwest::Method::POST,
            "/sandboxes/import-config",
            Some(&Body { name, config }),
        )
        .await
    }

    // -----------------------------------------------------------------
    // Interactive Permissions
    // -----------------------------------------------------------------

    /// List active permission grants.
    pub async fn list_permissions(&self) -> anyhow::Result<Vec<crate::types::PermissionGrant>> {
        self.request(reqwest::Method::GET, "/permissions", None::<&()>)
            .await
    }

    /// Grant a permission.
    pub async fn grant_permission(
        &self,
        req: &crate::types::GrantPermissionRequest,
    ) -> anyhow::Result<serde_json::Value> {
        self.request(reqwest::Method::POST, "/permissions/grant", Some(req))
            .await
    }

    /// Revoke a permission grant.
    pub async fn revoke_permission(&self, id: &str) -> anyhow::Result<()> {
        let _: String = self
            .request(
                reqwest::Method::DELETE,
                &format!("/permissions/{id}"),
                None::<&()>,
            )
            .await?;
        Ok(())
    }

    /// Check if a permission is granted.
    pub async fn check_permission(
        &self,
        kind: &str,
        sandbox: Option<&str>,
    ) -> anyhow::Result<crate::types::PermissionCheckResult> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            kind: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            sandbox: Option<&'a str>,
        }
        self.request(
            reqwest::Method::POST,
            "/permissions/check",
            Some(&Body { kind, sandbox }),
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
