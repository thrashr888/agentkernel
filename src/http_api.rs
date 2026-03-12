//! HTTP API server for agentkernel.
//!
//! Provides RESTful endpoints for sandbox management.
//!
//! ## Authentication
//!
//! API key authentication is optional. To enable:
//! - Set `AGENTKERNEL_API_KEY` environment variable
//! - Or configure `[api].api_key` / `[api].api_key_env` in `agentkernel.toml`
//!
//! Root execution (`sudo: true`) is disabled by default for HTTP API requests.
//! To allow it explicitly, set `[api].allow_sudo_exec = true`.
//!
//! When enabled, requests must include the API key in the Authorization header:
//! ```text
//! Authorization: Bearer <api_key>
//! ```

use anyhow::Result;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{Duration, sleep};

use crate::languages;
use crate::opencode::OpenCodeState;
use crate::orchestration_store::{
    CreateDurableObject, CreateDurableStore, CreateOrchestration, CreateSchedule,
    DurableObjectRecord, DurableStoreCommandResult, DurableStoreExecuteResult, DurableStoreKind,
    DurableStoreQueryResult, OrchestrationEvent, OrchestrationRecord, OrchestrationStatus,
    OrchestrationStore, ScheduleRecord, UpdateOrchestration,
};
use crate::permissions::SecurityProfile;
use crate::secrets::{SecretBackend, SecretVault};
use crate::validation;
use crate::vmm::VmManager;

pub type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;
const MAX_HTTP_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

fn full<T: Into<bytes::Bytes>>(chunk: T) -> BoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn request_body_too_large() -> Response<BoxBody> {
    json_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        &ApiResponse::<()>::error(format!(
            "Request body exceeds the maximum size of {MAX_HTTP_REQUEST_BODY_BYTES} bytes"
        )),
    )
}

fn invalid_content_length() -> Response<BoxBody> {
    json_response(
        StatusCode::BAD_REQUEST,
        &ApiResponse::<()>::error("Invalid Content-Length header"),
    )
}

async fn read_body_bytes(req: Request<Incoming>) -> Result<bytes::Bytes, Response<BoxBody>> {
    if let Some(content_length) = req.headers().get("content-length") {
        let len = match content_length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
        {
            Some(len) => len,
            None => return Err(invalid_content_length()),
        };

        if len > MAX_HTTP_REQUEST_BODY_BYTES {
            return Err(request_body_too_large());
        }
    }

    let body_bytes = req
        .collect()
        .await
        .map_err(|_| {
            json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error("Failed to read body"),
            )
        })?
        .to_bytes();

    if body_bytes.len() > MAX_HTTP_REQUEST_BODY_BYTES {
        return Err(request_body_too_large());
    }

    Ok(body_bytes)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn load_api_key_from_config() -> Option<String> {
    let config_path = std::path::PathBuf::from("agentkernel.toml");
    if !config_path.exists() {
        return None;
    }

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[api] Failed to read {}: {}", config_path.display(), e);
            return None;
        }
    };
    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[api] Failed to parse {}: {}", config_path.display(), e);
            return None;
        }
    };
    let api_cfg = parsed.get("api").and_then(|v| v.as_table())?;

    if let Some(env_name) = api_cfg
        .get("api_key_env")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Ok(key) = std::env::var(env_name)
        && !key.trim().is_empty()
    {
        return Some(key);
    }

    api_cfg
        .get("api_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::string::ToString::to_string)
}

fn load_api_key() -> Option<String> {
    if let Ok(key) = std::env::var("AGENTKERNEL_API_KEY")
        && !key.trim().is_empty()
    {
        return Some(key);
    }
    load_api_key_from_config()
}

fn load_api_allow_sudo_exec_from_config() -> bool {
    let config_path = std::path::PathBuf::from("agentkernel.toml");
    if !config_path.exists() {
        return false;
    }

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[api] Failed to read {}: {}", config_path.display(), e);
            return false;
        }
    };

    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[api] Failed to parse {}: {}", config_path.display(), e);
            return false;
        }
    };

    parsed
        .get("api")
        .and_then(|v| v.as_table())
        .and_then(|api| api.get("allow_sudo_exec"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Request to run a command
#[derive(Debug, Deserialize)]
struct RunRequest {
    command: Vec<String>,
    image: Option<String>,
    profile: Option<String>,
    /// Use container pool for faster execution (default: true for /run)
    #[serde(default = "default_fast")]
    fast: bool,
}

fn default_fast() -> bool {
    true // Default to fast mode for HTTP API
}

/// Lifecycle automation policy from API requests.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct LifecyclePolicyRequest {
    #[serde(default)]
    auto_stop_after_seconds: Option<u64>,
    #[serde(default)]
    auto_archive_after_seconds: Option<u64>,
    #[serde(default)]
    auto_delete_after_seconds: Option<u64>,
}

impl From<LifecyclePolicyRequest> for crate::vmm::SandboxLifecyclePolicy {
    fn from(value: LifecyclePolicyRequest) -> Self {
        Self {
            auto_stop_after_seconds: value.auto_stop_after_seconds,
            auto_archive_after_seconds: value.auto_archive_after_seconds,
            auto_delete_after_seconds: value.auto_delete_after_seconds,
        }
    }
}

/// Request to create a sandbox
#[derive(Debug, Deserialize)]
struct CreateRequest {
    name: String,
    image: Option<String>,
    vcpus: Option<u32>,
    memory_mb: Option<u64>,
    profile: Option<String>,
    /// Port mappings (e.g., ["8080:80", "3000", "5353:53/udp"])
    #[serde(default)]
    ports: Vec<String>,
    /// Git repo URL to clone into /workspace (e.g., "https://github.com/user/repo")
    #[serde(default)]
    source_url: Option<String>,
    /// Git ref to checkout after cloning (branch, tag, or commit)
    #[serde(default)]
    source_ref: Option<String>,
    /// Agent CLI to auto-install on start (e.g., "claude", "gemini", "codex")
    #[serde(default)]
    agent: Option<String>,
    /// Secret bindings for proxy injection (e.g., ["OPENAI_API_KEY:api.openai.com"])
    #[serde(default)]
    secrets: Vec<String>,
    /// Secret keys to inject as files (e.g., ["MY_SECRET"])
    #[serde(default)]
    secret_files: Vec<String>,
    /// Use placeholder tokens instead of real secret values
    #[serde(default)]
    placeholder_secrets: bool,
    /// Shell script to run inside sandbox after start (e.g., install CLIs)
    #[serde(default)]
    init_script: Option<String>,
    /// Template name used to create this sandbox (for UI provenance).
    #[serde(default)]
    created_from_template: Option<String>,
    /// Human help text from the selected template.
    #[serde(default)]
    template_help_text: Option<String>,
    /// Template secret mappings: env_var → target_host.
    /// Persisted so the UI can show expected secrets even when not yet bound.
    #[serde(default)]
    secret_mappings: std::collections::HashMap<String, String>,
    /// User-defined labels for fleet management and filtering.
    #[serde(default)]
    labels: std::collections::HashMap<String, String>,
    /// User-defined description.
    #[serde(default)]
    description: Option<String>,
    /// Lifecycle automation policy.
    #[serde(default)]
    lifecycle: Option<LifecyclePolicyRequest>,
}

/// Request to write a file
#[derive(Debug, Deserialize)]
struct FileWriteRequest {
    content: String,
    /// "utf8" (default) or "base64"
    #[serde(default = "default_encoding")]
    encoding: String,
}

fn default_encoding() -> String {
    "utf8".to_string()
}

/// Request to write multiple files at once
#[derive(Debug, Deserialize)]
struct BatchFileWriteRequest {
    files: std::collections::HashMap<String, String>,
}

/// Response for file read
#[derive(Debug, Serialize)]
struct FileReadResponse {
    content: String,
    encoding: String,
    size: usize,
}

/// Request for batch run
#[derive(Debug, Deserialize)]
struct BatchRunRequest {
    commands: Vec<BatchCommand>,
}

#[derive(Debug, Deserialize)]
struct BatchCommand {
    command: Vec<String>,
}

/// Response for batch run
#[derive(Debug, Serialize)]
struct BatchRunResponse {
    results: Vec<BatchResult>,
}

#[derive(Debug, Serialize)]
struct BatchResult {
    output: Option<String>,
    error: Option<String>,
}

/// Request to execute in a sandbox
#[derive(Debug, Deserialize)]
struct ExecRequest {
    command: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    workdir: Option<String>,
    #[serde(default)]
    sudo: Option<bool>,
}

/// Request to create an orchestration.
#[derive(Debug, Deserialize)]
struct CreateOrchestrationRequest {
    name: String,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

/// Request to update orchestration state.
#[derive(Debug, Deserialize)]
struct UpdateOrchestrationRequest {
    #[serde(default)]
    status: Option<OrchestrationStatus>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

/// Request to raise an external event for an orchestration.
#[derive(Debug, Deserialize)]
struct RaiseOrchestrationEventRequest {
    name: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

/// Request to terminate an orchestration.
#[derive(Debug, Deserialize)]
struct TerminateOrchestrationRequest {
    #[serde(default)]
    reason: Option<String>,
}

/// Request to create durable store metadata.
#[derive(Debug, Deserialize)]
struct CreateDurableStoreRequest {
    name: String,
    kind: DurableStoreKind,
    #[serde(default)]
    sandbox: Option<String>,
    #[serde(default)]
    config: Option<serde_json::Value>,
}

/// Request to run SQL against a durable store.
#[derive(Debug, Deserialize)]
struct DurableStoreSqlRequest {
    sql: String,
    #[serde(default)]
    params: Vec<serde_json::Value>,
}

/// Request to run command-oriented operations against a durable store.
#[derive(Debug, Deserialize)]
struct DurableStoreCommandRequest {
    command: Vec<String>,
}

/// Detailed orchestration payload including append-only history.
#[derive(Debug, Serialize)]
struct OrchestrationDetails {
    #[serde(flatten)]
    orchestration: OrchestrationRecord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    history: Vec<OrchestrationEvent>,
}

/// Runtime input contract for a server-side orchestration.
#[derive(Debug, Deserialize)]
struct RuntimeOrchestrationInput {
    #[serde(default)]
    wait_for_event: Option<String>,
    #[serde(default)]
    activity: Option<RuntimeActivity>,
    #[serde(default)]
    activities: Option<Vec<RuntimeActivity>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeActivity {
    #[serde(default = "default_activity_name")]
    name: String,
    command: Vec<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default = "default_fast")]
    fast: bool,
    #[serde(default)]
    retry_policy: Option<RuntimeRetryPolicy>,
}

fn default_activity_name() -> String {
    "activity".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeRetryPolicy {
    #[serde(default = "default_max_attempts")]
    max_attempts: u32,
    #[serde(default = "default_initial_interval_ms")]
    initial_interval_ms: u64,
    #[serde(default = "default_backoff_coefficient")]
    backoff_coefficient: f64,
    #[serde(default = "default_max_interval_ms")]
    max_interval_ms: u64,
    #[serde(default)]
    non_retryable_errors: Vec<String>,
}

fn default_max_attempts() -> u32 {
    3
}

fn default_initial_interval_ms() -> u64 {
    1000
}

fn default_backoff_coefficient() -> f64 {
    2.0
}

fn default_max_interval_ms() -> u64 {
    30_000
}

impl Default for RuntimeRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            initial_interval_ms: default_initial_interval_ms(),
            backoff_coefficient: default_backoff_coefficient(),
            max_interval_ms: default_max_interval_ms(),
            non_retryable_errors: Vec::new(),
        }
    }
}

/// Response for detached command logs
#[derive(Debug, Serialize)]
struct DetachedLogsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
}

/// API response
#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn error(msg: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

/// Sandbox info for list response
#[derive(Debug, Serialize)]
struct SandboxInfo {
    name: String,
    uuid: String,
    status: String,
    backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vcpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_from_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_help_text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    secret_files: Vec<String>,
    #[serde(default)]
    placeholder_secrets: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy_port: Option<u16>,
    /// Secret mappings: env_var → target_host (values are stripped for security).
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    secret_mappings: std::collections::HashMap<String, String>,
    /// User-defined labels.
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    labels: std::collections::HashMap<String, String>,
    /// User-defined description.
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// Last observed sandbox activity.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_activity_at: Option<String>,
    /// When sandbox was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_at: Option<String>,
    /// Archive reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_reason: Option<String>,
    /// Lifecycle automation policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle: Option<crate::vmm::SandboxLifecyclePolicy>,
}

/// Extract env_var → host from secret binding strings.
/// Input format: "KEY=value:host" → ("KEY", "host")
/// Strips secret values so they're never exposed in API responses.
fn extract_secret_mappings(bindings: &[String]) -> std::collections::HashMap<String, String> {
    bindings
        .iter()
        .filter_map(|raw| {
            let (key, rest) = raw.split_once('=')?;
            let host = rest.rsplit_once(':')?.1;
            Some((key.to_string(), host.to_string()))
        })
        .collect()
}

/// Build the full secret_mappings for a sandbox by merging the persisted
/// template mappings with any actually-bound secrets extracted from bindings.
fn build_secret_mappings(
    state: &crate::vmm::SandboxState,
) -> std::collections::HashMap<String, String> {
    let mut m = state.secret_mappings.clone();
    m.extend(extract_secret_mappings(&state.secret_bindings));
    m
}

fn sandbox_status(state: Option<&crate::vmm::SandboxState>, running: bool) -> String {
    if let Some(s) = state {
        s.status(running).to_string()
    } else if running {
        "running".to_string()
    } else {
        "stopped".to_string()
    }
}

/// Run command response
#[derive(Debug, Serialize)]
struct RunResponse {
    output: String,
}

/// Shared state for the HTTP server
struct AppState {
    /// API keys for authentication (empty = no auth required)
    api_keys: Vec<String>,
    /// Whether HTTP API callers may request root execution (`sudo: true`).
    allow_sudo_exec: bool,
    /// Server start time for uptime calculation
    started_at: std::time::Instant,
    /// OpenCode API state
    opencode: Arc<OpenCodeState>,
    /// Durable orchestration persistence store
    orchestration_store: Option<Arc<OrchestrationStore>>,
    /// Shared VmManager for durable object wake/hibernate operations
    vm_manager: Option<Arc<tokio::sync::RwLock<VmManager>>>,
    /// Event bus for sandbox lifecycle events (webhook/SSE/OTel)
    event_bus: Option<crate::events::EventBus>,
    /// OpenTelemetry tracer provider for span export
    otel_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    /// Enterprise configuration (when enterprise feature is enabled)
    #[cfg(feature = "enterprise")]
    enterprise_config: Option<crate::config::EnterpriseConfig>,
    /// Enterprise policy engine (when enterprise feature is enabled)
    #[cfg(feature = "enterprise")]
    policy_engine: Option<tokio::sync::RwLock<crate::policy::PolicyEngine>>,
}

impl AppState {
    fn new(
        api_keys_override: Vec<String>,
        otel_endpoint: Option<String>,
        webhook_urls: Vec<String>,
    ) -> Self {
        let mut api_keys = api_keys_override;
        // If no keys provided via CLI, fall back to env var / config file
        if api_keys.is_empty()
            && let Some(key) = load_api_key()
        {
            api_keys.push(key);
        }
        let allow_sudo_exec = load_api_allow_sudo_exec_from_config();
        if !api_keys.is_empty() {
            eprintln!(
                "API key authentication enabled ({} key{})",
                api_keys.len(),
                if api_keys.len() == 1 { "" } else { "s" }
            );
        }
        if allow_sudo_exec {
            eprintln!("[api] Root exec via HTTP API is enabled");
        }

        // Initialize event bus if webhooks or OTel are configured
        let event_bus = if !webhook_urls.is_empty() || otel_endpoint.is_some() {
            let bus = crate::events::new_event_bus();
            if !webhook_urls.is_empty() {
                eprintln!(
                    "Webhook notifications enabled ({} URL{})",
                    webhook_urls.len(),
                    if webhook_urls.len() == 1 { "" } else { "s" }
                );
                let rx = bus.subscribe();
                tokio::spawn(crate::events::webhook_dispatcher(rx, webhook_urls));
            }
            Some(bus)
        } else {
            None
        };

        // Initialize OTel tracer provider
        let otel_provider =
            otel_endpoint.and_then(|endpoint| match crate::observe::init_tracer(&endpoint) {
                Ok(provider) => {
                    eprintln!("OpenTelemetry trace export enabled → {}", endpoint);
                    Some(provider)
                }
                Err(e) => {
                    eprintln!("[otel] Failed to initialize tracer: {}", e);
                    None
                }
            });

        #[cfg(feature = "enterprise")]
        let (enterprise_config, policy_engine) = Self::init_enterprise();

        Self {
            api_keys,
            allow_sudo_exec,
            started_at: std::time::Instant::now(),
            opencode: Arc::new(OpenCodeState::new()),
            orchestration_store: Self::init_orchestration_store(),
            vm_manager: match VmManager::new() {
                Ok(mgr) => Some(Arc::new(tokio::sync::RwLock::new(mgr))),
                Err(_) => None,
            },
            event_bus,
            otel_provider,
            #[cfg(feature = "enterprise")]
            enterprise_config,
            #[cfg(feature = "enterprise")]
            policy_engine,
        }
    }

    /// Create state with explicit API keys
    #[allow(dead_code)]
    fn with_api_keys(api_keys: Vec<String>) -> Self {
        if !api_keys.is_empty() {
            eprintln!("API key authentication enabled");
        }
        Self {
            api_keys,
            allow_sudo_exec: false,
            started_at: std::time::Instant::now(),
            opencode: Arc::new(OpenCodeState::new()),
            orchestration_store: Self::init_orchestration_store(),
            vm_manager: None,
            event_bus: None,
            otel_provider: None,
            #[cfg(feature = "enterprise")]
            enterprise_config: None,
            #[cfg(feature = "enterprise")]
            policy_engine: None,
        }
    }

    #[cfg(feature = "enterprise")]
    fn init_enterprise() -> (
        Option<crate::config::EnterpriseConfig>,
        Option<tokio::sync::RwLock<crate::policy::PolicyEngine>>,
    ) {
        let config_path = std::path::PathBuf::from("agentkernel.toml");
        if !config_path.exists() {
            return (None, None);
        }

        let cfg = match crate::config::Config::from_file(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[enterprise] Failed to load config: {}", e);
                return (None, None);
            }
        };

        if !cfg.enterprise.enabled {
            return (Some(cfg.enterprise), None);
        }

        match crate::policy::PolicyEngine::new(&cfg.enterprise) {
            Ok(engine) => {
                eprintln!("[enterprise] Policy engine initialized for HTTP API");
                (Some(cfg.enterprise), Some(tokio::sync::RwLock::new(engine)))
            }
            Err(e) => {
                eprintln!("[enterprise] Failed to initialize policy engine: {}", e);
                (Some(cfg.enterprise), None)
            }
        }
    }

    async fn get_manager(&self) -> Result<tokio::sync::RwLockWriteGuard<'_, VmManager>> {
        match self.vm_manager.as_ref() {
            Some(m) => Ok(m.write().await),
            None => anyhow::bail!("VmManager not initialized"),
        }
    }

    fn init_orchestration_store() -> Option<Arc<OrchestrationStore>> {
        match OrchestrationStore::open_default() {
            Ok(store) => Some(Arc::new(store)),
            Err(e) => {
                eprintln!("[durable] Failed to initialize orchestration store: {}", e);
                None
            }
        }
    }

    #[allow(clippy::result_large_err)]
    fn orchestration_store(&self) -> Result<&Arc<OrchestrationStore>, Response<BoxBody>> {
        self.orchestration_store.as_ref().ok_or_else(|| {
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error("Durable orchestration storage unavailable"),
            )
        })
    }

    /// Check if a request is authenticated
    #[allow(clippy::result_large_err)]
    fn check_auth(&self, req: &Request<Incoming>) -> Result<(), Response<BoxBody>> {
        // If no API keys configured, allow all requests
        if self.api_keys.is_empty() {
            return Ok(());
        }

        // Get Authorization header
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                if self.api_keys.iter().any(|key| constant_time_eq(token, key)) {
                    Ok(())
                } else {
                    Err(json_response(
                        StatusCode::UNAUTHORIZED,
                        &ApiResponse::<()>::error("Invalid API key"),
                    ))
                }
            }
            Some(_) => Err(json_response(
                StatusCode::UNAUTHORIZED,
                &ApiResponse::<()>::error("Invalid authorization format. Use: Bearer <api_key>"),
            )),
            None => Err(json_response(
                StatusCode::UNAUTHORIZED,
                &ApiResponse::<()>::error("Missing Authorization header"),
            )),
        }
    }
}

