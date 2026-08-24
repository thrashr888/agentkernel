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

use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{Duration, sleep};

use crate::asciicast::{self, AsciicastEvent, AsciicastHeader, EventType};
use crate::backend::{
    BackendCapabilities, BackendType, FileInjection, backend_capabilities, backend_readiness,
    detect_best_backend,
};
use crate::job_scheduler::{JobScheduleStatus, JobScheduler, JobSchedulerHandle};
use crate::languages;
use crate::opencode::OpenCodeState;
use crate::orchestration_store::{
    CreateDurableObject, CreateDurableStore, CreateOrchestration, CreateSchedule,
    DurableObjectRecord, DurableStoreCommandResult, DurableStoreExecuteResult, DurableStoreKind,
    DurableStoreQueryResult, OrchestrationEvent, OrchestrationRecord, OrchestrationStatus,
    OrchestrationStore, ScheduleRecord, UpdateOrchestration,
};
use crate::permissions::{Permissions, SecurityProfile};
use crate::secrets::{SecretBackend, SecretVault};
use crate::task_worker::TaskWorker;
use crate::task_worker_vmm::VmTaskExecutor;
use crate::tasks::{CancelOutcome, TaskManager};
use crate::validation;
use crate::vmm::VmManager;
use crate::volume::{VolumeManager, VolumeMount};

pub type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;
const MAX_HTTP_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

pub(crate) fn full<T: Into<bytes::Bytes>>(chunk: T) -> BoxBody {
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

#[allow(clippy::result_large_err)]
pub(crate) async fn read_body_bytes(
    req: Request<Incoming>,
) -> Result<bytes::Bytes, Response<BoxBody>> {
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

fn load_api_key_from_config(config_path: Option<&std::path::Path>) -> Option<String> {
    let config_path = config_path.map(std::path::Path::to_path_buf).or_else(|| {
        let fallback = std::path::PathBuf::from("agentkernel.toml");
        fallback.exists().then_some(fallback)
    })?;
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

fn load_api_key(config_path: Option<&std::path::Path>) -> Option<String> {
    if let Ok(key) = std::env::var("AGENTKERNEL_API_KEY")
        && !key.trim().is_empty()
    {
        return Some(key);
    }
    load_api_key_from_config(config_path)
}

fn load_api_allow_sudo_exec_from_config(config_path: Option<&std::path::Path>) -> bool {
    let Some(config_path) = config_path.map(std::path::Path::to_path_buf).or_else(|| {
        let fallback = std::path::PathBuf::from("agentkernel.toml");
        fallback.exists().then_some(fallback)
    }) else {
        return false;
    };
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

/// Resolve the single tenant served by this agentkernel instance.
///
/// SCIM base URLs are normally provisioned per IdP/tenant.  We use the
/// enterprise organization identifier when present, with an explicit env var
/// override for installations that do not enable the enterprise policy
/// engine.  The value is never accepted from a request, which prevents an
/// authenticated caller from selecting another tenant by changing the URL.
fn load_scim_tenant_id(config_path: Option<&std::path::Path>) -> String {
    if let Ok(value) = std::env::var("AGENTKERNEL_SCIM_TENANT_ID") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    let config_path = config_path
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("agentkernel.toml"));
    if let Ok(content) = std::fs::read_to_string(config_path)
        && let Ok(parsed) = toml::from_str::<toml::Value>(&content)
        && let Some(value) = parsed
            .get("enterprise")
            .and_then(|enterprise| enterprise.get("org_id"))
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        return value.to_string();
    }
    "default".to_string()
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
    /// Backend to use for this sandbox. Omit or use `automatic` to select the
    /// server default.
    #[serde(default)]
    backend: Option<String>,
    image: Option<String>,
    vcpus: Option<u32>,
    memory_mb: Option<u64>,
    profile: Option<String>,
    /// Port mappings (e.g., ["8080:80", "3000", "5353:53/udp"])
    #[serde(default)]
    ports: Vec<String>,
    /// Optional Docker/Podman-managed bridge configuration.
    #[serde(default)]
    network: Option<crate::backend::ManagedNetworkConfig>,
    /// Persistent volume mounts (e.g., ["my-data:/data", "cache:/cache:ro"])
    #[serde(default)]
    volumes: Vec<String>,
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

/// Optional request body for starting a sandbox.
///
/// An omitted or empty body preserves the historical API behavior: moderate
/// permissions and no startup file injections. The CLI may select a private
/// host-side manifest when it delegates ownership of a Firecracker VM to the
/// long-running server. Callers never supply capability values in this body.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartSandboxRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configuration: Option<PersistedStartReference>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedStartReference {
    source: StartConfigurationSource,
    token: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StartConfigurationSource {
    Persisted,
}

/// Private on-disk start configuration written by the local CLI.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedStartConfiguration {
    sandbox_name: String,
    sandbox_uuid: String,
    sandbox_state_sha256: String,
    tenant_id: Option<String>,
    owner_user_id: Option<String>,
    owner_org_id: Option<String>,
    request_owner_id: String,
    expires_at: String,
    permissions: Permissions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<StartFileInjectionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersistedStartBinding {
    sandbox_name: String,
    sandbox_uuid: String,
    tenant_id: Option<String>,
    owner_user_id: Option<String>,
    owner_org_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StartFileInjectionRequest {
    dest: String,
    content_base64: String,
}

impl StartSandboxRequest {
    fn into_runtime(
        self,
        data_dir: &Path,
        binding: &PersistedStartBinding,
        request_owner_id: &str,
    ) -> Result<(Permissions, Vec<FileInjection>, Option<String>)> {
        match self.configuration {
            None => Ok((Permissions::default(), Vec::new(), None)),
            Some(reference) => {
                let configuration = take_persisted_start_configuration(data_dir, &reference)?;
                configuration.validate_binding(binding, request_owner_id)?;
                let state_sha256 = configuration.sandbox_state_sha256.clone();
                let (permissions, files) = configuration.into_runtime()?;
                Ok((permissions, files, Some(state_sha256)))
            }
        }
    }
}

impl PersistedStartConfiguration {
    fn from_runtime(
        sandbox: &crate::vmm::SandboxState,
        request_owner_id: String,
        permissions: &Permissions,
        files: &[FileInjection],
    ) -> Result<Self> {
        Ok(Self {
            sandbox_name: sandbox.name.clone(),
            sandbox_uuid: sandbox.uuid.clone(),
            sandbox_state_sha256: sandbox_state_sha256(sandbox)?,
            tenant_id: sandbox.tenant_id.clone(),
            owner_user_id: sandbox.owner_user_id.clone(),
            owner_org_id: sandbox.owner_org_id.clone(),
            request_owner_id,
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            permissions: permissions.clone(),
            files: files
                .iter()
                .map(|file| StartFileInjectionRequest {
                    dest: file.dest.clone(),
                    content_base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &file.content,
                    ),
                })
                .collect(),
        })
    }

    fn validate_binding(
        &self,
        binding: &PersistedStartBinding,
        request_owner_id: &str,
    ) -> Result<()> {
        let expires_at = chrono::DateTime::parse_from_rfc3339(&self.expires_at)
            .context("persisted start configuration has an invalid expiration")?;
        if chrono::Utc::now() > expires_at {
            anyhow::bail!("persisted start configuration has expired");
        }
        if self.sandbox_name != binding.sandbox_name
            || self.sandbox_uuid != binding.sandbox_uuid
            || self.tenant_id != binding.tenant_id
            || self.owner_user_id != binding.owner_user_id
            || self.owner_org_id != binding.owner_org_id
            || self.request_owner_id != request_owner_id
        {
            anyhow::bail!(
                "persisted start configuration does not belong to this sandbox generation and owner"
            );
        }
        Ok(())
    }

    fn into_runtime(self) -> Result<(Permissions, Vec<FileInjection>)> {
        let files = self
            .files
            .into_iter()
            .map(|file| {
                let content = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &file.content_base64,
                )
                .with_context(|| {
                    format!("invalid base64 content for startup file '{}'", file.dest)
                })?;
                Ok(FileInjection {
                    content,
                    dest: file.dest,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((self.permissions, files))
    }
}

fn sandbox_state_sha256(sandbox: &crate::vmm::SandboxState) -> Result<String> {
    let value = serde_json::to_value(sandbox).context("failed to encode sandbox state")?;
    let mut encoded = Vec::new();
    write_canonical_json(&value, &mut encoded)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn write_canonical_json(value: &serde_json::Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        serde_json::Value::Null => output.extend_from_slice(b"null"),
        serde_json::Value::Bool(value) => {
            output.extend_from_slice(if *value { b"true" } else { b"false" })
        }
        serde_json::Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        serde_json::Value::String(value) => serde_json::to_writer(output, value)?,
        serde_json::Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        serde_json::Value::Object(values) => {
            output.push(b'{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                write_canonical_json(&values[key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

impl PersistedStartBinding {
    fn from_state(sandbox: &crate::vmm::SandboxState) -> Self {
        Self {
            sandbox_name: sandbox.name.clone(),
            sandbox_uuid: sandbox.uuid.clone(),
            tenant_id: sandbox.tenant_id.clone(),
            owner_user_id: sandbox.owner_user_id.clone(),
            owner_org_id: sandbox.owner_org_id.clone(),
        }
    }
}

fn persisted_start_configuration_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("delegated-start")
}

fn validate_persisted_start_token(token: &str) -> Result<()> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        anyhow::bail!("invalid persisted start configuration token");
    }
    Ok(())
}

fn persisted_start_configuration_path(data_dir: &Path, token: &str) -> Result<PathBuf> {
    validate_persisted_start_token(token)?;
    Ok(persisted_start_configuration_dir(data_dir).join(format!("{token}.json")))
}

pub(crate) fn persist_start_configuration(
    data_dir: &Path,
    sandbox: &crate::vmm::SandboxState,
    permissions: &Permissions,
    files: &[FileInjection],
) -> Result<StartSandboxRequest> {
    validation::validate_sandbox_name(&sandbox.name)?;
    // A newer resolved configuration supersedes every unused token for this
    // sandbox, so tightened settings cannot be bypassed with a stale request.
    remove_persisted_start_configurations_for_sandbox(data_dir, &sandbox.name)?;
    let token = hex::encode(rand::random::<[u8; 32]>());
    let path = persisted_start_configuration_path(data_dir, &token)?;
    crate::secure_fs::write_private_json(
        &path,
        &PersistedStartConfiguration::from_runtime(
            sandbox,
            local_start_request_owner_id(),
            permissions,
            files,
        )?,
    )
    .with_context(|| {
        format!(
            "failed to persist start configuration for sandbox '{}'",
            sandbox.name
        )
    })?;
    Ok(StartSandboxRequest {
        configuration: Some(PersistedStartReference {
            source: StartConfigurationSource::Persisted,
            token,
        }),
    })
}

pub(crate) fn discard_persisted_start_configuration(
    data_dir: &Path,
    request: &StartSandboxRequest,
) -> Result<()> {
    let Some(reference) = request.configuration.as_ref() else {
        return Ok(());
    };
    let path = persisted_start_configuration_path(data_dir, &reference.token)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed to discard persisted start configuration"),
    }
}

fn take_persisted_start_configuration(
    data_dir: &Path,
    reference: &PersistedStartReference,
) -> Result<PersistedStartConfiguration> {
    let StartConfigurationSource::Persisted = reference.source;
    let path = persisted_start_configuration_path(data_dir, &reference.token)?;
    let consuming = persisted_start_configuration_dir(data_dir)
        .join(format!(".consuming-{}", uuid::Uuid::now_v7()));
    std::fs::rename(&path, &consuming)
        .context("persisted start configuration is unavailable or already consumed")?;
    let bytes = std::fs::read(&consuming)
        .context("failed to read persisted start configuration after claiming it");
    let removed = std::fs::remove_file(&consuming)
        .context("failed to delete consumed persisted start configuration");
    let bytes = bytes?;
    removed?;
    serde_json::from_slice(&bytes).context("persisted start configuration is invalid")
}

fn remove_persisted_start_configurations_for_sandbox(data_dir: &Path, name: &str) -> Result<()> {
    let directory = persisted_start_configuration_dir(data_dir);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).context("failed to inspect persisted start configurations");
        }
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json")
            && !file_name.starts_with(".consuming-")
        {
            continue;
        }
        let matches = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistedStartConfiguration>(&bytes).ok())
            .is_some_and(|configuration| configuration.sandbox_name == name);
        if matches {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .context("failed to remove persisted start configuration for sandbox");
                }
            }
        }
    }
    Ok(())
}

fn local_start_request_owner_id() -> String {
    std::env::var("AGENTKERNEL_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .map(|key| api_key_owner_id(&key))
        .unwrap_or_else(|| "anonymous".to_string())
}

fn start_request_owner_id(req: &Request<Incoming>) -> String {
    req.headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
        .map(api_key_owner_id)
        .unwrap_or_else(|| "anonymous".to_string())
}

fn parse_start_sandbox_request(body: &[u8]) -> Result<StartSandboxRequest> {
    if body.is_empty() {
        return Ok(StartSandboxRequest::default());
    }
    serde_json::from_slice(body).context("invalid start request JSON")
}

/// Request to fork a paused full-state sandbox.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkSandboxRequest {
    /// Name for the forked sandbox.
    as_name: String,
}

/// Parse and validate the persistent volume mounts accepted by sandbox create.
///
/// Keep this in the HTTP layer so malformed specs are rejected before a
/// sandbox is created.  The existence check intentionally uses the same
/// `VolumeManager` path as the CLI and the VMM start path.
fn validate_volume_specs(
    specs: &[String],
    volume_base_dir: Option<&std::path::Path>,
) -> Result<()> {
    let mounts: Vec<VolumeMount> = specs
        .iter()
        .map(|spec| VolumeMount::parse(spec))
        .collect::<Result<Vec<_>>>()?;
    if !mounts.is_empty() {
        let manager = match volume_base_dir {
            Some(base_dir) => VolumeManager::new_in(base_dir)?,
            None => VolumeManager::new()?,
        };
        manager.validate_mounts(&mounts)?;
    }
    Ok(())
}

fn validate_backend_volume_support(backend: BackendType, specs: &[String]) -> Result<()> {
    if !specs.is_empty() && !backend_capabilities(backend).host_volumes {
        return Err(anyhow::anyhow!(
            "Backend '{}' does not support host volume mounts",
            backend
        ));
    }
    Ok(())
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

/// Path and optional remote parameters used by the sandbox Git API.
///
/// Paths are interpreted inside the sandbox.  Relative paths follow Daytona's
/// convention and are resolved below `/workspace`; absolute paths are passed
/// through after the same sandbox-path checks used by file and exec APIs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitRepoRequest {
    path: String,
    #[serde(default)]
    remote: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    set_upstream: bool,
    // Kept for SDK compatibility.  Credentials are intentionally rejected
    // below rather than putting secrets in a process argument or environment.
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitAddRequest {
    path: String,
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitCommitRequest {
    path: String,
    message: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    allow_empty: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitFileStatus {
    name: String,
    extra: String,
    staging: String,
    worktree: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitStatusResponse {
    current_branch: String,
    file_status: Vec<GitFileStatus>,
    branch_published: bool,
    ahead: u32,
    behind: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream: Option<String>,
    detached: bool,
}

#[derive(Debug, Serialize)]
struct GitBranchesResponse {
    branches: Vec<String>,
    current: String,
}

#[derive(Debug, Serialize)]
struct GitCommitResponse {
    hash: String,
}

#[derive(Debug, Serialize)]
struct GitOperationResponse {
    output: String,
}

/// Request to create an orchestration.
#[derive(Debug, Deserialize)]
struct CreateOrchestrationRequest {
    name: String,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

/// Request to queue an agent task for a sandbox.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTaskRequest {
    prompt: String,
    #[serde(alias = "target_sandbox")]
    sandbox: String,
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
    endpoints: Vec<crate::backend::ResolvedEndpoint>,
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
    /// Last synchronized remote workspace revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_revision: Option<String>,
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

/// Response data for a full-state sandbox fork.
#[derive(Debug, Serialize)]
struct ForkSandboxResult {
    sandbox: SandboxInfo,
    security_warning: String,
}

#[derive(Debug, Serialize)]
struct BackendDiscovery {
    /// Backend selected by automatic sandbox creation, when one is ready.
    default_backend: Option<String>,
    backends: Vec<BackendDescriptor>,
}

#[derive(Debug, Serialize)]
struct BackendDescriptor {
    backend: String,
    configured: bool,
    usable: bool,
    readiness_reason: String,
    capabilities: BackendCapabilities,
}

fn backend_discovery(default_backend: Option<BackendType>) -> BackendDiscovery {
    let backends = BackendType::all()
        .into_iter()
        .map(|backend| {
            let readiness = backend_readiness(backend);
            BackendDescriptor {
                backend: backend.to_string(),
                configured: readiness.configured,
                usable: readiness.usable,
                readiness_reason: readiness.reason,
                capabilities: backend_capabilities(backend),
            }
        })
        .collect();
    BackendDiscovery {
        default_backend: default_backend.map(|backend| backend.to_string()),
        backends,
    }
}

/// Prefer the backend persisted with a sandbox over the manager's automatic
/// default. An explicit per-sandbox selection must remain visible in every
/// response even when another backend is the server default.
fn recorded_backend(recorded: Option<BackendType>, manager_default: BackendType) -> BackendType {
    recorded.unwrap_or(manager_default)
}

fn parse_backend_selection(value: Option<&str>) -> Result<Option<BackendType>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if matches!(value.to_ascii_lowercase().as_str(), "automatic" | "auto") {
        return Ok(None);
    }
    value.parse().map(Some)
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
    /// Canonical configuration path selected at server startup. Keeping this
    /// explicit avoids silently changing policy based on the process cwd.
    #[cfg_attr(not(feature = "enterprise"), allow(dead_code))]
    config_path: Option<std::path::PathBuf>,
    /// API keys for authentication (empty = no auth required)
    api_keys: Vec<String>,
    /// Optional explicit volume data root. Production uses the standard home
    /// directory; tests inject an isolated root without changing global HOME.
    volume_base_dir: Option<std::path::PathBuf>,
    /// Whether HTTP API callers may request root execution (`sudo: true`).
    allow_sudo_exec: bool,
    /// Server start time for uptime calculation
    started_at: std::time::Instant,
    /// OpenCode API state
    opencode: Arc<OpenCodeState>,
    /// Durable orchestration persistence store
    orchestration_store: Option<Arc<OrchestrationStore>>,
    /// Durable agent task queue.
    task_manager: Option<Arc<TaskManager>>,
    /// Config-driven user job scheduler and its owned daemon loop.
    job_scheduler: Option<Arc<JobScheduler>>,
    job_scheduler_handle: Option<JobSchedulerHandle>,
    /// Durable SCIM 2.0 provisioning store.
    pub(crate) scim_store: Option<Arc<crate::scim::ScimStore>>,
    /// Tenant selected by the server configuration for SCIM provisioning.
    pub(crate) scim_tenant_id: String,
    /// Lazily initialized VmManager shared by sandbox and durable-object APIs.
    ///
    /// Backend discovery can fail while a service is starting (for example,
    /// before Docker Desktop or Apple Containers is ready).  Retrying through
    /// the cell lets `/doctor` and the first sandbox attempt recover without
    /// requiring a server restart.
    vm_manager: Arc<std::sync::OnceLock<Arc<tokio::sync::RwLock<VmManager>>>>,
    /// Test-only switch used to exercise the reachable-server recovery path
    /// without depending on whichever host runtimes happen to be installed.
    #[cfg(test)]
    force_backend_unavailable: bool,
    /// Event bus for sandbox lifecycle events (webhook/SSE/OTel)
    event_bus: Option<crate::events::EventBus>,
    /// OpenTelemetry tracer provider for span export
    otel_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    /// Trusted server configuration used for sandbox ownership and proxy
    /// governance. This is captured at server startup, not reparsed from the
    /// request working directory.
    server_config_path: Option<std::path::PathBuf>,
    llm_governance: crate::config::LlmGovernanceConfig,
    /// Enterprise configuration (when enterprise feature is enabled)
    #[cfg(feature = "enterprise")]
    enterprise_config: Option<crate::config::EnterpriseConfig>,
    /// Enterprise policy engine (when enterprise feature is enabled)
    #[cfg(feature = "enterprise")]
    policy_engine: Option<tokio::sync::RwLock<crate::policy::PolicyEngine>>,
    /// Initialization failures are retained for status and fail-closed
    /// request handling; they must never be represented as enabled.
    #[cfg(feature = "enterprise")]
    policy_init_error: Option<String>,
    /// Serialized quota checks and lifecycle mutations.
    #[cfg(feature = "enterprise")]
    quota_controller: Arc<tokio::sync::Mutex<crate::quota::QuotaController>>,
}

impl AppState {
    fn authentication_required(&self) -> bool {
        #[cfg(feature = "enterprise")]
        {
            !self.api_keys.is_empty()
                || self
                    .enterprise_config
                    .as_ref()
                    .and_then(|config| config.jwks_url.as_deref())
                    .is_some()
        }
        #[cfg(not(feature = "enterprise"))]
        {
            !self.api_keys.is_empty()
        }
    }

    fn new(
        api_keys_override: Vec<String>,
        otel_endpoint: Option<String>,
        webhook_urls: Vec<String>,
        config_path: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let mut api_keys = api_keys_override;
        // If no keys provided via CLI, fall back to env var / config file
        if api_keys.is_empty()
            && let Some(key) = load_api_key(config_path.as_deref())
        {
            api_keys.push(key);
        }
        let allow_sudo_exec = load_api_allow_sudo_exec_from_config(config_path.as_deref());
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

        let server_config_path = config_path.clone().or_else(|| {
            let default_path = std::path::PathBuf::from("agentkernel.toml");
            default_path.exists().then_some(default_path)
        });
        let server_config = if let Some(path) = server_config_path.as_deref() {
            // An explicitly supplied path is authoritative. Do not turn an
            // unreadable or malformed governance config into "disabled".
            Some(crate::config::Config::from_file(path)?)
        } else {
            None
        };
        let llm_governance = server_config
            .as_ref()
            .map(|config| config.llm_governance.clone())
            .unwrap_or_default();
        crate::model_governance::ModelGovernancePolicy::validate_config(&llm_governance)?;

        #[cfg(feature = "enterprise")]
        let (enterprise_config, policy_engine, policy_init_error) =
            Self::init_enterprise(server_config_path.as_deref());
        #[cfg(feature = "enterprise")]
        let quota_controller =
            Arc::new(tokio::sync::Mutex::new(crate::quota::QuotaController::new(
                enterprise_config
                    .as_ref()
                    .map(|config| config.quotas.clone())
                    .unwrap_or_default(),
            )));

        let vm_manager = Arc::new(std::sync::OnceLock::new());
        if let Ok(mgr) = VmManager::new() {
            let _ = vm_manager.set(Arc::new(tokio::sync::RwLock::new(mgr)));
        }
        Ok(Self {
            api_keys,
            volume_base_dir: None,
            allow_sudo_exec,
            started_at: std::time::Instant::now(),
            opencode: Arc::new(OpenCodeState::new(vm_manager.clone())),
            orchestration_store: Self::init_orchestration_store(),
            task_manager: Self::init_task_manager(),
            job_scheduler: None,
            job_scheduler_handle: None,
            scim_store: Self::init_scim_store(config_path.as_deref()),
            scim_tenant_id: load_scim_tenant_id(config_path.as_deref()),
            vm_manager,
            #[cfg(test)]
            force_backend_unavailable: false,
            event_bus,
            otel_provider,
            config_path,
            server_config_path,
            llm_governance,
            #[cfg(feature = "enterprise")]
            enterprise_config,
            #[cfg(feature = "enterprise")]
            policy_engine,
            #[cfg(feature = "enterprise")]
            policy_init_error,
            #[cfg(feature = "enterprise")]
            quota_controller,
        })
    }

    #[cfg(test)]
    fn new_with_config(
        api_keys_override: Vec<String>,
        otel_endpoint: Option<String>,
        webhook_urls: Vec<String>,
        config_path: Option<&std::path::Path>,
    ) -> Result<Self> {
        Self::new(
            api_keys_override,
            otel_endpoint,
            webhook_urls,
            config_path.map(std::path::Path::to_path_buf),
        )
    }

    /// Create state with explicit API keys
    #[allow(dead_code)]
    fn with_api_keys(api_keys: Vec<String>) -> Self {
        if !api_keys.is_empty() {
            eprintln!("API key authentication enabled");
        }
        let vm_manager = Arc::new(std::sync::OnceLock::new());
        Self {
            api_keys,
            volume_base_dir: None,
            allow_sudo_exec: false,
            started_at: std::time::Instant::now(),
            opencode: Arc::new(OpenCodeState::new(vm_manager.clone())),
            orchestration_store: Self::init_orchestration_store(),
            task_manager: Self::init_task_manager(),
            job_scheduler: None,
            job_scheduler_handle: None,
            scim_store: Self::init_scim_store(None),
            scim_tenant_id: "default".to_string(),
            vm_manager,
            #[cfg(test)]
            force_backend_unavailable: false,
            event_bus: None,
            otel_provider: None,
            config_path: None,
            server_config_path: None,
            llm_governance: crate::config::LlmGovernanceConfig::default(),
            #[cfg(feature = "enterprise")]
            enterprise_config: None,
            #[cfg(feature = "enterprise")]
            policy_engine: None,
            #[cfg(feature = "enterprise")]
            policy_init_error: None,
            #[cfg(feature = "enterprise")]
            quota_controller: Arc::new(tokio::sync::Mutex::new(
                crate::quota::QuotaController::new(Default::default()),
            )),
        }
    }

    #[cfg(test)]
    fn with_task_manager_for_tests(manager: TaskManager) -> Self {
        let mut state = Self::with_api_keys(vec![]);
        state.task_manager = Some(Arc::new(manager));
        state
    }

    #[cfg(feature = "enterprise")]
    fn init_enterprise(
        config_path: Option<&std::path::Path>,
    ) -> (
        Option<crate::config::EnterpriseConfig>,
        Option<tokio::sync::RwLock<crate::policy::PolicyEngine>>,
        Option<String>,
    ) {
        let config_path = config_path.map(std::path::Path::to_path_buf).or_else(|| {
            let fallback = std::path::PathBuf::from("agentkernel.toml");
            fallback.exists().then_some(fallback)
        });
        let Some(config_path) = config_path else {
            return (None, None, None);
        };
        if !config_path.exists() {
            return (None, None, None);
        }

        let cfg = match crate::config::Config::from_file(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[enterprise] Failed to load config: {}", e);
                // A present but malformed daemon config must not silently
                // disable an intended quota policy. Keep the enterprise
                // state fail-closed with zero limits until the file is fixed.
                let fail_closed = crate::config::EnterpriseConfig {
                    enabled: true,
                    quotas: crate::config::ResourceQuotaConfig {
                        enabled: true,
                        default_limits: crate::config::ResourceQuotaLimits {
                            max_running_sandboxes: Some(0),
                            max_total_sandboxes: Some(0),
                            max_total_vcpus: Some(0),
                            max_total_memory_mb: Some(0),
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                };
                return (
                    Some(fail_closed),
                    None,
                    Some(format!("Failed to load config: {e}")),
                );
            }
        };

        if !cfg.enterprise.enabled {
            return (Some(cfg.enterprise), None, None);
        }

        let base_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
        match crate::policy::PolicyEngine::new_with_base_dir(&cfg.enterprise, base_dir) {
            Ok(engine) => {
                eprintln!("[enterprise] Policy engine initialized for HTTP API");
                (
                    Some(cfg.enterprise),
                    Some(tokio::sync::RwLock::new(engine)),
                    None,
                )
            }
            Err(e) => {
                eprintln!("[enterprise] Failed to initialize policy engine: {}", e);
                (Some(cfg.enterprise), None, Some(e.to_string()))
            }
        }
    }

    fn ensure_manager(&self) -> Result<Arc<tokio::sync::RwLock<VmManager>>> {
        if let Some(manager) = self.vm_manager.get() {
            return Ok(manager.clone());
        }

        let manager = Arc::new(tokio::sync::RwLock::new(VmManager::new()?));
        let _ = self.vm_manager.set(manager);
        self.vm_manager
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("VmManager could not be initialized"))
    }

    /// Return an owned handle for daemon-owned lifecycle tasks.
    ///
    /// Unlike `get_manager`, this can be moved into a spawned task so dropping
    /// an HTTP request future cannot cancel a mutation while it owns the only
    /// live Firecracker handle.
    fn manager_handle(&self) -> Result<Arc<tokio::sync::RwLock<VmManager>>> {
        self.ensure_manager()
    }

    /// Load, validate, and attach user schedules before the listener starts.
    /// Invalid schedule IDs and targets therefore fail daemon startup instead
    /// of silently disabling automation.
    fn configure_job_scheduler(&mut self, config_path: Option<&std::path::Path>) -> Result<()> {
        let path = config_path
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("agentkernel.toml"));
        if !path.exists() {
            return Ok(());
        }
        let config = crate::config::Config::from_file(&path)
            .with_context(|| format!("failed to load daemon config {}", path.display()))?;
        let Some(store) = self.orchestration_store.clone() else {
            if config.schedules.is_empty() {
                return Ok(());
            }
            anyhow::bail!("configured schedules require durable orchestration storage")
        };
        if let Some(scheduler) = JobScheduler::from_config(&config, store, self.vm_manager.clone())?
        {
            self.job_scheduler = Some(Arc::new(scheduler));
        }
        Ok(())
    }

    fn start_job_scheduler(&mut self) {
        if let Some(scheduler) = self.job_scheduler.clone() {
            self.job_scheduler_handle = Some(scheduler.spawn());
        }
    }

    async fn get_manager(&self) -> Result<tokio::sync::RwLockWriteGuard<'_, VmManager>> {
        #[cfg(test)]
        if self.force_backend_unavailable {
            anyhow::bail!("No sandbox backend available (test fixture)");
        }

        if self.vm_manager.get().is_none() {
            let manager = Arc::new(tokio::sync::RwLock::new(VmManager::new()?));
            let _ = self.vm_manager.set(manager);
        }
        let manager = self
            .vm_manager
            .get()
            .ok_or_else(|| anyhow::anyhow!("VmManager could not be initialized"))?;
        Ok(manager.write().await)
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

    fn init_task_manager() -> Option<Arc<TaskManager>> {
        match TaskManager::open_default() {
            Ok(manager) => Some(Arc::new(manager)),
            Err(e) => {
                eprintln!("[tasks] Failed to initialize task storage: {}", e);
                None
            }
        }
    }

    fn init_scim_store(
        config_path: Option<&std::path::Path>,
    ) -> Option<Arc<crate::scim::ScimStore>> {
        let mappings = config_path
            .and_then(|path| crate::config::Config::from_file(path).ok())
            .map(|config| config.enterprise.scim_group_mappings)
            .unwrap_or_default();
        match crate::scim::ScimStore::open_default_with_mappings(mappings) {
            Ok(store) => Some(Arc::new(store)),
            Err(e) => {
                eprintln!("[scim] Failed to initialize SCIM storage: {e}");
                None
            }
        }
    }

    #[allow(clippy::result_large_err)]
    fn task_manager(&self) -> Result<&Arc<TaskManager>, Response<BoxBody>> {
        self.task_manager.as_ref().ok_or_else(|| {
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error("Task storage unavailable"),
            )
        })
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
    async fn check_auth(&self, req: &Request<Incoming>) -> Result<(), Response<BoxBody>> {
        #[cfg(feature = "enterprise")]
        let jwks_url = self
            .enterprise_config
            .as_ref()
            .and_then(|config| config.jwks_url.as_deref());
        if !self.authentication_required() {
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
                    return Ok(());
                }

                #[cfg(feature = "enterprise")]
                if let Some(jwks_url) = jwks_url
                    && crate::identity::validate_jwt(token, jwks_url).await.is_ok()
                {
                    return Ok(());
                }

                Err(json_response(
                    StatusCode::UNAUTHORIZED,
                    &ApiResponse::<()>::error("Invalid bearer token"),
                ))
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

#[cfg(feature = "enterprise")]
fn trusted_owner_identity(
    identity: &crate::identity::AgentIdentity,
    state: &AppState,
) -> Option<(String, String)> {
    let tenant = identity
        .org_id()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            state
                .enterprise_config
                .as_ref()
                .and_then(|config| config.org_id.as_deref())
        })
        .unwrap_or("local")
        .to_string();
    if let Some(subject) = identity.subject().filter(|value| !value.trim().is_empty()) {
        return Some((tenant, subject.to_string()));
    }
    // API keys are server-scoped, so use a one-way fingerprint as the user
    // dimension. The secret itself never enters state, logs, or metrics.
    identity.api_key.as_deref().and_then(|key| {
        state
            .api_keys
            .iter()
            .any(|expected| constant_time_eq(key, expected))
            .then(|| (tenant, api_key_owner_id(key)))
    })
}

#[cfg(not(feature = "enterprise"))]
fn trusted_owner_identity(req: &Request<Incoming>, state: &AppState) -> Option<(String, String)> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))?;
    state
        .api_keys
        .iter()
        .any(|expected| constant_time_eq(token, expected))
        .then(|| ("local".to_string(), api_key_owner_id(token)))
}

