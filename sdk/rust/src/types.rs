use serde::{Deserialize, Serialize};

/// Security profile for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityProfile {
    Permissive,
    Moderate,
    Restrictive,
}

/// Options for running a command.
#[derive(Debug, Default, Serialize)]
pub struct RunOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<SecurityProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast: Option<bool>,
}

/// Output from a command execution.
#[derive(Debug, Deserialize)]
pub struct RunOutput {
    pub output: String,
}

/// Information about a sandbox.
#[derive(Debug, Deserialize)]
pub struct SandboxInfo {
    pub name: String,
    #[serde(default)]
    pub uuid: Option<String>,
    pub status: String,
    pub backend: String,
    pub image: Option<String>,
    pub vcpus: Option<u32>,
    pub memory_mb: Option<u64>,
    pub created_at: Option<String>,
}

/// SSE stream event.
#[derive(Debug)]
pub struct StreamEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

/// Opaque orchestration payload.
pub type Orchestration = serde_json::Value;
pub type OrchestrationCreateRequest = serde_json::Value;
pub type OrchestrationDefinition = serde_json::Value;

/// Opaque durable object payload.
pub type DurableObject = serde_json::Value;
pub type DurableObjectCreateRequest = serde_json::Value;

/// Opaque schedule payload.
pub type Schedule = serde_json::Value;
pub type ScheduleCreateRequest = serde_json::Value;

/// Opaque durable store payload.
pub type DurableStore = serde_json::Value;
pub type DurableStoreCreateRequest = serde_json::Value;
pub type DurableStoreQueryResult = serde_json::Value;
pub type DurableStoreExecuteResult = serde_json::Value;
pub type DurableStoreCommandResult = serde_json::Value;

/// API response wrapper (internal).
#[derive(Debug, Deserialize)]
pub(crate) struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

/// Run request body (internal).
#[derive(Serialize)]
pub(crate) struct RunRequest {
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<SecurityProfile>,
    pub fast: bool,
}

/// Options for creating a sandbox with a git source.
#[derive(Debug, Default, Serialize)]
pub struct CreateSandboxOptions {
    /// Docker image to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<SecurityProfile>,
    /// Git repository URL to clone into the sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Git ref to checkout after cloning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// Volume mounts (slug:/path or slug:/path:ro). Create volumes via CLI first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub volumes: Vec<String>,
    /// Secret bindings for proxy-based injection (Gondolin pattern).
    /// Secrets are injected as HTTP headers by the host proxy — they never enter the VM.
    /// Formats: `"KEY=value:host"`, `"KEY:host"`, `"KEY:host:header"`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Secret keys to inject as files at `/run/agentkernel/secrets/KEY`.
    /// Values are resolved from the secret vault.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub secret_files: Vec<String>,
}

/// Create sandbox request body (internal).
#[derive(Serialize)]
pub(crate) struct CreateRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<SecurityProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub secret_files: Vec<String>,
}

/// Options for executing a command in a sandbox.
#[derive(Debug, Default, Serialize)]
pub struct ExecOptions {
    /// Environment variables (`KEY=VALUE`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// Working directory inside the container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// Run as root (maps to `--sudo` on CLI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sudo: Option<bool>,
}

/// Exec request body (internal).
#[derive(Serialize)]
pub(crate) struct ExecRequest {
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sudo: Option<bool>,
}

/// File write request body (internal).
#[derive(Serialize)]
pub(crate) struct FileWriteRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

/// Response from reading a file.
#[derive(Debug, Deserialize)]
pub struct FileReadResponse {
    pub content: String,
    pub encoding: String,
    pub size: usize,
}

/// Batch run request body (internal).
#[derive(Serialize)]
pub(crate) struct BatchRunRequest {
    pub commands: Vec<BatchCommand>,
}

/// A command for batch execution.
#[derive(Debug, Serialize)]
pub struct BatchCommand {
    pub command: Vec<String>,
}

/// Result of a single batch command.
#[derive(Debug, Deserialize)]
pub struct BatchResult {
    pub output: Option<String>,
    pub error: Option<String>,
}

/// Response from batch execution.
#[derive(Debug, Deserialize)]
pub struct BatchRunResponse {
    pub results: Vec<BatchResult>,
}

/// Batch file write request body (internal).
#[derive(Serialize)]
pub(crate) struct BatchFileWriteRequest {
    pub files: std::collections::HashMap<String, String>,
}

/// Result of a batch file write.
#[derive(Debug, Deserialize)]
pub struct BatchFileWriteResponse {
    pub written: usize,
}

/// Response from extending a sandbox's TTL.
#[derive(Debug, Deserialize)]
pub struct ExtendTtlResponse {
    pub expires_at: Option<String>,
}

/// Request body for extending TTL (internal).
#[derive(Serialize)]
pub(crate) struct ExtendTtlRequest {
    pub by: String,
}

/// Metadata for a sandbox snapshot.
#[derive(Debug, Deserialize)]
pub struct SnapshotMeta {
    pub name: String,
    pub sandbox: String,
    pub image_tag: String,
    pub backend: String,
    pub base_image: Option<String>,
    pub vcpus: Option<u32>,
    pub memory_mb: Option<u64>,
    pub created_at: String,
}

/// Options for taking a snapshot.
#[derive(Debug, Default, Serialize)]
pub struct TakeSnapshotOptions {
    pub sandbox: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Status of a detached command.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetachedStatus {
    Running,
    Completed,
    Failed,
}

/// A detached (background) command running in a sandbox.
#[derive(Debug, Deserialize)]
pub struct DetachedCommand {
    pub id: String,
    pub sandbox: String,
    pub command: Vec<String>,
    pub pid: u32,
    pub status: DetachedStatus,
    pub exit_code: Option<i32>,
    pub started_at: String,
}

/// Response from detached command logs.
#[derive(Debug, Deserialize)]
pub struct DetachedLogsResponse {
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

// -- Browser types --

/// A link extracted from a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLink {
    pub text: String,
    pub href: String,
}

/// Result of navigating to a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResult {
    pub title: String,
    pub url: String,
    pub text: String,
    pub links: Vec<PageLink>,
}

/// ARIA accessibility tree snapshot of a web page (v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaSnapshot {
    /// ARIA tree as YAML.
    pub snapshot: String,
    /// Current page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Available ref IDs (e.g. `["e1", "e2"]`).
    #[serde(default)]
    pub refs: Vec<String>,
}

/// A browser interaction event for debugging and context recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserEvent {
    /// Monotonic sequence number.
    pub seq: u64,
    /// Event type (e.g. "page.navigated", "page.clicked").
    #[serde(rename = "type")]
    pub event_type: String,
    /// Page name.
    pub page: String,
    /// ISO 8601 timestamp.
    pub ts: String,
}