/// Extract identity from a request for enterprise policy evaluation.
///
/// Builds an AgentIdentity from the Authorization header.
/// If a JWKS URL is configured, Bearer tokens are validated as JWTs.
/// Otherwise, Bearer tokens are treated as API key identities.
#[cfg(feature = "enterprise")]
async fn extract_identity(
    req: &Request<Incoming>,
    state: &AppState,
) -> crate::identity::AgentIdentity {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..];

            // Try JWT validation if jwks_url is configured
            if let Some(ref config) = state.enterprise_config
                && let Some(ref jwks_url) = config.jwks_url
            {
                match crate::identity::validate_jwt(token, jwks_url).await {
                    Ok(claims) => return crate::identity::AgentIdentity::from_jwt(claims),
                    Err(e) => {
                        eprintln!("[enterprise] JWT validation failed: {}", e);
                        if state
                            .api_keys
                            .iter()
                            .any(|k| crate::identity::validate_api_key(token, k).is_ok())
                        {
                            return crate::identity::AgentIdentity::from_api_key(token.to_string());
                        }
                        return crate::identity::AgentIdentity::anonymous();
                    }
                }
            }

            crate::identity::AgentIdentity::from_api_key(token.to_string())
        }
        _ => crate::identity::AgentIdentity::anonymous(),
    }
}

/// Enforce enterprise policy for an action on a sandbox.
///
/// Returns Ok(()) if the action is permitted (or no policy engine is active).
/// Returns a 403 Forbidden response if the action is denied.
#[cfg(feature = "enterprise")]
async fn enforce_policy(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
    action: crate::policy::Action,
    sandbox_name: &str,
) -> Result<(), Response<BoxBody>> {
    let Some(ref engine_lock) = state.policy_engine else {
        return Ok(());
    };
    let Some(ref enterprise) = state.enterprise_config else {
        return Ok(());
    };

    let principal = identity.to_principal(
        enterprise.org_id.as_deref().unwrap_or("default"),
        &enterprise.default_roles,
    );
    let resource = crate::policy::Resource {
        name: sandbox_name.to_string(),
        agent_type: "api".to_string(),
        runtime: "unknown".to_string(),
    };

    let engine = engine_lock.read().await;
    let decision = engine.evaluate(&principal, action, &resource).await;

    if !decision.is_permit() {
        return Err(json_response(
            StatusCode::FORBIDDEN,
            &ApiResponse::<()>::error(format!("Policy denied: {}", decision.reason)),
        ));
    }
    Ok(())
}

/// Handle HTTP requests
async fn handle_request(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody>, hyper::Error> {
    let method = req.method().clone();
    let method_str = method.to_string();
    let path = req.uri().path().to_string();
    let start = std::time::Instant::now();

    // OTel: extract trace context and create server span
    let mut _otel_span = state.otel_provider.as_ref().map(|provider| {
        let parent_ctx = crate::observe::extract_context(&req);
        let span_name = format!("{} {}", method_str, path);
        crate::observe::start_span(
            provider,
            &parent_ctx,
            &span_name,
            vec![
                opentelemetry::KeyValue::new("http.method", method_str.clone()),
                opentelemetry::KeyValue::new("http.target", path.clone()),
            ],
        )
    });

    // Parse path segments
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // Health check doesn't require authentication
    if method == Method::GET && segments.as_slice() == ["health"] {
        return Ok(json_response(StatusCode::OK, &ApiResponse::success("ok")));
    }

    // Prometheus metrics endpoint (no auth, like health)
    if method == Method::GET && segments.as_slice() == ["metrics"] {
        let body = crate::metrics::gather();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
            .body(full(body))
            .unwrap());
    }

    // Stats endpoint doesn't require auth (used by fleet load-balancers)
    if method == Method::GET && segments.as_slice() == ["stats"] {
        return Ok(handle_stats(state).await);
    }

    // Check authentication for all other endpoints
    if let Err(resp) = state.check_auth(&req) {
        return Ok(resp);
    }

    // SSE event stream (requires auth when API keys are configured)
    if method == Method::GET && segments.as_slice() == ["events"] {
        if let Some(ref bus) = state.event_bus {
            return Ok(crate::events::handle_events_sse(&req, bus).await);
        }
        return Ok(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &ApiResponse::<()>::error(
                "Event streaming not enabled. Start server with --webhook-url or --otel-endpoint",
            ),
        ));
    }

    // Handle OpenCode API routes
    if segments.first() == Some(&"opencode") {
        let path_suffix = if segments.len() > 1 {
            segments[1..].join("/")
        } else {
            String::new()
        };
        return Ok(crate::opencode::handle_opencode_request(
            req,
            &path_suffix,
            state.opencode.clone(),
        )
        .await);
    }

    let response = match (method, segments.as_slice()) {
        // Run a command in a temporary sandbox
        (Method::POST, ["run"]) => handle_run(req, state).await,

        // Run a command with SSE streaming output
        (Method::POST, ["run", "stream"]) => handle_run_stream(req, state).await,

        // Batch run commands in parallel
        (Method::POST, ["batch", "run"]) => handle_batch_run(req, state).await,

        // Durable orchestration scaffolding
        (Method::POST, ["orchestrations"]) => handle_create_orchestration(req, state).await,
        (Method::GET, ["orchestrations"]) => handle_list_orchestrations(state).await,
        (Method::POST, ["orchestrations", "definitions"]) => {
            handle_put_orchestration_definition(req, state).await
        }
        (Method::GET, ["orchestrations", "definitions"]) => {
            handle_list_orchestration_definitions(state).await
        }
        (Method::GET, ["orchestrations", "definitions", definition_name]) => {
            handle_get_orchestration_definition(definition_name, state).await
        }
        (Method::DELETE, ["orchestrations", "definitions", definition_name]) => {
            handle_delete_orchestration_definition(definition_name, state).await
        }
        (Method::GET, ["orchestrations", orchestration_id]) => {
            handle_get_orchestration(orchestration_id, state).await
        }
        (Method::POST, ["orchestrations", orchestration_id, "events"]) => {
            handle_raise_orchestration_event(req, orchestration_id, state).await
        }
        (Method::POST, ["orchestrations", orchestration_id, "terminate"]) => {
            handle_terminate_orchestration(req, orchestration_id, state).await
        }
        (Method::PATCH, ["orchestrations", orchestration_id]) => {
            handle_update_orchestration(req, orchestration_id, state).await
        }
        (Method::DELETE, ["orchestrations", orchestration_id]) => {
            handle_delete_orchestration(orchestration_id, state).await
        }

        // Durable store scaffolding
        (Method::GET, ["stores"]) => handle_list_durable_stores(state).await,
        (Method::POST, ["stores"]) => handle_create_durable_store(req, state).await,
        (Method::POST, ["stores", store_id, "query"]) => {
            handle_query_durable_store(req, store_id, state).await
        }
        (Method::POST, ["stores", store_id, "execute"]) => {
            handle_execute_durable_store(req, store_id, state).await
        }
        (Method::POST, ["stores", store_id, "command"]) => {
            handle_command_durable_store(req, store_id, state).await
        }
        (Method::GET, ["stores", store_id]) => handle_get_durable_store(store_id, state).await,
        (Method::DELETE, ["stores", store_id]) => {
            handle_delete_durable_store(store_id, state).await
        }

        // Durable Objects
        (Method::GET, ["objects"]) => handle_list_objects(state).await,
        (Method::POST, ["objects"]) => handle_create_object(req, state).await,
        (Method::GET, ["objects", object_id]) => handle_get_object(object_id, state).await,
        (Method::DELETE, ["objects", object_id]) => handle_delete_object(object_id, state).await,
        (Method::PATCH, ["objects", object_id]) => handle_patch_object(req, object_id, state).await,

        // Durable Object call (auto-create + auto-wake)
        (Method::POST, ["objects", class, object_id, "call", method]) => {
            handle_object_call_request(req, class, object_id, method, state).await
        }
        // Durable Object alarm
        (Method::POST, ["objects", class, object_id, "alarm"]) => {
            handle_object_alarm(req, class, object_id, state).await
        }

        // Schedules
        (Method::GET, ["schedules"]) => handle_list_schedules(state).await,
        (Method::POST, ["schedules"]) => handle_create_schedule(req, state).await,
        (Method::GET, ["schedules", schedule_id]) => handle_get_schedule(schedule_id, state).await,
        (Method::DELETE, ["schedules", schedule_id]) => {
            handle_delete_schedule(schedule_id, state).await
        }
        (Method::POST, ["schedules", schedule_id, "trigger"]) => {
            handle_trigger_schedule(schedule_id, state).await
        }

        // List sandboxes (supports ?label=key:value filtering)
        (Method::GET, ["sandboxes"]) => handle_list_sandboxes(req, state).await,

        // Create a sandbox
        (Method::POST, ["sandboxes"]) => handle_create_sandbox(req, state).await,

        // Get sandbox info by UUID
        (Method::GET, ["sandboxes", "by-uuid", uuid]) => {
            handle_get_sandbox_by_uuid(uuid, state).await
        }

        // Get sandbox info
        (Method::GET, ["sandboxes", name]) => handle_get_sandbox(name, state).await,

        // Execute in a sandbox
        (Method::POST, ["sandboxes", name, "exec"]) => handle_exec_sandbox(req, name, state).await,

        // Detached exec: start a background command
        (Method::POST, ["sandboxes", name, "exec", "detach"]) => {
            handle_exec_detach(req, name, state).await
        }

        // List detached commands in a sandbox
        (Method::GET, ["sandboxes", name, "exec", "detached"]) => {
            handle_detached_list(name, state).await
        }

        // Get detached command status
        (Method::GET, ["sandboxes", name, "exec", "detached", cmd_id]) => {
            handle_detached_status(name, cmd_id, state).await
        }

        // Get detached command logs
        (Method::GET, ["sandboxes", name, "exec", "detached", cmd_id, "logs"]) => {
            handle_detached_logs(req, name, cmd_id, state).await
        }

        // Kill a detached command
        (Method::DELETE, ["sandboxes", name, "exec", "detached", cmd_id]) => {
            handle_detached_kill(name, cmd_id, state).await
        }

        // Sandbox logs
        (Method::GET, ["sandboxes", name, "logs"]) => handle_sandbox_logs(name, state).await,

        // Batch file write: POST /sandboxes/{name}/files
        (Method::POST, ["sandboxes", name, "files"]) => {
            handle_batch_file_write(req, name, state).await
        }

        // File operations: GET /sandboxes/{name}/files/{path...}
        (Method::GET, ["sandboxes", name, "files", ..]) => {
            let file_path = segments[3..].join("/");
            handle_file_read(name, &file_path, state).await
        }

        // File operations: PUT /sandboxes/{name}/files/{path...}
        (Method::PUT, ["sandboxes", name, "files", ..]) => {
            let file_path = segments[3..].join("/");
            handle_file_write(req, name, &file_path, state).await
        }

        // File operations: DELETE /sandboxes/{name}/files/{path...}
        (Method::DELETE, ["sandboxes", name, "files", ..]) => {
            let file_path = segments[3..].join("/");
            handle_file_delete(name, &file_path, state).await
        }

        // Delete a sandbox
        (Method::DELETE, ["sandboxes", name]) => handle_delete_sandbox(name, state).await,

        // Start a stopped sandbox
        (Method::POST, ["sandboxes", name, "start"]) => handle_start_sandbox(name, state).await,

        // Stop a running sandbox
        (Method::POST, ["sandboxes", name, "stop"]) => handle_stop_sandbox(name, state).await,

        // Extend sandbox TTL
        (Method::POST, ["sandboxes", name, "extend"]) => handle_extend_ttl(req, name, state).await,

        // Resize sandbox (stop + recreate with new resources)
        (Method::POST, ["sandboxes", name, "resize"]) => {
            handle_resize_sandbox(req, name, state).await
        }

        // Recover archived sandbox
        (Method::POST, ["sandboxes", name, "recover"]) => {
            handle_recover_sandbox(req, name, state).await
        }

        // Update sandbox metadata (labels, etc.)
        (Method::PATCH, ["sandboxes", name]) => handle_patch_sandbox(req, name, state).await,

        // Snapshot endpoints
        (Method::GET, ["snapshots"]) => handle_list_snapshots(state).await,
        (Method::POST, ["snapshots"]) => handle_take_snapshot(req, state).await,
        (Method::GET, ["snapshots", name]) => handle_get_snapshot(name).await,
        (Method::DELETE, ["snapshots", name]) => handle_delete_snapshot(name).await,
        (Method::POST, ["snapshots", name, "restore"]) => {
            handle_restore_snapshot(req, name, state).await
        }

        // Audit log
        (Method::GET, ["audit"]) => handle_audit_log(req).await,

        // Diagnostics: installation status
        (Method::GET, ["status"]) => handle_status(state).await,

        // Stats: lightweight utilization endpoint
        (Method::GET, ["stats"]) => handle_stats(state).await,

        // Diagnostics: health checks
        (Method::GET, ["doctor"]) => handle_doctor(state).await,

        // Secrets management
        (Method::GET, ["secrets"]) => handle_list_secrets().await,
        (Method::POST, ["secrets"]) => handle_create_secret(req).await,
        (Method::DELETE, ["secrets", name]) => handle_delete_secret(name).await,

        // Proxy hooks
        (Method::GET, ["proxy", "hooks"]) => handle_list_proxy_hooks(state).await,
        (Method::POST, ["proxy", "hooks"]) => handle_register_proxy_hook(req, state).await,
        (Method::DELETE, ["proxy", "hooks", name]) => handle_remove_proxy_hook(name, state).await,

        // LLM usage
        (Method::GET, ["llm", "usage"]) => handle_llm_usage_all().await,
        (Method::GET, ["llm", "usage", sandbox]) => handle_llm_usage_sandbox(sandbox).await,

        // LLM key management
        (Method::GET, ["llm", "keys"]) => handle_llm_keys_list().await,
        (Method::PUT, ["llm", "keys", provider]) => handle_llm_keys_set(req, provider).await,
        (Method::DELETE, ["llm", "keys", provider]) => handle_llm_keys_remove(provider).await,

        // Garbage collection
        (Method::POST, ["gc"]) => handle_gc(state).await,

        // Lifecycle policy reconciliation
        (Method::POST, ["lifecycle", "reconcile"]) => handle_reconcile_lifecycle(req, state).await,

        // Agents/plugins
        (Method::GET, ["agents"]) => handle_list_agents(state).await,

        // Browser v2: persistent pages with ARIA snapshots
        (Method::POST, ["sandboxes", name, "browser", "start"]) => {
            handle_browser_start(name, state).await
        }
        (Method::GET, ["sandboxes", name, "browser", "pages"]) => {
            handle_browser_list_pages(name, state).await
        }
        (Method::POST, ["sandboxes", name, "browser", "pages"]) => {
            handle_browser_create_page(req, name, state).await
        }
        (Method::DELETE, ["sandboxes", name, "browser", "pages", page]) => {
            handle_browser_close_page(name, page, state).await
        }
        (Method::POST, ["sandboxes", name, "browser", "pages", page, "goto"]) => {
            handle_browser_goto(req, name, page, state).await
        }
        (Method::GET, ["sandboxes", name, "browser", "pages", page, "snapshot"]) => {
            handle_browser_snapshot(name, page, state).await
        }
        (Method::GET, ["sandboxes", name, "browser", "pages", page, "content"]) => {
            handle_browser_content(name, page, state).await
        }
        (Method::POST, ["sandboxes", name, "browser", "pages", page, "click"]) => {
            handle_browser_click(req, name, page, state).await
        }
        (Method::POST, ["sandboxes", name, "browser", "pages", page, "fill"]) => {
            handle_browser_fill(req, name, page, state).await
        }
        (Method::POST, ["sandboxes", name, "browser", "pages", page, "screenshot"]) => {
            handle_browser_screenshot(name, page, state).await
        }
        (Method::POST, ["sandboxes", name, "browser", "pages", page, "evaluate"]) => {
            handle_browser_evaluate(req, name, page, state).await
        }
        (Method::GET, ["sandboxes", name, "browser", "events"]) => {
            handle_browser_events(req, name, state).await
        }

        // Docker image management
        (Method::GET, ["images"]) => handle_list_images(state).await,
        (Method::DELETE, ["images", id]) => handle_delete_image(id, state).await,

        // Hardware benchmark
        (Method::POST, ["benchmark"]) => handle_benchmark(state).await,

        // Session recording
        (Method::GET, ["sessions"]) => handle_list_sandbox_sessions(state).await,
        (Method::GET, ["sandboxes", name, "session"]) => {
            handle_get_sandbox_session(name, state).await
        }

        // Sandbox config export/import
        (Method::GET, ["sandboxes", name, "config"]) => {
            handle_export_sandbox_config(name, state).await
        }
        (Method::POST, ["sandboxes", name, "config"]) => {
            handle_import_sandbox_config(req, name, state).await
        }

        // Interactive permissions
        (Method::GET, ["permissions"]) => handle_list_permissions().await,
        (Method::POST, ["permissions", "grant"]) => handle_grant_permission(req).await,
        (Method::DELETE, ["permissions", id]) => handle_revoke_permission(id).await,
        (Method::POST, ["permissions", "check"]) => handle_check_permission(req).await,

        // Enterprise policy endpoints
        #[cfg(feature = "enterprise")]
        (Method::GET, ["policy", "status"]) => handle_policy_status(state).await,
        #[cfg(feature = "enterprise")]
        (Method::POST, ["policy", "check"]) => handle_policy_check(req, state).await,
        #[cfg(feature = "enterprise")]
        (Method::POST, ["policy", "reload"]) => handle_policy_reload(state).await,
        #[cfg(feature = "enterprise")]
        (Method::GET, ["policy", "audit"]) => handle_policy_audit(req, state).await,

        // 404 for everything else
        _ => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Not found"),
        ),
    };

    // OTel: finish the server span with status info
    if let Some(ref mut span) = _otel_span {
        let status_code = response.status().as_u16();
        crate::observe::finish_span(
            span,
            status_code < 400,
            vec![opentelemetry::KeyValue::new(
                "http.status_code",
                status_code as i64,
            )],
        );
    }

    crate::metrics::record_http_request(
        &method_str,
        &path,
        response.status().as_u16(),
        start.elapsed().as_secs_f64(),
    );

    Ok(response)
}

fn json_response<T: Serialize>(status: StatusCode, data: &T) -> Response<BoxBody> {
    let body = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(full(body))
        .unwrap()
}

async fn read_json_body<T: for<'de> Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, Response<BoxBody>> {
    let body_bytes = read_body_bytes(req).await?;

    serde_json::from_slice(&body_bytes).map_err(|e| {
        json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid JSON: {}", e)),
        )
    })
}

async fn handle_run(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Run, "ephemeral").await
        {
            return resp;
        }
    }

    let body: RunRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("command is required"),
        );
    }

    // Fast path: use container pool (default for HTTP API)
    if body.fast {
        if body.image.is_some() {
            // Pool uses alpine:3.20, warn if custom image requested
            eprintln!("Warning: custom image ignored in fast mode (pool uses alpine:3.20)");
        }

        match VmManager::run_pooled(&body.command).await {
            Ok(output) => {
                return json_response(
                    StatusCode::OK,
                    &ApiResponse::success(RunResponse { output }),
                );
            }
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
        }
    }

    // Slow path: full sandbox lifecycle (when fast=false or custom image needed)

    // Validate Docker image name if provided (security: prevents injection)
    if let Some(ref img) = body.image
        && let Err(e) = validation::validate_docker_image(img)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let image = body
        .image
        .unwrap_or_else(|| languages::detect_image(&body.command));
    let profile = body.profile.as_deref().unwrap_or("moderate");
    let perms = SecurityProfile::from_str(profile)
        .unwrap_or_default()
        .permissions();

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandbox_name = format!("api-run-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Create
    if let Err(e) = manager.create(&sandbox_name, &image, 1, 512).await {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Start
    if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
        let _ = manager.remove(&sandbox_name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Execute
    let result = manager.exec_cmd(&sandbox_name, &body.command).await;

    // Cleanup
    let _ = manager.remove(&sandbox_name).await;

    match result {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(RunResponse { output }),
        ),
        Err(e) => {
            if let Some(cmd_err) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                json_response(
                    StatusCode::CONFLICT,
                    &serde_json::json!({
                        "success": false,
                        "error": cmd_err.to_string(),
                        "exit_code": cmd_err.exit_code,
                        "output": cmd_err.output,
                    }),
                )
            } else {
                json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                )
            }
        }
    }
}