fn api_key_owner_id(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    format!("api-key:{}", hex::encode(digest))
}

/// Return the tenant that the server has actually authenticated for a new
/// sandbox. A bearer value is not enough when API-key auth is disabled: that
/// would let a caller spoof an organization by choosing an arbitrary token.
#[cfg(feature = "enterprise")]
fn trusted_tenant_for_sandbox(
    identity: &crate::identity::AgentIdentity,
    state: &AppState,
) -> Option<String> {
    if let Some(org_id) = identity.org_id().map(str::trim).filter(|id| !id.is_empty()) {
        return Some(org_id.to_string());
    }

    let api_key_is_valid = identity.api_key.as_deref().is_some_and(|token| {
        state
            .api_keys
            .iter()
            .any(|configured| constant_time_eq(token, configured))
    });
    if api_key_is_valid {
        return state
            .enterprise_config
            .as_ref()
            .and_then(|config| config.org_id.clone())
            .filter(|org_id| !org_id.trim().is_empty());
    }

    None
}

/// Enforce enterprise policy for an action on a sandbox.
///
/// Returns Ok(()) if the action is permitted (or no policy engine is active).
/// Returns a 403 Forbidden response if the action is denied.
#[cfg(feature = "enterprise")]
#[allow(clippy::result_large_err)]
async fn enforce_policy(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
    action: crate::policy::Action,
    sandbox_name: &str,
) -> Result<(), Response<BoxBody>> {
    let Some(ref engine_lock) = state.policy_engine else {
        if state
            .enterprise_config
            .as_ref()
            .is_some_and(|config| config.enabled)
            || state.policy_init_error.is_some()
        {
            let detail = state
                .policy_init_error
                .as_deref()
                .unwrap_or("policy engine is not active");
            return Err(json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error(format!("Policy enforcement unavailable: {detail}")),
            ));
        }
        return Ok(());
    };
    let Some(ref enterprise) = state.enterprise_config else {
        return Ok(());
    };

    let mut principal = identity.to_principal(
        enterprise.org_id.as_deref().unwrap_or("default"),
        &enterprise.default_roles,
    );
    // SCIM grants are materialized from explicit tenant/group mappings and
    // joined only by the validated JWT `sub` ↔ SCIM `externalId` contract.
    // API-key IDs and email/userName are never used as fallback joins.
    if let Some(subject) = identity.subject()
        && let Some(store) = state.scim_store.as_ref()
    {
        // The tenant comes from the validated JWT claim, never from the
        // server default, so a colliding subject in another organization
        // cannot inherit this tenant's SCIM grants.
        match store.principal_bindings(&principal.org_id, subject) {
            Ok((roles, teams)) => {
                principal.roles.extend(roles);
                principal.teams.extend(teams);
                principal.roles.sort();
                principal.roles.dedup();
                principal.teams.sort();
                principal.teams.dedup();
            }
            Err(error) => {
                return Err(json_response(
                    StatusCode::FORBIDDEN,
                    &ApiResponse::<()>::error(format!(
                        "SCIM authorization state unavailable: {:?}",
                        error
                    )),
                ));
            }
        }
    }
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

    // Status intentionally remains public like health so local clients can
    // distinguish a reachable daemon from an unavailable backend before they
    // have credentials or attempt a lifecycle mutation.
    if method == Method::GET && segments.as_slice() == ["status"] {
        return Ok(handle_status(state).await);
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
    if let Err(resp) = state.check_auth(&req).await {
        if segments.first() == Some(&"scim") {
            return Ok(crate::scim::authentication_required());
        }
        return Ok(resp);
    }

    // Provisioning is always privileged, even when the legacy sandbox API is
    // running in its optional-authentication mode.
    if segments.first() == Some(&"scim") && state.api_keys.is_empty() {
        return Ok(crate::scim::authentication_required());
    }

    // Every persistent sandbox-scoped route shares this gate. Keeping it at
    // dispatch time prevents a newly added sub-route (exec, files, browser,
    // Git, detached logs, etc.) from becoming a name-guessing ownership
    // bypass. Individual lifecycle handlers retain their checks where they
    // need the identity for policy/quota decisions.
    #[cfg(feature = "enterprise")]
    if segments.first() == Some(&"sandboxes")
        && let Some(name) = segments.get(1).copied()
        && name != "import-config"
        && name != "by-uuid"
        // POST /sandboxes/{name}/config is the legacy config-import create
        // endpoint and has no existing sandbox to authorize.
        && !(method == Method::POST
            && segments.len() == 3
            && segments.get(2) == Some(&"config"))
        // Start performs its ownership check after it has consumed and
        // validated the one-shot, UUID-bound first-claim token. Owned starts
        // and unowned starts without a valid token are still denied there.
        && !(method == Method::POST
            && segments.len() == 3
            && segments.get(2) == Some(&"start"))
        && let Ok(manager) = state.get_manager().await
        && let Some(sandbox) = manager.get_state(name)
    {
        let identity = extract_identity(&req, &state).await;
        if !sandbox_access_allowed(&state, &identity, sandbox) {
            return Ok(sandbox_access_denied());
        }
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
        // SCIM 2.0 provisioning (authenticated by the API-key middleware
        // above; the configured tenant is never selected from request data).
        (Method::GET, ["scim", "v2", "ServiceProviderConfig"]) => {
            crate::scim::handle_service_provider_config().await
        }
        (Method::GET, ["scim", "v2", "ResourceTypes"]) => {
            crate::scim::handle_resource_types().await
        }
        (Method::GET, ["scim", "v2", "ResourceTypes", id]) => {
            crate::scim::handle_resource_type(id).await
        }
        (Method::GET, ["scim", "v2", "Schemas"]) => crate::scim::handle_schemas().await,
        (Method::GET, ["scim", "v2", "Schemas", id]) => crate::scim::handle_schema(id).await,
        (Method::GET, ["scim", "v2", "Users"]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_list_users(store, &state.scim_tenant_id, req.uri().query()).await
        }
        (Method::POST, ["scim", "v2", "Users"]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_create_user(req, store, &state.scim_tenant_id).await
        }
        (Method::GET, ["scim", "v2", "Users", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_get_user(id, store, &state.scim_tenant_id).await
        }
        (Method::PUT, ["scim", "v2", "Users", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_replace_user(req, id, store, &state.scim_tenant_id).await
        }
        (Method::PATCH, ["scim", "v2", "Users", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_patch_user(req, id, store, &state.scim_tenant_id).await
        }
        // Deactivation is PATCH active:false; DELETE tombstones the resource.
        (Method::DELETE, ["scim", "v2", "Users", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_delete_user(id, store, &state.scim_tenant_id).await
        }
        (Method::GET, ["scim", "v2", "Groups"]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_list_groups(store, &state.scim_tenant_id, req.uri().query()).await
        }
        (Method::POST, ["scim", "v2", "Groups"]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_create_group(req, store, &state.scim_tenant_id).await
        }
        (Method::GET, ["scim", "v2", "Groups", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_get_group(id, store, &state.scim_tenant_id).await
        }
        (Method::PUT, ["scim", "v2", "Groups", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_replace_group(req, id, store, &state.scim_tenant_id).await
        }
        (Method::PATCH, ["scim", "v2", "Groups", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_patch_group(req, id, store, &state.scim_tenant_id).await
        }
        (Method::DELETE, ["scim", "v2", "Groups", id]) => {
            let Some(store) = state.scim_store.clone() else {
                return Ok(crate::scim::storage_unavailable());
            };
            crate::scim::handle_delete_group(id, store, &state.scim_tenant_id).await
        }

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

        // Agent task queue
        (Method::POST, ["tasks"]) => handle_create_task(req, state).await,
        (Method::GET, ["tasks"]) => handle_list_tasks(state).await,
        (Method::GET, ["tasks", task_id]) => handle_get_task(task_id, state).await,
        (Method::DELETE, ["tasks", task_id]) => handle_cancel_task(task_id, state).await,

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
        (Method::GET, ["schedules", "configured"]) => handle_list_configured_schedules(state).await,
        (Method::GET, ["schedules", "configured", schedule_id]) => {
            handle_get_configured_schedule(schedule_id, state).await
        }
        (Method::GET, ["schedules", "configured", schedule_id, "status"]) => {
            handle_get_configured_schedule(schedule_id, state).await
        }
        (Method::POST, ["schedules", "configured", schedule_id, "trigger"]) => {
            handle_trigger_configured_schedule(schedule_id, state).await
        }
        (Method::GET, ["schedules"]) => handle_list_schedules(state).await,
        (Method::POST, ["schedules"]) => handle_create_schedule(req, state).await,
        (Method::GET, ["schedules", schedule_id]) => handle_get_schedule(schedule_id, state).await,
        (Method::GET, ["schedules", schedule_id, "status"]) => {
            handle_get_schedule_status(schedule_id, state).await
        }
        (Method::DELETE, ["schedules", schedule_id]) => {
            handle_delete_schedule(schedule_id, state).await
        }
        (Method::POST, ["schedules", schedule_id, "trigger"]) => {
            handle_trigger_schedule(schedule_id, state).await
        }

        // List sandboxes (supports ?label=key:value filtering)
        (Method::GET, ["sandboxes"]) => handle_list_sandboxes(req, state).await,

        // Describe backend selection and capabilities without requiring a
        // sandbox manager to be initialized.
        (Method::GET, ["backends"]) => handle_list_backends(state).await,

        // Create a sandbox
        (Method::POST, ["sandboxes"]) => handle_create_sandbox(req, state).await,

        // Get sandbox info by UUID
        (Method::GET, ["sandboxes", "by-uuid", uuid]) => {
            handle_get_sandbox_by_uuid(req, uuid, state).await
        }

        // Get sandbox info
        (Method::GET, ["sandboxes", name]) => handle_get_sandbox(req, name, state).await,

        // Execute in a sandbox
        (Method::POST, ["sandboxes", name, "exec"]) => handle_exec_sandbox(req, name, state).await,

        // Sandbox-scoped Git operations (Daytona toolbox parity)
        (Method::GET, ["sandboxes", name, "git", "status"]) => {
            handle_git_status(req, name, state).await
        }
        (Method::GET, ["sandboxes", name, "git", "branches"]) => {
            handle_git_branches(req, name, state).await
        }
        (Method::POST, ["sandboxes", name, "git", "add"]) => handle_git_add(req, name, state).await,
        (Method::POST, ["sandboxes", name, "git", "commit"]) => {
            handle_git_commit(req, name, state).await
        }
        (Method::POST, ["sandboxes", name, "git", "pull"]) => {
            handle_git_pull(req, name, state).await
        }
        (Method::POST, ["sandboxes", name, "git", "push"]) => {
            handle_git_push(req, name, state).await
        }

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
        (Method::GET, ["sandboxes", name, "logs"]) => handle_sandbox_logs(req, name, state).await,

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
        (Method::DELETE, ["sandboxes", name]) => handle_delete_sandbox(req, name, state).await,

        // Start a stopped sandbox
        (Method::POST, ["sandboxes", name, "start"]) => {
            handle_start_sandbox(req, name, state).await
        }

        // Stop a running sandbox
        (Method::POST, ["sandboxes", name, "stop"]) => handle_stop_sandbox(req, name, state).await,

        // Pause a running Firecracker sandbox with full guest state preserved
        (Method::POST, ["sandboxes", name, "pause"]) => {
            handle_pause_sandbox(req, name, state).await
        }

        // Resume a full-state paused Firecracker sandbox
        (Method::POST, ["sandboxes", name, "resume"]) => {
            handle_resume_sandbox(req, name, state).await
        }

        // Fork a paused Firecracker sandbox into a new running sandbox
        (Method::POST, ["sandboxes", name, "fork"]) => handle_fork_sandbox(req, name, state).await,

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
        (Method::GET, ["snapshots"]) => handle_list_snapshots(req, state).await,
        (Method::POST, ["snapshots"]) => handle_take_snapshot(req, state).await,
        (Method::GET, ["snapshots", name]) => handle_get_snapshot(req, name, state).await,
        (Method::DELETE, ["snapshots", name]) => handle_delete_snapshot(req, name, state).await,
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
        (Method::GET, ["llm", "usage"]) => handle_llm_usage_all(req, state.clone()).await,
        (Method::GET, ["llm", "usage", sandbox]) => {
            handle_llm_usage_sandbox(req, sandbox, state.clone()).await
        }
        (Method::GET, ["llm", "spend"]) => handle_llm_spend(req, state.clone()).await,

        // LLM key management
        (Method::GET, ["llm", "keys"]) => handle_llm_keys_list().await,
        (Method::PUT, ["llm", "keys", provider]) => handle_llm_keys_set(req, provider).await,
        (Method::DELETE, ["llm", "keys", provider]) => handle_llm_keys_remove(provider).await,

        // Garbage collection
        (Method::POST, ["gc"]) => handle_gc(req, state).await,

        // Lifecycle policy reconciliation
        (Method::POST, ["lifecycle", "reconcile"]) => handle_reconcile_lifecycle(req, state).await,

        // Agents/plugins
        (Method::GET, ["agents"]) => handle_list_agents(state).await,
        (Method::POST, ["agents", name, "integration"]) => {
            handle_install_agent_integration(req, name).await
        }

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
        (Method::GET, ["images", "usage"]) => handle_image_disk_usage(state).await,
        (Method::POST, ["images", "pull"]) => handle_pull_image(req, state).await,
        (Method::POST, ["images", "prune"]) => handle_prune_images(req, state).await,
        (Method::DELETE, ["images", id]) => handle_delete_image(id, state).await,

        // Hardware benchmark
        (Method::POST, ["benchmark"]) => handle_benchmark(state).await,

        // Session recording
        (Method::GET, ["sessions"]) => handle_list_recordings().await,
        (Method::GET, ["sessions", id]) => handle_get_recording(id).await,
        (Method::GET, ["sessions", id, "cast"]) => handle_get_recording_cast(id).await,

        // Sandbox config export/import
        (Method::GET, ["sandboxes", name, "config"]) => {
            handle_export_sandbox_config(name, state).await
        }
        (Method::POST, ["sandboxes", "import-config"]) => {
            handle_import_sandbox_config(req, state, None).await
        }
        // Keep the original path working for existing API clients while the
        // desktop app uses the name-independent import endpoint.
        (Method::POST, ["sandboxes", name, "config"]) => {
            handle_import_sandbox_config(req, state, Some(name)).await
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
        (Method::GET, ["quotas"]) => handle_quota_status(req, state).await,
        #[cfg(feature = "enterprise")]
        (Method::POST, ["policy", "check"]) => handle_policy_check(req, state).await,
        #[cfg(feature = "enterprise")]
        (Method::POST, ["policy", "reload"]) => handle_policy_reload(state).await,
        #[cfg(feature = "enterprise")]
        (Method::GET, ["policy", "audit"]) => handle_policy_audit(req, state).await,

        // Keep errors under the SCIM media type for unknown SCIM resources.
        _ if segments.first() == Some(&"scim") => crate::scim::not_found_response(),

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

#[allow(clippy::result_large_err)]
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
            // Pool uses alpine:3.24, warn if custom image requested
            eprintln!("Warning: custom image ignored in fast mode (pool uses alpine:3.24)");
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
    let manager = match state.ensure_manager() {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(e) => {
            return e;
        }
    };

    match crate::object_runtime::handle_object_call(store, &manager, class, object_id, method, body)
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
    let manager = match state.ensure_manager() {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let body = match read_body_bytes(req).await {
        Ok(b) => b,
        Err(e) => {
            return e;
        }
    };

    match crate::object_runtime::handle_object_call(
        store, &manager, class, object_id, "alarm", body,
    )
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

async fn handle_get_schedule_status(schedule_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    handle_get_schedule(schedule_id, state).await
}

async fn handle_list_configured_schedules(state: Arc<AppState>) -> Response<BoxBody> {
    let Some(scheduler) = state.job_scheduler.as_ref() else {
        return json_response(
            StatusCode::OK,
            &ApiResponse::success(Vec::<JobScheduleStatus>::new()),
        );
    };
    match scheduler.list_status(chrono::Utc::now()) {
        Ok(schedules) => json_response(StatusCode::OK, &ApiResponse::success(schedules)),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
}

async fn handle_get_configured_schedule(
    schedule_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let Some(scheduler) = state.job_scheduler.as_ref() else {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Configured schedule not found"),
        );
    };
    match scheduler.get_status(schedule_id, chrono::Utc::now()) {
        Ok(Some(status)) => json_response(StatusCode::OK, &ApiResponse::success(status)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Configured schedule not found"),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
}

async fn handle_trigger_configured_schedule(
    schedule_id: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let Some(scheduler) = state.job_scheduler.as_ref() else {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Configured schedule not found"),
        );
    };
    match scheduler.trigger(schedule_id).await {
        Ok(execution) => json_response(StatusCode::OK, &ApiResponse::success(execution)),
        Err(error) if error.to_string().contains("not found") => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(error.to_string()),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
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

async fn handle_create_task(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: CreateTaskRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(resp) => return resp,
    };
    if let Err(error) = crate::tasks::validate_task_prompt(&body.prompt) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }
    if let Err(error) = validation::validate_sandbox_name(&body.sandbox) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }

    let manager = match state.task_manager() {
        Ok(manager) => manager,
        Err(resp) => return resp,
    };
    match manager.create(&body.prompt, &body.sandbox) {
        Ok(task) => json_response(StatusCode::CREATED, &ApiResponse::success(task)),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
}

async fn handle_list_tasks(state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.task_manager() {
        Ok(manager) => manager,
        Err(resp) => return resp,
    };
    match manager.list(200, 0) {
        Ok(tasks) => json_response(StatusCode::OK, &ApiResponse::success(tasks)),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
}

async fn handle_get_task(task_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if let Err(error) = crate::tasks::validate_task_id(task_id) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }
    let manager = match state.task_manager() {
        Ok(manager) => manager,
        Err(resp) => return resp,
    };
    match manager.get(task_id) {
        Ok(Some(task)) => json_response(StatusCode::OK, &ApiResponse::success(task)),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Task not found"),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
}

async fn handle_cancel_task(task_id: &str, state: Arc<AppState>) -> Response<BoxBody> {
    if let Err(error) = crate::tasks::validate_task_id(task_id) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }
    let manager = match state.task_manager() {
        Ok(manager) => manager,
        Err(resp) => return resp,
    };
    match manager.cancel(task_id) {
        Ok(CancelOutcome::Cancelled(task)) => {
            json_response(StatusCode::OK, &ApiResponse::success(task))
        }
        Ok(CancelOutcome::NotFound) => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Task not found"),
        ),
        Ok(CancelOutcome::NotCancellable(task)) => json_response(
            StatusCode::CONFLICT,
            &ApiResponse::<()>::error(format!(
                "Task cannot be cancelled after reaching '{}' status",
                task.status
            )),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
}

async fn handle_list_sandboxes(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;

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
            #[cfg(feature = "enterprise")]
            {
                manager
                    .get_state(name)
                    .is_some_and(|sandbox| sandbox_access_allowed(&state, &identity, sandbox))
            }
            #[cfg(not(feature = "enterprise"))]
            {
                let _ = name;
                true
            }
        })
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
                endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
                secret_files: state_info
                    .map(|s| s.secret_files.clone())
                    .unwrap_or_default(),
                placeholder_secrets: state_info.map(|s| s.placeholder_secrets).unwrap_or(false),
                proxy_port: state_info.and_then(|s| s.proxy_port),
                secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
                labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
                description: state_info.and_then(|s| s.description.clone()),
                last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
                workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
                archived_at: state_info.and_then(|s| s.archived_at.clone()),
                archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
                lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
            }
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(sandboxes))
}

async fn handle_list_backends(state: Arc<AppState>) -> Response<BoxBody> {
    let active_default = if let Some(manager) = state.vm_manager.get() {
        Some(manager.read().await.backend())
    } else {
        detect_best_backend()
    };
    json_response(
        StatusCode::OK,
        &ApiResponse::success(backend_discovery(active_default)),
    )
}

async fn handle_create_sandbox(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let start = std::time::Instant::now();

    // Enterprise policy enforcement (extract identity before consuming body)
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    let trusted_owner = {
        #[cfg(feature = "enterprise")]
        {
            trusted_owner_identity(&identity, &state)
        }
        #[cfg(not(feature = "enterprise"))]
        {
            trusted_owner_identity(&req, &state)
        }
    };
    #[cfg(feature = "enterprise")]
    let trusted_tenant = trusted_tenant_for_sandbox(&identity, &state);

    #[cfg(feature = "enterprise")]
    if state.llm_governance.enabled && trusted_tenant.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            &ApiResponse::<()>::error(
                "LLM model governance requires a validated tenant identity for sandbox creation",
            ),
        );
    }
    #[cfg(not(feature = "enterprise"))]
    if state.llm_governance.enabled {
        return json_response(
            StatusCode::FORBIDDEN,
            &ApiResponse::<()>::error(
                "LLM model governance requires the enterprise identity feature",
            ),
        );
    }

    let body: CreateRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let requested_backend = match parse_backend_selection(body.backend.as_deref()) {
        Ok(backend) => backend,
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, &ApiResponse::<()>::error(error));
        }
    };

    if let Some(backend) = requested_backend
        && !backend_readiness(backend).usable
    {
        return json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &ApiResponse::<()>::error(format!(
                "Backend '{}' is unavailable. Use GET /backends to see ready backends",
                backend
            )),
        );
    }

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

    let image = body.image.as_deref().unwrap_or("alpine:3.24");
    let vcpus = body.vcpus.unwrap_or(1);
    let memory_mb = body.memory_mb.unwrap_or(512);

    if let Some(network) = body.network.as_ref()
        && let Err(error) = network.validate()
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid managed network: {error}")),
        );
    }

    #[cfg(feature = "enterprise")]
    let quota_subject = quota_subject(&state, &identity);

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

    if let Err(e) = validate_volume_specs(&body.volumes, state.volume_base_dir.as_deref()) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid volume mount: {}", e)),
        );
    }

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

    #[cfg(feature = "enterprise")]
    let quota_guard = state.quota_controller.lock().await;

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if body.network.is_some()
        && !matches!(
            requested_backend.unwrap_or_else(|| manager.backend()),
            crate::backend::BackendType::Docker | crate::backend::BackendType::Podman
        )
    {
        return json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &ApiResponse::<()>::error(
                "Managed bridge networking is supported only by Docker and Podman backends",
            ),
        );
    }

    if !body.volumes.is_empty() {
        let volume_backend = requested_backend.unwrap_or_else(|| manager.backend());
        if let Err(e) = validate_backend_volume_support(volume_backend, &body.volumes) {
            return json_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    }

    #[cfg(feature = "enterprise")]
    if let Err(error) = quota_guard.check_create(&manager, &quota_subject, vcpus, memory_mb) {
        return quota_denial(&body.name, &quota_subject, "create", error);
    }

    let create_result = match requested_backend {
        Some(backend) => {
            manager
                .create_with_backend_options(
                    backend,
                    &body.name,
                    image,
                    vcpus,
                    memory_mb,
                    None,
                    ports.clone(),
                    body.agent.clone(),
                )
                .await
        }
        None => {
            manager
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
        }
    };
    if let Err(e) = create_result {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    if !body.volumes.is_empty()
        && let Err(e) = manager.set_volumes(&body.name, &body.volumes)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set volume mounts: {}", e)),
        );
    }

    if let Some(network) = body.network.clone()
        && let Err(error) = manager.set_managed_network(&body.name, Some(network))
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set managed network: {error}")),
        );
    }

    if let Some(config_path) = state.server_config_path.as_ref()
        && let Err(e) =
            manager.set_config_path(&body.name, Some(config_path.to_string_lossy().to_string()))
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to persist server config ownership: {}", e)),
        );
    }

    // Persist ownership before the guest starts. Later starts use this value,
    // never a tenant supplied by an LLM request or a start caller.
    #[cfg(feature = "enterprise")]
    if let Err(e) = manager.set_tenant_id(&body.name, trusted_tenant) {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to persist sandbox tenant ownership: {}", e)),
        );
    }

    if let Some((tenant, user)) = trusted_owner.as_ref()
        && let Err(e) = manager.set_owner_identity(&body.name, tenant, user)
    {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set sandbox owner: {e}")),
        );
    }

    #[cfg(feature = "enterprise")]
    if let Err(error) = manager.set_owner_metadata(
        &body.name,
        Some(&quota_subject.user_id),
        Some(&quota_subject.org_id),
    ) {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to persist sandbox ownership: {error}")),
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
    if body.placeholder_secrets
        && let Err(e) = manager.set_placeholder_secrets(&body.name, true)
    {
        eprintln!("Warning: Failed to set placeholder secrets: {}", e);
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
                "backend": recorded_backend(
                    state_info.and_then(|state| state.backend),
                    manager.backend(),
                )
                .to_string(),
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
            backend: recorded_backend(
                state_info.and_then(|state| state.backend),
                manager.backend(),
            )
            .to_string(),
            ip,
            image: Some(image.to_string()),
            vcpus: Some(vcpus),
            memory_mb: Some(memory_mb),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports: port_strings,
            endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
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
            workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
            archived_at: state_info.and_then(|s| s.archived_at.clone()),
            archived_reason: state_info.and_then(|s| s.archived_reason.clone()),
            lifecycle: state_info.and_then(|s| s.lifecycle_policy.clone()),
        }),
    )
}

async fn handle_get_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;

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

    if let Err(response) = refresh_sandbox_state(&mut manager, name) {
        return response;
    }

    let sandboxes = manager.list();
    for (sandbox_name, running, backend) in &sandboxes {
        if *sandbox_name == name {
            let state_info = manager.get_state(name);
            #[cfg(feature = "enterprise")]
            if let Some(sandbox) = state_info
                && let Err(response) = require_sandbox_access(&state, &identity, sandbox)
            {
                return response;
            }
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
                    endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
                    secret_files: state_info
                        .map(|s| s.secret_files.clone())
                        .unwrap_or_default(),
                    placeholder_secrets: state_info.map(|s| s.placeholder_secrets).unwrap_or(false),
                    proxy_port: state_info.and_then(|s| s.proxy_port),
                    secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
                    labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
                    description: state_info.and_then(|s| s.description.clone()),
                    last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
                    workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
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

async fn handle_get_sandbox_by_uuid(
    req: Request<Incoming>,
    uuid: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;

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

    #[cfg(feature = "enterprise")]
    if let Err(response) = require_sandbox_access(&state, &identity, state_info) {
        return response;
    }

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
    let backend = recorded_backend(state_info.backend, manager.backend());

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
            endpoints: state_info.endpoints.clone(),
            secret_files: state_info.secret_files.clone(),
            placeholder_secrets: state_info.placeholder_secrets,
            proxy_port: state_info.proxy_port,
            secret_mappings: build_secret_mappings(state_info),
            labels: state_info.labels.clone(),
            description: state_info.description.clone(),
            last_activity_at: state_info.last_activity_at.clone(),
            workspace_revision: state_info.workspace_revision.clone(),
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

// --- Sandbox Git handlers ---

const DEFAULT_GIT_PATH: &str = "/workspace";

fn validate_git_path(path: &str) -> anyhow::Result<String> {
    if path.is_empty() {
        anyhow::bail!("Git repository path is required");
    }
    if path.len() > 1024 {
        anyhow::bail!("Git repository path is too long (max 1024 characters)");
    }
    if path
        .chars()
        .any(|ch| ch == '\0' || ch == '\n' || ch == '\r' || ch == '\\')
    {
        anyhow::bail!("Git repository path contains invalid control or separator characters");
    }

    if path.starts_with('/') {
        validation::validate_exec_workdir(path)?;
        crate::backend::validate_sandbox_path(path)?;
        return Ok(path.to_string());
    }

    // Daytona accepts paths relative to the sandbox working directory.  Keep
    // that convenience while making the resulting path unambiguously sandbox
    // scoped and rejecting traversal/option-like values.
    if path.starts_with('-') {
        anyhow::bail!("Git repository path cannot start with '-'");
    }
    let relative = path.strip_prefix("./").unwrap_or(path);
    let relative = relative.strip_prefix("workspace/").unwrap_or(relative);
    let relative = relative.trim_end_matches('/');
    if relative.is_empty() || relative == "." {
        return Ok(DEFAULT_GIT_PATH.to_string());
    }
    let relative_path = std::path::Path::new(relative);
    if relative_path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        anyhow::bail!("Git repository path cannot contain parent directory references");
    }
    let resolved = format!("{DEFAULT_GIT_PATH}/{relative}");
    validation::validate_exec_workdir(&resolved)?;
    crate::backend::validate_sandbox_path(&resolved)?;
    Ok(resolved)
}

fn validate_git_file_path(path: &str) -> anyhow::Result<()> {
    if path.is_empty() {
        anyhow::bail!("Git file path cannot be empty");
    }
    if path.len() > 4096 {
        anyhow::bail!("Git file path is too long (max 4096 characters)");
    }
    if path.starts_with('-') || path.starts_with('/') {
        anyhow::bail!("Git file paths must be relative and cannot start with '-'");
    }
    if path
        .chars()
        .any(|ch| ch == '\0' || ch == '\n' || ch == '\r' || ch == '\\')
    {
        anyhow::bail!("Git file path contains invalid control or separator characters");
    }
    if std::path::Path::new(path)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!("Git file path cannot contain parent directory references");
    }
    Ok(())
}

fn query_param(query: &str, name: &str) -> anyhow::Result<String> {
    let mut value = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, raw) = pair
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid query parameter"))?;
        let key = urlencoding::decode(key)
            .map_err(|_| anyhow::anyhow!("Invalid query parameter encoding"))?;
        let decoded = urlencoding::decode(raw)
            .map_err(|_| anyhow::anyhow!("Invalid query parameter encoding"))?;
        if key != name {
            anyhow::bail!(
                "Unsupported query parameter '{}'; only 'path' is accepted",
                key
            );
        }
        if value.replace(decoded.into_owned()).is_some() {
            anyhow::bail!("Query parameter '{}' may only be specified once", name);
        }
    }
    value.ok_or_else(|| anyhow::anyhow!("Query parameter 'path' is required"))
}

fn git_validation_error(error: impl std::fmt::Display) -> Response<BoxBody> {
    json_response(
        StatusCode::BAD_REQUEST,
        &ApiResponse::<()>::error(error.to_string()),
    )
}

fn validate_git_name(name: &str) -> Result<(), Box<Response<BoxBody>>> {
    validation::validate_sandbox_name(name).map_err(|error| Box::new(git_validation_error(error)))
}

fn manager_error(error: anyhow::Error) -> Response<BoxBody> {
    json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        &ApiResponse::<()>::error(error.to_string()),
    )
}

fn ensure_git_sandbox(manager: &VmManager, name: &str) -> Result<(), Box<Response<BoxBody>>> {
    if !manager.exists(name) {
        return Err(Box::new(json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
        )));
    }
    if !manager.is_running(name) {
        return Err(Box::new(json_response(
            StatusCode::CONFLICT,
            &ApiResponse::<()>::error(format!("Sandbox '{}' is not running", name)),
        )));
    }
    Ok(())
}

async fn run_git(
    manager: &mut VmManager,
    name: &str,
    path: &str,
    args: &[String],
) -> anyhow::Result<String> {
    let mut command = Vec::with_capacity(args.len() + 1);
    command.push("git".to_string());
    command.extend(args.iter().cloned());
    manager
        .exec_cmd_full(
            name,
            &command,
            &crate::backend::ExecOptions {
                workdir: Some(path.to_string()),
                ..Default::default()
            },
        )
        .await
}

fn git_command_error(operation: &str, error: &anyhow::Error) -> Response<BoxBody> {
    let detail = if let Some(command_error) = error.downcast_ref::<crate::vmm::CommandFailed>() {
        format!(
            "git {} failed (exit code {}): {}",
            operation,
            command_error.exit_code,
            command_error.output.trim()
        )
    } else {
        format!("git {} failed: {}", operation, error)
    };
    json_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        &ApiResponse::<()>::error(detail),
    )
}

fn parse_git_status(output: &str) -> GitStatusResponse {
    let mut current_branch = String::new();
    let mut upstream = None;
    let mut detached = false;
    let mut ahead = 0;
    let mut behind = 0;
    let mut file_status = Vec::new();

    for (line_number, line) in output.lines().enumerate() {
        if line_number == 0 && line.starts_with("## ") {
            let header = &line[3..];
            let (branch, tracking) = header.split_once("...").unwrap_or((header, ""));
            let branch = branch.strip_prefix("No commits yet on ").unwrap_or(branch);
            if branch.starts_with("HEAD (") || branch == "HEAD" {
                detached = true;
            } else {
                current_branch = branch.to_string();
            }
            if !tracking.is_empty() {
                let tracking = tracking
                    .split_once(" [")
                    .map(|(remote, _)| remote)
                    .unwrap_or(tracking);
                if !tracking.is_empty() {
                    upstream = Some(tracking.to_string());
                }
            }
            if let Some((_, counts)) = header.split_once(" [") {
                let counts = counts.trim_end_matches(']');
                for item in counts.split(", ") {
                    if let Some(value) = item.strip_prefix("ahead ") {
                        ahead = value.parse().unwrap_or(0);
                    } else if let Some(value) = item.strip_prefix("behind ") {
                        behind = value.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        if line.len() < 3 {
            continue;
        }
        let bytes = line.as_bytes();
        let staging = git_file_status(bytes[0]);
        let worktree = git_file_status(bytes[1]);
        let raw_name = &line[3..];
        let (extra, name) = raw_name
            .split_once(" -> ")
            .map(|(old, new)| (old.to_string(), new.to_string()))
            .unwrap_or_else(|| (String::new(), raw_name.to_string()));
        file_status.push(GitFileStatus {
            name,
            extra,
            staging: staging.to_string(),
            worktree: worktree.to_string(),
        });
    }

    GitStatusResponse {
        current_branch,
        file_status,
        branch_published: upstream.is_some(),
        ahead,
        behind,
        upstream,
        detached,
    }
}

fn git_file_status(status: u8) -> &'static str {
    match status as char {
        'M' => "Modified",
        'A' => "Added",
        'D' => "Deleted",
        'R' => "Renamed",
        'C' => "Copied",
        'U' => "Updated but unmerged",
        '?' => "Untracked",
        _ => "Unmodified",
    }
}

fn validate_git_ref(value: &str, label: &str) -> anyhow::Result<()> {
    if value.is_empty() {
        anyhow::bail!("{} cannot be empty", label);
    }
    validation::validate_git_ref(value)
        .map_err(|error| anyhow::anyhow!("Invalid {}: {}", label, error))
}

fn reject_git_credentials(request: &GitRepoRequest) -> anyhow::Result<()> {
    if request
        .username
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || request
            .password
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    {
        anyhow::bail!(
            "Git username/password authentication is not supported; configure credentials inside the sandbox"
        );
    }
    Ok(())
}

async fn handle_git_status(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(response) = validate_git_name(name) {
        return *response;
    }
    let path = match query_param(req.uri().query().unwrap_or(""), "path")
        .and_then(|path| validate_git_path(&path))
    {
        Ok(path) => path,
        Err(error) => return git_validation_error(error),
    };
    let mut manager = match state.get_manager().await {
        Ok(manager) => manager,
        Err(error) => return manager_error(error),
    };
    if let Err(response) = ensure_git_sandbox(&manager, name) {
        return *response;
    }
    match run_git(
        &mut manager,
        name,
        &path,
        &[
            "status".to_string(),
            "--porcelain=v1".to_string(),
            "--branch".to_string(),
            "--ahead-behind".to_string(),
        ],
    )
    .await
    {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(parse_git_status(&output)),
        ),
        Err(error) => git_command_error("status", &error),
    }
}

async fn handle_git_branches(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(response) = validate_git_name(name) {
        return *response;
    }
    let path = match query_param(req.uri().query().unwrap_or(""), "path")
        .and_then(|path| validate_git_path(&path))
    {
        Ok(path) => path,
        Err(error) => return git_validation_error(error),
    };
    let mut manager = match state.get_manager().await {
        Ok(manager) => manager,
        Err(error) => return manager_error(error),
    };
    if let Err(response) = ensure_git_sandbox(&manager, name) {
        return *response;
    }
    let branches = match run_git(
        &mut manager,
        name,
        &path,
        &[
            "for-each-ref".to_string(),
            "--format=%(refname:short)".to_string(),
            "refs/heads".to_string(),
        ],
    )
    .await
    {
        Ok(output) => output.lines().map(str::to_string).collect(),
        Err(error) => return git_command_error("branches", &error),
    };
    let current = match run_git(
        &mut manager,
        name,
        &path,
        &["branch".to_string(), "--show-current".to_string()],
    )
    .await
    {
        Ok(output) => output.trim().to_string(),
        Err(error) => return git_command_error("branches", &error),
    };
    json_response(
        StatusCode::OK,
        &ApiResponse::success(GitBranchesResponse { branches, current }),
    )
}

async fn handle_git_add(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(response) = validate_git_name(name) {
        return *response;
    }
    let body: GitAddRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let path = match validate_git_path(&body.path) {
        Ok(path) => path,
        Err(error) => return git_validation_error(error),
    };
    if body.files.is_empty() {
        return git_validation_error("files must contain at least one path");
    }
    for file in &body.files {
        if let Err(error) = validate_git_file_path(file) {
            return git_validation_error(error);
        }
    }
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(body.files);
    let mut manager = match state.get_manager().await {
        Ok(manager) => manager,
        Err(error) => return manager_error(error),
    };
    if let Err(response) = ensure_git_sandbox(&manager, name) {
        return *response;
    }
    match run_git(&mut manager, name, &path, &args).await {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(GitOperationResponse { output }),
        ),
        Err(error) => git_command_error("add", &error),
    }
}

async fn handle_git_commit(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    if let Err(response) = validate_git_name(name) {
        return *response;
    }
    let body: GitCommitRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let path = match validate_git_path(&body.path) {
        Ok(path) => path,
        Err(error) => return git_validation_error(error),
    };
    if body.message.trim().is_empty()
        || body
            .message
            .chars()
            .any(|ch| ch == '\0' || ch == '\n' || ch == '\r')
    {
        return git_validation_error(
            "commit message must be non-empty and cannot contain control characters",
        );
    }
    if body.author.is_some() != body.email.is_some() {
        return git_validation_error("author and email must be provided together");
    }
    if let Some(author) = body.author.as_ref()
        && (author.trim().is_empty()
            || author
                .chars()
                .any(|ch| ch == '\0' || ch == '\n' || ch == '\r' || ch == '<' || ch == '>'))
    {
        return git_validation_error("author contains invalid characters");
    }
    if let Some(email) = body.email.as_ref()
        && (email.trim().is_empty()
            || email.chars().any(|ch| {
                ch.is_ascii_control() || ch.is_ascii_whitespace() || ch == '<' || ch == '>'
            }))
    {
        return git_validation_error("email contains invalid characters");
    }
    let mut args = vec!["commit".to_string()];
    if body.allow_empty {
        args.push("--allow-empty".to_string());
    }
    if let (Some(author), Some(email)) = (body.author, body.email) {
        args.extend(["--author".to_string(), format!("{} <{}>", author, email)]);
    }
    args.extend(["-m".to_string(), body.message]);
    let mut manager = match state.get_manager().await {
        Ok(manager) => manager,
        Err(error) => return manager_error(error),
    };
    if let Err(response) = ensure_git_sandbox(&manager, name) {
        return *response;
    }
    if let Err(error) = run_git(&mut manager, name, &path, &args).await {
        return git_command_error("commit", &error);
    }
    match run_git(
        &mut manager,
        name,
        &path,
        &["rev-parse".to_string(), "HEAD".to_string()],
    )
    .await
    {
        Ok(hash) => json_response(
            StatusCode::OK,
            &ApiResponse::success(GitCommitResponse {
                hash: hash.trim().to_string(),
            }),
        ),
        Err(error) => git_command_error("rev-parse", &error),
    }
}

async fn handle_git_pull(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    handle_git_remote_operation(req, name, state, false).await
}

async fn handle_git_push(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    handle_git_remote_operation(req, name, state, true).await
}

async fn handle_git_remote_operation(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
    push: bool,
) -> Response<BoxBody> {
    if let Err(response) = validate_git_name(name) {
        return *response;
    }
    let body: GitRepoRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let path = match validate_git_path(&body.path) {
        Ok(path) => path,
        Err(error) => return git_validation_error(error),
    };
    if let Err(error) = reject_git_credentials(&body) {
        return git_validation_error(error);
    }
    if let Some(remote) = body.remote.as_ref()
        && let Err(error) = validate_git_ref(remote, "remote")
    {
        return git_validation_error(error);
    }
    if let Some(branch) = body.branch.as_ref()
        && let Err(error) = validate_git_ref(branch, "branch")
    {
        return git_validation_error(error);
    }
    if body.branch.is_some() && body.remote.is_none() {
        return git_validation_error("remote is required when branch is provided");
    }
    if body.set_upstream && (!push || body.remote.is_none() || body.branch.is_none()) {
        return git_validation_error("set_upstream requires push with both remote and branch");
    }
    let mut args = vec![if push { "push" } else { "pull" }.to_string()];
    if body.set_upstream {
        args.push("--set-upstream".to_string());
    }
    if let Some(remote) = body.remote {
        args.push(remote);
    }
    if let Some(branch) = body.branch {
        args.push(branch);
    }
    let operation = if push { "push" } else { "pull" };
    let mut manager = match state.get_manager().await {
        Ok(manager) => manager,
        Err(error) => return manager_error(error),
    };
    if let Err(response) = ensure_git_sandbox(&manager, name) {
        return *response;
    }
    match run_git(&mut manager, name, &path, &args).await {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(GitOperationResponse { output }),
        ),
        Err(error) => git_command_error(operation, &error),
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

async fn handle_delete_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Enterprise policy enforcement (reuse Create action for lifecycle operations)
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;
    #[cfg(feature = "enterprise")]
    if let Err(response) =
        enforce_policy(&state, &identity, crate::policy::Action::Create, name).await
    {
        return response;
    }

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    #[cfg(feature = "enterprise")]
    let _quota_guard = state.quota_controller.lock().await;

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if let Err(response) = refresh_sandbox_state(&mut manager, name) {
        return response;
    }
    if !manager.exists(name) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Sandbox not found"),
        );
    }

    #[cfg(feature = "enterprise")]
    if let Some(sandbox) = manager.get_state(name)
        && let Err(response) = require_sandbox_access(&state, &identity, sandbox)
    {
        return response;
    }

    // Capture labels before removal (remove destroys the state)
    let event_labels = manager
        .get_state(name)
        .map(|s| s.labels.clone())
        .unwrap_or_default();
    let expected_binding = manager
        .get_state(name)
        .map(PersistedStartBinding::from_state);
    let manager_handle = match state.manager_handle() {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    drop(manager);
    #[cfg(feature = "enterprise")]
    drop(_quota_guard);
    let name = name.to_string();
    let task_state = Arc::clone(&state);
    let task = tokio::spawn(async move {
        #[cfg(feature = "enterprise")]
        let _quota_guard = task_state.quota_controller.lock().await;
        let mut manager = manager_handle.write().await;
        if manager
            .get_state(&name)
            .map(PersistedStartBinding::from_state)
            != expected_binding
        {
            return json_response(
                StatusCode::CONFLICT,
                &ApiResponse::<()>::error(
                    "Sandbox identity or ownership changed before the server-owned removal began",
                ),
            );
        }
        match manager.remove(&name).await {
            Ok(_) => {
                if let Err(error) =
                    remove_persisted_start_configurations_for_sandbox(manager.get_data_dir(), &name)
                {
                    return json_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &ApiResponse::<()>::error(format!(
                            "Sandbox was removed, but its pending start configuration could not be scrubbed: {error}"
                        )),
                    );
                }
                crate::events::emit(
                    task_state.event_bus.as_ref(),
                    crate::events::SandboxEvent {
                        event: "sandbox.deleted".to_string(),
                        timestamp: chrono::Utc::now(),
                        sandbox: name,
                        labels: event_labels,
                        metadata: serde_json::json!({}),
                    },
                );
                json_response(StatusCode::OK, &ApiResponse::success("Sandbox removed"))
            }
            Err(error) => sandbox_lifecycle_error("remove", error),
        }
    });
    await_server_owned_lifecycle("remove", task).await
}