/// Server-Sent Events response for streaming command output
fn sse_response(events: Vec<(&str, serde_json::Value)>) -> Response<BoxBody> {
    let mut body = String::new();
    for (event_type, data) in events {
        body.push_str(&format!(
            "event: {}\ndata: {}\n\n",
            event_type,
            serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
        ));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(full(body))
        .unwrap()
}

/// Handle /run/stream - runs command with SSE streaming output
async fn handle_run_stream(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Run, "ephemeral").await
        {
            return resp;
        }
    }

    let body: RunRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return sse_response(vec![(
            "error",
            serde_json::json!({"message": "command is required"}),
        )]);
    }

    let mut events = vec![];

    // Send started event
    events.push((
        "started",
        serde_json::json!({
            "command": body.command,
            "fast": body.fast,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    ));

    // Fast path: use container pool (default for HTTP API)
    if body.fast {
        match VmManager::run_pooled(&body.command).await {
            Ok(output) => {
                events.push((
                    "output",
                    serde_json::json!({
                        "data": output,
                        "stream": "stdout"
                    }),
                ));
                events.push((
                    "done",
                    serde_json::json!({
                        "exit_code": 0,
                        "success": true
                    }),
                ));
            }
            Err(e) => {
                if let Some(cmd_err) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                    events.push((
                        "done",
                        serde_json::json!({
                            "exit_code": cmd_err.exit_code,
                            "success": false,
                            "output": cmd_err.output
                        }),
                    ));
                } else {
                    events.push((
                        "error",
                        serde_json::json!({
                            "message": e.to_string()
                        }),
                    ));
                }
            }
        }
        return sse_response(events);
    }

    // Slow path: full sandbox lifecycle
    let profile = body.profile.as_deref().unwrap_or("moderate");
    let perms = SecurityProfile::from_str(profile)
        .unwrap_or_default()
        .permissions();

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            events.push(("error", serde_json::json!({"message": e.to_string()})));
            return sse_response(events);
        }
    };

    let image = body
        .image
        .clone()
        .unwrap_or_else(|| languages::detect_image(&body.command));

    let sandbox_name = format!("api-stream-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Create
    if let Err(e) = manager.create(&sandbox_name, &image, 1, 512).await {
        events.push(("error", serde_json::json!({"message": e.to_string()})));
        return sse_response(events);
    }

    events.push((
        "progress",
        serde_json::json!({
            "stage": "sandbox_created",
            "sandbox": sandbox_name
        }),
    ));

    // Start
    if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
        let _ = manager.remove(&sandbox_name).await;
        events.push(("error", serde_json::json!({"message": e.to_string()})));
        return sse_response(events);
    }

    events.push((
        "progress",
        serde_json::json!({
            "stage": "sandbox_started"
        }),
    ));

    // Execute
    let result = manager.exec_cmd(&sandbox_name, &body.command).await;

    // Cleanup
    let _ = manager.remove(&sandbox_name).await;

    match result {
        Ok(output) => {
            events.push((
                "output",
                serde_json::json!({
                    "data": output,
                    "stream": "stdout"
                }),
            ));
            events.push((
                "done",
                serde_json::json!({
                    "exit_code": 0,
                    "success": true
                }),
            ));
        }
        Err(e) => {
            if let Some(cmd_err) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                events.push((
                    "done",
                    serde_json::json!({
                        "exit_code": cmd_err.exit_code,
                        "success": false,
                        "output": cmd_err.output
                    }),
                ));
            } else {
                events.push((
                    "error",
                    serde_json::json!({
                        "message": e.to_string()
                    }),
                ));
            }
        }
    }

    sse_response(events)
}

async fn handle_create_orchestration(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: CreateOrchestrationRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.name.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("name is required"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.create(CreateOrchestration {
        name: body.name,
        input: body.input,
    }) {
        Ok(record) => json_response(StatusCode::ACCEPTED, &ApiResponse::success(record)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_list_orchestrations(state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.list(100, 0) {
        Ok(records) => json_response(StatusCode::OK, &ApiResponse::success(records)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_put_orchestration_definition(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: serde_json::Value = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let Some(name) = body
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
    else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("name is required"),
        );
    };

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.upsert_definition(&name, body) {
        Ok(definition) => json_response(StatusCode::OK, &ApiResponse::success(definition)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_list_orchestration_definitions(state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.list_definitions(200, 0) {
        Ok(definitions) => json_response(StatusCode::OK, &ApiResponse::success(definitions)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_orchestration_definition(
    definition_name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.get_definition(definition_name) {
        Ok(Some(definition)) => json_response(StatusCode::OK, &ApiResponse::success(definition)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Definition not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_orchestration_definition(
    definition_name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.delete_definition(definition_name) {
        Ok(true) => json_response(
            StatusCode::OK,
            &ApiResponse::success(serde_json::json!({
                "deleted": true,
                "name": definition_name,
            })),
        ),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Definition not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_create_durable_store(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: CreateDurableStoreRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.name.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("name is required"),
        );
    }

    // Validate sandbox exists if specified
    if let Some(ref sandbox_name) = body.sandbox
        && !sandbox_name.is_empty()
        && let Ok(manager) = state.get_manager().await
        && !manager.exists(sandbox_name)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("sandbox '{}' does not exist", sandbox_name)),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.create_store(CreateDurableStore {
        name: body.name,
        kind: body.kind,
        sandbox: body.sandbox,
        config: body.config,
    }) {
        Ok(created) => json_response(StatusCode::CREATED, &ApiResponse::success(created)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE constraint failed: stores.name") {
                return json_response(
                    StatusCode::CONFLICT,
                    &ApiResponse::<()>::error("Store name already exists"),
                );
            }
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(msg),
            )
        }
    }
}

async fn handle_list_durable_stores(state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.list_stores(200, 0) {
        Ok(stores) => json_response(StatusCode::OK, &ApiResponse::success(stores)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_durable_store(store_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.get_store(store_id) {
        Ok(Some(found)) => json_response(StatusCode::OK, &ApiResponse::success(found)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Store not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_durable_store(store_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.delete_store(store_id) {
        Ok(true) => json_response(StatusCode::OK, &ApiResponse::success("deleted")),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Store not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_query_durable_store(
    req: Request<Incoming>,
    store_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: DurableStoreSqlRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    if body.sql.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("sql is required"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.query_store(store_id, &body.sql, body.params) {
        Ok(Some(result)) => json_response(
            StatusCode::OK,
            &ApiResponse::<DurableStoreQueryResult>::success(result),
        ),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Store not found"),
        ),
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not executable in this runtime yet") {
                StatusCode::NOT_IMPLEMENTED
            } else {
                StatusCode::BAD_REQUEST
            };
            json_response(status, &ApiResponse::<()>::error(msg))
        }
    }
}

async fn handle_execute_durable_store(
    req: Request<Incoming>,
    store_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: DurableStoreSqlRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    if body.sql.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("sql is required"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.execute_store(store_id, &body.sql, body.params) {
        Ok(Some(result)) => json_response(
            StatusCode::OK,
            &ApiResponse::<DurableStoreExecuteResult>::success(result),
        ),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Store not found"),
        ),
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not executable in this runtime yet") {
                StatusCode::NOT_IMPLEMENTED
            } else {
                StatusCode::BAD_REQUEST
            };
            json_response(status, &ApiResponse::<()>::error(msg))
        }
    }
}

async fn handle_command_durable_store(
    req: Request<Incoming>,
    store_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: DurableStoreCommandRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    if body.command.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("command must not be empty"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.command_store(store_id, body.command) {
        Ok(Some(result)) => json_response(
            StatusCode::OK,
            &ApiResponse::<DurableStoreCommandResult>::success(result),
        ),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Store not found"),
        ),
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not executable in this runtime yet") {
                StatusCode::NOT_IMPLEMENTED
            } else {
                StatusCode::BAD_REQUEST
            };
            json_response(status, &ApiResponse::<()>::error(msg))
        }
    }
}

// --- Durable Object handlers ---

async fn handle_list_objects(state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.list_objects(200, 0) {
        Ok(objects) => json_response(StatusCode::OK, &ApiResponse::success(objects)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_create_object(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: CreateDurableObject = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    if body.class.trim().is_empty() || body.object_id.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("class and object_id are required"),
        );
    }

    // Validate sandbox exists if specified
    if let Some(ref sandbox_name) = body.sandbox
        && !sandbox_name.is_empty()
        && let Ok(manager) = state.get_manager().await
        && !manager.exists(sandbox_name)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("sandbox '{}' does not exist", sandbox_name)),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.create_object(body) {
        Ok(object) => json_response(StatusCode::CREATED, &ApiResponse::success(object)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_object(object_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.get_object(object_id) {
        Ok(Some(object)) => json_response(
            StatusCode::OK,
            &ApiResponse::<DurableObjectRecord>::success(object),
        ),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Object not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_object(object_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.delete_object(object_id) {
        Ok(true) => json_response(StatusCode::OK, &ApiResponse::success("deleted")),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Object not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_patch_object(
    req: Request<Incoming>,
    object_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[derive(serde::Deserialize)]
    struct PatchBody {
        storage: Option<serde_json::Value>,
        status: Option<String>,
    }

    let body: PatchBody = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    if let Some(storage) = &body.storage {
        match store.update_object_storage(object_id, storage) {
            Ok(false) => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &ApiResponse::<()>::error("Object not found"),
                );
            }
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
            Ok(true) => {}
        }
    }

    if let Some(status_str) = &body.status {
        let status = match status_str.as_str() {
            "active" => crate::orchestration_store::DurableObjectStatus::Active,
            "hibernating" => crate::orchestration_store::DurableObjectStatus::Hibernating,
            "deleted" => crate::orchestration_store::DurableObjectStatus::Deleted,
            _ => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &ApiResponse::<()>::error(
                        "invalid status: use active, hibernating, or deleted",
                    ),
                );
            }
        };
        match store.update_object_status(object_id, status, None) {
            Ok(false) => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &ApiResponse::<()>::error("Object not found"),
                );
            }
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
            Ok(true) => {}
        }
    }

    // Return the updated object
    match store.get_object(object_id) {
        Ok(Some(object)) => json_response(StatusCode::OK, &ApiResponse::success(object)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Object not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_object_call_request(
    req: Request<Incoming>,
    class: &str,
    object_id: &str,
    method: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let store = match state.orchestration_store.as_ref() {
        Some(s) => s,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error("Orchestration store not available"),
            );
        }
    };
    let manager = match state.vm_manager.as_ref() {
        Some(m) => m,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error("VmManager not available"),
            );
        }
    };

    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(e) => {
            return e;
        }
    };

    match crate::object_runtime::handle_object_call(store, manager, class, object_id, method, body)
        .await
    {
        Ok((status, resp_body)) => {
            let http_status =
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Response::builder()
                .status(http_status)
                .header("Content-Type", "application/json")
                .body(full(resp_body))
                .unwrap()
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_object_alarm(
    req: Request<Incoming>,
    class: &str,
    object_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Alarm is just a call to the "alarm" method
    let store = match state.orchestration_store.as_ref() {
        Some(s) => s,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error("Orchestration store not available"),
            );
        }
    };
    let manager = match state.vm_manager.as_ref() {
        Some(m) => m,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error("VmManager not available"),
            );
        }
    };

    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(e) => {
            return e;
        }
    };

    match crate::object_runtime::handle_object_call(store, manager, class, object_id, "alarm", body)
        .await
    {
        Ok((status, resp_body)) => {
            let http_status =
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Response::builder()
                .status(http_status)
                .header("Content-Type", "application/json")
                .body(full(resp_body))
                .unwrap()
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

// --- Schedule handlers ---

async fn handle_list_schedules(state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.list_schedules(200, 0) {
        Ok(schedules) => json_response(StatusCode::OK, &ApiResponse::success(schedules)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_create_schedule(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: CreateSchedule = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    if body.name.trim().is_empty() || body.method.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("name and method are required"),
        );
    }
    if body.cron.is_none() && body.fire_at.is_none() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("either cron or fire_at is required"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.create_schedule(body) {
        Ok(schedule) => json_response(StatusCode::CREATED, &ApiResponse::success(schedule)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_schedule(schedule_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.get_schedule(schedule_id) {
        Ok(Some(schedule)) => json_response(
            StatusCode::OK,
            &ApiResponse::<ScheduleRecord>::success(schedule),
        ),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Schedule not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_schedule(schedule_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.delete_schedule(schedule_id) {
        Ok(true) => json_response(StatusCode::OK, &ApiResponse::success("deleted")),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Schedule not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_trigger_schedule(schedule_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    // Verify schedule exists
    match store.get_schedule(schedule_id) {
        Ok(Some(schedule)) => {
            // Mark as fired
            if let Err(e) = store.mark_schedule_fired(schedule_id) {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
            // Audit log
            crate::audit::log_event(crate::audit::AuditEvent::ScheduleTriggered {
                schedule_id: schedule.id.clone(),
                schedule_name: schedule.name.clone(),
                method: schedule.method.clone(),
            });
            json_response(StatusCode::OK, &ApiResponse::success(schedule))
        }
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Schedule not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_orchestration(
    orchestration_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.get(orchestration_id) {
        Ok(Some(record)) => match store.list_events(orchestration_id, 1000, 0) {
            Ok(history) => json_response(
                StatusCode::OK,
                &ApiResponse::success(OrchestrationDetails {
                    orchestration: record,
                    history,
                }),
            ),
            Err(e) => json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            ),
        },
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Orchestration not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_raise_orchestration_event(
    req: Request<Incoming>,
    orchestration_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: RaiseOrchestrationEventRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.name.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("name is required"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let current = match store.get(orchestration_id) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error("Orchestration not found"),
            );
        }
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if matches!(
        current.status,
        OrchestrationStatus::Completed
            | OrchestrationStatus::Failed
            | OrchestrationStatus::Terminated
    ) {
        return json_response(
            StatusCode::CONFLICT,
            &ApiResponse::<()>::error("Orchestration already completed"),
        );
    }

    let payload = serde_json::json!({
        "name": body.name,
        "data": body.data
    });

    match store.append_event(orchestration_id, "EventRaised", payload) {
        Ok(event) => json_response(
            StatusCode::ACCEPTED,
            &ApiResponse::success(serde_json::json!({
                "accepted": true,
                "id": orchestration_id,
                "event": event
            })),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_terminate_orchestration(
    req: Request<Incoming>,
    orchestration_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: TerminateOrchestrationRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let current = match store.get(orchestration_id) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error("Orchestration not found"),
            );
        }
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if matches!(
        current.status,
        OrchestrationStatus::Completed
            | OrchestrationStatus::Failed
            | OrchestrationStatus::Terminated
    ) {
        return json_response(
            StatusCode::CONFLICT,
            &ApiResponse::<()>::error("Orchestration already completed"),
        );
    }

    let reason = body
        .reason
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| "Manual termination".to_string());

    if let Err(e) = store.append_event(
        orchestration_id,
        "OrchestratorTerminated",
        serde_json::json!({ "reason": reason.clone() }),
    ) {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    match store.update(
        orchestration_id,
        UpdateOrchestration {
            status: Some(OrchestrationStatus::Terminated),
            output: None,
            error: Some(reason),
        },
    ) {
        Ok(Some(record)) => json_response(StatusCode::OK, &ApiResponse::success(record)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Orchestration not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_update_orchestration(
    req: Request<Incoming>,
    orchestration_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body: UpdateOrchestrationRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.status.is_none() && body.output.is_none() && body.error.is_none() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("at least one field must be provided"),
        );
    }

    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.update(
        orchestration_id,
        UpdateOrchestration {
            status: body.status,
            output: body.output,
            error: body.error,
        },
    ) {
        Ok(Some(record)) => json_response(StatusCode::OK, &ApiResponse::success(record)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Orchestration not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_orchestration(
    orchestration_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let store = match state.orchestration_store() {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    match store.delete(orchestration_id) {
        Ok(true) => json_response(
            StatusCode::OK,
            &ApiResponse::success(serde_json::json!({
                "deleted": true,
                "id": orchestration_id,
            })),
        ),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Orchestration not found"),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_list_sandboxes(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    // Parse label filters from query string: ?label=key:value&label=env:prod
    let label_filters: Vec<(String, String)> = req
        .uri()
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|param| {
                    let (k, v) = param.split_once('=')?;
                    if k != "label" {
                        return None;
                    }
                    // Percent-decode the value for labels with special chars
                    let decoded = urlencoding::decode(v).unwrap_or(std::borrow::Cow::Borrowed(v));
                    let (lk, lv) = decoded.split_once(':')?;
                    Some((lk.to_string(), lv.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandboxes: Vec<SandboxInfo> = manager
        .list()
        .into_iter()
        .filter(|(name, _, _)| {
            if label_filters.is_empty() {
                return true;
            }
            let state_info = manager.get_state(name);
            label_filters.iter().all(|(fk, fv)| {
                state_info
                    .and_then(|s| s.labels.get(fk))
                    .is_some_and(|v| v == fv)
            })
        })
        .map(|(name, running, backend)| {
            let state_info = manager.get_state(name);
            let ports = state_info
                .map(|s| s.ports.iter().map(|p| p.to_string()).collect())
                .unwrap_or_default();
            let ip = if running {
                manager.get_container_ip(name)
            } else {
                None
            };
            SandboxInfo {
                name: name.to_string(),
                uuid: state_info
                    .map(|s| s.uuid.clone())
                    .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
                status: sandbox_status(state_info, running),
                backend: backend
                    .map(|b| format!("{}", b))
                    .unwrap_or_else(|| "unknown".to_string()),
                ip,
                image: state_info.map(|s| s.image.clone()),
                vcpus: state_info.map(|s| s.vcpus),
                memory_mb: state_info.map(|s| s.memory_mb),
                created_at: state_info.map(|s| s.created_at.clone()),
                created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
                template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
                ports,
                secret_files: state_info
                    .map(|s| s.secret_files.clone())
                    .unwrap_or_default(),
                placeholder_secrets: state_info
                    .map(|s| s.placeholder_secrets)
                    .unwrap_or(false),
                proxy_port: state_info.and_then(|s| s.proxy_port),
                secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
                labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
                description: state_info.and_then(|s| s.description.clone()),
                last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
                archived_at: state_info.and_then(|s| s.archived_at.clone()),
                archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
                lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
            }
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(sandboxes))
}

async fn handle_create_sandbox(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let start = std::time::Instant::now();

    // Enterprise policy enforcement (extract identity before consuming body)
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;

    let body: CreateRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    #[cfg(feature = "enterprise")]
    {
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Create, &body.name).await
        {
            return resp;
        }
    }

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(&body.name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }
    if body.source_ref.is_some() && body.source_url.is_none() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("source_ref requires source_url"),
        );
    }
    if let Some(ref source_url) = body.source_url {
        let normalized = source_url.strip_prefix("git:").unwrap_or(source_url);
        if let Err(e) = validation::validate_git_source_url(normalized) {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    }
    if let Some(ref git_ref) = body.source_ref
        && let Err(e) = validation::validate_git_ref(git_ref)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let image = body.image.as_deref().unwrap_or("alpine:3.20");
    let vcpus = body.vcpus.unwrap_or(1);
    let memory_mb = body.memory_mb.unwrap_or(512);

    // Validate Docker image name if provided
    if let Some(ref img) = body.image
        && let Err(e) = validation::validate_docker_image(img)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Parse port mappings
    let ports: Vec<crate::backend::PortMapping> = match body
        .ports
        .iter()
        .map(|s| crate::backend::PortMapping::parse(s))
        .collect::<Result<Vec<_>>>()
    {
        Ok(p) => p,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Invalid port mapping: {}", e)),
            );
        }
    };

    // Enterprise policy enforcement for port mapping
    #[cfg(feature = "enterprise")]
    if !ports.is_empty()
        && let Err(resp) = enforce_policy(
            &state,
            &identity,
            crate::policy::Action::PortMap,
            &body.name,
        )
        .await
    {
        return resp;
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if let Err(e) = manager
        .create_with_agent(
            &body.name,
            image,
            vcpus,
            memory_mb,
            None,
            ports.clone(),
            body.agent.clone(),
        )
        .await
    {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Set secret bindings if provided
    if !body.secrets.is_empty()
        && let Err(e) = manager.set_secret_bindings(&body.name, &body.secrets)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid secret bindings: {}", e)),
        );
    }

    // Persist template secret mappings (env_var → host) for UI display
    if !body.secret_mappings.is_empty()
        && let Err(e) = manager.set_secret_mappings(&body.name, &body.secret_mappings)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set secret mappings: {}", e)),
        );
    }

    // Set secret file keys if provided
    if !body.secret_files.is_empty()
        && let Err(e) = manager.set_secret_files(&body.name, &body.secret_files)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid secret file keys: {}", e)),
        );
    }

    // Set placeholder secrets mode if enabled
    if body.placeholder_secrets {
        if let Err(e) = manager.set_placeholder_secrets(&body.name, true) {
            eprintln!("Warning: Failed to set placeholder secrets: {}", e);
        }
    }

    // Set init script if provided
    if let Some(ref script) = body.init_script
        && let Err(e) = manager.set_init_script(&body.name, script)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set init script: {}", e)),
        );
    }

    // Store template provenance/help text when provided by caller.
    if (body.created_from_template.is_some() || body.template_help_text.is_some())
        && let Err(e) = manager.set_template_metadata(
            &body.name,
            body.created_from_template.as_deref(),
            body.template_help_text.as_deref(),
        )
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set template metadata: {}", e)),
        );
    }

    // Set labels if provided
    if !body.labels.is_empty()
        && let Err(e) = manager.set_labels(&body.name, &body.labels)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set labels: {}", e)),
        );
    }

    // Set description if provided
    if body.description.is_some()
        && let Err(e) = manager.set_description(&body.name, body.description.as_deref())
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set description: {}", e)),
        );
    }

    // Set lifecycle policy if provided
    if let Some(policy) = body.lifecycle.clone()
        && let Err(e) = manager.set_lifecycle_policy(&body.name, Some(policy.into()))
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set lifecycle policy: {}", e)),
        );
    }

    // Resolve profile for start_with_permissions
    let perms = if let Some(ref profile_str) = body.profile {
        match resolve_profile(profile_str) {
            Some(profile) => profile.permissions(),
            None => {
                let _ = manager.remove(&body.name).await;
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &ApiResponse::<()>::error(format!(
                        "Invalid profile '{}'. Use: permissive, moderate, restrictive",
                        profile_str
                    )),
                );
            }
        }
    } else {
        crate::permissions::SecurityProfile::default().permissions()
    };

    if let Err(e) = manager.start_with_permissions(&body.name, &perms).await {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Clone git repo if source_url is specified
    if let Some(ref source_url) = body.source_url {
        let url = source_url.strip_prefix("git:").unwrap_or(source_url);

        // Install git
        let install = vec![
            "sh".to_string(),
            "-c".to_string(),
            "which git >/dev/null 2>&1 || apk add --no-cache git >/dev/null 2>&1 || apt-get update -qq && apt-get install -y -qq git >/dev/null 2>&1 || true".to_string(),
        ];
        let _ = manager.exec_cmd(&body.name, &install).await;

        let clone = vec![
            "git".to_string(),
            "clone".to_string(),
            url.to_string(),
            "/workspace".to_string(),
        ];
        if let Err(e) = manager.exec_cmd(&body.name, &clone).await {
            let _ = manager.remove(&body.name).await;
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to clone {}: {}", url, e)),
            );
        }

        if let Some(ref git_ref) = body.source_ref {
            let checkout = vec![
                "git".to_string(),
                "-C".to_string(),
                "/workspace".to_string(),
                "checkout".to_string(),
                git_ref.clone(),
            ];
            if let Err(e) = manager.exec_cmd(&body.name, &checkout).await {
                let _ = manager.remove(&body.name).await;
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Failed to checkout {}: {}", git_ref, e)),
                );
            }
        }
    }

    let port_strings: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
    let ip = manager.get_container_ip(&body.name);
    let state_info = manager.get_state(&body.name);

    let duration_ms = start.elapsed().as_millis() as u64;

    // Emit sandbox.created event
    crate::events::emit(
        state.event_bus.as_ref(),
        crate::events::SandboxEvent {
            event: "sandbox.created".to_string(),
            timestamp: chrono::Utc::now(),
            sandbox: body.name.clone(),
            labels: body.labels.clone(),
            metadata: serde_json::json!({
                "image": image,
                "backend": format!("{}", manager.backend()),
                "vcpus": vcpus,
                "memory_mb": memory_mb,
                "duration_ms": duration_ms,
            }),
        },
    );

    json_response(
        StatusCode::CREATED,
        &ApiResponse::success(SandboxInfo {
            name: body.name,
            uuid: state_info
                .map(|s| s.uuid.clone())
                .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
            status: "running".to_string(),
            backend: format!("{}", manager.backend()),
            ip,
            image: Some(image.to_string()),
            vcpus: Some(vcpus),
            memory_mb: Some(memory_mb),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports: port_strings,
            secret_files: body.secret_files.clone(),
            placeholder_secrets: body.placeholder_secrets,
            proxy_port: state_info.and_then(|s| s.proxy_port),
            secret_mappings: {
                let mut m = body.secret_mappings.clone();
                m.extend(extract_secret_mappings(&body.secrets));
                m
            },
            labels: body.labels.clone(),
            description: body.description.clone(),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            archived_at: state_info.and_then(|s| s.archived_at.clone()),
            archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
            lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
        }),
    )
}

async fn handle_get_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandboxes = manager.list();
    for (sandbox_name, running, backend) in &sandboxes {
        if *sandbox_name == name {
            let state_info = manager.get_state(name);
            let ports = state_info
                .map(|s| s.ports.iter().map(|p| p.to_string()).collect())
                .unwrap_or_default();
            let ip = if *running {
                manager.get_container_ip(name)
            } else {
                None
            };
            return json_response(
                StatusCode::OK,
                &ApiResponse::success(SandboxInfo {
                    name: sandbox_name.to_string(),
                    uuid: state_info
                        .map(|s| s.uuid.clone())
                        .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
                    status: sandbox_status(state_info, *running),
                    backend: backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "unknown".to_string()),
                    ip,
                    image: state_info.map(|s| s.image.clone()),
                    vcpus: state_info.map(|s| s.vcpus),
                    memory_mb: state_info.map(|s| s.memory_mb),
                    created_at: state_info.map(|s| s.created_at.clone()),
                    created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
                    template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
                    ports,
                    secret_files: state_info
                        .map(|s| s.secret_files.clone())
                        .unwrap_or_default(),
                    placeholder_secrets: state_info
                        .map(|s| s.placeholder_secrets)
                        .unwrap_or(false),
                    proxy_port: state_info.and_then(|s| s.proxy_port),
                    secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
                    labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
                    description: state_info.and_then(|s| s.description.clone()),
                    last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
                    archived_at: state_info.and_then(|s| s.archived_at.clone()),
                    archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
                    lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
                }),
            );
        }
    }

    json_response(
        StatusCode::NOT_FOUND,
        &ApiResponse::<()>::error("Sandbox not found"),
    )
}

async fn handle_get_sandbox_by_uuid(uuid: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if uuid::Uuid::parse_str(uuid).is_err() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("Invalid sandbox UUID"),
        );
    }

    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let Some(state_info) = manager.get_state_by_uuid(uuid) else {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Sandbox not found"),
        );
    };

    let running = manager.is_running(&state_info.name);
    let ip = if running {
        manager.get_container_ip(&state_info.name)
    } else {
        None
    };
    let ports = state_info
        .ports
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    let backend = state_info.backend.unwrap_or_else(|| manager.backend());

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxInfo {
            name: state_info.name.clone(),
            uuid: state_info.uuid.clone(),
            status: state_info.status(running).to_string(),
            backend: format!("{}", backend),
            ip,
            image: Some(state_info.image.clone()),
            vcpus: Some(state_info.vcpus),
            memory_mb: Some(state_info.memory_mb),
            created_at: Some(state_info.created_at.clone()),
            created_from_template: state_info.created_from_template.clone(),
            template_help_text: state_info.template_help_text.clone(),
            ports,
            secret_files: state_info.secret_files.clone(),
            placeholder_secrets: state_info.placeholder_secrets,
            proxy_port: state_info.proxy_port,
            secret_mappings: build_secret_mappings(state_info),
            labels: state_info.labels.clone(),
            description: state_info.description.clone(),
            last_activity_at: state_info.last_activity_at.clone(),
            archived_at: state_info.archived_at.clone(),
            archived_reason: state_info.archived_reason.clone(),
            lifecycle: state_info.lifecycle_policy.clone(),
        }),
    )
}

async fn handle_exec_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let exec_start = std::time::Instant::now();

    // Extract trace context from incoming request for OTel + env injection
    let (traceparent_hdr, tracestate_hdr) = crate::observe::extract_trace_headers(&req);

    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Exec, name).await
        {
            return resp;
        }
    }

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: ExecRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("command is required"),
        );
    }
    let sudo_requested = body.sudo.unwrap_or(false);
    if sudo_requested && !state.allow_sudo_exec {
        return json_response(
            StatusCode::FORBIDDEN,
            &ApiResponse::<()>::error(
                "sudo execution is disabled for HTTP API. Set [api].allow_sudo_exec = true to enable it",
            ),
        );
    }
    if let Some(ref workdir) = body.workdir
        && let Err(e) = validation::validate_exec_workdir(workdir)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Propagate trace context into sandbox environment variables.
    // Filter any caller-supplied TRACEPARENT/TRACESTATE to avoid duplicates.
    let mut env: Vec<String> = body
        .env
        .into_iter()
        .filter(|e| !e.starts_with("TRACEPARENT=") && !e.starts_with("TRACESTATE="))
        .collect();
    if let Some(ref tp) = traceparent_hdr {
        env.push(format!("TRACEPARENT={}", tp));
    }
    if let Some(ref ts) = tracestate_hdr {
        env.push(format!("TRACESTATE={}", ts));
    }

    let opts = crate::backend::ExecOptions {
        env,
        workdir: body.workdir,
        user: if sudo_requested {
            Some("root".to_string())
        } else {
            None
        },
    };

    let cmd_str = body.command.join(" ");
    let result = manager.exec_cmd_full(name, &body.command, &opts).await;
    let duration_ms = exec_start.elapsed().as_millis() as u64;

    let (success, exit_code) = match &result {
        Ok(_) => (true, Some(0)),
        Err(e) => {
            if let Some(cmd_err) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                (false, Some(cmd_err.exit_code))
            } else {
                (false, None)
            }
        }
    };

    // Emit sandbox.exec.completed event
    let event_labels = manager
        .get_state(name)
        .map(|s| s.labels.clone())
        .unwrap_or_default();
    crate::events::emit(
        state.event_bus.as_ref(),
        crate::events::SandboxEvent {
            event: "sandbox.exec.completed".to_string(),
            timestamp: chrono::Utc::now(),
            sandbox: name.to_string(),
            labels: event_labels,
            metadata: serde_json::json!({
                "command": cmd_str,
                "duration_ms": duration_ms,
                "success": success,
                "exit_code": exit_code,
            }),
        },
    );

    match result {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(RunResponse { output }),
        ),
        Err(e) => {
            if let Some(cmd_err) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                json_response(
                    StatusCode::CONFLICT,
                    &serde_json::json!({
                        "success": false,
                        "error": cmd_err.to_string(),
                        "exit_code": cmd_err.exit_code,
                        "output": cmd_err.output,
                    }),
                )
            } else {
                json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                )
            }
        }
    }
}