async fn handle_start_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Enterprise policy enforcement
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    // Bind the private handoff to the exact bearer presented by the local CLI.
    // This works for both API keys and JWTs without persisting either secret.
    let request_owner_id = start_request_owner_id(&req);
    #[cfg(not(feature = "enterprise"))]
    let trusted_start_owner = trusted_owner_identity(&req, &state);

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body = match read_body_bytes(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let start_request = match parse_start_sandbox_request(&body) {
        Ok(request) => request,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("{error:#}")),
            );
        }
    };
    #[cfg(feature = "enterprise")]
    {
        if let Err(response) =
            enforce_policy(&state, &identity, crate::policy::Action::Run, name).await
        {
            return response;
        }
        if start_request.configuration.is_some()
            && let Err(response) =
                enforce_policy(&state, &identity, crate::policy::Action::Create, name).await
        {
            return response;
        }
    }
    #[cfg(feature = "enterprise")]
    let quota_guard = state.quota_controller.lock().await;

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if let Err(response) = refresh_sandbox_state(&mut manager, name) {
        return response;
    }
    let start_binding = match manager.get_state(name) {
        Some(sandbox) => PersistedStartBinding::from_state(sandbox),
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error("Sandbox not found"),
            );
        }
    };
    let token_authorizes_first_claim = start_request.configuration.is_some();
    let (permissions, files, expected_state_sha256) =
        match start_request.into_runtime(manager.get_data_dir(), &start_binding, &request_owner_id)
        {
            Ok(runtime) => runtime,
            Err(error) => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &ApiResponse::<()>::error(error.to_string()),
                );
            }
        };

    if let Some(expected_state_sha256) = expected_state_sha256 {
        if let Err(error) = manager.refresh_stopped_sandbox_from_disk(name) {
            return json_response(
                StatusCode::CONFLICT,
                &ApiResponse::<()>::error(format!(
                    "Failed to adopt the final persisted sandbox configuration: {error}"
                )),
            );
        }
        let actual_state_sha256 = match manager.get_state(name).map(sandbox_state_sha256) {
            Some(Ok(hash)) => hash,
            Some(Err(error)) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(error.to_string()),
                );
            }
            None => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &ApiResponse::<()>::error("Sandbox not found"),
                );
            }
        };
        if actual_state_sha256 != expected_state_sha256 {
            return json_response(
                StatusCode::CONFLICT,
                &ApiResponse::<()>::error(
                    "Persisted sandbox configuration changed after the start handoff was created",
                ),
            );
        }
    }

    #[cfg(feature = "enterprise")]
    if let Err(response) = claim_unowned_start_sandbox(
        &mut manager,
        name,
        token_authorizes_first_claim,
        &identity,
        &state,
    ) {
        return response;
    }
    #[cfg(not(feature = "enterprise"))]
    if let Err(response) = claim_unowned_local_start_sandbox(
        &mut manager,
        name,
        token_authorizes_first_claim,
        trusted_start_owner.as_ref(),
    ) {
        return response;
    }

    #[cfg(feature = "enterprise")]
    if let Some(sandbox) = manager.get_state(name)
        && let Err(response) = require_sandbox_access(&state, &identity, sandbox)
    {
        return response;
    }

    #[cfg(feature = "enterprise")]
    if let Err(error) = quota_guard.check_start(&manager, name) {
        let subject = manager
            .get_state(name)
            .map(|sandbox| crate::quota::QuotaSubject {
                user_id: sandbox
                    .owner_user_id
                    .clone()
                    .unwrap_or_else(|| "anonymous".to_string()),
                org_id: sandbox
                    .owner_org_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
            })
            .unwrap_or(crate::quota::QuotaSubject {
                user_id: "anonymous".to_string(),
                org_id: "default".to_string(),
            });
        return quota_denial(name, &subject, "start", error);
    }

    let expected_binding = manager
        .get_state(name)
        .map(PersistedStartBinding::from_state)
        .expect("sandbox presence was validated before server-owned start");
    let manager_handle = match state.manager_handle() {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    drop(manager);
    #[cfg(feature = "enterprise")]
    drop(quota_guard);
    let name = name.to_string();
    #[cfg(feature = "enterprise")]
    let task_state = Arc::clone(&state);
    let task = tokio::spawn(async move {
        #[cfg(feature = "enterprise")]
        let quota_guard = task_state.quota_controller.lock().await;
        let mut manager = manager_handle.write().await;
        let Some(current) = manager.get_state(&name) else {
            return json_response(
                StatusCode::NOT_FOUND,
                &ApiResponse::<()>::error("Sandbox not found"),
            );
        };
        if PersistedStartBinding::from_state(current) != expected_binding {
            return json_response(
                StatusCode::CONFLICT,
                &ApiResponse::<()>::error(
                    "Sandbox identity or ownership changed before the server-owned start began",
                ),
            );
        }
        #[cfg(feature = "enterprise")]
        if let Err(error) = quota_guard.check_start(&manager, &name) {
            let subject = manager
                .get_state(&name)
                .map(|sandbox| crate::quota::QuotaSubject {
                    user_id: sandbox
                        .owner_user_id
                        .clone()
                        .unwrap_or_else(|| "anonymous".to_string()),
                    org_id: sandbox
                        .owner_org_id
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                })
                .unwrap_or(crate::quota::QuotaSubject {
                    user_id: "anonymous".to_string(),
                    org_id: "default".to_string(),
                });
            return quota_denial(&name, &subject, "start", error);
        }
        match manager
            .start_with_permissions_and_files_authorized(&name, &permissions, &files)
            .await
        {
            Ok(_) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox started")),
            Err(error) => sandbox_lifecycle_error("start", error),
        }
    });
    await_server_owned_lifecycle("start", task).await
}

async fn handle_stop_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;
    #[cfg(feature = "enterprise")]
    {
        if let Err(resp) = enforce_policy(&state, &identity, crate::policy::Action::Run, name).await
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

    #[cfg(feature = "enterprise")]
    let _quota_guard = state.quota_controller.lock().await;

    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    #[cfg(feature = "enterprise")]
    if let Some(sandbox) = manager.get_state(name)
        && let Err(response) = require_sandbox_access(&state, &identity, sandbox)
    {
        return response;
    }

    let expected_binding = manager
        .get_state(name)
        .map(PersistedStartBinding::from_state);
    let manager_handle = match state.manager_handle() {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    drop(manager);
    #[cfg(feature = "enterprise")]
    drop(_quota_guard);
    let name = name.to_string();
    #[cfg(feature = "enterprise")]
    let task_state = Arc::clone(&state);
    let task = tokio::spawn(async move {
        #[cfg(feature = "enterprise")]
        let _quota_guard = task_state.quota_controller.lock().await;
        let mut manager = manager_handle.write().await;
        if manager
            .get_state(&name)
            .map(PersistedStartBinding::from_state)
            != expected_binding
        {
            return json_response(
                StatusCode::CONFLICT,
                &ApiResponse::<()>::error(
                    "Sandbox identity or ownership changed before the server-owned stop began",
                ),
            );
        }
        match manager.stop(&name).await {
            Ok(_) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox stopped")),
            Err(error) => sandbox_lifecycle_error("stop", error),
        }
    });
    await_server_owned_lifecycle("stop", task).await
}

#[allow(clippy::result_large_err)]
fn require_full_state_firecracker(
    manager: &VmManager,
    name: &str,
) -> Result<(), Response<BoxBody>> {
    let Some(sandbox) = manager.get_state(name) else {
        return Err(json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Sandbox not found"),
        ));
    };
    let backend = recorded_backend(sandbox.backend, manager.backend());
    if backend != BackendType::Firecracker {
        return Err(json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &ApiResponse::<()>::error(format!(
                "Backend '{}' does not support full-state pause, resume, or fork; use the Firecracker backend",
                backend
            )),
        ));
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn refresh_sandbox_state(manager: &mut VmManager, name: &str) -> Result<bool, Response<BoxBody>> {
    manager.refresh_sandbox_if_missing(name).map_err(|error| {
        json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!(
                "Failed to refresh sandbox state for '{name}': {error}"
            )),
        )
    })
}

#[cfg(feature = "enterprise")]
#[allow(clippy::result_large_err)]
fn claim_unowned_start_sandbox(
    manager: &mut VmManager,
    name: &str,
    token_authorizes_first_claim: bool,
    identity: &crate::identity::AgentIdentity,
    state: &AppState,
) -> Result<(), Response<BoxBody>> {
    // A refresh is not an authorization event: the server may have loaded the
    // same unowned state during startup or a prior GET. Only a valid, consumed
    // local start token bound to this UUID/generation may authorize first claim.
    let Some(sandbox) = manager.get_state(name) else {
        return Ok(());
    };
    if sandbox.owner_user_id.is_some() || sandbox.owner_org_id.is_some() {
        return Ok(());
    }
    if !token_authorizes_first_claim {
        return Err(sandbox_access_denied());
    }

    let trusted_identity = trusted_owner_identity(identity, state).is_some()
        || (!identity.is_authenticated() && state.api_keys.is_empty());
    if !trusted_identity {
        return Err(json_response(
            StatusCode::FORBIDDEN,
            &ApiResponse::<()>::error(
                "A trusted identity is required to claim a CLI-created sandbox",
            ),
        ));
    }

    let subject = quota_subject(state, identity);
    manager
        .set_trusted_ownership(
            name,
            trusted_tenant_for_sandbox(identity, state),
            Some(&subject.user_id),
            Some(&subject.org_id),
        )
        .map_err(|error| {
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!(
                    "Failed to atomically persist refreshed sandbox ownership: {error}"
                )),
            )
        })
}

#[cfg(not(feature = "enterprise"))]
#[allow(clippy::result_large_err)]
fn claim_unowned_local_start_sandbox(
    manager: &mut VmManager,
    name: &str,
    token_authorizes_first_claim: bool,
    trusted_owner: Option<&(String, String)>,
) -> Result<(), Response<BoxBody>> {
    if !token_authorizes_first_claim {
        return Ok(());
    }
    let Some(sandbox) = manager.get_state(name) else {
        return Ok(());
    };
    if sandbox.owner_user_id.is_some() || sandbox.owner_org_id.is_some() {
        return Ok(());
    }
    let Some((tenant, user)) = trusted_owner else {
        // With no configured API authentication there is no durable identity
        // to stamp; the unowned local-only behavior remains unchanged.
        return Ok(());
    };
    manager
        .set_owner_identity(name, tenant, user)
        .map_err(|error| {
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!(
                    "Failed to persist refreshed sandbox ownership: {error}"
                )),
            )
        })
}

fn sandbox_lifecycle_error(operation: &str, error: anyhow::Error) -> Response<BoxBody> {
    // Full-state operations reject proxy-backed secret workloads before they
    // reach the backend. Their remaining error chains contain lifecycle and
    // checkpoint diagnostics (including actionable recovery paths), not secret
    // values, so preserve the complete anyhow context for operators.
    let message = format!("{error:#}");
    let normalized = message.to_ascii_lowercase();
    let status = if normalized.contains("not found") {
        StatusCode::NOT_FOUND
    } else if normalized.contains("not support")
        || normalized.contains("unsupported")
        || normalized.contains("requires firecracker")
        || normalized.contains("requires linux x86_64")
        || normalized.contains("firecracker backend")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if normalized.contains("already")
        || normalized.contains("must be")
        || normalized.contains("not running")
        || normalized.contains("not paused")
        || normalized.contains("is paused")
        || normalized.contains("cold start")
        || normalized.contains("refusing to stop")
        || normalized.contains("cannot pause")
        || normalized.contains("cannot resume")
        || normalized.contains("cannot fork")
    {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    json_response(
        status,
        &ApiResponse::<()>::error(format!("Failed to {operation} sandbox: {message}")),
    )
}

async fn await_server_owned_lifecycle(
    operation: &'static str,
    task: tokio::task::JoinHandle<Response<BoxBody>>,
) -> Response<BoxBody> {
    match task.await {
        Ok(response) => response,
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!(
                "Server-owned {operation} task failed unexpectedly: {error}"
            )),
        ),
    }
}

#[cfg(feature = "enterprise")]
const FULL_STATE_RUNTIME_POLICY_ACTION: crate::policy::Action = crate::policy::Action::Run;

#[cfg(feature = "enterprise")]
const FORK_SOURCE_POLICY_ACTION: crate::policy::Action = FULL_STATE_RUNTIME_POLICY_ACTION;

#[cfg(feature = "enterprise")]
const FORK_CHILD_POLICY_ACTIONS: [crate::policy::Action; 2] =
    [crate::policy::Action::Create, crate::policy::Action::Run];

fn fork_identity_matches_source(
    source: &crate::vmm::SandboxState,
    tenant_id: Option<&str>,
    owner_user_id: Option<&str>,
    owner_org_id: Option<&str>,
) -> bool {
    source.tenant_id.as_deref() == tenant_id
        && source.owner_user_id.as_deref() == owner_user_id
        && source.owner_org_id.as_deref() == owner_org_id
}

#[allow(clippy::result_large_err)]
fn sandbox_info_from_manager(
    manager: &VmManager,
    name: &str,
) -> Result<SandboxInfo, Response<BoxBody>> {
    let Some(state_info) = manager.get_state(name) else {
        return Err(json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Sandbox not found"),
        ));
    };
    let running = manager.is_running(name);
    let ip = running.then(|| manager.get_container_ip(name)).flatten();
    Ok(SandboxInfo {
        name: name.to_string(),
        uuid: state_info.uuid.clone(),
        status: state_info.status(running).to_string(),
        backend: recorded_backend(state_info.backend, manager.backend()).to_string(),
        ip,
        image: Some(state_info.image.clone()),
        vcpus: Some(state_info.vcpus),
        memory_mb: Some(state_info.memory_mb),
        created_at: Some(state_info.created_at.clone()),
        created_from_template: state_info.created_from_template.clone(),
        template_help_text: state_info.template_help_text.clone(),
        ports: state_info
            .ports
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        endpoints: state_info.endpoints.clone(),
        secret_files: state_info.secret_files.clone(),
        placeholder_secrets: state_info.placeholder_secrets,
        proxy_port: state_info.proxy_port,
        secret_mappings: build_secret_mappings(state_info),
        labels: state_info.labels.clone(),
        description: state_info.description.clone(),
        last_activity_at: state_info.last_activity_at.clone(),
        workspace_revision: state_info.workspace_revision.clone(),
        archived_at: state_info.archived_at.clone(),
        archived_reason: state_info.archived_reason.clone(),
        lifecycle: state_info.lifecycle_policy.clone(),
    })
}

async fn handle_pause_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;

    if let Err(error) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }

    #[cfg(feature = "enterprise")]
    if let Err(response) =
        enforce_policy(&state, &identity, FULL_STATE_RUNTIME_POLICY_ACTION, name).await
    {
        return response;
    }

    let manager = match state.manager_handle() {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    let name = name.to_string();
    #[cfg(feature = "enterprise")]
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        // Pause changes running quota usage. Keep serialization in the task so
        // it remains held even if the HTTP waiter disconnects.
        #[cfg(feature = "enterprise")]
        let _quota_guard = task_state.quota_controller.lock().await;
        let mut manager = manager.write().await;
        if let Err(response) = refresh_sandbox_state(&mut manager, &name) {
            return response;
        }
        if let Err(response) = require_full_state_firecracker(&manager, &name) {
            return response;
        }
        #[cfg(feature = "enterprise")]
        if let Some(sandbox) = manager.get_state(&name)
            && let Err(response) = require_sandbox_access(&task_state, &identity, sandbox)
        {
            return response;
        }
        match manager.pause_authorized(&name).await {
            Ok(_) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox paused")),
            Err(error) => sandbox_lifecycle_error("pause", error),
        }
    });
    await_server_owned_lifecycle("pause", task).await
}

async fn handle_resume_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;

    if let Err(error) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }

    #[cfg(feature = "enterprise")]
    if let Err(response) =
        enforce_policy(&state, &identity, FULL_STATE_RUNTIME_POLICY_ACTION, name).await
    {
        return response;
    }

    let manager = match state.manager_handle() {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    let name = name.to_string();
    #[cfg(feature = "enterprise")]
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        #[cfg(feature = "enterprise")]
        let quota_guard = task_state.quota_controller.lock().await;
        let mut manager = manager.write().await;
        if let Err(response) = refresh_sandbox_state(&mut manager, &name) {
            return response;
        }
        if let Err(response) = require_full_state_firecracker(&manager, &name) {
            return response;
        }
        #[cfg(feature = "enterprise")]
        if let Some(sandbox) = manager.get_state(&name)
            && let Err(response) = require_sandbox_access(&task_state, &identity, sandbox)
        {
            return response;
        }
        #[cfg(feature = "enterprise")]
        if let Err(error) = quota_guard.check_start(&manager, &name) {
            let subject = manager
                .get_state(&name)
                .map(|sandbox| crate::quota::QuotaSubject {
                    user_id: sandbox
                        .owner_user_id
                        .clone()
                        .unwrap_or_else(|| "anonymous".to_string()),
                    org_id: sandbox
                        .owner_org_id
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                })
                .unwrap_or(crate::quota::QuotaSubject {
                    user_id: "anonymous".to_string(),
                    org_id: "default".to_string(),
                });
            return quota_denial(&name, &subject, "resume", error);
        }
        match manager.resume_authorized(&name).await {
            Ok(()) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox resumed")),
            Err(error) => sandbox_lifecycle_error("resume", error),
        }
    });
    await_server_owned_lifecycle("resume", task).await
}