// --- Detached command handlers ---

async fn handle_exec_detach(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: ExecRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("command is required"),
        );
    }
    let sudo_requested = body.sudo.unwrap_or(false);
    if sudo_requested && !state.allow_sudo_exec {
        return json_response(
            StatusCode::FORBIDDEN,
            &ApiResponse::<()>::error(
                "sudo execution is disabled for HTTP API. Set [api].allow_sudo_exec = true to enable it",
            ),
        );
    }
    if let Some(ref workdir) = body.workdir
        && let Err(e) = validation::validate_exec_workdir(workdir)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let opts = crate::backend::ExecOptions {
        env: body.env,
        workdir: body.workdir,
        user: if sudo_requested {
            Some("root".to_string())
        } else {
            None
        },
    };

    match manager.exec_detached(name, &body.command, &opts).await {
        Ok(cmd) => json_response(StatusCode::OK, &ApiResponse::success(cmd)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_detached_list(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let commands = manager.detached_list(Some(name));
    json_response(StatusCode::OK, &ApiResponse::success(commands))
}

async fn handle_detached_status(
    _name: &str,
    cmd_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.detached_status(cmd_id).await {
        Ok(cmd) => json_response(StatusCode::OK, &ApiResponse::success(cmd)),
        Err(e) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_detached_logs(
    req: Request<Incoming>,
    _name: &str,
    cmd_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Check for ?stream=stderr query param
    let stream = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("stream=")))
        .filter(|s| *s == "stderr");

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.detached_logs(cmd_id, stream).await {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(DetachedLogsResponse {
                stdout: if stream.is_none() {
                    Some(output.clone())
                } else {
                    None
                },
                stderr: if stream.is_some() {
                    Some(output.clone())
                } else {
                    None
                },
            }),
        ),
        Err(e) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_detached_kill(
    _name: &str,
    cmd_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.detached_kill(cmd_id).await {
        Ok(()) => json_response(StatusCode::OK, &ApiResponse::success("Command killed")),
        Err(e) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    // Enterprise policy enforcement (reuse Create action for lifecycle operations)
    #[cfg(feature = "enterprise")]
    {
        let identity = crate::identity::AgentIdentity::anonymous();
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Create, name).await
        {
            return resp;
        }
    }

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Capture labels before removal (remove destroys the state)
    let event_labels = manager
        .get_state(name)
        .map(|s| s.labels.clone())
        .unwrap_or_default();

    match manager.remove(name).await {
        Ok(_) => {
            crate::events::emit(
                state.event_bus.as_ref(),
                crate::events::SandboxEvent {
                    event: "sandbox.deleted".to_string(),
                    timestamp: chrono::Utc::now(),
                    sandbox: name.to_string(),
                    labels: event_labels,
                    metadata: serde_json::json!({}),
                },
            );
            json_response(StatusCode::OK, &ApiResponse::success("Sandbox removed"))
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_start_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = crate::identity::AgentIdentity::anonymous();
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Create, name).await
        {
            return resp;
        }
    }

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.start(name).await {
        Ok(_) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox started")),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_stop_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    {
        let identity = crate::identity::AgentIdentity::anonymous();
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Create, name).await
        {
            return resp;
        }
    }

    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.stop(name).await {
        Ok(_) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox stopped")),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

/// Request body for extending TTL
#[derive(Debug, Deserialize)]
struct ExtendTtlRequest {
    /// Additional time in seconds (or time string like "1h", "30m")
    #[serde(default = "default_extend_by")]
    by: String,
}

fn default_extend_by() -> String {
    "1h".to_string()
}

/// Response for extend TTL
#[derive(Debug, Serialize)]
struct ExtendTtlResponse {
    expires_at: Option<String>,
}

async fn handle_extend_ttl(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Validate sandbox name
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Parse request body (optional - defaults to 1h if empty)
    let body: ExtendTtlRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(_) => ExtendTtlRequest {
            by: "1h".to_string(),
        },
    };

    // Parse the time string into seconds
    let additional_secs = match crate::ssh::parse_ttl_to_secs(&body.by) {
        Ok(secs) => secs,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Invalid time format: {}", e)),
            );
        }
    };

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Check if sandbox exists
    if !manager.exists(name) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
        );
    }

    // Extend the TTL
    match manager.extend_ttl(name, additional_secs) {
        Ok(new_expiry) => json_response(
            StatusCode::OK,
            &ApiResponse::success(ExtendTtlResponse {
                expires_at: new_expiry,
            }),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

// --- Resize handler ---

#[derive(Debug, Deserialize)]
struct ResizeSandboxRequest {
    vcpus: Option<u32>,
    memory_mb: Option<u64>,
}

async fn handle_resize_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: ResizeSandboxRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Sandbox must exist
    let sandbox_state = match manager.get_state(name) {
        Some(s) => s.clone(),
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
            );
        }
    };

    let new_vcpus = body.vcpus.unwrap_or(sandbox_state.vcpus);
    let new_memory = body.memory_mb.unwrap_or(sandbox_state.memory_mb);

    let was_running = manager.is_running(name);
    if !was_running {
        if let Err(e) = manager.update_resources(name, new_vcpus, new_memory) {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to update sandbox resources: {}", e)),
            );
        }
    } else {
        let resized_in_place = match manager
            .try_resize_in_place(name, new_vcpus, new_memory)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Failed to resize sandbox in-place: {}", e)),
                );
            }
        };

        if !resized_in_place {
            // Fallback path for backends that don't support live resize yet:
            // stop -> recreate with preserved metadata -> restart.
            if let Err(e) = manager.stop(name).await {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Failed to stop sandbox: {}", e)),
                );
            }

            let image = sandbox_state.image.clone();
            let ports = sandbox_state.ports.clone();
            let agent = sandbox_state.agent.clone();
            let ttl_seconds = sandbox_state.ttl_seconds;

            if let Err(e) = manager.remove(name).await {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Failed to remove sandbox: {}", e)),
                );
            }

            if let Err(e) = manager
                .create_with_agent(
                    name,
                    &image,
                    new_vcpus,
                    new_memory,
                    ttl_seconds,
                    ports,
                    agent,
                )
                .await
            {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Failed to recreate sandbox: {}", e)),
                );
            }

            // Preserve identity/history metadata so resize does not look like a
            // brand-new sandbox to external systems.
            if let Err(e) = manager.set_identity_metadata(
                name,
                &sandbox_state.uuid,
                &sandbox_state.created_at,
                sandbox_state.expires_at.as_deref(),
            ) {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!(
                        "Failed to preserve sandbox identity metadata: {}",
                        e
                    )),
                );
            }

            // Restore mutable metadata that isn't part of create_with_agent.
            let _ = manager.set_ssh_enabled(name, sandbox_state.ssh_enabled);
            let _ = manager.set_secret_bindings(name, &sandbox_state.secret_bindings);
            let _ = manager.set_secret_mappings(name, &sandbox_state.secret_mappings);
            let _ = manager.set_secret_files(name, &sandbox_state.secret_files);
            let _ = manager.set_placeholder_secrets(name, sandbox_state.placeholder_secrets);
            let _ = manager.set_labels(name, &sandbox_state.labels);
            let _ = manager.set_description(name, sandbox_state.description.as_deref());
            let _ = manager.set_lifecycle_policy(name, sandbox_state.lifecycle_policy.clone());
            let _ = manager.set_template_metadata(
                name,
                sandbox_state.created_from_template.as_deref(),
                sandbox_state.template_help_text.as_deref(),
            );
            let _ = manager.set_volumes(name, &sandbox_state.volumes);
            if let Some(script) = sandbox_state.init_script.as_deref() {
                let _ = manager.set_init_script(name, script);
            }

            if let Err(e) = manager.start(name).await {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Resized but failed to restart: {}", e)),
                );
            }
        }
    }

    let running = manager.is_running(name);
    let ip = if running {
        manager.get_container_ip(name)
    } else {
        None
    };
    let state_info = manager.get_state(name);
    let result_ports: Vec<String> = state_info
        .map(|s| s.ports.iter().map(|p| p.to_string()).collect())
        .unwrap_or_default();

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxInfo {
            name: name.to_string(),
            status: sandbox_status(state_info, running),
            backend: format!("{}", manager.backend()),
            ip,
            image: state_info.map(|s| s.image.clone()),
            vcpus: Some(new_vcpus),
            memory_mb: Some(new_memory),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports: result_ports,
            secret_files: state_info
                .map(|s| s.secret_files.clone())
                .unwrap_or_default(),
            placeholder_secrets: state_info
                .map(|s| s.placeholder_secrets)
                .unwrap_or(false),
            proxy_port: state_info.and_then(|s| s.proxy_port),
            uuid: state_info
                .map(|s| s.uuid.clone())
                .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
            secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
            labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
            description: state_info.and_then(|s| s.description.clone()),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            archived_at: state_info.and_then(|s| s.archived_at.clone()),
            archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
            lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
        }),
    )
}