async fn handle_fork_sandbox(
    req: Request<Incoming>,
    source: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let trusted_owner = trusted_owner_identity(&req, &state);
    #[cfg(feature = "enterprise")]
    let trusted_tenant = trusted_tenant_for_sandbox(&identity, &state);

    if let Err(error) = validation::validate_sandbox_name(source) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }

    let body: ForkSandboxRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    if let Err(error) = validation::validate_sandbox_name(&body.as_name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid fork name: {error}")),
        );
    }

    #[cfg(feature = "enterprise")]
    {
        if let Err(response) =
            enforce_policy(&state, &identity, FORK_SOURCE_POLICY_ACTION, source).await
        {
            return response;
        }
        for action in FORK_CHILD_POLICY_ACTIONS {
            if let Err(response) = enforce_policy(&state, &identity, action, &body.as_name).await {
                return response;
            }
        }
    }

    #[cfg(feature = "enterprise")]
    let quota_subject = quota_subject(&state, &identity);
    let manager = match state.manager_handle() {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    let source = source.to_string();
    let child = body.as_name;
    #[cfg(feature = "enterprise")]
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        #[cfg(feature = "enterprise")]
        let quota_guard = task_state.quota_controller.lock().await;
        let mut manager = manager.write().await;
        if let Err(response) = refresh_sandbox_state(&mut manager, &source) {
            return response;
        }
        if let Err(response) = refresh_sandbox_state(&mut manager, &child) {
            return response;
        }
        if let Err(response) = require_full_state_firecracker(&manager, &source) {
            return response;
        }
        if manager.exists(&child) {
            return json_response(
                StatusCode::CONFLICT,
                &ApiResponse::<()>::error(format!("Sandbox '{child}' already exists")),
            );
        }
        #[cfg(feature = "enterprise")]
        if let Some(sandbox) = manager.get_state(&source)
            && let Err(response) = require_sandbox_access(&task_state, &identity, sandbox)
        {
            return response;
        }
        #[cfg(feature = "enterprise")]
        if let Some(source_state) = manager.get_state(&source)
            && let Err(error) = quota_guard.check_create(
                &manager,
                &quota_subject,
                source_state.vcpus,
                source_state.memory_mb,
            )
        {
            return quota_denial(&child, &quota_subject, "fork", error);
        }

        let source_state = match manager.get_state(&source) {
            Some(source_state) => source_state,
            None => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &ApiResponse::<()>::error("Sandbox not found"),
                );
            }
        };
        #[cfg(feature = "enterprise")]
        let requested_identity = (
            trusted_tenant.as_deref(),
            Some(quota_subject.user_id.as_str()),
            Some(quota_subject.org_id.as_str()),
        );
        #[cfg(not(feature = "enterprise"))]
        let requested_identity = (
            source_state.tenant_id.as_deref(),
            trusted_owner.as_ref().map(|(_, user)| user.as_str()),
            trusted_owner.as_ref().map(|(tenant, _)| tenant.as_str()),
        );
        if !fork_identity_matches_source(
            source_state,
            requested_identity.0,
            requested_identity.1,
            requested_identity.2,
        ) {
            return json_response(
                StatusCode::FORBIDDEN,
                &ApiResponse::<()>::error(
                    "Forking across sandbox owner or tenant boundaries is not supported",
                ),
            );
        }

        if let Err(error) = manager.fork_sandbox_authorized(&source, &child).await {
            return sandbox_lifecycle_error("fork", error);
        }
        let sandbox = match sandbox_info_from_manager(&manager, &child) {
            Ok(sandbox) => sandbox,
            Err(response) => return response,
        };
        json_response(
            StatusCode::CREATED,
            &ApiResponse::success(ForkSandboxResult {
                sandbox,
                security_warning: crate::full_state::FORK_SECURITY_WARNING.to_string(),
            }),
        )
    });
    await_server_owned_lifecycle("fork", task).await
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
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(feature = "enterprise")]
    if let Err(response) =
        enforce_policy(&state, &identity, crate::policy::Action::Create, name).await
    {
        return response;
    }

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

    if let Err(response) = refresh_sandbox_state(&mut manager, name) {
        return response;
    }

    // Check if sandbox exists
    if !manager.exists(name) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Sandbox '{}' not found", name)),
        );
    }

    #[cfg(feature = "enterprise")]
    if let Some(sandbox) = manager.get_state(name)
        && let Err(response) = require_sandbox_access(&state, &identity, sandbox)
    {
        return response;
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
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;

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

    #[cfg(feature = "enterprise")]
    let quota_guard = state.quota_controller.lock().await;

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

    #[cfg(feature = "enterprise")]
    if let Err(response) = require_sandbox_access(&state, &identity, &sandbox_state) {
        return response;
    }

    let new_vcpus = body.vcpus.unwrap_or(sandbox_state.vcpus);
    let new_memory = body.memory_mb.unwrap_or(sandbox_state.memory_mb);

    #[cfg(feature = "enterprise")]
    if let Err(error) = quota_guard.check_resize(&manager, name, new_vcpus, new_memory) {
        let subject = quota_subject(&state, &identity);
        return quota_denial(name, &subject, "resize", error);
    }

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

            let recreate_result = match sandbox_state.backend {
                Some(backend) => {
                    manager
                        .create_with_backend_options(
                            backend,
                            name,
                            &image,
                            new_vcpus,
                            new_memory,
                            ttl_seconds,
                            ports,
                            agent,
                        )
                        .await
                }
                None => {
                    manager
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
                }
            };
            if let Err(e) = recreate_result {
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

            #[cfg(feature = "enterprise")]
            if let Err(e) = manager.set_owner_metadata(
                name,
                sandbox_state.owner_user_id.as_deref(),
                sandbox_state.owner_org_id.as_deref(),
            ) {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!(
                        "Failed to preserve sandbox ownership metadata: {e}"
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
            backend: recorded_backend(
                state_info.and_then(|state| state.backend),
                manager.backend(),
            )
            .to_string(),
            ip,
            image: state_info.map(|s| s.image.clone()),
            vcpus: Some(new_vcpus),
            memory_mb: Some(new_memory),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports: result_ports,
            endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
            secret_files: state_info
                .map(|s| s.secret_files.clone())
                .unwrap_or_default(),
            placeholder_secrets: state_info.map(|s| s.placeholder_secrets).unwrap_or(false),
            proxy_port: state_info.and_then(|s| s.proxy_port),
            uuid: state_info
                .map(|s| s.uuid.clone())
                .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
            secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
            labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
            description: state_info.and_then(|s| s.description.clone()),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
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
            backend: recorded_backend(
                state_info.and_then(|state| state.backend),
                manager.backend(),
            )
            .to_string(),
            ip,
            image: state_info.map(|s| s.image.clone()),
            vcpus: state_info.map(|s| s.vcpus),
            memory_mb: state_info.map(|s| s.memory_mb),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports: result_ports,
            endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
            secret_files: state_info
                .map(|s| s.secret_files.clone())
                .unwrap_or_default(),
            placeholder_secrets: state_info.map(|s| s.placeholder_secrets).unwrap_or(false),
            proxy_port: state_info.and_then(|s| s.proxy_port),
            uuid: state_info
                .map(|s| s.uuid.clone())
                .unwrap_or_else(|| uuid::Uuid::nil().to_string()),
            secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
            labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
            description: state_info.and_then(|s| s.description.clone()),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
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
            backend: recorded_backend(
                state_info.and_then(|state| state.backend),
                manager.backend(),
            )
            .to_string(),
            ip,
            image: state_info.map(|s| s.image.clone()),
            vcpus: state_info.map(|s| s.vcpus),
            memory_mb: state_info.map(|s| s.memory_mb),
            created_at: state_info.map(|s| s.created_at.clone()),
            created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
            template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
            ports,
            endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
            secret_files: state_info
                .map(|s| s.secret_files.clone())
                .unwrap_or_default(),
            placeholder_secrets: state_info.map(|s| s.placeholder_secrets).unwrap_or(false),
            proxy_port: state_info.and_then(|s| s.proxy_port),
            secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
            labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
            description: state_info.and_then(|s| s.description.clone()),
            last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
            workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
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
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if !identity.has_role("admin") {
            return json_response(
                StatusCode::FORBIDDEN,
                &ApiResponse::<()>::error(
                    "Lifecycle reconciliation requires an administrator identity",
                ),
            );
        }
    }
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

async fn handle_list_snapshots(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = (&req, &state);

    match crate::snapshot::list() {
        Ok(snapshots) => {
            #[cfg(feature = "enterprise")]
            let snapshots = snapshots
                .into_iter()
                .filter(|snapshot| snapshot_access_allowed(&state, &identity, snapshot))
                .collect::<Vec<_>>();
            json_response(StatusCode::OK, &ApiResponse::success(snapshots))
        }
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
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;

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

    #[cfg(feature = "enterprise")]
    if let Err(response) = require_sandbox_access(&state, &identity, sandbox_state) {
        return response;
    }

    let input = crate::snapshot::SnapshotInput {
        image: sandbox_state.image.clone(),
        backend: sandbox_state
            .backend
            .map(|b| format!("{:?}", b).to_lowercase())
            .unwrap_or_else(|| "docker".to_string()),
        vcpus: sandbox_state.vcpus,
        memory_mb: sandbox_state.memory_mb,
        remote_id: sandbox_state.remote_id.clone(),
        remote_namespace: sandbox_state.remote_namespace.clone(),
        remote_metadata: sandbox_state.remote_metadata.clone(),
        workspace_revision: sandbox_state.workspace_revision.clone(),
        work_dir: sandbox_state.work_dir.clone(),
        config_path: sandbox_state.config_path.clone(),
    };

    match crate::snapshot::take(&body.sandbox, &body.name, &input) {
        Ok(meta) => {
            #[cfg(feature = "enterprise")]
            let meta = {
                let mut meta = meta;
                let subject = quota_subject(&state, &identity);
                if let Err(error) = crate::snapshot::set_owner(
                    &body.name,
                    Some(&subject.user_id),
                    Some(&subject.org_id),
                ) {
                    return json_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &ApiResponse::<()>::error(format!(
                            "Failed to persist snapshot ownership: {error}"
                        )),
                    );
                }
                meta.owner_user_id = Some(subject.user_id);
                meta.owner_org_id = Some(subject.org_id);
                meta
            };
            json_response(StatusCode::OK, &ApiResponse::success(meta))
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_get_snapshot(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = (&req, &state);

    // Validate snapshot name
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    match crate::snapshot::get(name) {
        Ok(Some(meta)) => {
            #[cfg(feature = "enterprise")]
            if !snapshot_access_allowed(&state, &identity, &meta) {
                return sandbox_access_denied();
            }
            json_response(StatusCode::OK, &ApiResponse::success(meta))
        }
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

async fn handle_delete_snapshot(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = (&req, &state);

    // Validate snapshot name
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    #[cfg(feature = "enterprise")]
    match crate::snapshot::get(name) {
        Ok(Some(meta)) if snapshot_access_allowed(&state, &identity, &meta) => {}
        Ok(Some(_)) | Ok(None) => return sandbox_access_denied(),
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
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
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;

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
        Ok(Some(m)) => {
            #[cfg(feature = "enterprise")]
            if !snapshot_access_allowed(&state, &identity, &m) {
                return sandbox_access_denied();
            }
            m
        }
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

    #[cfg(feature = "enterprise")]
    let quota_subject = quota_subject(&state, &identity);

    if let Err(e) = validation::validate_sandbox_name(&restore_name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid restore name: {}", e)),
        );
    }

    // Create the restored sandbox. Hold the quota guard before the manager
    // lock, matching create/start/resize so concurrent restores cannot race
    // past the persisted usage check.
    #[cfg(feature = "enterprise")]
    let quota_guard = state.quota_controller.lock().await;

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    #[cfg(feature = "enterprise")]
    if let Err(error) =
        quota_guard.check_create_stopped(&manager, &quota_subject, meta.vcpus, meta.memory_mb)
    {
        return quota_denial(&restore_name, &quota_subject, "restore", error);
    }

    let restore_result =
        if let Ok(snapshot_backend) = meta.backend.parse::<crate::backend::BackendType>() {
            manager
                .create_with_backend(
                    snapshot_backend,
                    &restore_name,
                    meta.restore_image(),
                    meta.vcpus,
                    meta.memory_mb,
                )
                .await
        } else {
            manager
                .create(
                    &restore_name,
                    meta.restore_image(),
                    meta.vcpus,
                    meta.memory_mb,
                )
                .await
        };

    match restore_result {
        Ok(()) => {
            #[cfg(feature = "enterprise")]
            if let Err(e) = manager.set_owner_metadata(
                &restore_name,
                Some(&quota_subject.user_id),
                Some(&quota_subject.org_id),
            ) {
                let _ = manager.remove(&restore_name).await;
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!(
                        "Failed to persist restored sandbox ownership: {e}"
                    )),
                );
            }

            if let Err(e) = manager.set_work_dir(&restore_name, meta.work_dir.clone()) {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
            if let Err(e) = manager.set_config_path(&restore_name, meta.config_path.clone()) {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
            if let Some(snapshot_handle) = meta.remote_snapshot.as_deref()
                && let Err(e) = manager.set_remote_restore_snapshot(&restore_name, snapshot_handle)
            {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }

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
                    image: state_info
                        .map(|s| s.image.clone())
                        .or_else(|| Some(meta.restore_image().to_string())),
                    vcpus: Some(meta.vcpus),
                    memory_mb: Some(meta.memory_mb),
                    created_at: state_info.map(|s| s.created_at.clone()),
                    created_from_template: state_info.and_then(|s| s.created_from_template.clone()),
                    template_help_text: state_info.and_then(|s| s.template_help_text.clone()),
                    ports,
                    endpoints: state_info.map(|s| s.endpoints.clone()).unwrap_or_default(),
                    secret_files: state_info
                        .map(|s| s.secret_files.clone())
                        .unwrap_or_default(),
                    placeholder_secrets: state_info.map(|s| s.placeholder_secrets).unwrap_or(false),
                    proxy_port: state_info.and_then(|s| s.proxy_port),
                    secret_mappings: state_info.map(build_secret_mappings).unwrap_or_default(),
                    labels: state_info.map(|s| s.labels.clone()).unwrap_or_default(),
                    description: state_info.and_then(|s| s.description.clone()),
                    last_activity_at: state_info.and_then(|s| s.last_activity_at.clone()),
                    workspace_revision: state_info.and_then(|s| s.workspace_revision.clone()),
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

async fn handle_sandbox_logs(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;

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

    #[cfg(feature = "enterprise")]
    if let Some(sandbox) = manager.get_state(name)
        && let Err(response) = require_sandbox_access(&state, &identity, sandbox)
    {
        return response;
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

// --- Enterprise quota and policy handlers ---

#[cfg(feature = "enterprise")]
fn quota_subject(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
) -> crate::quota::QuotaSubject {
    let enterprise = state.enterprise_config.as_ref();
    let org_id = identity
        .org_id()
        .or_else(|| enterprise.and_then(|config| config.org_id.as_deref()))
        .unwrap_or("default")
        .to_string();
    crate::quota::QuotaSubject {
        user_id: identity.quota_user_id(),
        org_id,
    }
}

/// Persistent sandbox ownership is enforced independently of Cedar policy.
/// Cedar decides whether a principal may perform an action; this check keeps
/// one authenticated tenant from reading or mutating another tenant's state.
/// Admin is intentionally the only cross-owner escape hatch, and the role is
/// taken from validated JWT claims rather than caller-controlled request data.
#[cfg(feature = "enterprise")]
fn sandbox_access_allowed(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
    sandbox: &crate::vmm::SandboxState,
) -> bool {
    if sandbox.owner_user_id.is_none() && sandbox.owner_org_id.is_none() {
        // CLI-created --no-start state has not been claimed yet. A principal
        // accepted by the server (or anonymous access when auth is disabled)
        // may inspect/manage it, while partial or owned metadata remains
        // tenant-scoped below. Start still requires its one-shot token before
        // this state is assigned to an owner.
        return trusted_owner_identity(identity, state).is_some()
            || (!state.authentication_required() && !identity.is_authenticated());
    }
    owner_access_allowed(
        state,
        identity,
        sandbox.owner_user_id.as_deref(),
        sandbox.owner_org_id.as_deref(),
    )
}

#[cfg(feature = "enterprise")]
fn owner_access_allowed(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
    owner_user_id: Option<&str>,
    owner_org_id: Option<&str>,
) -> bool {
    if identity.has_role("admin") {
        return true;
    }

    let (Some(owner_user), Some(owner_org)) = (owner_user_id, owner_org_id) else {
        // Partial ownership metadata is never trusted. The sandbox-specific
        // caller handles the intentional fully-unowned CLI handoff case before
        // reaching this generic owner comparison.
        return false;
    };
    let subject = quota_subject(state, identity);
    owner_user == subject.user_id && owner_org == subject.org_id
}

#[cfg(feature = "enterprise")]
fn snapshot_access_allowed(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
    snapshot: &crate::snapshot::SnapshotMeta,
) -> bool {
    owner_access_allowed(
        state,
        identity,
        snapshot.owner_user_id.as_deref(),
        snapshot.owner_org_id.as_deref(),
    )
}

#[cfg(feature = "enterprise")]
fn sandbox_access_denied() -> Response<BoxBody> {
    // Do not distinguish a missing sandbox from an unauthorized one. This
    // avoids leaking names, UUIDs, ownership metadata, or audit existence.
    json_response(
        StatusCode::NOT_FOUND,
        &ApiResponse::<()>::error("Sandbox not found"),
    )
}

#[cfg(feature = "enterprise")]
#[allow(clippy::result_large_err)]
fn require_sandbox_access(
    state: &AppState,
    identity: &crate::identity::AgentIdentity,
    sandbox: &crate::vmm::SandboxState,
) -> Result<(), Response<BoxBody>> {
    if sandbox_access_allowed(state, identity, sandbox) {
        Ok(())
    } else {
        Err(sandbox_access_denied())
    }
}

#[cfg(feature = "enterprise")]
fn quota_denial(
    sandbox: &str,
    subject: &crate::quota::QuotaSubject,
    action: &str,
    error: anyhow::Error,
) -> Response<BoxBody> {
    let reason = error.to_string();
    crate::audit::log_event(crate::audit::AuditEvent::QuotaDenied {
        sandbox: sandbox.to_string(),
        principal: subject.user_id.clone(),
        org_id: subject.org_id.clone(),
        action: action.to_string(),
        reason: reason.clone(),
    });
    json_response(
        StatusCode::TOO_MANY_REQUESTS,
        &ApiResponse::<()>::error(format!("Resource quota denied: {reason}")),
    )
}

#[cfg(feature = "enterprise")]
async fn handle_quota_status(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let identity = extract_identity(&req, &state).await;
    let subject = quota_subject(&state, &identity);
    let quota = state.quota_controller.lock().await;
    let manager = match state.get_manager().await {
        Ok(manager) => manager,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };
    json_response(
        StatusCode::OK,
        &ApiResponse::success(quota.status(&manager, &subject)),
    )
}

/// Policy status response
#[cfg(feature = "enterprise")]
#[derive(Debug, Serialize)]
struct PolicyStatusResponse {
    compiled: bool,
    configured: bool,
    active: bool,
    enforcing: bool,
    healthy: bool,
    enabled: bool,
    version: u64,
    org_id: Option<String>,
    offline_mode: String,
    policy_server: Option<String>,
    source: String,
    policy_source: String,
    config_path: Option<String>,
    initialization_error: Option<String>,
    init_error: Option<String>,
    fail_closed: bool,
    meaningful: bool,
    admin_guidance: Option<String>,
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
                compiled: cfg!(feature = "enterprise"),
                configured: false,
                active: false,
                enforcing: false,
                healthy: state.policy_init_error.is_none(),
                enabled: false,
                version: 0,
                org_id: None,
                offline_mode: "disabled".to_string(),
                policy_server: None,
                source: "none".to_string(),
                policy_source: "none".to_string(),
                config_path: state.config_path.as_ref().map(|p| p.display().to_string()),
                initialization_error: state.policy_init_error.clone(),
                init_error: state.policy_init_error.clone(),
                fail_closed: false,
                meaningful: false,
                admin_guidance: None,
            }),
        );
    };

    let (version, source, meaningful) = if let Some(ref engine_lock) = state.policy_engine {
        let engine = engine_lock.read().await;
        (
            engine.version().await,
            engine.policy_source().await,
            engine.meaningful().await,
        )
    } else {
        (0, "none".to_string(), false)
    };
    let configured = enterprise.enabled;
    let active = state.policy_engine.is_some();
    let healthy = state.policy_init_error.is_none();
    let enforcing = active && meaningful;
    let fail_closed = enterprise.offline_mode == "fail_closed";
    let admin_guidance = enterprise.policy_server.as_ref().map(|_| {
        "Remote policy servers are read-only here; ask an administrator to update the server configuration.".to_string()
    });

    json_response(
        StatusCode::OK,
        &ApiResponse::success(PolicyStatusResponse {
            compiled: cfg!(feature = "enterprise"),
            configured,
            active,
            enforcing,
            healthy,
            enabled: active && enforcing && healthy,
            version,
            org_id: enterprise.org_id.clone(),
            offline_mode: enterprise.offline_mode.clone(),
            policy_server: enterprise.policy_server.clone(),
            source: source.clone(),
            policy_source: source,
            config_path: state.config_path.as_ref().map(|p| p.display().to_string()),
            initialization_error: state.policy_init_error.clone(),
            init_error: state.policy_init_error.clone(),
            fail_closed,
            meaningful,
            admin_guidance,
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
        let configured = state
            .enterprise_config
            .as_ref()
            .is_some_and(|config| config.enabled)
            || state.policy_init_error.is_some();
        let fail_closed = state
            .enterprise_config
            .as_ref()
            .is_some_and(|config| config.enabled && config.offline_mode == "fail_closed");
        return json_response(
            StatusCode::OK,
            &ApiResponse::success(PolicyCheckResponse {
                decision: if configured { "deny" } else { "permit" }.to_string(),
                reason: if fail_closed {
                    "Policy engine unavailable; fail-closed configuration denies the request"
                        .to_string()
                } else if configured {
                    format!(
                        "Policy engine initialization failed: {}",
                        state
                            .policy_init_error
                            .as_deref()
                            .unwrap_or("unknown error")
                    )
                } else {
                    "No policy engine active (enterprise disabled)".to_string()
                },
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
        teams: Vec::new(),
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

/// Run one durable task at a time. Parallelism belongs to the dse.3
/// coordinator; this loop only provides the production execution path for a
/// queued task and keeps claiming atomic in `TaskManager`.
fn spawn_task_worker(state: Arc<AppState>) {
    let Some(task_manager) = state.task_manager.clone() else {
        return;
    };

    tokio::spawn(async move {
        eprintln!("[tasks] task worker started");
        loop {
            let vm_manager = match state.ensure_manager() {
                Ok(manager) => manager,
                Err(error) => {
                    eprintln!("[tasks] worker backend unavailable: {error}");
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            let mut worker =
                TaskWorker::new((*task_manager).clone(), VmTaskExecutor::new(vm_manager));
            match worker.recover_interrupted().await {
                Ok(count) => {
                    if count > 0 {
                        eprintln!("[tasks] recovered {count} interrupted task(s)");
                    }
                }
                Err(error) => {
                    eprintln!("[tasks] worker recovery failed: {error:#}");
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }
            if let Err(error) = worker.run_once().await {
                eprintln!("[tasks] worker tick failed: {error:#}");
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
    hex::encode(hasher.finalize())
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
fn spawn_workspace_scheduler(state: Arc<AppState>, config_path: Option<&std::path::Path>) {
    let Some(manager) = state.vm_manager.get().cloned() else {
        return;
    };

    let default_config_path = std::path::Path::new("agentkernel.toml");
    let config_path = config_path.unwrap_or(default_config_path);
    let scheduling = match crate::config::Config::from_file(config_path) {
        Ok(config) => config.scheduling,
        Err(error) if config_path.exists() => {
            eprintln!(
                "[workspace-scheduler] unable to load {}: {error}",
                config_path.display()
            );
            return;
        }
        Err(_) => crate::config::WorkspaceSchedulingConfig::default(),
    };
    let _ = crate::workspace_scheduler::spawn_enforcement_loop(manager, scheduling);
}

/// Run the HTTP API server (plain HTTP)
#[allow(dead_code)]
pub async fn run_server(addr: SocketAddr, api_keys: Vec<String>) -> Result<()> {
    let mut app_state = AppState::new(api_keys, None, vec![], None)?;
    app_state.configure_job_scheduler(None)?;
    app_state.start_job_scheduler();
    let state = Arc::new(app_state);
    spawn_orchestration_worker(state.clone());
    spawn_task_worker(state.clone());
    spawn_workspace_scheduler(state.clone(), None);
    // Spawn hibernation daemon for durable objects
    if let (Some(store), Some(manager)) = (
        state.orchestration_store.clone(),
        state.vm_manager.get().cloned(),
    ) {
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
#[allow(dead_code)]
pub async fn run_server_with_tls(
    addr: SocketAddr,
    tls_config: Option<crate::tls::TlsConfig>,
    api_keys: Vec<String>,
    otel_endpoint: Option<String>,
    webhook_urls: Vec<String>,
    config_path: Option<std::path::PathBuf>,
) -> Result<()> {
    run_server_with_tls_config(
        addr,
        tls_config,
        api_keys,
        otel_endpoint,
        webhook_urls,
        config_path,
    )
    .await
}

/// Run the HTTP API server with an explicit configuration path.
pub async fn run_server_with_tls_config(
    addr: SocketAddr,
    tls_config: Option<crate::tls::TlsConfig>,
    api_keys: Vec<String>,
    otel_endpoint: Option<String>,
    webhook_urls: Vec<String>,
    config_path: Option<std::path::PathBuf>,
) -> Result<()> {
    // Resolve the path once at process start. The policy engine, API status,
    // and scheduler must all refer to the same file even when the process is
    // launched from a different working directory later.
    let config_path = config_path
        .as_deref()
        .map(canonical_config_path)
        .transpose()?;
    let acceptor = match tls_config {
        Some(ref tls) => {
            let acceptor = tls.load_or_generate()?;
            Some(acceptor)
        }
        None => None,
    };

    let mut app_state = AppState::new(api_keys, otel_endpoint, webhook_urls, config_path.clone())?;
    app_state.configure_job_scheduler(config_path.as_deref())?;
    app_state.start_job_scheduler();
    let state = Arc::new(app_state);
    spawn_orchestration_worker(state.clone());
    spawn_task_worker(state.clone());
    spawn_workspace_scheduler(state.clone(), config_path.as_deref());
    // Spawn hibernation daemon for durable objects
    if let (Some(store), Some(manager)) = (
        state.orchestration_store.clone(),
        state.vm_manager.get().cloned(),
    ) {
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

/// Resolve a server configuration path without requiring the file to exist.
/// Existing path components are canonicalized so symlinks cannot make the
/// status endpoint report a different file from the one the server loaded.
fn canonical_config_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    if absolute.exists() {
        return Ok(std::fs::canonicalize(absolute)?);
    }

    let file_name = absolute
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Server configuration path has no file name"))?;
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Server configuration path has no parent"))?;
    let canonical_parent = if parent.exists() {
        std::fs::canonicalize(parent)?
    } else {
        parent.to_path_buf()
    };
    Ok(canonical_parent.join(file_name))
}

// ---------------------------------------------------------------------------
// Diagnostics handlers
// ---------------------------------------------------------------------------

async fn handle_status(state: Arc<AppState>) -> Response<BoxBody> {
    // The HTTP server can be healthy before a local sandbox runtime is ready.
    // Keep /status available in that state so clients can distinguish a
    // reachable server from a missing backend and offer recovery guidance.
    let backend = match state.get_manager().await {
        Ok(manager) => manager.backend().to_string(),
        Err(error) => {
            eprintln!("[status] Sandbox backend unavailable: {error}");
            "unavailable".to_string()
        }
    };
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

    // Check Docker CLI and daemon separately.  A service launched by
    // launchd may not inherit the user's interactive PATH, and a missing
    // daemon should not be reported as a missing installation.
    let docker_cli_available = std::process::Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
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
        } else if docker_cli_available {
            "Docker is installed but its daemon is not running".to_string()
        } else {
            "Docker CLI not found".to_string()
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
                "Apple Containers CLI available; the system starts when needed".to_string()
            } else {
                "Apple Containers CLI not found".to_string()
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

async fn handle_gc(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(&req, &state).await;
        if !identity.has_role("admin") {
            return json_response(
                StatusCode::FORBIDDEN,
                &ApiResponse::<()>::error("Garbage collection requires an administrator identity"),
            );
        }
    }
    #[cfg(not(feature = "enterprise"))]
    let _ = &req;

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

#[allow(clippy::result_large_err)]
async fn llm_scope(
    req: &Request<Incoming>,
    state: &AppState,
) -> Result<(String, String, bool), Response<BoxBody>> {
    #[cfg(feature = "enterprise")]
    {
        let identity = extract_identity(req, state).await;
        let Some((tenant, user)) = trusted_owner_identity(&identity, state) else {
            return Err(json_response(
                StatusCode::UNAUTHORIZED,
                &ApiResponse::<()>::error(
                    "LLM usage requires a validated JWT or configured API key",
                ),
            ));
        };
        Ok((
            tenant,
            user,
            identity.has_role("admin") || identity.has_role("billing_admin"),
        ))
    }
    #[cfg(not(feature = "enterprise"))]
    {
        let Some((tenant, user)) = trusted_owner_identity(req, state) else {
            return Err(json_response(
                StatusCode::UNAUTHORIZED,
                &ApiResponse::<()>::error("LLM usage requires a configured API key"),
            ));
        };
        Ok((tenant, user, false))
    }
}

async fn handle_llm_usage_all(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let (tenant, user, is_admin) = match llm_scope(&req, &state).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let store = crate::llm_intercept::LLM_USAGE.read().await;
    let usage: std::collections::HashMap<_, _> = store
        .all_usage()
        .iter()
        .filter(|(sandbox, _)| {
            store
                .scope_for_sandbox(sandbox)
                .is_some_and(|scope| scope.tenant == tenant && (is_admin || scope.user == user))
        })
        .map(|(sandbox, entries)| (sandbox.clone(), entries.clone()))
        .collect();
    json_response(StatusCode::OK, &ApiResponse::success(usage))
}

async fn handle_llm_usage_sandbox(
    req: Request<Incoming>,
    sandbox: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    let (tenant, user, is_admin) = match llm_scope(&req, &state).await {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let store = crate::llm_intercept::LLM_USAGE.read().await;
    let visible = store
        .scope_for_sandbox(sandbox)
        .is_some_and(|scope| scope.tenant == tenant && (is_admin || scope.user == user));
    if !visible {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("LLM usage not found"),
        );
    }
    let usage = store.usage_for_sandbox(sandbox);
    json_response(StatusCode::OK, &ApiResponse::success(usage))
}

fn llm_query_value(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        (name == key).then(|| {
            urlencoding::decode(value)
                .unwrap_or(std::borrow::Cow::Borrowed(value))
                .into_owned()
        })
    })
}

fn llm_pagination_value(
    query: Option<&str>,
    key: &str,
    default: usize,
    max: usize,
) -> Result<usize, String> {
    let Some(raw) = llm_query_value(query, key) else {
        return Ok(default);
    };
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("{key} must be a non-negative integer"))?;
    if (key == "limit" && !(1..=max).contains(&value)) || (key == "offset" && value > max) {
        return Err(if key == "limit" {
            format!("limit must be between 1 and {max}")
        } else {
            format!("offset must be at most {max}")
        });
    }
    Ok(value)
}

async fn handle_llm_spend(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let limit = match llm_pagination_value(
        req.uri().query(),
        "limit",
        100,
        crate::llm_spend::MAX_PAGE_SIZE,
    ) {
        Ok(value) => value,
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, &ApiResponse::<()>::error(error));
        }
    };
    let offset = match llm_pagination_value(
        req.uri().query(),
        "offset",
        0,
        crate::llm_spend::MAX_QUERY_OFFSET,
    ) {
        Ok(value) => value,
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, &ApiResponse::<()>::error(error));
        }
    };
    let mut filter = crate::llm_spend::LlmSpendFilter {
        agent: llm_query_value(req.uri().query(), "agent"),
        user: llm_query_value(req.uri().query(), "user"),
        project: llm_query_value(req.uri().query(), "project"),
        from: llm_query_value(req.uri().query(), "from"),
        to: llm_query_value(req.uri().query(), "to"),
        limit: Some(limit),
        offset: Some(offset),
        tenant: None,
    };

    let (tenant, owner_user, is_admin) = match llm_scope(&req, &state).await {
        Ok((tenant, user, is_admin)) => (tenant, Some(user), is_admin),
        Err(response) => return response,
    };
    filter.tenant = Some(tenant);
    if !is_admin {
        let Some(owner_user) = owner_user else {
            return json_response(
                StatusCode::FORBIDDEN,
                &ApiResponse::<()>::error("Authenticated identity has no spend user scope"),
            );
        };
        if let Some(requested_user) = filter.user.as_deref()
            && requested_user != owner_user
        {
            return json_response(
                StatusCode::FORBIDDEN,
                &ApiResponse::<()>::error("Spend user filter is outside the authenticated scope"),
            );
        }
        filter.user = Some(owner_user);
    }

    let Some(store) = crate::llm_spend::global_store() else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &ApiResponse::<()>::error("Durable LLM spend storage is unavailable"),
        );
    };
    match store.query(&filter) {
        Ok(report) => json_response(StatusCode::OK, &ApiResponse::success(report)),
        Err(error) => json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        ),
    }
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
        /// Deprecated compatibility alias for `cli_installed`.
        enabled: bool,
        description: String,
        package: Option<String>,
        cli_installed: bool,
        cli_version: Option<String>,
        tested_version: String,
        compatibility_status: String,
        install_command: String,
        integration_supported: bool,
        integration_project_installed: bool,
        integration_global_installed: bool,
        integration_global_supported: bool,
    }

    let agents: Vec<AgentInfo> = crate::agent_catalog::agents()
        .iter()
        .map(|entry| {
            let cli_installed = binary_exists_in_path(&entry.executable);
            let cli_version = cli_installed.then(|| {
                std::process::Command::new(&entry.executable)
                    .arg(&entry.smoke_arg)
                    .output()
                    .map(|output| {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        format!("{} {}", stdout.trim(), stderr.trim())
                            .trim()
                            .to_string()
                    })
                    .unwrap_or_else(|_| "unknown".into())
            });
            let compatibility_status = match cli_version.as_deref() {
                None => "not_installed",
                Some(version) if version.contains(&entry.expected_output) => "tested",
                Some(_) => "untested_version",
            };
            let target = entry
                .integration_target
                .as_deref()
                .and_then(crate::plugin_installer::PluginTarget::from_str);
            let (project_installed, global_installed) = target
                .map(crate::plugin_installer::installation_scopes)
                .unwrap_or((false, false));
            AgentInfo {
                name: entry.id.clone(),
                display_name: entry.display_name.clone(),
                enabled: cli_installed,
                description: entry.description.clone(),
                package: entry.package.clone(),
                cli_installed,
                cli_version,
                tested_version: entry.version.clone(),
                compatibility_status: compatibility_status.into(),
                install_command: entry.install_command.clone(),
                integration_supported: target.is_some(),
                integration_project_installed: project_installed,
                integration_global_installed: global_installed,
                integration_global_supported: target
                    .map(|target| target.supports_global())
                    .unwrap_or(false),
            }
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(agents))
}

#[derive(Debug, Deserialize)]
struct AgentIntegrationRequest {
    #[serde(default = "default_plugin_scope")]
    scope: String,
    #[serde(default)]
    confirm: bool,
}

fn default_plugin_scope() -> String {
    "project".into()
}

#[derive(Serialize)]
struct AgentIntegrationResult {
    target: String,
    scope: String,
    confirmed: bool,
    files: Vec<String>,
}

async fn handle_install_agent_integration(req: Request<Incoming>, name: &str) -> Response<BoxBody> {
    let request: AgentIntegrationRequest = match read_json_body(req).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    let Some(entry) = crate::agent_catalog::find(name) else {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error(format!("Unknown agent: {name}")),
        );
    };
    let Some(target_name) = entry.integration_target.as_deref() else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!(
                "{} does not have a managed AgentKernel integration",
                entry.display_name
            )),
        );
    };
    let target = crate::plugin_installer::PluginTarget::from_str(target_name)
        .expect("catalog integration targets are validated by tests");
    let global = match request.scope.as_str() {
        "project" => false,
        "global" if target.supports_global() => true,
        "global" => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!(
                    "{} integrations are project-only",
                    entry.display_name
                )),
            );
        }
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error("scope must be project or global"),
            );
        }
    };
    let files = match crate::plugin_installer::preview_plugin(target, global) {
        Ok(files) => files
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    };

    if request.confirm {
        let options = crate::plugin_installer::InstallOptions {
            global,
            force: false,
            dry_run: false,
        };
        if let Err(error) = crate::plugin_installer::install_plugin(target, &options) {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(error.to_string()),
            );
        }
    }

    json_response(
        StatusCode::OK,
        &ApiResponse::success(AgentIntegrationResult {
            target: target.name().into(),
            scope: request.scope,
            confirmed: request.confirm,
            files,
        }),
    )
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
#[allow(clippy::result_large_err)]
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

#[derive(Debug, Deserialize)]
struct ImagePullRequest {
    image: String,
}

#[derive(Debug, Default, Deserialize)]
struct ImagePruneRequest {
    #[serde(default)]
    agentkernel_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct DockerImageDiskUsage {
    #[serde(rename = "type", alias = "Type")]
    kind: String,
    #[serde(
        alias = "Total",
        alias = "TotalCount",
        deserialize_with = "deserialize_string_or_number"
    )]
    total: String,
    #[serde(alias = "Active", deserialize_with = "deserialize_string_or_number")]
    active: String,
    #[serde(alias = "Size", deserialize_with = "deserialize_string_or_number")]
    size: String,
    #[serde(
        alias = "Reclaimable",
        deserialize_with = "deserialize_string_or_number"
    )]
    reclaimable: String,
}

#[derive(Debug, Deserialize)]
struct DockerImageRecord {
    #[serde(alias = "ID")]
    id: String,
    #[serde(alias = "Repository")]
    repository: String,
    #[serde(alias = "Tag")]
    tag: String,
}

fn parse_docker_image_records(stdout: &str) -> Vec<DockerImageRecord> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(value) => Ok(value),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        serde_json::Value::Bool(value) => Ok(value.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        value => Err(serde::de::Error::custom(format!(
            "expected a string or number, got {value}"
        ))),
    }
}

fn image_record_name(image: &DockerImageRecord) -> String {
    if image.tag == "<none>" {
        image.id.clone()
    } else {
        format!("{}:{}", image.repository, image.tag)
    }
}

fn parse_docker_disk_usage(stdout: &str) -> Result<Vec<DockerImageDiskUsage>, String> {
    // Docker emits one JSON object per line for `{{json .}}`; Podman emits a
    // JSON array when asked for the `json` format. Accept both so the desktop
    // page follows whichever runtime the server selected.
    if let Ok(entries) = serde_json::from_str::<Vec<DockerImageDiskUsage>>(stdout.trim()) {
        return Ok(entries);
    }

    let mut entries = Vec::new();
    for (line_number, line) in stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let entry = serde_json::from_str(line).map_err(|error| {
            format!(
                "invalid disk-usage response on line {}: {error}",
                line_number + 1
            )
        })?;
        entries.push(entry);
    }

    if stdout.trim().is_empty() {
        Ok(entries)
    } else if entries.is_empty() {
        Err("container runtime returned non-empty disk-usage output with no entries".to_string())
    } else {
        Ok(entries)
    }
}

fn is_agentkernel_image(image: &DockerImageRecord) -> bool {
    // Images created by the image builder, snapshots, and setup all use the
    // `agentkernel-` namespace. Keep the match anchored: a user's unrelated
    // image such as `my-agentkernel-tools` must never be pruned.
    image.repository == "agentkernel" || image.repository.starts_with("agentkernel-")
}

fn runtime_or_error() -> Result<crate::docker_backend::ContainerRuntime, Box<Response<BoxBody>>> {
    crate::docker_backend::detect_container_runtime().ok_or_else(|| {
        Box::new(json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error("No container runtime available"),
        ))
    })
}

async fn handle_list_images(_state: Arc<AppState>) -> Response<BoxBody> {
    let runtime = match runtime_or_error() {
        Ok(runtime) => runtime,
        Err(response) => return *response,
    };
    let cmd = runtime.cmd();

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

async fn handle_image_disk_usage(_state: Arc<AppState>) -> Response<BoxBody> {
    let runtime = match runtime_or_error() {
        Ok(runtime) => runtime,
        Err(response) => return *response,
    };
    let format = match runtime {
        crate::docker_backend::ContainerRuntime::Docker => "{{json .}}",
        crate::docker_backend::ContainerRuntime::Podman => "json",
    };

    let output = match tokio::process::Command::new(runtime.cmd())
        .args(["system", "df", "--format", format])
        .output()
        .await
    {
        Ok(output) => output,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to inspect image disk usage: {error}")),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!(
                "Container runtime disk usage failed: {}",
                stderr.trim()
            )),
        );
    }

    let usage = match parse_docker_disk_usage(&String::from_utf8_lossy(&output.stdout)) {
        Ok(usage) => usage,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to parse image disk usage: {error}")),
            );
        }
    };
    json_response(StatusCode::OK, &ApiResponse::success(usage))
}