// --- Patch sandbox handler ---

#[derive(Debug, Deserialize)]
struct PatchSandboxRequest {
    #[serde(default)]
    labels: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    description: Option<String>,
    /// Present with `null` clears policy, object sets policy.
    #[serde(default)]
    lifecycle: Option<Option<LifecyclePolicyRequest>>,
}

async fn handle_patch_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: PatchSandboxRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Sandbox must exist
    if manager.get_state(name).is_none() {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
        );
    }

    // Apply label updates
    if let Some(labels) = body.labels
        && let Err(e) = manager.set_labels(name, &labels)
    {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to update labels: {}", e)),
        );
    }

    // Apply description update
    if body.description.is_some()
        && let Err(e) = manager.set_description(name, body.description.as_deref())
    {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to update description: {}", e)),
        );
    }

    // Apply lifecycle policy update (null clears the policy)
    if let Some(lifecycle) = body.lifecycle {
        let policy = lifecycle.map(Into::into);
        if let Err(e) = manager.set_lifecycle_policy(name, policy) {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to update lifecycle policy: {}", e)),
            );
        }
    }

    // Return updated sandbox info
    let running = manager.is_running(name);
    let ip = if running {
        manager.get_container_ip(name)
    } else {
        None
    };
    let state_info = manager.get_state(name);
    let result_ports: Vec<String> = state_info
        .map(|s| s.ports.iter().map(|p| p.to_string()).collect())
        .unwrap_or_default();

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxInfo {
            name: name.to_string(),
            status: sandbox_status(state_info, running),
            backend: format!("{}", manager.backend()),
            ip,
            image: state_info.map(|s| s.image.clone()),
            vcpus: state_info.map(|s| s.vcpus),
            memory_mb: state_info.map(|s| s.memory_mb),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports: result_ports,
            secret_files: state_info
                .map(|s| s.secret_files.clone())
                .unwrap_or_default(),
            placeholder_secrets: state_info
                .map(|s| s.placeholder_secrets)
                .unwrap_or(false),
            proxy_port: state_info.and_then(|s| s.proxy_port),
            uuid: state_info
                .map(|s| s.uuid.clone())
                .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
            secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
            labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
            description: state_info.and_then(|s| s.description.clone()),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            archived_at: state_info.and_then(|s| s.archived_at.clone()),
            archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
            lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
        }),
    )
}

#[derive(Debug, Deserialize)]
struct ReconcileLifecycleRequest {
    #[serde(default)]
    dry_run: bool,
}

async fn handle_recover_sandbox(
    _req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if manager.get_state(name).is_none() {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
        );
    }

    if let Err(e) = manager.recover(name) {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to recover sandbox: {}", e)),
        );
    }

    let running = manager.is_running(name);
    let ip = if running {
        manager.get_container_ip(name)
    } else {
        None
    };
    let state_info = manager.get_state(name);
    let ports: Vec<String> = state_info
        .map(|s| s.ports.iter().map(|p| p.to_string()).collect())
        .unwrap_or_default();

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxInfo {
            name: name.to_string(),
            uuid: state_info
                .map(|s| s.uuid.clone())
                .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
            status: sandbox_status(state_info, running),
            backend: format!("{}", manager.backend()),
            ip,
            image: state_info.map(|s| s.image.clone()),
            vcpus: state_info.map(|s| s.vcpus),
            memory_mb: state_info.map(|s| s.memory_mb),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports,
            secret_files: state_info
                .map(|s| s.secret_files.clone())
                .unwrap_or_default(),
            placeholder_secrets: state_info
                .map(|s| s.placeholder_secrets)
                .unwrap_or(false),
            proxy_port: state_info.and_then(|s| s.proxy_port),
            secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
            labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
            description: state_info.and_then(|s| s.description.clone()),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            archived_at: state_info.and_then(|s| s.archived_at.clone()),
            archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
            lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
        }),
    )
}

async fn handle_reconcile_lifecycle(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let body_bytes = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let body = if body_bytes.is_empty() {
        ReconcileLifecycleRequest { dry_run: false }
    } else {
        match serde_json::from_slice::<ReconcileLifecycleRequest>(&body_bytes) {
            Ok(parsed) => parsed,
            Err(_) => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &ApiResponse::<()>::error("Invalid JSON body"),
                );
            }
        }
    };

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.reconcile_lifecycle(body.dry_run).await {
        Ok(result) => json_response(StatusCode::OK, &ApiResponse::success(result)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to reconcile lifecycle policies: {}", e)),
        ),
    }
}

// --- Snapshot handlers ---

async fn handle_list_snapshots(_state: Arc<AppState>) -> Response<BoxBody> {
    match crate::snapshot::list() {
        Ok(snapshots) => json_response(StatusCode::OK, &ApiResponse::success(snapshots)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

/// Request body for taking a snapshot
#[derive(Debug, Deserialize)]
struct TakeSnapshotRequest {
    /// Name of the sandbox to snapshot
    sandbox: String,
    /// Name for the snapshot
    name: String,
}

async fn handle_take_snapshot(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: TakeSnapshotRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Validate names
    if let Err(e) = validation::validate_sandbox_name(&body.sandbox) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }
    if let Err(e) = validation::validate_sandbox_name(&body.name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid snapshot name: {}", e)),
        );
    }

    // Get sandbox info
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandbox_state = match manager.get_state(&body.sandbox) {
        Some(s) => s,
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error(format!("Sandbox '{}' not found", body.sandbox)),
            );
        }
    };

    let input = crate::snapshot::SnapshotInput {
        image: sandbox_state.image.clone(),
        backend: sandbox_state
            .backend
            .map(|b| format!("{:?}", b).to_lowercase())
            .unwrap_or_else(|| "docker".to_string()),
        vcpus: sandbox_state.vcpus,
        memory_mb: sandbox_state.memory_mb,
    };

    match crate::snapshot::take(&body.sandbox, &body.name, &input) {
        Ok(meta) => json_response(StatusCode::OK, &ApiResponse::success(meta)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_snapshot(name: &str) -> Response<BoxBody> {
    // Validate snapshot name
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    match crate::snapshot::get(name) {
        Ok(Some(meta)) => json_response(StatusCode::OK, &ApiResponse::success(meta)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Snapshot '{}' not found", name)),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_snapshot(name: &str) -> Response<BoxBody> {
    // Validate snapshot name
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    match crate::snapshot::delete(name) {
        Ok(()) => json_response(StatusCode::OK, &ApiResponse::success("Snapshot deleted")),
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            json_response(status, &ApiResponse::<()>::error(e.to_string()))
        }
    }
}

/// Request body for restoring a snapshot
#[derive(Debug, Deserialize)]
struct RestoreSnapshotRequest {
    /// Name for the restored sandbox (defaults to original + "-restored")
    #[serde(default)]
    as_name: Option<String>,
}

async fn handle_restore_snapshot(
    req: Request<Incoming>,
    snapshot_name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Validate snapshot name
    if let Err(e) = validation::validate_sandbox_name(snapshot_name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Parse optional body
    let body: RestoreSnapshotRequest = read_json_body(req)
        .await
        .unwrap_or(RestoreSnapshotRequest { as_name: None });

    // Get snapshot metadata
    let meta = match crate::snapshot::get(snapshot_name) {
        Ok(Some(m)) => m,
        Ok(None) => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error(format!("Snapshot '{}' not found", snapshot_name)),
            );
        }
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Determine restore name
    let restore_name = body
        .as_name
        .unwrap_or_else(|| format!("{}-restored", meta.sandbox));

    if let Err(e) = validation::validate_sandbox_name(&restore_name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid restore name: {}", e)),
        );
    }

    // Create the restored sandbox
    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager
        .create(&restore_name, &meta.image_tag, meta.vcpus, meta.memory_mb)
        .await
    {
        Ok(()) => {
            // Build SandboxInfo from manager state
            let state_info = manager.get_state(&restore_name);
            let running = manager.is_running(&restore_name);
            let ip = if running {
                manager.get_container_ip(&restore_name)
            } else {
                None
            };
            let ports = state_info
                .map(|s| s.ports.iter().map(|p| p.to_string()).collect())
                .unwrap_or_default();
            json_response(
                StatusCode::OK,
                &ApiResponse::success(SandboxInfo {
                    name: restore_name,
                    uuid: state_info
                        .map(|s| s.uuid.clone())
                        .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
                    status: sandbox_status(state_info, running),
                    backend: meta.backend.clone(),
                    ip,
                    image: Some(meta.image_tag.clone()),
                    vcpus: Some(meta.vcpus),
                    memory_mb: Some(meta.memory_mb),
                    created_at: state_info.map(|s| s.created_at.clone()),
                    created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
                    template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
                    ports,
                    secret_files: state_info
                        .map(|s| s.secret_files.clone())
                        .unwrap_or_default(),
                    placeholder_secrets: state_info
                        .map(|s| s.placeholder_secrets)
                        .unwrap_or(false),
                    proxy_port: state_info.and_then(|s| s.proxy_port),
                    secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
                    labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
                    description: state_info.and_then(|s| s.description.clone()),
                    last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
                    archived_at: state_info.and_then(|s| s.archived_at.clone()),
                    archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
                    lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
                }),
            )
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

/// Resolve a profile name to a SecurityProfile
fn resolve_profile(name: &str) -> Option<SecurityProfile> {
    match name.to_lowercase().as_str() {
        "permissive" => Some(SecurityProfile::Permissive),
        "moderate" => Some(SecurityProfile::Moderate),
        "restrictive" => Some(SecurityProfile::Restrictive),
        _ => None,
    }
}

// --- File operation handlers ---

async fn handle_file_read(name: &str, file_path: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let abs_path = format!("/{}", file_path);
    if let Err(e) = crate::backend::validate_sandbox_path(&abs_path) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.read_file(name, &abs_path).await {
        Ok(content) => {
            let size = content.len();
            let (content_str, encoding) = match String::from_utf8(content.clone()) {
                Ok(s) => (s, "utf8"),
                Err(_) => (
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &content),
                    "base64",
                ),
            };
            json_response(
                StatusCode::OK,
                &ApiResponse::success(FileReadResponse {
                    content: content_str,
                    encoding: encoding.to_string(),
                    size,
                }),
            )
        }
        Err(e) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_file_write(
    req: Request<Incoming>,
    name: &str,
    file_path: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Mount, name).await
        {
            return resp;
        }
    }

    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let abs_path = format!("/{}", file_path);
    if let Err(e) = crate::backend::validate_sandbox_path(&abs_path) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: FileWriteRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let bytes = if body.encoding == "base64" {
        match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &body.content) {
            Ok(b) => b,
            Err(e) => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &ApiResponse::<()>::error(format!("Invalid base64: {}", e)),
                );
            }
        }
    } else {
        body.content.into_bytes()
    };

    let size = bytes.len();

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.write_file(name, &abs_path, &bytes).await {
        Ok(()) => json_response(
            StatusCode::OK,
            &ApiResponse::success(format!("Wrote {} bytes to {}", size, abs_path)),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_file_delete(
    name: &str,
    file_path: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = crate::identity::AgentIdentity::anonymous();
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Mount, name).await
        {
            return resp;
        }
    }

    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let abs_path = format!("/{}", file_path);
    if let Err(e) = crate::backend::validate_sandbox_path(&abs_path) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.delete_file(name, &abs_path).await {
        Ok(()) => json_response(
            StatusCode::OK,
            &ApiResponse::success(format!("Deleted {}", abs_path)),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_batch_file_write(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Mount, name).await
        {
            return resp;
        }
    }

    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: BatchFileWriteRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.files.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("files map is empty"),
        );
    }

    // Validate all paths before writing any
    for path in body.files.keys() {
        if let Err(e) = crate::backend::validate_sandbox_path(path) {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("{}: {}", path, e)),
            );
        }
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let count = body.files.len();
    for (path, content) in &body.files {
        if let Err(e) = manager.write_file(name, path, content.as_bytes()).await {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to write {}: {}", path, e)),
            );
        }
    }

    json_response(
        StatusCode::OK,
        &ApiResponse::success(format!("Wrote {} file(s)", count)),
    )
}

// --- Sandbox logs handler ---

async fn handle_sandbox_logs(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Verify sandbox exists
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if !manager.exists(name) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Sandbox not found"),
        );
    }

    let audit = crate::audit::audit();
    match audit.read_by_sandbox(name) {
        Ok(entries) => json_response(StatusCode::OK, &ApiResponse::success(entries)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

// --- Batch run handler ---

async fn handle_batch_run(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if let Err(resp) =
            enforce_policy(&state, &identity, crate::policy::Action::Run, "batch").await
        {
            return resp;
        }
    }

    let body: BatchRunRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.commands.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("commands array is required and must not be empty"),
        );
    }

    // Verify we can get a manager (validates backend availability)
    if let Err(e) = state.get_manager().await {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Run all commands in parallel using the container pool
    let handles: Vec<_> = body
        .commands
        .into_iter()
        .map(|batch_cmd| {
            tokio::spawn(async move { VmManager::run_pooled(&batch_cmd.command).await })
        })
        .collect();

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(output)) => results.push(BatchResult {
                output: Some(output),
                error: None,
            }),
            Ok(Err(e)) => results.push(BatchResult {
                output: None,
                error: Some(e.to_string()),
            }),
            Err(e) => results.push(BatchResult {
                output: None,
                error: Some(format!("Task failed: {}", e)),
            }),
        }
    }

    json_response(
        StatusCode::OK,
        &ApiResponse::success(BatchRunResponse { results }),
    )
}

// --- Enterprise policy handlers ---

/// Policy status response
#[cfg(feature = "enterprise")]
#[derive(Debug, Serialize)]
struct PolicyStatusResponse {
    enabled: bool,
    version: u64,
    org_id: Option<String>,
    offline_mode: String,
    policy_server: Option<String>,
}

/// Policy check request
#[cfg(feature = "enterprise")]
#[derive(Debug, Deserialize)]
struct PolicyCheckRequest {
    action: String,
    sandbox: String,
}

/// Policy check response
#[cfg(feature = "enterprise")]
#[derive(Debug, Serialize)]
struct PolicyCheckResponse {
    decision: String,
    reason: String,
    matched_policies: Vec<String>,
    evaluation_time_us: u64,
}

#[cfg(feature = "enterprise")]
async fn handle_policy_status(state: Arc<AppState>) -> Response<BoxBody> {
    let Some(ref enterprise) = state.enterprise_config else {
        return json_response(
            StatusCode::OK,
            &ApiResponse::success(PolicyStatusResponse {
                enabled: false,
                version: 0,
                org_id: None,
                offline_mode: "disabled".to_string(),
                policy_server: None,
            }),
        );
    };

    let version = if let Some(ref engine_lock) = state.policy_engine {
        let engine = engine_lock.read().await;
        engine.version().await
    } else {
        0
    };

    json_response(
        StatusCode::OK,
        &ApiResponse::success(PolicyStatusResponse {
            enabled: enterprise.enabled,
            version,
            org_id: enterprise.org_id.clone(),
            offline_mode: enterprise.offline_mode.clone(),
            policy_server: enterprise.policy_server.clone(),
        }),
    )
}

#[cfg(feature = "enterprise")]
async fn handle_policy_check(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: PolicyCheckRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let Some(ref engine_lock) = state.policy_engine else {
        return json_response(
            StatusCode::OK,
            &ApiResponse::success(PolicyCheckResponse {
                decision: "permit".to_string(),
                reason: "No policy engine active (enterprise disabled)".to_string(),
                matched_policies: vec![],
                evaluation_time_us: 0,
            }),
        );
    };

    let Some(ref enterprise) = state.enterprise_config else {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error("Enterprise config missing"),
        );
    };

    let action = match body.action.to_lowercase().as_str() {
        "run" => crate::policy::Action::Run,
        "exec" => crate::policy::Action::Exec,
        "create" => crate::policy::Action::Create,
        "attach" => crate::policy::Action::Attach,
        "mount" => crate::policy::Action::Mount,
        "network" => crate::policy::Action::Network,
        "portmap" => crate::policy::Action::PortMap,
        "ssh" => crate::policy::Action::SSH,
        other => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!(
                    "Invalid action '{}'. Use: run, exec, create, attach, mount, network, portmap, ssh",
                    other
                )),
            );
        }
    };

    // Build principal from the local user (for CLI-like testing)
    let principal = crate::policy::Principal {
        id: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        email: String::new(),
        org_id: enterprise
            .org_id
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        roles: enterprise.default_roles.clone(),
        mfa_verified: false,
    };

    let resource = crate::policy::Resource {
        name: body.sandbox,
        agent_type: "api".to_string(),
        runtime: "unknown".to_string(),
    };

    let engine = engine_lock.read().await;
    let decision = engine.evaluate(&principal, action, &resource).await;

    json_response(
        StatusCode::OK,
        &ApiResponse::success(PolicyCheckResponse {
            decision: if decision.is_permit() {
                "permit".to_string()
            } else {
                "deny".to_string()
            },
            reason: decision.reason,
            matched_policies: decision.matched_policies,
            evaluation_time_us: decision.evaluation_time_us,
        }),
    )
}

#[cfg(feature = "enterprise")]
#[derive(Debug, Serialize)]
struct PolicyReloadResponse {
    reloaded: bool,
    version: u64,
}

#[cfg(feature = "enterprise")]
async fn handle_policy_reload(state: Arc<AppState>) -> Response<BoxBody> {
    let Some(ref engine_lock) = state.policy_engine else {
        return json_response(
            StatusCode::OK,
            &ApiResponse::success(PolicyReloadResponse {
                reloaded: false,
                version: 0,
            }),
        );
    };

    let mut engine = engine_lock.write().await;
    match engine.reload().await {
        Ok(()) => {
            let version = engine.version().await;
            json_response(
                StatusCode::OK,
                &ApiResponse::success(PolicyReloadResponse {
                    reloaded: true,
                    version,
                }),
            )
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Policy reload failed: {e}")),
        ),
    }
}