async fn handle_pull_image(req: Request<Incoming>, _state: Arc<AppState>) -> Response<BoxBody> {
    let body: ImagePullRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };

    if let Err(error) = validation::validate_docker_image(&body.image) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(error.to_string()),
        );
    }

    let runtime = match runtime_or_error() {
        Ok(runtime) => runtime,
        Err(response) => return *response,
    };
    let output = match tokio::process::Command::new(runtime.cmd())
        .args(["pull", &body.image])
        .output()
        .await
    {
        Ok(output) => output,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to pull image: {error}")),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to pull image: {}", stderr.trim())),
        );
    }

    let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    json_response(StatusCode::OK, &ApiResponse::success(message))
}

async fn handle_prune_images(req: Request<Incoming>, _state: Arc<AppState>) -> Response<BoxBody> {
    let body: ImagePruneRequest = match read_json_body(req).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let runtime = match runtime_or_error() {
        Ok(runtime) => runtime,
        Err(response) => return *response,
    };

    if body.agentkernel_only {
        let output = match tokio::process::Command::new(runtime.cmd())
            .args(["images", "--format", "{{json .}}"])
            .output()
            .await
        {
            Ok(output) => output,
            Err(error) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(format!(
                        "Failed to list images for pruning: {error}"
                    )),
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!(
                    "Failed to list images for pruning: {}",
                    stderr.trim()
                )),
            );
        }

        let mut removed = 0usize;
        for image in parse_docker_image_records(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .filter(is_agentkernel_image)
        {
            if crate::images::sandbox_usage(&image_record_name(&image)).unwrap_or(0) != 0 {
                continue;
            }
            let result = tokio::process::Command::new(runtime.cmd())
                .args(["rmi", &image.id])
                .output()
                .await;
            if result.is_ok_and(|result| result.status.success()) {
                removed += 1;
            }
        }

        return json_response(
            StatusCode::OK,
            &ApiResponse::success(format!("{removed} AgentKernel image(s) removed")),
        );
    }

    let output = match tokio::process::Command::new(runtime.cmd())
        .args(["image", "prune", "-f"])
        .output()
        .await
    {
        Ok(output) => output,
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(format!("Failed to prune images: {error}")),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to prune images: {}", stderr.trim())),
        );
    }

    let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    json_response(StatusCode::OK, &ApiResponse::success(message))
}

async fn handle_delete_image(id: &str, _state: Arc<AppState>) -> Response<BoxBody> {
    let runtime = match runtime_or_error() {
        Ok(runtime) => runtime,
        Err(response) => return *response,
    };
    let cmd = runtime.cmd();

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
    let image = "alpine:3.24";

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
// Session Recording (reads asciicast v2 artifacts from ~/.agentkernel/recordings)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SessionRecordingSummary {
    id: String,
    filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
    width: u32,
    height: u32,
    event_count: usize,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct SessionRecordingEvent {
    time: f64,
    event_type: &'static str,
    data: String,
}

#[derive(Debug, Serialize)]
struct SessionRecordingDetails {
    #[serde(flatten)]
    summary: SessionRecordingSummary,
    header: AsciicastHeader,
    events: Vec<SessionRecordingEvent>,
}

fn valid_recording_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn recording_path(id: &str) -> Result<std::path::PathBuf, &'static str> {
    if !valid_recording_id(id) {
        return Err("Invalid session id");
    }
    Ok(asciicast::default_recordings_dir().join(format!("{id}.cast")))
}

fn recording_is_regular_file(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn is_asciicast_v2(header: &AsciicastHeader) -> bool {
    header.version == 2
}

fn recording_event(event: AsciicastEvent) -> SessionRecordingEvent {
    let event_type = match event.event_type {
        EventType::Output => "output",
        EventType::Input => "input",
    };
    SessionRecordingEvent {
        time: event.time,
        event_type,
        data: event.data,
    }
}

fn recording_summary(
    path: &std::path::Path,
    header: &AsciicastHeader,
    events: &[AsciicastEvent],
) -> std::io::Result<SessionRecordingSummary> {
    let metadata = std::fs::metadata(path)?;
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    Ok(SessionRecordingSummary {
        id,
        filename: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        title: header.title.clone(),
        command: header.command.clone(),
        timestamp: header.timestamp,
        duration: header.duration,
        width: header.width,
        height: header.height,
        event_count: events.len(),
        size_bytes: metadata.len(),
    })
}

fn recording_error_response(error: &str) -> Response<BoxBody> {
    json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        &ApiResponse::<()>::error(error),
    )
}

async fn handle_list_recordings() -> Response<BoxBody> {
    let directory = asciicast::default_recordings_dir();
    if !directory.exists() {
        return json_response(
            StatusCode::OK,
            &ApiResponse::success(Vec::<SessionRecordingSummary>::new()),
        );
    }

    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            return recording_error_response(&format!("Failed to read recordings: {error}"));
        }
    };

    let mut recordings = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("cast")
            || !entry.file_type().is_ok_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(id) if valid_recording_id(id) => id,
            _ => continue,
        };
        if let Ok((header, events)) = asciicast::read_asciicast(&path)
            && is_asciicast_v2(&header)
            && let Ok(summary) = recording_summary(&path, &header, &events)
        {
            debug_assert_eq!(summary.id, id);
            recordings.push(summary);
        }
    }
    recordings.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| left.id.cmp(&right.id))
    });
    json_response(StatusCode::OK, &ApiResponse::success(recordings))
}

async fn handle_get_recording(id: &str) -> Response<BoxBody> {
    let path = match recording_path(id) {
        Ok(path) => path,
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, &ApiResponse::<()>::error(error));
        }
    };
    if !recording_is_regular_file(&path) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Session recording not found"),
        );
    }

    let (header, events) = match asciicast::read_asciicast(&path) {
        Ok(recording) => recording,
        Err(error) => {
            return recording_error_response(&format!("Invalid session recording: {error}"));
        }
    };
    if !is_asciicast_v2(&header) {
        return recording_error_response(
            "Unsupported session recording format; expected asciicast v2",
        );
    }
    let summary = match recording_summary(&path, &header, &events) {
        Ok(summary) => summary,
        Err(error) => {
            return recording_error_response(&format!(
                "Failed to inspect session recording: {error}"
            ));
        }
    };
    let events = events.into_iter().map(recording_event).collect();
    json_response(
        StatusCode::OK,
        &ApiResponse::success(SessionRecordingDetails {
            summary,
            header,
            events,
        }),
    )
}

fn text_response(
    status: StatusCode,
    content_type: &'static str,
    body: String,
) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .body(full(body))
        .unwrap()
}

async fn handle_get_recording_cast(id: &str) -> Response<BoxBody> {
    let path = match recording_path(id) {
        Ok(path) => path,
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, &ApiResponse::<()>::error(error));
        }
    };
    if !recording_is_regular_file(&path) {
        return json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Session recording not found"),
        );
    }

    match asciicast::read_asciicast(&path) {
        Ok((header, _)) if is_asciicast_v2(&header) => {}
        Ok(_) => {
            return recording_error_response(
                "Unsupported session recording format; expected asciicast v2",
            );
        }
        Err(error) => {
            return recording_error_response(&format!("Invalid session recording: {error}"));
        }
    }
    match std::fs::read_to_string(&path) {
        Ok(cast) => text_response(StatusCode::OK, "text/plain; charset=utf-8", cast),
        Err(error) => {
            recording_error_response(&format!("Failed to read session recording: {error}"))
        }
    }
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

    let config = crate::config::SandboxConfigExport::from_parts(
        &sandbox_state.name,
        &sandbox_state.image,
        sandbox_state.init_script.as_deref(),
        sandbox_state.vcpus,
        sandbox_state.memory_mb,
        sandbox_state.agent.as_deref(),
        sandbox_state
            .ports
            .iter()
            .map(ToString::to_string)
            .collect(),
        sandbox_state.managed_network.as_ref(),
    );

    match toml::to_string_pretty(&config) {
        Ok(config) => json_response(StatusCode::OK, &ApiResponse::success(config)),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to serialize config: {e}")),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct LegacySandboxConfig {
    name: String,
    image: String,
    vcpus: u32,
    memory_mb: u64,
    #[serde(default)]
    ports: Vec<crate::backend::PortMapping>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    init_script: Option<String>,
    #[serde(default)]
    managed_network: Option<crate::backend::ManagedNetworkConfig>,
}

#[derive(Debug)]
struct ImportedSandboxConfig {
    name: String,
    image: String,
    vcpus: u32,
    memory_mb: u64,
    ports: Vec<crate::backend::PortMapping>,
    agent: Option<String>,
    init_script: Option<String>,
    managed_network: Option<crate::backend::ManagedNetworkConfig>,
    permissions: crate::permissions::Permissions,
}

/// Parse the current portable config format, while accepting TOML exported by
/// older HTTP API versions that serialized the complete `SandboxState`.
fn parse_imported_sandbox_config(content: &str) -> anyhow::Result<ImportedSandboxConfig> {
    match crate::config::Config::from_str(content) {
        Ok(config) => {
            let ports = config.network.port_mappings()?;
            let managed_network = config.network.managed_bridge()?;
            let agent = toml::from_str::<toml::Value>(content)
                .ok()
                .and_then(|value| {
                    value
                        .get("agent")
                        .and_then(|agent| agent.get("preferred"))
                        .and_then(toml::Value::as_str)
                        .map(str::to_string)
                });

            Ok(ImportedSandboxConfig {
                name: config.sandbox.name.clone(),
                image: config.docker_image(),
                vcpus: config.resources.vcpus,
                memory_mb: config.resources.memory_mb,
                ports,
                agent,
                init_script: config.sandbox.init_script.clone(),
                managed_network,
                permissions: config.get_permissions(),
            })
        }
        Err(config_error) => match toml::from_str::<LegacySandboxConfig>(content) {
            Ok(legacy) => Ok(ImportedSandboxConfig {
                name: legacy.name,
                image: legacy.image,
                vcpus: legacy.vcpus,
                memory_mb: legacy.memory_mb,
                ports: legacy.ports,
                agent: legacy.agent,
                init_script: legacy.init_script,
                managed_network: legacy.managed_network,
                permissions: crate::permissions::Permissions::default(),
            }),
            Err(legacy_error) => Err(anyhow::anyhow!(
                "{config_error} (legacy format: {legacy_error})"
            )),
        },
    }
}