#[cfg(feature = "enterprise")]
async fn handle_policy_audit(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    // Parse ?last=N query param (default 50)
    let last: usize = req
        .uri()
        .query()
        .and_then(|q| {
            q.split('&')
                .find_map(|pair| pair.strip_prefix("last="))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(50);

    let Some(ref engine_lock) = state.policy_engine else {
        // No engine → return empty list
        let empty: Vec<crate::policy::PolicyDecisionLog> = Vec::new();
        return json_response(StatusCode::OK, &ApiResponse::success(empty));
    };

    let engine = engine_lock.read().await;
    match engine.audit_logger().read_last(last) {
        Ok(entries) => json_response(StatusCode::OK, &ApiResponse::success(entries)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to read policy audit log: {e}")),
        ),
    }
}

fn spawn_orchestration_worker(state: Arc<AppState>) {
    let Some(store) = state.orchestration_store.clone() else {
        return;
    };

    tokio::spawn(async move {
        eprintln!("[durable] orchestration worker started");
        loop {
            if let Err(e) = process_orchestrations_tick(store.clone()).await {
                eprintln!("[durable] worker tick failed: {e}");
            }
            sleep(Duration::from_millis(750)).await;
        }
    });
}

async fn process_orchestrations_tick(store: Arc<OrchestrationStore>) -> Result<()> {
    let records = store.list(200, 0)?;

    for record in records {
        if matches!(
            record.status,
            OrchestrationStatus::Pending | OrchestrationStatus::Running
        ) && let Err(e) = process_orchestration_record(store.clone(), record).await
        {
            eprintln!("[durable] orchestration processing error: {e}");
        }
    }

    Ok(())
}

async fn process_orchestration_record(
    store: Arc<OrchestrationStore>,
    record: OrchestrationRecord,
) -> Result<()> {
    let orchestration_id = record.id.clone();
    let history = store.list_events(&orchestration_id, 5000, 0)?;

    if let Some(output) = history.iter().rev().find_map(|event| {
        if event.event_type != "OrchestratorCompleted" {
            return None;
        }
        event
            .data
            .as_ref()
            .and_then(|d| d.get("output"))
            .cloned()
            .or(Some(serde_json::Value::Null))
    }) {
        if record.status != OrchestrationStatus::Completed {
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Completed),
                    output: Some(output),
                    error: None,
                },
            )?;
        }
        return Ok(());
    }

    if let Some(error) = history.iter().rev().find_map(|event| {
        if event.event_type != "OrchestratorFailed" {
            return None;
        }
        event
            .data
            .as_ref()
            .and_then(|d| d.get("error"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }) {
        if record.status != OrchestrationStatus::Failed {
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Failed),
                    output: None,
                    error: Some(error),
                },
            )?;
        }
        return Ok(());
    }

    let runtime_input = match parse_runtime_input(record.input.clone()) {
        Ok(input) => input,
        Err(parse_error) => {
            let error = format!("invalid orchestration input: {parse_error}");
            store.append_event(
                &orchestration_id,
                "OrchestratorFailed",
                serde_json::json!({ "error": error }),
            )?;
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Failed),
                    output: None,
                    error: Some(error),
                },
            )?;
            return Ok(());
        }
    };

    let mut wait_name = runtime_input.wait_for_event.clone();
    let mut single_activity = runtime_input.activity.clone();
    let mut activity_sequence = runtime_input.activities.clone().unwrap_or_default();

    if wait_name.is_none()
        && single_activity.is_none()
        && activity_sequence.is_empty()
        && let Some(definition) = store.get_definition(&record.name)?
    {
        let parsed = match parse_runtime_input(Some(definition.definition)) {
            Ok(value) => value,
            Err(parse_error) => {
                let error = format!("invalid orchestration definition: {parse_error}");
                store.append_event(
                    &orchestration_id,
                    "OrchestratorFailed",
                    serde_json::json!({ "error": error }),
                )?;
                let _ = store.update(
                    &orchestration_id,
                    UpdateOrchestration {
                        status: Some(OrchestrationStatus::Failed),
                        output: None,
                        error: Some(error),
                    },
                )?;
                return Ok(());
            }
        };
        wait_name = parsed.wait_for_event;
        single_activity = parsed.activity;
        activity_sequence = parsed.activities.unwrap_or_default();
    }

    if activity_sequence.is_empty()
        && let Some(activity) = single_activity
    {
        activity_sequence.push(activity);
    }

    if let Some(wait_name) = wait_name {
        if record.status == OrchestrationStatus::Pending {
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Running),
                    output: None,
                    error: None,
                },
            )?;
        }

        if !history.iter().any(|event| {
            event.event_type == "EventConsumed"
                && event
                    .data
                    .as_ref()
                    .and_then(|d| d.get("name"))
                    .and_then(serde_json::Value::as_str)
                    == Some(wait_name.as_str())
        }) && let Some(signal_data) = history.iter().rev().find_map(|event| {
            if event.event_type != "EventRaised" {
                return None;
            }
            let payload = event.data.as_ref()?;
            let name = payload.get("name")?.as_str()?;
            if name == wait_name {
                Some(
                    payload
                        .get("data")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                )
            } else {
                None
            }
        }) {
            store.append_event(
                &orchestration_id,
                "EventConsumed",
                serde_json::json!({ "name": wait_name }),
            )?;
            store.append_event(
                &orchestration_id,
                "OrchestratorCompleted",
                serde_json::json!({ "output": signal_data }),
            )?;
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Completed),
                    output: Some(signal_data),
                    error: None,
                },
            )?;
        }

        return Ok(());
    }

    if !activity_sequence.is_empty() {
        if activity_sequence
            .iter()
            .any(|activity| activity.command.is_empty())
        {
            let error = "activity.command must not be empty".to_string();
            store.append_event(
                &orchestration_id,
                "OrchestratorFailed",
                serde_json::json!({ "error": error }),
            )?;
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Failed),
                    output: None,
                    error: Some(error),
                },
            )?;
            return Ok(());
        }

        if record.status == OrchestrationStatus::Pending {
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Running),
                    output: None,
                    error: None,
                },
            )?;
        }

        let completed_steps = history
            .iter()
            .filter(|event| event.event_type == "ActivityCompleted")
            .count();

        if completed_steps >= activity_sequence.len() {
            let output = history
                .iter()
                .rev()
                .find_map(|event| {
                    if event.event_type != "ActivityCompleted" {
                        return None;
                    }
                    event
                        .data
                        .as_ref()
                        .and_then(|d| d.get("output"))
                        .cloned()
                        .or(Some(serde_json::Value::Null))
                })
                .unwrap_or(serde_json::Value::Null);
            if !history
                .iter()
                .any(|event| event.event_type == "OrchestratorCompleted")
            {
                store.append_event(
                    &orchestration_id,
                    "OrchestratorCompleted",
                    serde_json::json!({ "output": output }),
                )?;
            }
            let _ = store.update(
                &orchestration_id,
                UpdateOrchestration {
                    status: Some(OrchestrationStatus::Completed),
                    output: Some(output),
                    error: None,
                },
            )?;
            return Ok(());
        }

        let current_step = completed_steps;
        let activity = activity_sequence[current_step].clone();
        let retry_policy = activity.retry_policy.clone().unwrap_or_default();

        let failure_events: Vec<&OrchestrationEvent> = history
            .iter()
            .rev()
            .take_while(|event| event.event_type != "ActivityCompleted")
            .filter(|event| event.event_type == "ActivityFailed")
            .collect();
        let failure_attempts = failure_events.len() as u32;

        if failure_attempts > 0 {
            let last_error = failure_events
                .first()
                .and_then(|event| event.data.as_ref())
                .and_then(|data| data.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("activity failed")
                .to_string();

            if failure_attempts >= retry_policy.max_attempts {
                if !history
                    .iter()
                    .any(|event| event.event_type == "OrchestratorFailed")
                {
                    store.append_event(
                        &orchestration_id,
                        "OrchestratorFailed",
                        serde_json::json!({ "error": last_error }),
                    )?;
                }
                let _ = store.update(
                    &orchestration_id,
                    UpdateOrchestration {
                        status: Some(OrchestrationStatus::Failed),
                        output: None,
                        error: Some(last_error),
                    },
                )?;
                return Ok(());
            }

            if let Some(last_failure) = failure_events.first()
                && let Ok(last_failure_at) =
                    chrono::DateTime::parse_from_rfc3339(&last_failure.timestamp)
            {
                let required_delay = compute_retry_delay_ms(&retry_policy, failure_attempts);
                let elapsed_ms = (chrono::Utc::now() - last_failure_at.with_timezone(&chrono::Utc))
                    .num_milliseconds()
                    .max(0) as u64;
                if elapsed_ms < required_delay {
                    return Ok(());
                }
            }
        }

        if !history.iter().any(|event| {
            event.event_type == "ActivityScheduled"
                && event
                    .data
                    .as_ref()
                    .and_then(|data| data.get("step"))
                    .and_then(serde_json::Value::as_u64)
                    == Some(current_step as u64)
        }) {
            let idempotency_key = compute_idempotency_key(
                &orchestration_id,
                &activity.name,
                (current_step + 1) as i64,
            );
            store.append_event(
                &orchestration_id,
                "ActivityScheduled",
                serde_json::json!({
                    "name": activity.name.clone(),
                    "step": current_step,
                    "input": {
                        "command": activity.command.clone(),
                        "image": activity.image.clone(),
                        "fast": activity.fast,
                        "retry_policy": {
                            "max_attempts": retry_policy.max_attempts,
                            "initial_interval_ms": retry_policy.initial_interval_ms,
                            "backoff_coefficient": retry_policy.backoff_coefficient,
                            "max_interval_ms": retry_policy.max_interval_ms,
                            "non_retryable_errors": retry_policy.non_retryable_errors.clone(),
                        }
                    },
                    "idempotency_key": idempotency_key
                }),
            )?;
        }

        let attempt = failure_attempts + 1;
        store.append_event(
            &orchestration_id,
            "ActivityStarted",
            serde_json::json!({
                "step": current_step,
                "attempt": attempt
            }),
        )?;

        match execute_runtime_activity(&activity).await {
            Ok(output) => {
                let output_json = serde_json::Value::String(output);
                store.append_event(
                    &orchestration_id,
                    "ActivityCompleted",
                    serde_json::json!({
                        "step": current_step,
                        "name": activity.name,
                        "output": output_json
                    }),
                )?;
                let _ = store.update(
                    &orchestration_id,
                    UpdateOrchestration {
                        status: Some(OrchestrationStatus::Running),
                        output: None,
                        error: None,
                    },
                )?;
            }
            Err(e) => {
                let error = e.to_string();
                let retryable = is_retryable_error(&error, &retry_policy)
                    && attempt < retry_policy.max_attempts;
                store.append_event(
                    &orchestration_id,
                    "ActivityFailed",
                    serde_json::json!({
                        "step": current_step,
                        "name": activity.name,
                        "error": error,
                        "attempt": attempt,
                        "retryable": retryable
                    }),
                )?;

                if retryable {
                    let _ = store.update(
                        &orchestration_id,
                        UpdateOrchestration {
                            status: Some(OrchestrationStatus::Running),
                            output: None,
                            error: Some(error),
                        },
                    )?;
                } else {
                    store.append_event(
                        &orchestration_id,
                        "OrchestratorFailed",
                        serde_json::json!({ "error": error }),
                    )?;
                    let _ = store.update(
                        &orchestration_id,
                        UpdateOrchestration {
                            status: Some(OrchestrationStatus::Failed),
                            output: None,
                            error: Some(error),
                        },
                    )?;
                }
            }
        }

        return Ok(());
    }

    let output = record.input.clone().unwrap_or(serde_json::Value::Null);
    if !history
        .iter()
        .any(|event| event.event_type == "OrchestratorCompleted")
    {
        store.append_event(
            &orchestration_id,
            "OrchestratorCompleted",
            serde_json::json!({ "output": output }),
        )?;
    }
    let _ = store.update(
        &orchestration_id,
        UpdateOrchestration {
            status: Some(OrchestrationStatus::Completed),
            output: Some(output),
            error: None,
        },
    )?;

    Ok(())
}

fn parse_runtime_input(
    input: Option<serde_json::Value>,
) -> std::result::Result<RuntimeOrchestrationInput, serde_json::Error> {
    match input {
        Some(value) => serde_json::from_value(value),
        None => serde_json::from_value(serde_json::json!({})),
    }
}

fn compute_idempotency_key(orchestration_id: &str, activity_name: &str, sequence: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{orchestration_id}:{activity_name}:{sequence}"));
    format!("{:x}", hasher.finalize())
}

fn compute_retry_delay_ms(policy: &RuntimeRetryPolicy, failure_attempts: u32) -> u64 {
    let exponent = failure_attempts.saturating_sub(1);
    let multiplier = policy.backoff_coefficient.powi(exponent as i32);
    let next = (policy.initial_interval_ms as f64 * multiplier).round();
    let clamped = next.max(policy.initial_interval_ms as f64) as u64;
    clamped.min(policy.max_interval_ms)
}

fn is_retryable_error(error: &str, policy: &RuntimeRetryPolicy) -> bool {
    !policy
        .non_retryable_errors
        .iter()
        .any(|marker| !marker.is_empty() && error.contains(marker))
}

async fn execute_runtime_activity(activity: &RuntimeActivity) -> Result<String> {
    if activity.fast {
        return VmManager::run_pooled(&activity.command).await;
    }

    let image = activity
        .image
        .clone()
        .unwrap_or_else(|| languages::detect_image(&activity.command));
    let mut manager = VmManager::new()?;
    let sandbox_name = format!("orch-activity-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let perms = SecurityProfile::Moderate.permissions();

    manager
        .create(&sandbox_name, &image, 1, 512)
        .await
        .map_err(|e| anyhow::anyhow!("failed to create activity sandbox: {e}"))?;

    if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
        let _ = manager.remove(&sandbox_name).await;
        return Err(anyhow::anyhow!("failed to start activity sandbox: {e}"));
    }

    let result = manager.exec_cmd(&sandbox_name, &activity.command).await;
    let _ = manager.remove(&sandbox_name).await;
    result.map_err(|e| anyhow::anyhow!("activity execution failed: {e}"))
}

/// Run the HTTP API server (plain HTTP)
#[allow(dead_code)]
pub async fn run_server(addr: SocketAddr, api_keys: Vec<String>) -> Result<()> {
    let state = Arc::new(AppState::new(api_keys, None, vec![]));
    spawn_orchestration_worker(state.clone());
    // Spawn hibernation daemon for durable objects
    if let (Some(store), Some(manager)) =
        (state.orchestration_store.clone(), state.vm_manager.clone())
    {
        tokio::spawn(crate::object_runtime::hibernation_daemon(store, manager));
    }
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            anyhow::anyhow!(
                "Port {} is already in use. Is another agentkernel server running?\n\
                 Try: kill the existing process or use --port to pick a different port.",
                addr.port()
            )
        } else {
            anyhow::anyhow!("Failed to bind to {}: {}", addr, e)
        }
    })?;

    eprintln!("agentkernel HTTP API server listening on http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state = state.clone();

        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                let state = state.clone();
                handle_request(req, state)
            });

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

/// Run the HTTP API server with optional TLS.
///
/// When `tls_config` is `Some`, the server will serve HTTPS using the provided
/// TLS configuration. When `None`, the server falls back to plain HTTP.
///
/// If `tls_config.require_tls` is set but no TLS config is provided, this
/// function returns an error immediately.
pub async fn run_server_with_tls(
    addr: SocketAddr,
    tls_config: Option<crate::tls::TlsConfig>,
    api_keys: Vec<String>,
    otel_endpoint: Option<String>,
    webhook_urls: Vec<String>,
) -> Result<()> {
    let acceptor = match tls_config {
        Some(ref tls) => {
            let acceptor = tls.load_or_generate()?;
            Some(acceptor)
        }
        None => None,
    };

    let state = Arc::new(AppState::new(api_keys, otel_endpoint, webhook_urls));
    spawn_orchestration_worker(state.clone());
    // Spawn hibernation daemon for durable objects
    if let (Some(store), Some(manager)) =
        (state.orchestration_store.clone(), state.vm_manager.clone())
    {
        tokio::spawn(crate::object_runtime::hibernation_daemon(store, manager));
    }
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            anyhow::anyhow!(
                "Port {} is already in use. Is another agentkernel server running?\n\
                 Try: kill the existing process or use --port to pick a different port.",
                addr.port()
            )
        } else {
            anyhow::anyhow!("Failed to bind to {}: {}", addr, e)
        }
    })?;

    if acceptor.is_some() {
        eprintln!("agentkernel HTTP API server listening on https://{}", addr);
    } else {
        eprintln!("agentkernel HTTP API server listening on http://{}", addr);
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        let acceptor = acceptor.clone();

        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                let state = state.clone();
                handle_request(req, state)
            });

            if let Some(acceptor) = acceptor {
                // TLS path: wrap TCP stream with TLS
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        let io = TokioIo::new(tls_stream);
                        if let Err(err) = http1::Builder::new().serve_connection(io, service).await
                        {
                            eprintln!("Error serving TLS connection: {:?}", err);
                        }
                    }
                    Err(err) => {
                        eprintln!("TLS handshake failed: {:?}", err);
                    }
                }
            } else {
                // Plain HTTP path
                let io = TokioIo::new(stream);
                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    eprintln!("Error serving connection: {:?}", err);
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Diagnostics handlers
// ---------------------------------------------------------------------------

async fn handle_status(state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let backend = manager.backend().to_string();
    let version = env!("CARGO_PKG_VERSION").to_string();

    #[derive(serde::Serialize)]
    struct StatusInfo {
        version: String,
        backend: String,
        api_key_configured: bool,
    }

    let info = StatusInfo {
        version,
        backend,
        api_key_configured: !state.api_keys.is_empty(),
    };

    json_response(StatusCode::OK, &ApiResponse::success(info))
}

async fn handle_stats(state: Arc<AppState>) -> Response<BoxBody> {
    #[derive(serde::Serialize)]
    struct ResourceUsage {
        cpu_percent: f32,
        memory_used_mb: u64,
        memory_total_mb: u64,
        disk_used_mb: u64,
    }

    #[derive(serde::Serialize)]
    struct Stats {
        sandbox_count: usize,
        sandbox_limit: usize,
        backend: String,
        uptime_seconds: u64,
        version: String,
        resource_usage: ResourceUsage,
    }

    let (sandbox_count, backend) = match state.get_manager().await {
        Ok(manager) => {
            let count = manager.list().len();
            let backend = manager.backend().to_string();
            (count, backend)
        }
        Err(_) => (0, "unknown".to_string()),
    };

    // Gather OS-level resource usage
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    // Brief sleep to get meaningful CPU measurement (sysinfo needs two samples)
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cpu_percent = sys.global_cpu_usage();
    let memory_used_mb = sys.used_memory() / (1024 * 1024);
    let memory_total_mb = sys.total_memory() / (1024 * 1024);

    // Disk usage for root filesystem
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk_used_mb = disks
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"))
        .map(|d| (d.total_space() - d.available_space()) / (1024 * 1024))
        .unwrap_or(0);

    let stats = Stats {
        sandbox_count,
        sandbox_limit: 0,
        backend,
        uptime_seconds: state.started_at.elapsed().as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        resource_usage: ResourceUsage {
            cpu_percent,
            memory_used_mb,
            memory_total_mb,
            disk_used_mb,
        },
    };

    json_response(StatusCode::OK, &ApiResponse::success(stats))
}

async fn handle_doctor(state: Arc<AppState>) -> Response<BoxBody> {
    #[derive(serde::Serialize)]
    struct HealthCheck {
        name: String,
        status: String,
        message: String,
    }

    #[derive(serde::Serialize)]
    struct DoctorResult {
        checks: Vec<HealthCheck>,
        healthy: bool,
    }

    let mut checks = Vec::new();

    // Check backend availability
    let manager = state.get_manager().await;
    let (backend_status, backend_message) = match &manager {
        Ok(m) => (
            "ok".to_string(),
            format!("Backend {} is available", m.backend()),
        ),
        Err(e) => ("error".to_string(), format!("Backend error: {e}")),
    };
    checks.push(HealthCheck {
        name: "backend".to_string(),
        status: backend_status,
        message: backend_message,
    });

    // Check Docker/container CLI availability
    let docker_available = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push(HealthCheck {
        name: "docker".to_string(),
        status: if docker_available {
            "ok".to_string()
        } else {
            "warning".to_string()
        },
        message: if docker_available {
            "Docker is available".to_string()
        } else {
            "Docker not found".to_string()
        },
    });

    // Check if Apple containers available (macOS)
    #[cfg(target_os = "macos")]
    {
        let apple_available = std::process::Command::new("container")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        checks.push(HealthCheck {
            name: "apple_containers".to_string(),
            status: if apple_available {
                "ok".to_string()
            } else {
                "info".to_string()
            },
            message: if apple_available {
                "Apple Containers available".to_string()
            } else {
                "Apple Containers not available".to_string()
            },
        });
    }

    let healthy = checks
        .iter()
        .all(|c| c.status == "ok" || c.status == "info" || c.status == "warning");

    let result = DoctorResult { checks, healthy };
    json_response(StatusCode::OK, &ApiResponse::success(result))
}

async fn handle_audit_log(req: Request<Incoming>) -> Response<BoxBody> {
    // Parse ?last=N query param (default 100)
    let last: usize = req
        .uri()
        .query()
        .and_then(|q| {
            q.split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(k, _)| *k == "last")
                .and_then(|(_, v)| v.parse().ok())
        })
        .unwrap_or(100);

    let audit = crate::audit::audit();
    match audit.read_last(last) {
        Ok(entries) => json_response(StatusCode::OK, &ApiResponse::success(entries)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_gc(state: Arc<AppState>) -> Response<BoxBody> {
    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.gc().await {
        Ok(removed) => {
            #[derive(serde::Serialize)]
            struct GcResult {
                removed: Vec<String>,
                count: usize,
            }
            let count = removed.len();
            json_response(
                StatusCode::OK,
                &ApiResponse::success(GcResult { removed, count }),
            )
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

// ---------------------------------------------------------------------------
// Secrets management
// ---------------------------------------------------------------------------

/// Entry returned by `GET /secrets` — name only, never the value.
#[derive(Debug, Serialize)]
struct SecretListEntry {
    name: String,
    created_at: Option<String>,
}

/// Request body for `POST /secrets`.
#[derive(Debug, Deserialize)]
struct CreateSecretRequest {
    name: String,
    value: String,
}

async fn handle_list_secrets() -> Response<BoxBody> {
    let vault = SecretVault::new(SecretBackend::File);
    match vault.list() {
        Ok(entries) => {
            let list: Vec<SecretListEntry> = entries
                .into_iter()
                .map(|(name, meta)| SecretListEntry {
                    name,
                    created_at: Some(meta.set_at),
                })
                .collect();
            json_response(StatusCode::OK, &ApiResponse::success(list))
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to list secrets: {e}")),
        ),
    }
}

async fn handle_create_secret(req: Request<Incoming>) -> Response<BoxBody> {
    let body: CreateSecretRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.name.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("name is required"),
        );
    }

    let vault = SecretVault::new(SecretBackend::File);
    match vault.set(&body.name, &body.value) {
        Ok(()) => json_response(
            StatusCode::OK,
            &ApiResponse::success("Secret stored".to_string()),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to store secret: {e}")),
        ),
    }
}

async fn handle_delete_secret(name: &str) -> Response<BoxBody> {
    let vault = SecretVault::new(SecretBackend::File);
    match vault.delete(name) {
        Ok(()) => json_response(
            StatusCode::OK,
            &ApiResponse::success("Secret deleted".to_string()),
        ),
        Err(e) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Failed to delete secret: {e}")),
        ),
    }
}

// ---------------------------------------------------------------------------
// LLM usage
// ---------------------------------------------------------------------------

async fn handle_llm_usage_all() -> Response<BoxBody> {
    let store = crate::llm_intercept::LLM_USAGE.read().await;
    json_response(StatusCode::OK, &ApiResponse::success(store.all_usage()))
}

async fn handle_llm_usage_sandbox(sandbox: &str) -> Response<BoxBody> {
    let store = crate::llm_intercept::LLM_USAGE.read().await;
    let usage = store.usage_for_sandbox(sandbox);
    json_response(StatusCode::OK, &ApiResponse::success(usage))
}

// ---------------------------------------------------------------------------
// LLM key management
// ---------------------------------------------------------------------------

fn llm_keys_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".agentkernel")
        .join("llm_keys.json")
}

fn provider_to_domain(provider: &str) -> String {
    match provider {
        "openai" => "api.openai.com".to_string(),
        "anthropic" => "api.anthropic.com".to_string(),
        "google" | "gemini" => "generativelanguage.googleapis.com".to_string(),
        "deepseek" => "api.deepseek.com".to_string(),
        "groq" => "api.groq.com".to_string(),
        "mistral" => "api.mistral.ai".to_string(),
        "cohere" => "api.cohere.com".to_string(),
        "together" => "api.together.xyz".to_string(),
        "fireworks" => "api.fireworks.ai".to_string(),
        other => other.to_string(),
    }
}

async fn handle_llm_keys_list() -> Response<BoxBody> {
    let keys_path = llm_keys_path();
    let keys: std::collections::BTreeMap<String, String> = if keys_path.exists() {
        match std::fs::read_to_string(&keys_path).and_then(|s| {
            serde_json::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        }) {
            Ok(k) => k,
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
        }
    } else {
        std::collections::BTreeMap::new()
    };
    // Return domain -> vault_key_name (never expose actual secret values)
    json_response(StatusCode::OK, &ApiResponse::success(keys))
}

async fn handle_llm_keys_set(req: Request<Incoming>, provider: &str) -> Response<BoxBody> {
    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(e) => {
            return e;
        }
    };

    #[derive(Debug, serde::Deserialize)]
    struct SetKeyRequest {
        vault_key_name: String,
        #[serde(default)]
        value: Option<String>,
    }

    let parsed: SetKeyRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let domain = provider_to_domain(provider);

    // Store value in vault if provided
    if let Some(ref val) = parsed.value {
        let vault = crate::secrets::SecretVault::new(crate::secrets::SecretBackend::default());
        if let Err(e) = vault.set(&parsed.vault_key_name, val) {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    }

    // Update llm_keys.json mapping
    let keys_path = llm_keys_path();
    let mut keys: std::collections::BTreeMap<String, String> = if keys_path.exists() {
        std::fs::read_to_string(&keys_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        std::collections::BTreeMap::new()
    };
    keys.insert(domain.clone(), parsed.vault_key_name.clone());
    if let Err(e) = crate::secure_fs::write_private_json(&keys_path, &keys) {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    json_response(
        StatusCode::OK,
        &ApiResponse::success(serde_json::json!({
            "domain": domain,
            "vault_key_name": parsed.vault_key_name,
        })),
    )
}

async fn handle_llm_keys_remove(provider: &str) -> Response<BoxBody> {
    let domain = provider_to_domain(provider);
    let keys_path = llm_keys_path();

    let mut keys: std::collections::BTreeMap<String, String> = if keys_path.exists() {
        std::fs::read_to_string(&keys_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("No LLM keys configured"),
        );
    };

    if keys.remove(&domain).is_some() {
        if let Err(e) = crate::secure_fs::write_private_json(&keys_path, &keys) {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
        json_response(
            StatusCode::OK,
            &ApiResponse::success(serde_json::json!({"removed": domain})),
        )
    } else {
        json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("No LLM key mapping for {}", domain)),
        )
    }
}

// ---------------------------------------------------------------------------
// Proxy hooks management
// ---------------------------------------------------------------------------

async fn handle_list_proxy_hooks(_state: Arc<AppState>) -> Response<BoxBody> {
    use crate::proxy_hooks::ProxyHook;

    // Collect hooks from all running proxy handles (global registry)
    let handles = VmManager::proxy_handles_registry().read().await;
    let mut all_hooks: Vec<ProxyHook> = Vec::new();
    for handle in handles.values() {
        let registry = handle.hook_registry.read().await;
        all_hooks.extend(registry.list());
    }
    drop(handles);

    json_response(StatusCode::OK, &ApiResponse::success(all_hooks))
}

async fn handle_register_proxy_hook(
    req: Request<Incoming>,
    _state: Arc<AppState>,
) -> Response<BoxBody> {
    use crate::proxy_hooks::ProxyHook;

    let body: ProxyHook = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Register the hook in all running proxies (global registry)
    let handles = VmManager::proxy_handles_registry().read().await;
    let mut registered = 0;
    for handle in handles.values() {
        let mut registry = handle.hook_registry.write().await;
        if let Err(e) = registry.register(body.clone()) {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Invalid hook: {e}")),
            );
        }
        registered += 1;
    }
    drop(handles);

    json_response(
        StatusCode::OK,
        &ApiResponse::success(format!(
            "Hook '{}' registered in {} proxies",
            body.name, registered
        )),
    )
}

async fn handle_remove_proxy_hook(name: &str, _state: Arc<AppState>) -> Response<BoxBody> {
    let handles = VmManager::proxy_handles_registry().read().await;
    let mut removed = 0;
    for handle in handles.values() {
        let mut registry = handle.hook_registry.write().await;
        if registry.remove(name) {
            removed += 1;
        }
    }
    drop(handles);

    if removed > 0 {
        json_response(
            StatusCode::OK,
            &ApiResponse::success(format!("Hook '{}' removed from {} proxies", name, removed)),
        )
    } else {
        json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Hook '{}' not found", name)),
        )
    }
}

async fn handle_list_agents(_state: Arc<AppState>) -> Response<BoxBody> {
    #[derive(Serialize)]
    struct AgentInfo {
        name: String,
        display_name: String,
        enabled: bool,
        description: String,
    }

    let agent_defs: Vec<(&str, &str, &str, &str)> = vec![
        (
            "claude",
            "Claude Code",
            "claude",
            "Anthropic's AI coding agent",
        ),
        (
            "copilot",
            "Copilot CLI",
            "copilot",
            "GitHub's AI coding agent",
        ),
        ("gemini", "Gemini CLI", "gemini", "Google's AI coding agent"),
        ("codex", "Codex CLI", "codex", "OpenAI's AI coding agent"),
        (
            "opencode",
            "OpenCode",
            "opencode",
            "Open-source AI coding agent",
        ),
        ("amp", "Amp", "amp", "Sourcegraph's AI coding agent"),
        ("pi", "Pi", "pi", "Mario Zechner's coding agent"),
    ];

    let agents: Vec<AgentInfo> = agent_defs
        .into_iter()
        .map(|(name, display, bin, desc)| {
            let enabled = binary_exists_in_path(bin);
            AgentInfo {
                name: name.into(),
                display_name: display.into(),
                enabled,
                description: desc.into(),
            }
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(agents))
}

fn is_executable(candidate: &std::path::Path) -> bool {
    if !candidate.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = candidate.metadata() {
            return (metadata.permissions().mode() & 0o111) != 0;
        }
        false
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn binary_exists_in_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    #[cfg(windows)]
    let exts: Vec<std::ffi::OsString> = {
        let pathext = std::env::var_os("PATHEXT")
            .unwrap_or_else(|| std::ffi::OsString::from(".COM;.EXE;.BAT;.CMD"));
        pathext
            .to_string_lossy()
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|ext| {
                if ext.starts_with('.') {
                    std::ffi::OsString::from(ext)
                } else {
                    std::ffi::OsString::from(format!(".{}", ext))
                }
            })
            .collect()
    };

    std::env::split_paths(&path).any(|dir| {
        #[cfg(windows)]
        {
            let has_ext = std::path::Path::new(bin)
                .extension()
                .map(|ext| !ext.is_empty())
                .unwrap_or(false);

            if has_ext {
                return is_executable(&dir.join(bin));
            }

            for ext in &exts {
                let mut name = std::ffi::OsString::from(bin);
                name.push(ext);
                if is_executable(&dir.join(name)) {
                    return true;
                }
            }
            false
        }

        #[cfg(not(windows))]
        {
            is_executable(&dir.join(bin))
        }
    })
}

// ---------------------------------------------------------------------------
// Browser v2 handlers: persistent pages with ARIA snapshots
// ---------------------------------------------------------------------------

use crate::browser_scripts;

/// Helper: ensure the browser server is running in the sandbox, start if needed.
async fn ensure_browser_server(name: &str, state: &Arc<AppState>) -> Result<(), Response<BoxBody>> {
    let mut manager = state.get_manager().await.map_err(|e| {
        json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to create manager: {}", e)),
        )
    })?;

    // Health check
    let health_cmd = vec![
        "python3".to_string(),
        "-c".to_string(),
        browser_scripts::BROWSER_SERVER_HEALTH_CMD.to_string(),
        browser_scripts::BROWSER_SERVER_PORT.to_string(),
    ];
    if let Ok(output) = manager.exec_cmd(name, &health_cmd).await
        && (output.contains("\"status\":\"ok\"") || output.contains("\"status\": \"ok\""))
    {
        return Ok(());
    }

    // Start the server
    let start_cmd = vec![
        "python3".to_string(),
        "-c".to_string(),
        browser_scripts::BROWSER_SERVER_START_CMD.to_string(),
        browser_scripts::ARIA_SNAPSHOT_JS.to_string(),
        browser_scripts::BROWSER_SERVER_PORT.to_string(),
        browser_scripts::BROWSER_SERVER_SCRIPT.to_string(),
    ];
    match manager.exec_cmd(name, &start_cmd).await {
        Ok(output) => {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&output)
                && let Some(err) = data.get("error").and_then(|v| v.as_str())
            {
                return Err(json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!("Browser server failed to start: {}", err)),
                ));
            }
            Ok(())
        }
        Err(e) => Err(json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to start browser server: {}", e)),
        )),
    }
}

/// Helper: send a request to the in-sandbox browser server.
async fn browser_request(
    name: &str,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
    state: &Arc<AppState>,
) -> Response<BoxBody> {
    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to create manager: {}", e)),
            );
        }
    };
    let mut cmd = vec![
        "python3".to_string(),
        "-c".to_string(),
        browser_scripts::BROWSER_SERVER_REQUEST_CMD.to_string(),
        browser_scripts::BROWSER_SERVER_PORT.to_string(),
        method.to_string(),
        path.to_string(),
    ];
    if let Some(b) = body {
        cmd.push(serde_json::to_string(b).unwrap_or_default());
    }
    match manager.exec_cmd(name, &cmd).await {
        Ok(output) => {
            // Return raw JSON from the browser server
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(full(output))
                .unwrap()
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Browser request failed: {}", e)),
        ),
    }
}

async fn handle_browser_start(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    match ensure_browser_server(name, &state).await {
        Ok(()) => json_response(
            StatusCode::OK,
            &ApiResponse::success("Browser server started"),
        ),
        Err(resp) => resp,
    }
}

async fn handle_browser_list_pages(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    browser_request(name, "GET", "/pages", None, &state).await
}

async fn handle_browser_create_page(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    let body: serde_json::Value = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    browser_request(name, "POST", "/pages", Some(&body), &state).await
}

async fn handle_browser_close_page(
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    browser_request(name, "DELETE", &format!("/pages/{}", page), None, &state).await
}

async fn handle_browser_goto(
    req: Request<Incoming>,
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    let body: serde_json::Value = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    browser_request(
        name,
        "POST",
        &format!("/pages/{}/goto", page),
        Some(&body),
        &state,
    )
    .await
}

async fn handle_browser_snapshot(
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    browser_request(
        name,
        "GET",
        &format!("/pages/{}/snapshot", page),
        None,
        &state,
    )
    .await
}

async fn handle_browser_content(name: &str, page: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    browser_request(
        name,
        "GET",
        &format!("/pages/{}/content", page),
        None,
        &state,
    )
    .await
}

async fn handle_browser_click(
    req: Request<Incoming>,
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    let body: serde_json::Value = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    browser_request(
        name,
        "POST",
        &format!("/pages/{}/click", page),
        Some(&body),
        &state,
    )
    .await
}

async fn handle_browser_fill(
    req: Request<Incoming>,
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    let body: serde_json::Value = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    browser_request(
        name,
        "POST",
        &format!("/pages/{}/fill", page),
        Some(&body),
        &state,
    )
    .await
}

async fn handle_browser_screenshot(
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    browser_request(
        name,
        "POST",
        &format!("/pages/{}/screenshot", page),
        None,
        &state,
    )
    .await
}

async fn handle_browser_evaluate(
    req: Request<Incoming>,
    name: &str,
    page: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    let body: serde_json::Value = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    browser_request(
        name,
        "POST",
        &format!("/pages/{}/evaluate", page),
        Some(&body),
        &state,
    )
    .await
}

async fn handle_browser_events(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(resp) = ensure_browser_server(name, &state).await {
        return resp;
    }
    // Parse query params from URI
    let query = req.uri().query().unwrap_or("");
    let path = format!("/events?{}", query);
    browser_request(name, "GET", &path, None, &state).await
}

// ---------------------------------------------------------------------------
// Docker Image Management
// ---------------------------------------------------------------------------

async fn handle_list_images(_state: Arc<AppState>) -> Response<BoxBody> {
    let runtime = crate::docker_backend::detect_container_runtime();
    let cmd = match runtime {
        Some(r) => r.cmd(),
        None => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error("No container runtime available"),
            );
        }
    };

    let output = match tokio::process::Command::new(cmd)
        .args(["images", "--format", "{{json .}}"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to list images: {e}")),
            );
        }
    };

    #[derive(Deserialize, Serialize)]
    struct DockerImage {
        #[serde(alias = "ID")]
        id: String,
        #[serde(alias = "Repository")]
        repository: String,
        #[serde(alias = "Tag")]
        tag: String,
        #[serde(alias = "Size")]
        size: String,
        #[serde(alias = "CreatedAt", alias = "CreatedSince")]
        created: String,
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let images: Vec<DockerImage> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(images))
}

async fn handle_delete_image(id: &str, _state: Arc<AppState>) -> Response<BoxBody> {
    let runtime = crate::docker_backend::detect_container_runtime();
    let cmd = match runtime {
        Some(r) => r.cmd(),
        None => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error("No container runtime available"),
            );
        }
    };

    // Validate image ID to prevent command injection
    if !id.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == ':' || c == '.' || c == '-' || c == '/' || c == '_'
    }) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("Invalid image ID"),
        );
    }

    match tokio::process::Command::new(cmd)
        .args(["rmi", id])
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            json_response(StatusCode::OK, &ApiResponse::success("Image removed"))
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to remove image: {stderr}")),
            )
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to remove image: {e}")),
        ),
    }
}

// ---------------------------------------------------------------------------
// Hardware Benchmark
// ---------------------------------------------------------------------------

async fn handle_benchmark(state: Arc<AppState>) -> Response<BoxBody> {
    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let benchmark_name = format!(
        "benchmark-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0")
    );
    let image = "alpine:3.20";

    let started_at = chrono::Utc::now();

    // Phase 1: Create + Start (includes VM boot for Firecracker)
    let create_start = std::time::Instant::now();
    if let Err(e) = manager.create(&benchmark_name, image, 1, 256).await {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Benchmark create failed: {e}")),
        );
    }
    let perms = crate::permissions::SecurityProfile::default().permissions();
    if let Err(e) = manager
        .start_with_permissions(&benchmark_name, &perms)
        .await
    {
        let _ = manager.remove(&benchmark_name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Benchmark start failed: {e}")),
        );
    }
    let create_ms = create_start.elapsed().as_secs_f64() * 1000.0;

    // Phase 2: Exec
    let exec_start = std::time::Instant::now();
    let exec_cmd = vec!["echo".to_string(), "hello".to_string()];
    if let Err(e) = manager.exec_cmd(&benchmark_name, &exec_cmd).await {
        let _ = manager.remove(&benchmark_name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Benchmark exec failed: {e}")),
        );
    }
    let exec_ms = exec_start.elapsed().as_secs_f64() * 1000.0;

    // Phase 3: Destroy
    let destroy_start = std::time::Instant::now();
    let _ = manager.remove(&benchmark_name).await;
    let destroy_ms = destroy_start.elapsed().as_secs_f64() * 1000.0;

    let total_ms = create_ms + exec_ms + destroy_ms;
    let finished_at = chrono::Utc::now();

    #[derive(Serialize)]
    struct BenchmarkResult {
        create_ms: f64,
        exec_ms: f64,
        destroy_ms: f64,
        total_ms: f64,
        image: String,
        backend: String,
        started_at: String,
        finished_at: String,
        timestamp: String,
    }

    let backend = manager.backend().to_string();

    json_response(
        StatusCode::OK,
        &ApiResponse::success(BenchmarkResult {
            create_ms,
            exec_ms,
            destroy_ms,
            total_ms,
            image: image.to_string(),
            backend,
            started_at: started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            finished_at: finished_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }),
    )
}

// ---------------------------------------------------------------------------
// Session Recording (reads from audit log)
// ---------------------------------------------------------------------------

async fn handle_list_sandbox_sessions(state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    #[derive(Serialize)]
    struct SandboxSession {
        sandbox: String,
        entry_count: usize,
    }

    // List all sandboxes and report which ones exist (as proxy for session data)
    let sessions: Vec<SandboxSession> = manager
        .list()
        .into_iter()
        .map(|(name, _, _)| SandboxSession {
            sandbox: name.to_string(),
            entry_count: 0,
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(sessions))
}

async fn handle_get_sandbox_session(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if !manager.exists(name) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
        );
    }

    #[derive(Serialize)]
    struct SandboxSession {
        sandbox: String,
        entries: Vec<()>,
    }

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxSession {
            sandbox: name.to_string(),
            entries: vec![],
        }),
    )
}

// ---------------------------------------------------------------------------
// Sandbox Config Export/Import
// ---------------------------------------------------------------------------

async fn handle_export_sandbox_config(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandbox_state = match manager.get_state(name) {
        Some(s) => s,
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
            );
        }
    };

    match toml::to_string_pretty(sandbox_state) {
        Ok(config) => json_response(StatusCode::OK, &ApiResponse::success(config)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to serialize config: {e}")),
        ),
    }
}