async fn handle_import_sandbox_config(
    req: Request<Incoming>,
    state: Arc<AppState>,
    path_name: Option<&str>,
) -> Response<BoxBody> {
    #[cfg(feature = "enterprise")]
    let identity = extract_identity(&req, &state).await;

    #[derive(Deserialize)]
    struct ImportRequest {
        /// Optional name override. When omitted, the name from [sandbox] is
        /// used, matching the CLI import command.
        #[serde(default, alias = "as_name")]
        name: Option<String>,
        config: String,
    }

    let body: ImportRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Parse before touching the manager so malformed uploads fail without
    // creating anything. The parser also accepts the old SandboxState-shaped
    // export emitted by earlier versions of GET /sandboxes/:name/config.
    let parsed = match parse_imported_sandbox_config(&body.config) {
        Ok(s) => s,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error(format!("Invalid TOML config: {e}")),
            );
        }
    };

    let name = path_name
        .map(str::to_string)
        .or(body.name)
        .unwrap_or_else(|| parsed.name.clone());
    let name = name.trim().to_string();
    if let Err(e) = validation::validate_sandbox_name(&name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    if let Err(e) = validation::validate_docker_image(&parsed.image) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    #[cfg(feature = "enterprise")]
    let quota_subject = quota_subject(&state, &identity);
    #[cfg(feature = "enterprise")]
    let quota_guard = state.quota_controller.lock().await;

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    #[cfg(feature = "enterprise")]
    if let Err(error) =
        quota_guard.check_create(&manager, &quota_subject, parsed.vcpus, parsed.memory_mb)
    {
        return quota_denial(&name, &quota_subject, "import", error);
    }

    if parsed.managed_network.is_some()
        && !matches!(
            manager.backend(),
            crate::backend::BackendType::Docker | crate::backend::BackendType::Podman
        )
    {
        return json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &ApiResponse::<()>::error(
                "Managed bridge networking is supported only by Docker and Podman backends",
            ),
        );
    }

    if let Err(e) = manager
        .create_with_agent(
            &name,
            &parsed.image,
            parsed.vcpus,
            parsed.memory_mb,
            None,
            parsed.ports.clone(),
            parsed.agent.clone(),
        )
        .await
    {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to create sandbox from config: {e}")),
        );
    }

    #[cfg(feature = "enterprise")]
    if let Err(e) = manager.set_owner_metadata(
        &name,
        Some(&quota_subject.user_id),
        Some(&quota_subject.org_id),
    ) {
        let _ = manager.remove(&name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to persist imported sandbox ownership: {e}")),
        );
    }

    if let Some(script) = parsed.init_script.as_deref()
        && let Err(e) = manager.set_init_script(&name, script)
    {
        let _ = manager.remove(&name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set init script: {e}")),
        );
    }

    if let Some(network) = parsed.managed_network.clone()
        && let Err(e) = manager.set_managed_network(&name, Some(network))
    {
        let _ = manager.remove(&name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to set managed network: {e}")),
        );
    }

    if let Err(e) = manager
        .start_with_permissions(&name, &parsed.permissions)
        .await
    {
        let _ = manager.remove(&name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(format!("Failed to start sandbox from config: {e}")),
        );
    }

    // Return info about the newly created sandbox
    let running = manager.is_running(&name);
    let ip = if running {
        manager.get_container_ip(&name)
    } else {
        None
    };

    json_response(
        StatusCode::OK,
        &ApiResponse::success(SandboxInfo {
            name: name.clone(),
            uuid: manager
                .get_state(&name)
                .map(|s| s.uuid.clone())
                .unwrap_or_default(),
            status: if running {
                "running".to_string()
            } else {
                "stopped".to_string()
            },
            backend: recorded_backend(
                manager.get_state(&name).and_then(|s| s.backend),
                manager.backend(),
            )
            .to_string(),
            ip,
            image: Some(parsed.image),
            vcpus: Some(parsed.vcpus),
            memory_mb: Some(parsed.memory_mb),
            created_at: manager.get_state(&name).map(|s| s.created_at.clone()),
            created_from_template: None,
            template_help_text: None,
            ports: manager
                .get_state(&name)
                .map(|s| s.ports.iter().map(ToString::to_string).collect())
                .unwrap_or_default(),
            endpoints: manager
                .get_state(&name)
                .map(|s| s.endpoints.clone())
                .unwrap_or_default(),
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            workspace_revision: manager
                .get_state(&name)
                .and_then(|s| s.workspace_revision.clone()),
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

    fn start_test_sandbox(name: &str, uuid: &str) -> crate::vmm::SandboxState {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "uuid": uuid,
            "image": "alpine:3.24",
            "vcpus": 1,
            "memory_mb": 512,
            "vsock_cid": 3,
            "created_at": "2026-01-01T00:00:00Z",
            "backend": "Firecracker",
            "owner_user_id": "persisted-owner",
            "owner_org_id": "persisted-org"
        }))
        .unwrap()
    }

    #[test]
    fn explicit_governance_config_parse_failure_is_not_silently_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agentkernel.toml");
        std::fs::write(&path, "[llm_governance\n").unwrap();
        assert!(AppState::new_with_config(vec![], None, vec![], Some(&path)).is_err());
    }

    #[test]
    fn explicit_config_path_is_the_governance_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        std::fs::write(
            &path,
            r#"
                [sandbox]
                name = "governed"
                [llm_governance]
                enabled = true
                [llm_governance.tenants.acme]
                openai = ["gpt-4o"]
            "#,
        )
        .unwrap();

        let state = AppState::new_with_config(vec![], None, vec![], Some(&path)).unwrap();
        assert!(state.llm_governance.enabled);
        assert_eq!(state.server_config_path.as_deref(), Some(path.as_path()));
    }

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

    #[test]
    fn persisted_start_configuration_preserves_permissions_and_binary_files() {
        let temp = tempfile::tempdir().unwrap();
        let sandbox = start_test_sandbox("configured", "019d0000-0000-7000-8000-000000000001");
        let permissions = Permissions {
            network: false,
            mount_cwd: true,
            mount_home: true,
            pass_env: true,
            allow_privileged: false,
            read_only_root: true,
            max_memory_mb: Some(2048),
            max_cpu_percent: Some(75),
            seccomp: Some("restrictive".to_string()),
        };
        let files = vec![FileInjection {
            content: vec![0, 1, 2, 0xff],
            dest: "/workspace/config.bin".to_string(),
        }];

        let request =
            persist_start_configuration(temp.path(), &sandbox, &permissions, &files).unwrap();
        let encoded = serde_json::to_vec(&request).unwrap();
        let encoded_json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(encoded_json["configuration"]["source"], "persisted");
        assert_eq!(
            encoded_json["configuration"]["token"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let token = encoded_json["configuration"]["token"].as_str().unwrap();
            let mode =
                std::fs::metadata(persisted_start_configuration_path(temp.path(), token).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(!String::from_utf8_lossy(&encoded).contains("mount_home"));
        assert!(!String::from_utf8_lossy(&encoded).contains("content_base64"));

        let (decoded_permissions, decoded_files, state_sha256) =
            parse_start_sandbox_request(&encoded)
                .unwrap()
                .into_runtime(
                    temp.path(),
                    &PersistedStartBinding::from_state(&sandbox),
                    &local_start_request_owner_id(),
                )
                .unwrap();
        assert!(decoded_permissions.mount_cwd);
        assert!(decoded_permissions.mount_home);
        assert!(!decoded_permissions.network);
        assert_eq!(decoded_permissions.max_memory_mb, Some(2048));
        assert_eq!(decoded_permissions.max_cpu_percent, Some(75));
        assert_eq!(decoded_files.len(), 1);
        assert_eq!(decoded_files[0].dest, "/workspace/config.bin");
        assert_eq!(decoded_files[0].content, vec![0, 1, 2, 0xff]);
        assert_eq!(state_sha256, Some(sandbox_state_sha256(&sandbox).unwrap()));

        let replay = parse_start_sandbox_request(&encoded)
            .unwrap()
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&sandbox),
                &local_start_request_owner_id(),
            )
            .unwrap_err();
        assert!(format!("{replay:#}").contains("already consumed"));
    }

    #[test]
    fn persisted_start_state_hash_is_stable_across_map_insertion_order() {
        let mut first = start_test_sandbox("canonical-a", "canonical-uuid");
        first.labels.clear();
        first.labels.insert("alpha".to_string(), "1".to_string());
        first.labels.insert("beta".to_string(), "2".to_string());

        let mut second = first.clone();
        second.labels.clear();
        second.labels.insert("beta".to_string(), "2".to_string());
        second.labels.insert("alpha".to_string(), "1".to_string());

        assert_eq!(
            sandbox_state_sha256(&first).unwrap(),
            sandbox_state_sha256(&second).unwrap()
        );
    }

    #[test]
    fn empty_start_request_keeps_legacy_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let sandbox = start_test_sandbox("legacy", "019d0000-0000-7000-8000-000000000002");
        let (permissions, files, state_sha256) = parse_start_sandbox_request(&[])
            .unwrap()
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&sandbox),
                "anonymous",
            )
            .unwrap();
        let defaults = Permissions::default();

        assert_eq!(permissions.network, defaults.network);
        assert_eq!(permissions.mount_cwd, defaults.mount_cwd);
        assert_eq!(permissions.mount_home, defaults.mount_home);
        assert_eq!(permissions.read_only_root, defaults.read_only_root);
        assert!(files.is_empty());
        assert!(state_sha256.is_none());
    }

    #[test]
    fn persisted_start_configuration_rejects_recreated_sandbox_and_is_consumed() {
        let temp = tempfile::tempdir().unwrap();
        let original = start_test_sandbox("reused", "019d0000-0000-7000-8000-000000000003");
        let recreated = start_test_sandbox("reused", "019d0000-0000-7000-8000-000000000004");
        let request =
            persist_start_configuration(temp.path(), &original, &Permissions::default(), &[])
                .unwrap();
        let encoded = serde_json::to_vec(&request).unwrap();

        let error = parse_start_sandbox_request(&encoded)
            .unwrap()
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&recreated),
                &local_start_request_owner_id(),
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("sandbox generation and owner"));

        let replay = parse_start_sandbox_request(&encoded)
            .unwrap()
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&original),
                &local_start_request_owner_id(),
            )
            .unwrap_err();
        assert!(format!("{replay:#}").contains("already consumed"));
    }

    #[test]
    fn persisted_start_configuration_is_bound_to_owner_and_request_identity() {
        let temp = tempfile::tempdir().unwrap();
        let sandbox = start_test_sandbox("owned", "019d0000-0000-7000-8000-000000000007");
        let mut wrong_owner = sandbox.clone();
        wrong_owner.owner_user_id = Some("other-owner".to_string());
        let request =
            persist_start_configuration(temp.path(), &sandbox, &Permissions::default(), &[])
                .unwrap();

        let error = request
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&wrong_owner),
                "different-request-owner",
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("sandbox generation and owner"));
    }

    #[test]
    fn newer_start_configuration_invalidates_stale_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let sandbox = start_test_sandbox("tightened", "019d0000-0000-7000-8000-000000000008");
        let permissive = Permissions {
            mount_home: true,
            allow_privileged: true,
            ..Permissions::default()
        };
        let stale = persist_start_configuration(temp.path(), &sandbox, &permissive, &[]).unwrap();

        let restrictive = Permissions::default();
        let current =
            persist_start_configuration(temp.path(), &sandbox, &restrictive, &[]).unwrap();
        let stale_error = stale
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&sandbox),
                &local_start_request_owner_id(),
            )
            .unwrap_err();
        assert!(format!("{stale_error:#}").contains("already consumed"));

        let (permissions, _, _) = current
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&sandbox),
                &local_start_request_owner_id(),
            )
            .unwrap();
        assert!(!permissions.mount_home);
        assert!(!permissions.allow_privileged);
    }

    #[test]
    fn pending_start_configuration_is_scrubbed_on_remove() {
        let temp = tempfile::tempdir().unwrap();
        let sandbox = start_test_sandbox("remove-me", "019d0000-0000-7000-8000-000000000005");
        let request =
            persist_start_configuration(temp.path(), &sandbox, &Permissions::default(), &[])
                .unwrap();
        remove_persisted_start_configurations_for_sandbox(temp.path(), "remove-me").unwrap();

        let error = request
            .into_runtime(
                temp.path(),
                &PersistedStartBinding::from_state(&sandbox),
                &local_start_request_owner_id(),
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("already consumed"));
    }

    #[test]
    fn public_start_request_rejects_caller_supplied_capabilities() {
        let request = br#"{"permissions":{"allow_privileged":true,"mount_home":true}}"#;
        let error = parse_start_sandbox_request(request).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("unknown field `permissions`"));
    }

    #[tokio::test]
    async fn authenticated_start_caller_cannot_expand_host_capabilities() {
        let state = Arc::new(AppState::with_api_keys(vec!["owner".to_string()]));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/demo/start"))
            .bearer_auth("owner")
            .json(&serde_json::json!({
                "permissions": {
                    "allow_privileged": true,
                    "mount_home": true,
                    "mount_cwd": true,
                    "pass_env": true
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.text().await.unwrap();
        assert!(body.contains("unknown field `permissions`"), "{body}");
        task.await.unwrap();
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn router_allows_only_valid_token_to_claim_unowned_first_start() {
        let temp = tempfile::tempdir().unwrap();
        let mut sandbox = start_test_sandbox("first-start", "first-start-uuid");
        sandbox.tenant_id = None;
        sandbox.owner_user_id = None;
        sandbox.owner_org_id = None;
        let token = "a".repeat(64);
        let manifest = PersistedStartConfiguration::from_runtime(
            &sandbox,
            api_key_owner_id("owner"),
            &Permissions::default(),
            &[],
        )
        .unwrap();
        crate::secure_fs::write_private_json(
            &persisted_start_configuration_path(temp.path(), &token).unwrap(),
            &manifest,
        )
        .unwrap();

        let mut manager = VmManager::for_tests(temp.path()).unwrap();
        manager.insert_state_for_tests(sandbox);
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        let state = AppState::with_api_keys(vec!["owner".to_string()]);
        assert!(state.vm_manager.set(manager.clone()).is_ok());
        *state.quota_controller.lock().await =
            crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
                enabled: true,
                default_limits: crate::config::ResourceQuotaLimits {
                    max_running_sandboxes: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            });
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let state = server_state.clone();
                let service = service_fn(move |request| {
                    let state = state.clone();
                    handle_request(request, state)
                });
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            }
        });

        let without_token = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/first-start/start"))
            .bearer_auth("owner")
            .header("connection", "close")
            .body("")
            .send()
            .await
            .unwrap();
        assert_eq!(without_token.status(), StatusCode::NOT_FOUND);

        let with_token = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/first-start/start"))
            .bearer_auth("owner")
            .header("connection", "close")
            .json(&StartSandboxRequest {
                configuration: Some(PersistedStartReference {
                    source: StartConfigurationSource::Persisted,
                    token,
                }),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(with_token.status(), StatusCode::TOO_MANY_REQUESTS);
        task.await.unwrap();

        let manager = manager.read().await;
        let claimed = manager.get_state("first-start").unwrap();
        let expected_owner = api_key_owner_id("owner");
        assert_eq!(
            claimed.owner_user_id.as_deref(),
            Some(expected_owner.as_str())
        );
        assert_eq!(claimed.owner_org_id.as_deref(), Some("default"));
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn delegated_start_refreshes_idle_state_and_enforces_manifest_hash() {
        let temp = tempfile::tempdir().unwrap();
        let owner_id = api_key_owner_id("owner");

        let mut stale = start_test_sandbox("handoff-refresh", "handoff-refresh-uuid");
        stale.owner_user_id = Some(owner_id.clone());
        stale.owner_org_id = Some("default".to_string());
        let mut final_state = stale.clone();
        final_state.memory_mb = 2048;
        final_state.work_dir = Some("/workspace/final".to_string());
        let expected_hash = sandbox_state_sha256(&final_state).unwrap();
        let valid_token = "b".repeat(64);
        let valid_manifest = PersistedStartConfiguration::from_runtime(
            &final_state,
            owner_id.clone(),
            &Permissions::default(),
            &[],
        )
        .unwrap();
        crate::secure_fs::write_private_json(
            &persisted_start_configuration_path(temp.path(), &valid_token).unwrap(),
            &valid_manifest,
        )
        .unwrap();

        let mut stale_mismatch = start_test_sandbox("handoff-mismatch", "handoff-mismatch-uuid");
        stale_mismatch.owner_user_id = Some(owner_id.clone());
        stale_mismatch.owner_org_id = Some("default".to_string());
        let expected_mismatch = stale_mismatch.clone();
        let mismatch_token = "c".repeat(64);
        let mismatch_manifest = PersistedStartConfiguration::from_runtime(
            &expected_mismatch,
            owner_id,
            &Permissions::default(),
            &[],
        )
        .unwrap();
        crate::secure_fs::write_private_json(
            &persisted_start_configuration_path(temp.path(), &mismatch_token).unwrap(),
            &mismatch_manifest,
        )
        .unwrap();
        let mut tampered_mismatch = expected_mismatch;
        tampered_mismatch.memory_mb = 4096;

        let mut manager = VmManager::for_tests(temp.path()).unwrap();
        manager.insert_state_for_tests(stale);
        manager.insert_state_for_tests(stale_mismatch);
        std::fs::write(
            temp.path().join("sandboxes/handoff-refresh.json"),
            serde_json::to_vec_pretty(&final_state).unwrap(),
        )
        .unwrap();
        std::fs::write(
            temp.path().join("sandboxes/handoff-mismatch.json"),
            serde_json::to_vec_pretty(&tampered_mismatch).unwrap(),
        )
        .unwrap();
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        let state = AppState::with_api_keys(vec!["owner".to_string()]);
        assert!(state.vm_manager.set(manager.clone()).is_ok());
        *state.quota_controller.lock().await =
            crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
                enabled: true,
                default_limits: crate::config::ResourceQuotaLimits {
                    max_running_sandboxes: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            });
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let state = server_state.clone();
                let service = service_fn(move |request| {
                    let state = state.clone();
                    handle_request(request, state)
                });
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            }
        });

        let valid = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/handoff-refresh/start"))
            .bearer_auth("owner")
            .header("connection", "close")
            .json(&StartSandboxRequest {
                configuration: Some(PersistedStartReference {
                    source: StartConfigurationSource::Persisted,
                    token: valid_token,
                }),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(valid.status(), StatusCode::TOO_MANY_REQUESTS);

        let mismatch = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/handoff-mismatch/start"))
            .bearer_auth("owner")
            .header("connection", "close")
            .json(&StartSandboxRequest {
                configuration: Some(PersistedStartReference {
                    source: StartConfigurationSource::Persisted,
                    token: mismatch_token,
                }),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(mismatch.status(), StatusCode::CONFLICT);
        assert!(
            mismatch
                .text()
                .await
                .unwrap()
                .contains("changed after the start handoff")
        );
        task.await.unwrap();

        let manager = manager.read().await;
        let adopted = manager.get_state("handoff-refresh").unwrap();
        assert_eq!(adopted.memory_mb, 2048);
        assert_eq!(adopted.work_dir.as_deref(), Some("/workspace/final"));
        assert_eq!(sandbox_state_sha256(adopted).unwrap(), expected_hash);
    }

    #[tokio::test]
    async fn get_refreshes_cli_created_firecracker_state_for_authoritative_status() {
        let temp = tempfile::tempdir().unwrap();
        let mut sandbox =
            start_test_sandbox("cli-no-start", "019d0000-0000-7000-8000-000000000006");
        sandbox.owner_user_id = Some("anonymous".to_string());
        sandbox.owner_org_id = Some("default".to_string());
        let manager = VmManager::for_tests(temp.path()).unwrap();
        std::fs::write(
            temp.path().join("sandboxes/cli-no-start.json"),
            serde_json::to_vec_pretty(&sandbox).unwrap(),
        )
        .unwrap();
        let state = AppState::with_api_keys(vec![]);
        assert!(
            state
                .vm_manager
                .set(Arc::new(tokio::sync::RwLock::new(manager)))
                .is_ok()
        );
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .get(format!("http://{address}/sandboxes/cli-no-start"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["data"]["status"], "stopped");
        assert_eq!(body["data"]["backend"], "firecracker");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn extend_ttl_route_updates_authoritative_manager_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut sandbox = start_test_sandbox("ttl-owner", "ttl-owner-uuid");
        sandbox.owner_user_id = Some("anonymous".to_string());
        sandbox.owner_org_id = Some("default".to_string());
        let mut manager = VmManager::for_tests(temp.path()).unwrap();
        manager.insert_state_for_tests(sandbox);
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        let state = AppState::with_api_keys(vec![]);
        assert!(state.vm_manager.set(manager.clone()).is_ok());
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/ttl-owner/extend"))
            .json(&serde_json::json!({"by": "30m"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        let response_expiry = body["data"]["expires_at"].as_str().unwrap().to_string();
        task.await.unwrap();

        let manager = manager.read().await;
        assert_eq!(
            manager
                .get_state("ttl-owner")
                .and_then(|sandbox| sandbox.expires_at.as_deref()),
            Some(response_expiry.as_str())
        );
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn disk_only_owned_delete_and_extend_deny_cross_tenant_mutation_after_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let owner_id = api_key_owner_id("owner");
        let mut delete_state = start_test_sandbox("disk-delete", "disk-delete-uuid");
        delete_state.owner_user_id = Some(owner_id.clone());
        delete_state.owner_org_id = Some("default".to_string());
        let mut extend_state = start_test_sandbox("disk-extend", "disk-extend-uuid");
        extend_state.owner_user_id = Some(owner_id);
        extend_state.owner_org_id = Some("default".to_string());
        extend_state.ttl_seconds = Some(600);
        extend_state.expires_at = Some("2026-09-01T00:00:00Z".to_string());

        let manager = VmManager::for_tests(temp.path()).unwrap();
        let delete_path = temp.path().join("sandboxes/disk-delete.json");
        let extend_path = temp.path().join("sandboxes/disk-extend.json");
        std::fs::write(
            &delete_path,
            serde_json::to_vec_pretty(&delete_state).unwrap(),
        )
        .unwrap();
        std::fs::write(
            &extend_path,
            serde_json::to_vec_pretty(&extend_state).unwrap(),
        )
        .unwrap();
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        let state = AppState::with_api_keys(vec!["owner".to_string(), "intruder".to_string()]);
        assert!(state.vm_manager.set(manager.clone()).is_ok());
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let state = server_state.clone();
                let service = service_fn(move |request| {
                    let state = state.clone();
                    handle_request(request, state)
                });
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            }
        });

        let delete = reqwest::Client::new()
            .delete(format!("http://{address}/sandboxes/disk-delete"))
            .bearer_auth("intruder")
            .header("connection", "close")
            .send()
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::NOT_FOUND);

        let extend = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/disk-extend/extend"))
            .bearer_auth("intruder")
            .header("connection", "close")
            .json(&serde_json::json!({"by": "30m"}))
            .send()
            .await
            .unwrap();
        assert_eq!(extend.status(), StatusCode::NOT_FOUND);
        task.await.unwrap();

        assert!(delete_path.exists());
        let persisted: crate::vmm::SandboxState =
            serde_json::from_slice(&std::fs::read(extend_path).unwrap()).unwrap();
        assert_eq!(persisted.expires_at, extend_state.expires_at);
        let manager = manager.read().await;
        assert!(manager.exists("disk-delete"));
        assert_eq!(
            manager
                .get_state("disk-extend")
                .and_then(|sandbox| sandbox.expires_at.as_deref()),
            extend_state.expires_at.as_deref()
        );
    }

    #[tokio::test]
    async fn sandbox_lifecycle_error_preserves_context_and_recovery_path() {
        let error = anyhow::anyhow!("Sandbox 'demo' is not paused").context(
            "checkpoint restore failed; recovery artifacts retained at /tmp/agentkernel-recovery",
        );
        let response = sandbox_lifecycle_error("resume", error);
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let message = body["error"].as_str().unwrap();
        assert!(message.contains("checkpoint restore failed"));
        assert!(message.contains("/tmp/agentkernel-recovery"));
        assert!(message.contains("Sandbox 'demo' is not paused"));
    }

    #[tokio::test]
    async fn server_owned_lifecycle_task_survives_http_waiter_cancellation() {
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel::<()>();
        let lifecycle_task = tokio::spawn(async move {
            let _ = release_rx.await;
            let _ = completed_tx.send(());
            json_response(StatusCode::OK, &ApiResponse::success("completed"))
        });
        let waiter = tokio::spawn(await_server_owned_lifecycle("pause", lifecycle_task));
        tokio::task::yield_now().await;
        waiter.abort();

        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), completed_rx)
            .await
            .expect("detached lifecycle task should keep running")
            .unwrap();
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn full_state_http_policy_requires_run_for_runtime_transitions() {
        assert_eq!(FULL_STATE_RUNTIME_POLICY_ACTION, crate::policy::Action::Run);
        assert_eq!(FORK_SOURCE_POLICY_ACTION, crate::policy::Action::Run);
        assert_eq!(
            FORK_CHILD_POLICY_ACTIONS,
            [crate::policy::Action::Create, crate::policy::Action::Run]
        );
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn non_admin_cannot_trigger_fleet_gc_or_lifecycle_reconcile() {
        let state = Arc::new(AppState::with_api_keys(vec!["owner".to_string()]));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let state = server_state.clone();
                let service = service_fn(move |request| {
                    let state = state.clone();
                    handle_request(request, state)
                });
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            }
        });

        for path in ["gc", "lifecycle/reconcile"] {
            let response = reqwest::Client::new()
                .post(format!("http://{address}/{path}"))
                .bearer_auth("owner")
                .header("connection", "close")
                .json(&serde_json::json!({"dry_run": true}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
        task.await.unwrap();
    }

    #[test]
    fn test_legacy_sandbox_state_config_remains_importable() {
        // This shape matches the raw SandboxState TOML previously returned by
        // GET /sandboxes/:name/config. Unknown runtime fields are intentionally
        // ignored so old exports remain useful as portable imports.
        let legacy = r#"
name = "legacy-sandbox"
uuid = "runtime-only-id"
# Legacy compatibility fixture: preserve imports produced with the former default.
image = "alpine:3.20"
vcpus = 2
memory_mb = 1024
vsock_cid = 7
created_at = "2026-01-01T00:00:00Z"
ports = [{ host_port = 18080, container_port = 80, protocol = "tcp" }]
agent = "codex"
init_script = "echo ready"
"#;

        let parsed = parse_imported_sandbox_config(legacy).unwrap();
        assert_eq!(parsed.name, "legacy-sandbox");
        // Legacy compatibility: the importer must preserve explicitly exported images.
        assert_eq!(parsed.image, "alpine:3.20");
        assert_eq!(parsed.vcpus, 2);
        assert_eq!(parsed.memory_mb, 1024);
        assert_eq!(parsed.ports.len(), 1);
        assert_eq!(parsed.ports[0].host_port, Some(18080));
        assert_eq!(parsed.ports[0].container_port, 80);
        assert_eq!(parsed.agent.as_deref(), Some("codex"));
        assert_eq!(parsed.init_script.as_deref(), Some("echo ready"));
    }

    #[test]
    fn test_recording_ids_cannot_escape_recordings_directory() {
        assert!(valid_recording_id("sandbox-20260823-120000"));
        assert!(valid_recording_id("recording.v2"));
        assert!(!valid_recording_id(""));
        assert!(!valid_recording_id("../secrets"));
        assert!(!valid_recording_id("recording/cast"));
        assert!(!valid_recording_id("%2e%2e%2fsecrets"));
        assert!(!valid_recording_id(".hidden"));
    }

    #[test]
    fn test_recording_event_serializes_asciicast_types() {
        let output = recording_event(AsciicastEvent::new(1.25, EventType::Output, "hello\r\n"));
        let input = recording_event(AsciicastEvent::new(1.5, EventType::Input, "ls\r\n"));
        assert_eq!(output.event_type, "output");
        assert_eq!(output.time, 1.25);
        assert_eq!(output.data, "hello\r\n");
        assert_eq!(input.event_type, "input");
    }

    #[test]
    fn test_recording_summary_reads_cast_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sandbox-20260823-120000.cast");
        let header = AsciicastHeader::with_size(120, 40)
            .with_title("Recorded shell")
            .with_command("agentkernel attach sandbox");
        let mut recorder = crate::asciicast::AsciicastRecorder::with_header(&path, header.clone());
        recorder.record_output("hello\r\n");
        recorder.save().unwrap();

        let (read_header, events) = crate::asciicast::read_asciicast(&path).unwrap();
        let summary = recording_summary(&path, &read_header, &events).unwrap();
        assert_eq!(summary.id, "sandbox-20260823-120000");
        assert_eq!(summary.filename, "sandbox-20260823-120000.cast");
        assert_eq!(summary.width, 120);
        assert_eq!(summary.height, 40);
        assert_eq!(summary.event_count, 1);
        assert_eq!(summary.title.as_deref(), Some("Recorded shell"));
        assert_eq!(
            summary.command.as_deref(),
            Some("agentkernel attach sandbox")
        );
        assert!(summary.size_bytes > 0);
    }

    // === Request deserialization tests ===

    #[test]
    fn test_run_request_deserialize() {
        // Legacy compatibility: explicitly requested old tags deserialize unchanged.
        let json = r#"{"command": ["echo", "hello"], "image": "alpine:3.20"}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["echo", "hello"]);
        assert_eq!(req.image, Some("alpine:3.20".to_string())); // legacy compatibility
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
        assert!(req.volumes.is_empty());
    }

    #[test]
    fn test_fork_sandbox_request_contract() {
        let request: ForkSandboxRequest =
            serde_json::from_str(r#"{"as_name":"experiment-b"}"#).unwrap();
        assert_eq!(request.as_name, "experiment-b");

        let unknown_field =
            serde_json::from_str::<ForkSandboxRequest>(r#"{"as_name":"child","start":true}"#);
        assert!(unknown_field.is_err());
    }

    #[test]
    fn test_create_request_deserialize_volumes() {
        let json = r#"{
            "name": "volume-sandbox",
            "volumes": ["my-data:/data", "cache:/cache:ro"]
        }"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.volumes,
            vec!["my-data:/data".to_string(), "cache:/cache:ro".to_string()]
        );
        let mounts: Vec<VolumeMount> = req
            .volumes
            .iter()
            .map(|spec| VolumeMount::parse(spec))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(mounts[0].slug, "my-data");
        assert_eq!(mounts[0].mount_path, "/data");
        assert!(mounts[1].read_only);
    }

    #[test]
    fn test_create_request_rejects_malformed_volumes_before_creation() {
        let error = validate_volume_specs(&["cache:/cache:rw".to_string()], None).unwrap_err();
        assert!(error.to_string().contains("Invalid volume mount format"));
    }

    #[test]
    fn test_create_request_rejects_unsupported_backend_volumes() {
        let error = validate_backend_volume_support(
            BackendType::Firecracker,
            &["cache:/cache".to_string()],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not support host volume mounts")
        );
        assert!(validate_backend_volume_support(BackendType::Docker, &[]).is_ok());
    }

    #[tokio::test]
    async fn create_route_rejects_malformed_volumes_without_creating_sandbox() {
        let temp = tempfile::tempdir().unwrap();
        let manager = VmManager::for_tests(temp.path()).unwrap();
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        let state = Arc::new(AppState::with_api_keys(vec![]));
        let _ = state.vm_manager.set(manager.clone());

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes"))
            .json(&serde_json::json!({
                "name": "malformed-volume",
                "volumes": ["cache:/cache:rw"]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            response
                .text()
                .await
                .unwrap()
                .contains("Invalid volume mount")
        );
        assert!(manager.read().await.list().is_empty());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn create_route_persists_valid_volumes_before_starting_sandbox() {
        let home = tempfile::tempdir().unwrap();
        let slug = format!("http-volume-{}", uuid::Uuid::now_v7().simple());
        let volume_base_dir = home.path().join(".agentkernel");
        let mut volume_manager = VolumeManager::new_in(&volume_base_dir).unwrap();
        volume_manager.create(&slug, None).unwrap();

        let temp = tempfile::tempdir().unwrap();
        let mut manager = VmManager::for_tests(temp.path()).unwrap();
        manager.set_volume_base_dir_for_tests(volume_base_dir.clone());
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        let mut state = AppState::with_api_keys(vec![]);
        state.volume_base_dir = Some(volume_base_dir);
        let state = Arc::new(state);
        let _ = state.vm_manager.set(manager.clone());

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let sandbox_name = format!("http-volume-sandbox-{}", uuid::Uuid::now_v7().simple());
        let volume_spec = format!("{slug}:/data");
        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes"))
            .json(&serde_json::json!({
                "name": sandbox_name,
                "image": "alpine:3.24",
                "volumes": [volume_spec]
            }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let response_body = response.text().await.unwrap();
        assert_eq!(status, StatusCode::CREATED, "{response_body}");
        task.await.unwrap();

        let persisted = manager
            .read()
            .await
            .get_state(&sandbox_name)
            .unwrap()
            .volumes
            .clone();
        assert_eq!(persisted, vec![volume_spec]);
        let saved = std::fs::read_to_string(
            temp.path()
                .join("sandboxes")
                .join(format!("{sandbox_name}.json")),
        )
        .unwrap();
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["volumes"], serde_json::json!(persisted));

        manager.write().await.remove(&sandbox_name).await.unwrap();
        volume_manager.delete(&slug).unwrap();
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
            endpoints: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            workspace_revision: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"test-sandbox\""));
        assert!(json.contains("\"uuid\":"));
        assert!(json.contains("\"status\":\"running\""));
    }

    #[test]
    fn test_fork_sandbox_result_includes_clone_security_warning() {
        let result = ForkSandboxResult {
            sandbox: SandboxInfo {
                name: "experiment-b".to_string(),
                uuid: uuid::Uuid::now_v7().to_string(),
                status: "running".to_string(),
                backend: "firecracker".to_string(),
                ip: None,
                image: Some("alpine:3.24".to_string()),
                vcpus: Some(1),
                memory_mb: Some(512),
                created_at: None,
                created_from_template: None,
                template_help_text: None,
                ports: vec![],
                endpoints: vec![],
                secret_files: vec![],
                placeholder_secrets: false,
                proxy_port: None,
                secret_mappings: std::collections::HashMap::new(),
                labels: std::collections::HashMap::new(),
                description: None,
                last_activity_at: None,
                workspace_revision: None,
                archived_at: None,
                archived_reason: None,
                lifecycle: None,
            },
            security_warning: crate::full_state::FORK_SECURITY_WARNING.to_string(),
        };

        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["sandbox"]["status"], "running");
        assert!(
            json["security_warning"]
                .as_str()
                .unwrap()
                .contains("cryptographic tokens")
        );
    }

    #[tokio::test]
    async fn pause_route_rejects_non_firecracker_backend() {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = VmManager::for_tests(temp.path()).unwrap();
        let sandbox: crate::vmm::SandboxState = serde_json::from_value(serde_json::json!({
            "name": "docker-source",
            "uuid": uuid::Uuid::now_v7().to_string(),
            "image": "alpine:3.24",
            "vcpus": 1,
            "memory_mb": 512,
            "vsock_cid": 3,
            "created_at": "2026-01-01T00:00:00Z",
            "backend": "Docker",
            "owner_user_id": "anonymous",
            "owner_org_id": "default"
        }))
        .unwrap();
        manager.insert_state_for_tests(sandbox);

        let state = Arc::new(AppState::with_api_keys(vec![]));
        let manager = Arc::new(tokio::sync::RwLock::new(manager));
        assert!(state.vm_manager.set(manager.clone()).is_ok());
        assert!(manager.read().await.exists("docker-source"));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/docker-source/pause"))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body.contains("Firecracker"));
        task.await.unwrap();
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

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn jwks_only_configuration_is_enforced_by_global_auth_gate() {
        let mut state = AppState::with_api_keys(vec![]);
        state.enterprise_config = Some(crate::config::EnterpriseConfig {
            jwks_url: Some("http://127.0.0.1:9/.well-known/jwks.json".to_string()),
            ..Default::default()
        });
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .get(format!("http://{address}/sandboxes"))
            .header("connection", "close")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        task.await.unwrap();
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn quota_subjects_isolate_same_prefix_api_keys_without_secret_leaks() {
        let first_key = "ak_live_shared_prefix_one";
        let second_key = "ak_live_shared_prefix_two";
        let state = AppState::with_api_keys(vec![first_key.to_string(), second_key.to_string()]);
        let first = quota_subject(
            &state,
            &crate::identity::AgentIdentity::from_api_key(first_key.to_string()),
        );
        let second = quota_subject(
            &state,
            &crate::identity::AgentIdentity::from_api_key(second_key.to_string()),
        );

        assert_ne!(first.user_id, second.user_id);
        assert!(first.user_id.starts_with("api-key:"));
        assert!(!first.user_id.contains("ak_live"));
        assert!(!second.user_id.contains("ak_live"));

        let temp = tempfile::tempdir().unwrap();
        let manager = VmManager::for_tests(temp.path()).unwrap();
        let status =
            crate::quota::QuotaController::new(Default::default()).status(&manager, &first);
        let status_json = serde_json::to_string(&status).unwrap();
        assert!(!status_json.contains(first_key));
        assert!(!status_json.contains(second_key));
        let audit = crate::audit::AuditEvent::QuotaDenied {
            sandbox: "example".to_string(),
            principal: first.user_id,
            org_id: first.org_id,
            action: "create".to_string(),
            reason: "limit reached".to_string(),
        };
        let audit_json = serde_json::to_string(&audit).unwrap();
        assert!(!audit_json.contains(first_key));
        assert!(!audit_json.contains(second_key));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn app_state_loads_quota_policy_from_explicit_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("daemon-config.toml");
        std::fs::write(
            &config_path,
            r#"
[sandbox]
name = "quota-test"

[enterprise]
enabled = true
org_id = "configured-org"

[enterprise.quotas]
enabled = true

[enterprise.quotas.default]
max_total_sandboxes = 3
"#,
        )
        .unwrap();

        let state = AppState::new_with_config(vec![], None, vec![], Some(&config_path)).unwrap();
        assert_eq!(
            state
                .enterprise_config
                .as_ref()
                .and_then(|config| config.org_id.as_deref()),
            Some("configured-org")
        );
        let quota = state.quota_controller.blocking_lock();
        assert!(quota.enabled());
        assert_eq!(
            quota
                .status(
                    &VmManager::for_tests(dir.path()).unwrap(),
                    &crate::quota::QuotaSubject {
                        user_id: "alice".to_string(),
                        org_id: "configured-org".to_string(),
                    },
                )
                .user
                .limits
                .max_total_sandboxes,
            Some(3)
        );
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn policy_init_failure_keeps_quota_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("malformed.toml");
        std::fs::write(
            &config_path,
            r#"
[sandbox]
name = "quota-test"

[enterprise]
enabled = true
policy_file = "missing-policy.cedar"

[enterprise.quotas]
enabled = true

[enterprise.quotas.default]
max_total_sandboxes = 0
"#,
        )
        .unwrap();

        let state = AppState::new_with_config(vec![], None, vec![], Some(&config_path)).unwrap();
        let quota = state.quota_controller.blocking_lock();
        assert!(quota.enabled());
        let manager = VmManager::for_tests(dir.path()).unwrap();
        assert!(
            quota
                .check_create(
                    &manager,
                    &crate::quota::QuotaSubject {
                        user_id: "alice".to_string(),
                        org_id: "default".to_string(),
                    },
                    1,
                    512,
                )
                .is_err()
        );
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn create_route_returns_429_for_zero_quota() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_api_keys(vec![]);
        let manager = VmManager::for_tests(dir.path()).unwrap();
        let _ = state
            .vm_manager
            .set(Arc::new(tokio::sync::RwLock::new(manager)));
        *state.quota_controller.lock().await =
            crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
                enabled: true,
                default_limits: crate::config::ResourceQuotaLimits {
                    max_total_sandboxes: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            });
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes"))
            .json(&serde_json::json!({"name": "quota-denied"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = response.text().await.unwrap();
        assert!(body.contains("max_total_sandboxes"));
        task.await.unwrap();
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn sandbox_access_is_owner_scoped_and_admin_can_cross_owners() {
        let state = AppState::with_api_keys(vec![]);
        let owner = crate::identity::AgentIdentity::from_api_key("owner".to_string());
        let other = crate::identity::AgentIdentity::from_api_key("other".to_string());
        let sandbox: crate::vmm::SandboxState = serde_json::from_value(serde_json::json!({
            "name": "owned",
            "uuid": "owned-uuid",
            "image": "alpine:3.24",
            "vcpus": 1,
            "memory_mb": 512,
            "vsock_cid": 3,
            "created_at": "2026-01-01T00:00:00Z",
            "owner_user_id": owner.quota_user_id(),
            "owner_org_id": "default"
        }))
        .unwrap();

        assert!(sandbox_access_allowed(&state, &owner, &sandbox));
        assert!(!sandbox_access_allowed(&state, &other, &sandbox));

        let admin = crate::identity::AgentIdentity::from_jwt(crate::identity::JwtClaims {
            sub: "administrator".to_string(),
            email: "admin@example.invalid".to_string(),
            org_id: "other-org".to_string(),
            roles: vec!["admin".to_string()],
            mfa_verified: true,
            exp: None,
            iat: None,
        });
        assert!(sandbox_access_allowed(&state, &admin, &sandbox));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn trusted_local_principal_can_manage_unowned_no_start_state() {
        let mut sandbox = start_test_sandbox("unowned", "unowned-uuid");
        sandbox.owner_user_id = None;
        sandbox.owner_org_id = None;

        let open_state = AppState::with_api_keys(vec![]);
        assert!(sandbox_access_allowed(
            &open_state,
            &crate::identity::AgentIdentity::anonymous(),
            &sandbox
        ));

        let authenticated_state = AppState::with_api_keys(vec!["owner".to_string()]);
        assert!(sandbox_access_allowed(
            &authenticated_state,
            &crate::identity::AgentIdentity::from_api_key("owner".to_string()),
            &sandbox
        ));
        assert!(!sandbox_access_allowed(
            &authenticated_state,
            &crate::identity::AgentIdentity::anonymous(),
            &sandbox
        ));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn valid_start_token_claims_unowned_sandbox_after_prior_refresh_without_overwrite() {
        fn sandbox(name: &str) -> crate::vmm::SandboxState {
            serde_json::from_value(serde_json::json!({
                "name": name,
                "uuid": format!("{name}-uuid"),
                "image": "alpine:3.24",
                "vcpus": 1,
                "memory_mb": 512,
                "vsock_cid": 3,
                "created_at": "2026-01-01T00:00:00Z",
                "backend": "Firecracker"
            }))
            .unwrap()
        }

        let dir = tempfile::tempdir().unwrap();
        let cli_state = sandbox("cli-created");
        let request =
            persist_start_configuration(dir.path(), &cli_state, &Permissions::default(), &[])
                .unwrap();
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(cli_state);
        let state = AppState::with_api_keys(vec![]);
        let anonymous = crate::identity::AgentIdentity::anonymous();

        // A prior GET may already have refreshed this state, so claim cannot
        // depend on the refresh result of the later start request.
        assert!(!manager.refresh_sandbox_if_missing("cli-created").unwrap());
        let binding = PersistedStartBinding::from_state(manager.get_state("cli-created").unwrap());
        let token_authorizes_first_claim = request.configuration.is_some();
        request
            .into_runtime(dir.path(), &binding, &local_start_request_owner_id())
            .unwrap();
        claim_unowned_start_sandbox(
            &mut manager,
            "cli-created",
            token_authorizes_first_claim,
            &anonymous,
            &state,
        )
        .unwrap();
        let claimed = manager.get_state("cli-created").unwrap();
        assert_eq!(claimed.owner_user_id.as_deref(), Some("anonymous"));
        assert_eq!(claimed.owner_org_id.as_deref(), Some("default"));

        manager
            .set_owner_metadata("cli-created", Some("existing-user"), Some("existing-org"))
            .unwrap();
        claim_unowned_start_sandbox(&mut manager, "cli-created", true, &anonymous, &state).unwrap();
        let preserved = manager.get_state("cli-created").unwrap();
        assert_eq!(preserved.owner_user_id.as_deref(), Some("existing-user"));
        assert_eq!(preserved.owner_org_id.as_deref(), Some("existing-org"));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn unowned_sandbox_cannot_be_claimed_without_valid_start_token() {
        let dir = tempfile::tempdir().unwrap();
        let mut sandbox = start_test_sandbox("unclaimed", "unclaimed-uuid");
        sandbox.owner_user_id = None;
        sandbox.owner_org_id = None;
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(sandbox);
        let state = AppState::with_api_keys(vec![]);
        let anonymous = crate::identity::AgentIdentity::anonymous();

        let response =
            claim_unowned_start_sandbox(&mut manager, "unclaimed", false, &anonymous, &state)
                .unwrap_err();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let unclaimed = manager.get_state("unclaimed").unwrap();
        assert_eq!(unclaimed.owner_user_id, None);
        assert_eq!(unclaimed.owner_org_id, None);
    }

    #[test]
    fn fork_identity_must_match_atomically_cloned_source_owner() {
        let mut source = start_test_sandbox("source", "source-uuid");
        source.tenant_id = Some("tenant-a".to_string());

        assert!(fork_identity_matches_source(
            &source,
            Some("tenant-a"),
            Some("persisted-owner"),
            Some("persisted-org"),
        ));
        assert!(!fork_identity_matches_source(
            &source,
            Some("tenant-b"),
            Some("persisted-owner"),
            Some("persisted-org"),
        ));
        assert!(!fork_identity_matches_source(
            &source,
            Some("tenant-a"),
            Some("other-owner"),
            Some("persisted-org"),
        ));
    }

    #[cfg(not(feature = "enterprise"))]
    #[test]
    fn configured_api_key_start_claim_allows_cli_origin_fork() {
        let mut source = start_test_sandbox("api-key-source", "api-key-source-uuid");
        source.tenant_id = None;
        source.owner_user_id = None;
        source.owner_org_id = None;
        let dir = tempfile::tempdir().unwrap();
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(source);
        let trusted_owner = ("local".to_string(), api_key_owner_id("owner"));

        claim_unowned_local_start_sandbox(
            &mut manager,
            "api-key-source",
            true,
            Some(&trusted_owner),
        )
        .unwrap();
        let source = manager.get_state("api-key-source").unwrap();
        assert!(fork_identity_matches_source(
            source,
            None,
            Some(trusted_owner.1.as_str()),
            Some(trusted_owner.0.as_str()),
        ));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn configured_org_start_claim_allows_cli_origin_fork() {
        let mut source = start_test_sandbox("org-source", "org-source-uuid");
        source.tenant_id = None;
        source.owner_user_id = None;
        source.owner_org_id = None;
        let dir = tempfile::tempdir().unwrap();
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(source);
        let mut state = AppState::with_api_keys(vec!["owner".to_string()]);
        state.enterprise_config = Some(crate::config::EnterpriseConfig {
            enabled: true,
            org_id: Some("acme".to_string()),
            ..Default::default()
        });
        let identity = crate::identity::AgentIdentity::from_api_key("owner".to_string());

        claim_unowned_start_sandbox(&mut manager, "org-source", true, &identity, &state).unwrap();
        let subject = quota_subject(&state, &identity);
        let source = manager.get_state("org-source").unwrap();
        assert!(fork_identity_matches_source(
            source,
            Some("acme"),
            Some(subject.user_id.as_str()),
            Some(subject.org_id.as_str()),
        ));
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn scoped_route_denies_other_tenants_without_leaking_names() {
        fn sandbox(name: &str, owner: &str) -> crate::vmm::SandboxState {
            serde_json::from_value(serde_json::json!({
                "name": name,
                "uuid": format!("{name}-uuid"),
                "image": "alpine:3.24",
                "vcpus": 1,
                "memory_mb": 512,
                "vsock_cid": 3,
                "created_at": "2026-01-01T00:00:00Z",
                "owner_user_id": format!("api-key:{}", hex::encode(sha2::Sha256::digest(owner.as_bytes()))),
                "owner_org_id": "default"
            }))
            .unwrap()
        }

        let dir = tempfile::tempdir().unwrap();
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(sandbox("visible-to-owner", "owner"));
        manager.insert_state_for_tests(sandbox("hidden-from-owner", "other"));
        let state = AppState::with_api_keys(vec!["owner".to_string()]);
        let _ = state
            .vm_manager
            .set(Arc::new(tokio::sync::RwLock::new(manager)));
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let hidden_file = reqwest::Client::new()
            .get(format!(
                "http://{address}/sandboxes/hidden-from-owner/files/etc/passwd"
            ))
            .bearer_auth("owner")
            .send()
            .await
            .unwrap();
        assert_eq!(hidden_file.status(), StatusCode::NOT_FOUND);
        assert!(
            !hidden_file
                .text()
                .await
                .unwrap()
                .contains("hidden-from-owner")
        );
        task.await.unwrap();
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn stopped_creation_is_not_counted_as_running_quota_usage() {
        let dir = tempfile::tempdir().unwrap();
        let manager = VmManager::for_tests(dir.path()).unwrap();
        let subject = crate::quota::QuotaSubject {
            user_id: "alice".to_string(),
            org_id: "acme".to_string(),
        };
        let controller = crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
            enabled: true,
            default_limits: crate::config::ResourceQuotaLimits {
                max_running_sandboxes: Some(0),
                max_total_sandboxes: Some(1),
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(
            controller
                .check_create_stopped(&manager, &subject, 1, 512)
                .is_ok()
        );
        assert!(controller.check_create(&manager, &subject, 1, 512).is_err());
    }

    #[cfg(feature = "enterprise")]
    #[tokio::test]
    async fn import_route_returns_429_for_zero_quota() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_api_keys(vec![]);
        let manager = VmManager::for_tests(dir.path()).unwrap();
        let _ = state
            .vm_manager
            .set(Arc::new(tokio::sync::RwLock::new(manager)));
        *state.quota_controller.lock().await =
            crate::quota::QuotaController::new(crate::config::ResourceQuotaConfig {
                enabled: true,
                default_limits: crate::config::ResourceQuotaLimits {
                    max_total_sandboxes: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            });
        let state = Arc::new(state);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = server_state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::Client::new()
            .post(format!("http://{address}/sandboxes/import-config"))
            .json(&serde_json::json!({
                "config": "name = \"imported\"\nimage = \"alpine:3.24\"\nvcpus = 1\nmemory_mb = 512\n"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            response
                .text()
                .await
                .unwrap()
                .contains("max_total_sandboxes")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn llm_spend_requires_credentials_even_when_other_routes_are_anonymous() {
        let state = Arc::new(AppState::with_api_keys(vec![]));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let state = state.clone();
                handle_request(request, state)
            });
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .unwrap();
        });

        let response = reqwest::get(format!("http://{address}/llm/spend"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response.text().await.unwrap();
        assert!(!body.contains("metrics"));
        task.await.unwrap();
    }

    #[test]
    fn explicit_config_path_is_canonical_even_before_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let path = nested.join("agentkernel.toml");
        let canonical = canonical_config_path(&path).unwrap();
        assert!(canonical.is_absolute());
        assert_eq!(
            canonical,
            std::fs::canonicalize(&nested)
                .unwrap()
                .join("agentkernel.toml")
        );
    }

    #[cfg(test)]
    fn unavailable_backend_state() -> Arc<AppState> {
        let mut state = AppState::with_api_keys(vec![]);
        state.force_backend_unavailable = true;
        Arc::new(state)
    }

    #[tokio::test]
    async fn reachable_http_server_reports_backend_unavailable() {
        async fn get_once(state: Arc<AppState>, path: &str) -> (u16, serde_json::Value) {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| {
                    let state = state.clone();
                    handle_request(request, state)
                });
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            });

            let response = reqwest::get(format!("http://{address}{path}"))
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body = response.json::<serde_json::Value>().await.unwrap();
            task.await.unwrap();
            (status, body)
        }

        let state = unavailable_backend_state();
        let (status, body) = get_once(state.clone(), "/status").await;
        assert_eq!(status, StatusCode::OK.as_u16());
        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["backend"], "unavailable");

        let (status, body) = get_once(state, "/sandboxes").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR.as_u16());
        assert_eq!(body["success"], false);
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|message| message.contains("No sandbox backend available"))
        );
    }

    #[tokio::test]
    async fn task_crud_routes_submit_list_inspect_and_cancel() {
        async fn request_once(
            state: Arc<AppState>,
            method: reqwest::Method,
            path: &str,
            body: Option<serde_json::Value>,
        ) -> (u16, serde_json::Value) {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| {
                    let state = state.clone();
                    handle_request(request, state)
                });
                http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                    .unwrap();
            });

            let client = reqwest::Client::new();
            let mut request = client.request(method, format!("http://{address}{path}"));
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let body = response.json::<serde_json::Value>().await.unwrap();
            drop(client);
            task.await.unwrap();
            (status, body)
        }

        let temp = tempfile::TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("tasks.db")).unwrap();
        let state = Arc::new(AppState::with_task_manager_for_tests(TaskManager::new(
            storage,
        )));

        let (status, body) = request_once(
            state.clone(),
            reqwest::Method::POST,
            "/tasks",
            Some(serde_json::json!({
                "prompt": "inspect the failing test",
                "target_sandbox": "sandbox-1"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED.as_u16());
        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["status"], "queued");
        let task_id = body["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(body["data"]["sandbox"], "sandbox-1");

        let (status, body) =
            request_once(state.clone(), reqwest::Method::GET, "/tasks", None).await;
        assert_eq!(status, StatusCode::OK.as_u16());
        assert_eq!(body["data"].as_array().unwrap().len(), 1);

        let (status, body) = request_once(
            state.clone(),
            reqwest::Method::GET,
            &format!("/tasks/{task_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK.as_u16());
        assert_eq!(body["data"]["status"], "queued");

        let (status, body) = request_once(
            state.clone(),
            reqwest::Method::DELETE,
            &format!("/tasks/{task_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK.as_u16());
        assert_eq!(body["data"]["status"], "cancelled");

        // Cancellation is idempotent for a task already cancelled by another caller.
        let (status, body) = request_once(
            state.clone(),
            reqwest::Method::DELETE,
            &format!("/tasks/{task_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK.as_u16());
        assert_eq!(body["data"]["status"], "cancelled");

        let (status, _) = request_once(
            state.clone(),
            reqwest::Method::GET,
            "/tasks/not-a-uuid",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST.as_u16());

        let (status, _) = request_once(
            state.clone(),
            reqwest::Method::POST,
            "/tasks",
            Some(serde_json::json!({
                "prompt": "valid prompt",
                "sandbox": "sandbox-1",
                "unexpected": true
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST.as_u16());

        let (status, _) = request_once(
            state,
            reqwest::Method::POST,
            "/tasks",
            Some(serde_json::json!({
                "prompt": "valid prompt",
                "sandbox": "bad/name"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST.as_u16());
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
            endpoints: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            workspace_revision: None,
            archived_at: None,
            archived_reason: None,
            lifecycle: None,
        };
        let response = json_response(StatusCode::CREATED, &ApiResponse::success(info));
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_parse_docker_disk_usage_line_format() {
        let usage = parse_docker_disk_usage(
            r#"{"Type":"Images","TotalCount":"3","Active":"1","Size":"2.4GB","Reclaimable":"1.2GB (50%)"}"#,
        )
        .unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].kind, "Images");
        assert_eq!(usage[0].total, "3");
        assert_eq!(usage[0].reclaimable, "1.2GB (50%)");
    }

    #[test]
    fn test_parse_docker_disk_usage_array_with_numeric_fields() {
        let usage = parse_docker_disk_usage(
            r#"[{"type":"Images","TotalCount":3,"Active":1,"Size":2400,"Reclaimable":1200}]"#,
        )
        .unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].total, "3");
        assert_eq!(usage[0].size, "2400");
    }

    #[test]
    fn test_parse_docker_disk_usage_rejects_unparseable_output() {
        let error = parse_docker_disk_usage("not json").unwrap_err();
        assert!(error.contains("invalid disk-usage response"));
    }

    #[test]
    fn test_agentkernel_image_match_is_anchored() {
        let managed = DockerImageRecord {
            id: "sha256:managed".to_string(),
            repository: "agentkernel-my-project".to_string(),
            tag: "latest".to_string(),
        };
        let snapshot = DockerImageRecord {
            id: "sha256:snapshot".to_string(),
            repository: "agentkernel-snap".to_string(),
            tag: "checkpoint".to_string(),
        };
        let unrelated = DockerImageRecord {
            id: "sha256:unrelated".to_string(),
            repository: "my-agentkernel-tools".to_string(),
            tag: "latest".to_string(),
        };
        assert!(is_agentkernel_image(&managed));
        assert!(is_agentkernel_image(&snapshot));
        assert!(!is_agentkernel_image(&unrelated));
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
    fn test_create_request_backend_selection_is_optional_and_normalized() {
        let automatic: CreateRequest = serde_json::from_str(r#"{"name":"auto"}"#).unwrap();
        assert!(automatic.backend.is_none());

        let explicit: CreateRequest =
            serde_json::from_str(r#"{"name":"vm","backend":" FIRECRACKER "}"#).unwrap();
        assert_eq!(
            parse_backend_selection(explicit.backend.as_deref()).unwrap(),
            Some(crate::backend::BackendType::Firecracker)
        );
        assert_eq!(parse_backend_selection(Some("automatic")).unwrap(), None);
        assert!(parse_backend_selection(Some("not-a-backend")).is_err());
    }

    #[test]
    fn test_backend_discovery_serializes_capabilities_and_default() {
        let discovery = backend_discovery(Some(crate::backend::BackendType::Docker));
        let json = serde_json::to_value(discovery).unwrap();
        assert_eq!(json["default_backend"], "docker");
        assert_eq!(json["backends"].as_array().unwrap().len(), 12);
        assert!(json["backends"][0]["capabilities"]["mount_cwd"].is_boolean());
        assert!(json["backends"][0]["readiness_reason"].is_string());
    }

    #[test]
    fn test_backend_discovery_reports_configured_and_usable_readiness() {
        let discovery = backend_discovery(Some(crate::backend::BackendType::Docker));
        let docker = discovery
            .backends
            .iter()
            .find(|backend| backend.backend == "docker")
            .unwrap();
        let readiness = crate::backend::backend_readiness(crate::backend::BackendType::Docker);

        assert_eq!(docker.configured, readiness.configured);
        assert_eq!(docker.usable, readiness.usable);
        assert_eq!(docker.readiness_reason, readiness.reason);
        assert!(!docker.usable || docker.configured);
    }

    #[test]
    fn test_recorded_backend_wins_over_server_default() {
        assert_eq!(
            recorded_backend(
                Some(crate::backend::BackendType::Podman),
                crate::backend::BackendType::Docker,
            ),
            crate::backend::BackendType::Podman
        );
        assert_eq!(
            recorded_backend(None, crate::backend::BackendType::Docker),
            crate::backend::BackendType::Docker
        );
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
            endpoints: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            workspace_revision: None,
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
            endpoints: vec![],
            secret_files: vec![],
            placeholder_secrets: false,
            proxy_port: None,
            secret_mappings: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            workspace_revision: None,
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

    // === Sandbox Git API tests ===

    #[test]
    fn test_validate_git_path_resolves_daytona_relative_paths() {
        assert_eq!(
            validate_git_path("workspace/repo").unwrap(),
            "/workspace/repo"
        );
        assert_eq!(validate_git_path("./repo").unwrap(), "/workspace/repo");
        assert_eq!(
            validate_git_path("/workspace/repo").unwrap(),
            "/workspace/repo"
        );
        assert!(validate_git_path("../outside").is_err());
        assert!(validate_git_path("/workspace/../etc").is_err());
        assert!(validate_git_path("-repo").is_err());
    }

    #[test]
    fn test_validate_git_file_path_rejects_option_and_traversal() {
        assert!(validate_git_file_path("src/main.rs").is_ok());
        assert!(validate_git_file_path(".").is_ok());
        assert!(validate_git_file_path("--cached").is_err());
        assert!(validate_git_file_path("../secret").is_err());
        assert!(validate_git_file_path("/absolute/file").is_err());
    }

    #[test]
    fn test_parse_git_status_matches_daytona_shape() {
        let status = parse_git_status(
            "## feature/api...origin/feature/api [ahead 2, behind 1]\n M src/lib.rs\nA  added.txt\n?? untracked.txt\nR  old.txt -> new.txt\n",
        );
        assert_eq!(status.current_branch, "feature/api");
        assert_eq!(status.upstream.as_deref(), Some("origin/feature/api"));
        assert_eq!(status.ahead, 2);
        assert_eq!(status.behind, 1);
        assert!(status.branch_published);
        assert!(!status.detached);
        assert_eq!(status.file_status.len(), 4);
        assert_eq!(status.file_status[0].name, "src/lib.rs");
        assert_eq!(status.file_status[0].worktree, "Modified");
        assert_eq!(status.file_status[1].staging, "Added");
        assert_eq!(status.file_status[2].worktree, "Untracked");
        assert_eq!(status.file_status[3].name, "new.txt");
        assert_eq!(status.file_status[3].extra, "old.txt");

        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["currentBranch"], "feature/api");
        assert!(json["fileStatus"].is_array());
        assert_eq!(json["branchPublished"], true);
    }

    #[test]
    fn test_parse_git_status_detached_head() {
        let status = parse_git_status("## HEAD (no branch)\n?? notes.txt\n");
        assert!(status.detached);
        assert!(status.current_branch.is_empty());
        assert!(!status.branch_published);
    }

    #[test]
    fn test_git_query_path_is_percent_decoded_and_strict() {
        assert_eq!(
            query_param("path=%2Fworkspace%2Frepo", "path").unwrap(),
            "/workspace/repo"
        );
        assert!(query_param("path=%2Fworkspace&extra=x", "path").is_err());
        assert!(query_param("path=%2Fworkspace&path=%2Frepo", "path").is_err());
        assert!(query_param("", "path").is_err());
    }

    #[test]
    fn test_git_request_rejects_unknown_fields() {
        let error = serde_json::from_str::<GitAddRequest>(
            r#"{"path":"/workspace/repo","files":["."],"command":["git"]}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}