async fn handle_import_sandbox_config(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[derive(Deserialize)]
    struct ImportRequest {
        config: String,
    }

    let body: ImportRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Parse the TOML config to extract sandbox parameters
    let parsed: crate::vmm::SandboxState = match toml::from_str(&body.config) {
        Ok(s) => s,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Invalid TOML config: {e}")),
            );
        }
    };

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    // Create a new sandbox with the imported config
    if let Err(e) = manager
        .create(name, &parsed.image, parsed.vcpus, parsed.memory_mb)
        .await
    {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to create sandbox from config: {e}")),
        );
    }

    // Return info about the newly created sandbox
    let running = manager.is_running(name);
    let ip = if running {
        manager.get_container_ip(name)
    } else {
        None
    };

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxInfo {
            name: name.to_string(),
            uuid: manager
                .get_state(name)
                .map(|s| s.uuid.clone())
                .unwrap_or_default(),
            status: if running {
                "running".to_string()
            } else {
                "stopped".to_string()
            },
            backend: manager
                .get_state(name)
                .and_then(|s| s.backend)
                .map(|b| format!("{}", b))
                .unwrap_or_else(|| "unknown".to_string()),
            ip,
            image: Some(parsed.image),
            vcpus: Some(parsed.vcpus),
            memory_mb: Some(parsed.memory_mb),
            created_at: manager.get_state(name).map(|s| s.created_at.clone()),
            created_from_template: None,
            template_help_text: None,
            ports: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        }),
    )
}

// -----------------------------------------------------------------
// Interactive Permissions
// -----------------------------------------------------------------

async fn handle_list_permissions() -> Response<BoxBody> {
    let store = crate::mcp::default_permission_store();
    let grants = store.list();
    json_response(StatusCode::OK, &ApiResponse::success(grants))
}

async fn handle_grant_permission(req: Request<Incoming>) -> Response<BoxBody> {
    use crate::interactive_permissions::{GrantScope, PermissionKind};

    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    #[derive(Deserialize)]
    struct GrantRequest {
        kind: String,
        scope: Option<String>,
        sandbox: Option<String>,
    }

    let parsed: GrantRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let kind = match PermissionKind::from_str(&parsed.kind) {
        Some(k) => k,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Unknown permission kind: {}", parsed.kind)),
            );
        }
    };

    let scope = match parsed.scope.as_deref() {
        Some("session") => GrantScope::Session,
        Some("always") => GrantScope::Always,
        _ => GrantScope::Once,
    };

    let store = crate::mcp::default_permission_store();
    let grant_id = store.grant(kind, scope, parsed.sandbox, "http_user");

    json_response(
        StatusCode::OK,
        &ApiResponse::success(serde_json::json!({
            "grant_id": grant_id,
            "kind": parsed.kind,
        })),
    )
}

async fn handle_revoke_permission(id: &str) -> Response<BoxBody> {
    let store = crate::mcp::default_permission_store();
    if store.revoke(id) {
        json_response(StatusCode::OK, &ApiResponse::success("Permission revoked"))
    } else {
        json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Grant '{id}' not found")),
        )
    }
}

async fn handle_check_permission(req: Request<Incoming>) -> Response<BoxBody> {
    use crate::interactive_permissions::PermissionKind;

    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    #[derive(Deserialize)]
    struct CheckRequest {
        kind: String,
        sandbox: Option<String>,
    }

    let parsed: CheckRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let kind = match PermissionKind::from_str(&parsed.kind) {
        Some(k) => k,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Unknown permission kind: {}", parsed.kind)),
            );
        }
    };

    let store = crate::mcp::default_permission_store();
    let permitted = store.check(kind, parsed.sandbox.as_deref());

    json_response(
        StatusCode::OK,
        &ApiResponse::success(serde_json::json!({
            "permitted": permitted,
            "kind": parsed.kind,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_storage::DurableStorage;
    use std::sync::Arc;

    // === ApiResponse tests ===

    #[test]
    fn test_api_response_success() {
        let response = ApiResponse::success("test data");
        assert!(response.success);
        assert_eq!(response.data, Some("test data"));
        assert!(response.error.is_none());
    }

    #[test]
    fn test_api_response_error() {
        let response = ApiResponse::<()>::error("test error");
        assert!(!response.success);
        assert!(response.data.is_none());
        assert_eq!(response.error, Some("test error".to_string()));
    }

    #[test]
    fn test_api_response_success_serialization() {
        let response = ApiResponse::success("data");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"data\":\"data\""));
        assert!(!json.contains("\"error\"")); // error is skipped when None
    }

    #[test]
    fn test_api_response_error_serialization() {
        let response = ApiResponse::<()>::error("failed");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":false"));
        assert!(!json.contains("\"data\"")); // data is skipped when None
        assert!(json.contains("\"error\":\"failed\""));
    }

    // === Request deserialization tests ===

    #[test]
    fn test_run_request_deserialize() {
        let json = r#"{"command": ["echo", "hello"], "image": "alpine:3.20"}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["echo", "hello"]);
        assert_eq!(req.image, Some("alpine:3.20".to_string()));
        assert!(req.fast); // default is true
    }

    #[test]
    fn test_run_request_deserialize_minimal() {
        let json = r#"{"command": ["ls"]}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["ls"]);
        assert!(req.image.is_none());
        assert!(req.profile.is_none());
        assert!(req.fast);
    }

    #[test]
    fn test_run_request_deserialize_fast_false() {
        let json = r#"{"command": ["ls"], "fast": false}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert!(!req.fast);
    }

    #[test]
    fn test_create_request_deserialize() {
        let json = r#"{"name": "my-sandbox", "image": "python:3.12"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "my-sandbox");
        assert_eq!(req.image, Some("python:3.12".to_string()));
    }

    #[test]
    fn test_create_request_deserialize_minimal() {
        let json = r#"{"name": "my-sandbox"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "my-sandbox");
        assert!(req.image.is_none());
    }

    #[test]
    fn test_exec_request_deserialize() {
        let json = r#"{"command": ["npm", "test"]}"#;
        let req: ExecRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["npm", "test"]);
    }

    // === SandboxInfo tests ===

    #[test]
    fn test_sandbox_info_serialize() {
        let info = SandboxInfo {
            name: "test-sandbox".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            status: "running".to_string(),
            backend: "docker".to_string(),
            ip: None,
            image: None,
            vcpus: None,
            memory_mb: None,
            created_at: None,
            created_from_template: None,
            template_help_text: None,
            ports: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"test-sandbox\""));
        assert!(json.contains("\"uuid\":"));
        assert!(json.contains("\"status\":\"running\""));
    }

    // === RunResponse tests ===

    #[test]
    fn test_run_response_serialize() {
        let response = RunResponse {
            output: "hello world".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"output\":\"hello world\""));
    }

    // === AppState tests ===

    #[test]
    fn test_app_state_with_api_key() {
        let state = AppState::with_api_keys(vec!["secret123".to_string()]);
        assert_eq!(state.api_keys, vec!["secret123".to_string()]);
    }

    #[test]
    fn test_app_state_without_api_key() {
        let state = AppState::with_api_keys(vec![]);
        assert!(state.api_keys.is_empty());
    }

    // === default_fast tests ===

    #[test]
    fn test_default_fast_returns_true() {
        assert!(default_fast());
    }

    // === json_response tests ===

    #[test]
    fn test_json_response_ok() {
        let response = json_response(StatusCode::OK, &ApiResponse::success("data"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_json_response_not_found() {
        let response = json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("not found"),
        );
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_json_response_created() {
        let info = SandboxInfo {
            name: "test".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            status: "running".to_string(),
            backend: "docker".to_string(),
            ip: None,
            image: None,
            vcpus: None,
            memory_mb: None,
            created_at: None,
            created_from_template: None,
            template_help_text: None,
            ports: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        };
        let response = json_response(StatusCode::CREATED, &ApiResponse::success(info));
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // === Path parsing tests (unit test the segment logic) ===

    #[test]
    fn test_path_segments_parsing() {
        let path = "/sandboxes/my-sandbox/exec";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes", "my-sandbox", "exec"]);
    }

    #[test]
    fn test_path_segments_health() {
        let path = "/health";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["health"]);
    }

    #[test]
    fn test_path_segments_run() {
        let path = "/run";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["run"]);
    }

    #[test]
    fn test_path_segments_sandboxes() {
        let path = "/sandboxes";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes"]);
    }

    #[test]
    fn test_path_segments_sandbox_by_name() {
        let path = "/sandboxes/test-123";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes", "test-123"]);
    }

    #[test]
    fn test_path_segments_sandbox_by_uuid() {
        let path = "/sandboxes/by-uuid/019abc12-1234-7def-89ab-0123456789ab";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(
            segments,
            vec![
                "sandboxes",
                "by-uuid",
                "019abc12-1234-7def-89ab-0123456789ab"
            ]
        );
    }

    #[test]
    fn test_path_segments_orchestration_events() {
        let path = "/orchestrations/orch-1/events";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["orchestrations", "orch-1", "events"]);
    }

    #[test]
    fn test_path_segments_orchestration_terminate() {
        let path = "/orchestrations/orch-1/terminate";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["orchestrations", "orch-1", "terminate"]);
    }

    #[test]
    fn test_path_segments_orchestration_definitions() {
        let path = "/orchestrations/definitions";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["orchestrations", "definitions"]);
    }

    #[test]
    fn test_path_segments_orchestration_definition_by_name() {
        let path = "/orchestrations/definitions/deploy-pipeline";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(
            segments,
            vec!["orchestrations", "definitions", "deploy-pipeline"]
        );
    }

    #[test]
    fn test_path_segments_stores() {
        let path = "/stores";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["stores"]);
    }

    #[test]
    fn test_path_segments_store_id() {
        let path = "/stores/store-1";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["stores", "store-1"]);
    }

    #[test]
    fn test_path_segments_store_query() {
        let path = "/stores/store-1/query";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["stores", "store-1", "query"]);
    }

    #[test]
    fn test_path_segments_store_command() {
        let path = "/stores/store-1/command";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["stores", "store-1", "command"]);
    }

    #[tokio::test]
    async fn test_runtime_tick_auto_completes_orchestration() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(OrchestrationStore::new(
            DurableStorage::new(temp.path().join("durable.db")).unwrap(),
        ));

        let created = store
            .create(CreateOrchestration {
                name: "auto-complete".to_string(),
                input: Some(serde_json::json!({"hello": "world"})),
            })
            .unwrap();

        process_orchestrations_tick(store.clone()).await.unwrap();

        let updated = store.get(&created.id).unwrap().unwrap();
        assert_eq!(updated.status, OrchestrationStatus::Completed);
        assert_eq!(updated.output, Some(serde_json::json!({"hello": "world"})));

        let history = store.list_events(&created.id, 50, 0).unwrap();
        assert!(
            history
                .iter()
                .any(|event| event.event_type == "OrchestratorCompleted")
        );
    }

    #[tokio::test]
    async fn test_runtime_tick_wait_for_event() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(OrchestrationStore::new(
            DurableStorage::new(temp.path().join("durable.db")).unwrap(),
        ));

        let created = store
            .create(CreateOrchestration {
                name: "wait-for-event".to_string(),
                input: Some(serde_json::json!({"wait_for_event": "approval"})),
            })
            .unwrap();

        process_orchestrations_tick(store.clone()).await.unwrap();
        let running = store.get(&created.id).unwrap().unwrap();
        assert_eq!(running.status, OrchestrationStatus::Running);

        store
            .append_event(
                &created.id,
                "EventRaised",
                serde_json::json!({
                    "name": "approval",
                    "data": {"approved": true}
                }),
            )
            .unwrap();

        process_orchestrations_tick(store.clone()).await.unwrap();

        let completed = store.get(&created.id).unwrap().unwrap();
        assert_eq!(completed.status, OrchestrationStatus::Completed);
        assert_eq!(
            completed.output,
            Some(serde_json::json!({"approved": true}))
        );

        let history = store.list_events(&created.id, 100, 0).unwrap();
        assert!(
            history
                .iter()
                .any(|event| event.event_type == "EventConsumed")
        );
    }

    #[tokio::test]
    async fn test_runtime_tick_uses_definition_wait_for_event() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(OrchestrationStore::new(
            DurableStorage::new(temp.path().join("durable.db")).unwrap(),
        ));

        store
            .upsert_definition(
                "approval-flow",
                serde_json::json!({
                    "name": "approval-flow",
                    "wait_for_event": "approval"
                }),
            )
            .unwrap();

        let created = store
            .create(CreateOrchestration {
                name: "approval-flow".to_string(),
                input: None,
            })
            .unwrap();

        process_orchestrations_tick(store.clone()).await.unwrap();
        let running = store.get(&created.id).unwrap().unwrap();
        assert_eq!(running.status, OrchestrationStatus::Running);

        store
            .append_event(
                &created.id,
                "EventRaised",
                serde_json::json!({
                    "name": "approval",
                    "data": {"approved": true}
                }),
            )
            .unwrap();

        process_orchestrations_tick(store.clone()).await.unwrap();
        let completed = store.get(&created.id).unwrap().unwrap();
        assert_eq!(completed.status, OrchestrationStatus::Completed);
        assert_eq!(
            completed.output,
            Some(serde_json::json!({"approved": true}))
        );
    }

    #[test]
    fn test_compute_idempotency_key_is_stable() {
        let first = compute_idempotency_key("orch-1", "run-tests", 7);
        let second = compute_idempotency_key("orch-1", "run-tests", 7);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn test_compute_retry_delay_backoff() {
        let policy = RuntimeRetryPolicy {
            max_attempts: 3,
            initial_interval_ms: 1000,
            backoff_coefficient: 2.0,
            max_interval_ms: 30_000,
            non_retryable_errors: vec![],
        };
        assert_eq!(compute_retry_delay_ms(&policy, 1), 1000);
        assert_eq!(compute_retry_delay_ms(&policy, 2), 2000);
        assert_eq!(compute_retry_delay_ms(&policy, 3), 4000);
    }

    #[test]
    fn test_non_retryable_error_match() {
        let policy = RuntimeRetryPolicy {
            max_attempts: 3,
            initial_interval_ms: 1000,
            backoff_coefficient: 2.0,
            max_interval_ms: 30_000,
            non_retryable_errors: vec!["PermissionDenied".to_string()],
        };
        assert!(!is_retryable_error("PermissionDenied: blocked", &policy));
        assert!(is_retryable_error("Temporary network issue", &policy));
    }

    // === Extended CreateRequest tests ===

    #[test]
    fn test_create_request_with_resources() {
        let json = r#"{"name": "big-sandbox", "vcpus": 4, "memory_mb": 2048}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "big-sandbox");
        assert_eq!(req.vcpus, Some(4));
        assert_eq!(req.memory_mb, Some(2048));
        assert!(req.image.is_none());
        assert!(req.profile.is_none());
    }

    #[test]
    fn test_create_request_with_profile() {
        let json = r#"{"name": "secure", "profile": "restrictive"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "secure");
        assert_eq!(req.profile, Some("restrictive".to_string()));
        assert!(req.vcpus.is_none());
        assert!(req.memory_mb.is_none());
    }

    #[test]
    fn test_create_request_full() {
        let json = r#"{
            "name": "full-sandbox",
            "image": "python:3.12",
            "vcpus": 2,
            "memory_mb": 1024,
            "profile": "moderate"
        }"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "full-sandbox");
        assert_eq!(req.image, Some("python:3.12".to_string()));
        assert_eq!(req.vcpus, Some(2));
        assert_eq!(req.memory_mb, Some(1024));
        assert_eq!(req.profile, Some("moderate".to_string()));
    }

    // === SandboxInfo extended serialization tests ===

    #[test]
    fn test_sandbox_info_with_resources() {
        let info = SandboxInfo {
            name: "big".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            status: "running".to_string(),
            backend: "docker".to_string(),
            ip: None,
            image: Some("python:3.12".to_string()),
            vcpus: Some(4),
            memory_mb: Some(2048),
            created_at: Some("2026-01-30T12:00:00Z".to_string()),
            created_from_template: None,
            template_help_text: None,
            ports: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"image\":\"python:3.12\""));
        assert!(json.contains("\"vcpus\":4"));
        assert!(json.contains("\"memory_mb\":2048"));
        assert!(json.contains("\"created_at\":\"2026-01-30T12:00:00Z\""));
    }

    #[test]
    fn test_sandbox_info_skips_none_fields() {
        let info = SandboxInfo {
            name: "test".to_string(),
            uuid: uuid::Uuid::now_v7().to_string(),
            status: "stopped".to_string(),
            backend: "docker".to_string(),
            ip: None,
            image: None,
            vcpus: None,
            memory_mb: None,
            created_at: None,
            created_from_template: None,
            template_help_text: None,
            ports: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("image"));
        assert!(!json.contains("vcpus"));
        assert!(!json.contains("memory_mb"));
        assert!(!json.contains("created_at"));
    }

    // === FileWriteRequest tests ===

    #[test]
    fn test_file_write_request_utf8() {
        let json = r#"{"content": "hello world"}"#;
        let req: FileWriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "hello world");
        assert_eq!(req.encoding, "utf8"); // default
    }

    #[test]
    fn test_file_write_request_base64() {
        let json = r#"{"content": "aGVsbG8=", "encoding": "base64"}"#;
        let req: FileWriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "aGVsbG8=");
        assert_eq!(req.encoding, "base64");
    }

    // === FileReadResponse tests ===

    #[test]
    fn test_file_read_response_serialize() {
        let resp = FileReadResponse {
            content: "file contents".to_string(),
            encoding: "utf8".to_string(),
            size: 13,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"content\":\"file contents\""));
        assert!(json.contains("\"encoding\":\"utf8\""));
        assert!(json.contains("\"size\":13"));
    }

    // === BatchRunRequest tests ===

    #[test]
    fn test_batch_run_request_deserialize() {
        let json = r#"{
            "commands": [
                {"command": ["echo", "a"]},
                {"command": ["echo", "b"]}
            ]
        }"#;
        let req: BatchRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.commands.len(), 2);
        assert_eq!(req.commands[0].command, vec!["echo", "a"]);
        assert_eq!(req.commands[1].command, vec!["echo", "b"]);
    }

    #[test]
    fn test_batch_run_request_single_command() {
        let json = r#"{"commands": [{"command": ["ls", "-la"]}]}"#;
        let req: BatchRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.commands.len(), 1);
        assert_eq!(req.commands[0].command, vec!["ls", "-la"]);
    }

    // === BatchRunResponse tests ===

    #[test]
    fn test_batch_run_response_serialize() {
        let resp = BatchRunResponse {
            results: vec![
                BatchResult {
                    output: Some("hello".to_string()),
                    error: None,
                },
                BatchResult {
                    output: None,
                    error: Some("failed".to_string()),
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"output\":\"hello\""));
        assert!(json.contains("\"error\":\"failed\""));
    }

    // === resolve_profile tests ===

    #[test]
    fn test_resolve_profile_permissive() {
        let profile = resolve_profile("permissive");
        assert!(profile.is_some());
    }

    #[test]
    fn test_resolve_profile_moderate() {
        let profile = resolve_profile("moderate");
        assert!(profile.is_some());
    }

    #[test]
    fn test_resolve_profile_restrictive() {
        let profile = resolve_profile("restrictive");
        assert!(profile.is_some());
    }

    #[test]
    fn test_resolve_profile_unknown() {
        let profile = resolve_profile("nonexistent");
        assert!(profile.is_none());
    }

    // === File path segment extraction tests ===

    #[test]
    fn test_path_segments_file_simple() {
        let path = "/sandboxes/my-box/files/tmp/hello.txt";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(
            segments,
            vec!["sandboxes", "my-box", "files", "tmp", "hello.txt"]
        );
        let file_path = segments[3..].join("/");
        assert_eq!(file_path, "tmp/hello.txt");
    }

    #[test]
    fn test_path_segments_file_nested() {
        let path = "/sandboxes/dev/files/home/user/projects/src/main.rs";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let file_path = segments[3..].join("/");
        assert_eq!(file_path, "home/user/projects/src/main.rs");
    }

    #[test]
    fn test_path_segments_batch_run() {
        let path = "/batch/run";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["batch", "run"]);
    }

    #[test]
    fn test_path_segments_sandbox_logs() {
        let path = "/sandboxes/my-sandbox/logs";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes", "my-sandbox", "logs"]);
    }

    // === default_encoding tests ===

    #[test]
    fn test_default_encoding_returns_utf8() {
        assert_eq!(default_encoding(), "utf8");
    }
}
