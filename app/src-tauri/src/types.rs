use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Security profile for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityProfile {
    Permissive,
    Moderate,
    Restrictive,
}

/// Status of a detached command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetachedStatus {
    Running,
    Completed,
    Failed,
}

// ---------------------------------------------------------------------------
// Response types (returned to the frontend)
// ---------------------------------------------------------------------------

/// Information about a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub name: String,
    #[serde(default)]
    pub uuid: Option<String>,
    pub status: String,
    pub backend: String,
    #[serde(default)]
    pub ip: Option<String>,
    pub image: Option<String>,
    pub vcpus: Option<u32>,
    pub memory_mb: Option<u64>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub created_from_template: Option<String>,
    #[serde(default)]
    pub template_help_text: Option<String>,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub secret_files: Vec<String>,
    /// Secret mappings: env_var → target_host (values stripped for security).
    #[serde(default)]
    pub secret_mappings: std::collections::HashMap<String, String>,
    /// User-defined labels for fleet management and filtering.
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
    /// User-defined description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Output from a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutput {
    pub output: String,
}

/// Response from reading a file in a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadResponse {
    pub content: String,
    pub encoding: String,
    pub size: usize,
}

/// Metadata for a sandbox snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Response from extending a sandbox's TTL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendTtlResponse {
    pub expires_at: Option<String>,
}

/// A detached (background) command running in a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachedLogsResponse {
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

// ---------------------------------------------------------------------------
// Request types (sent from the frontend)
// ---------------------------------------------------------------------------

/// Request body for creating a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSandboxRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<SecurityProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<String>>,
    /// Agent CLI to auto-install on start (e.g., "claude", "gemini", "codex")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Secret bindings for the proxy (e.g., "OPENAI_API_KEY=sk-...:api.openai.com")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// Template secret mappings: env_var → target_host.
    /// Resolved from host env vars at sandbox creation time.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub secret_mappings: std::collections::BTreeMap<String, String>,
    /// Shell script to run inside sandbox after start (e.g., install CLIs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_script: Option<String>,
    /// Template name used to create this sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_from_template: Option<String>,
    /// Plain-text guidance associated with the selected template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_help_text: Option<String>,
    /// User-defined labels for fleet management and filtering.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub labels: std::collections::BTreeMap<String, String>,
    /// User-defined description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Request body for executing a command in a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sudo: Option<bool>,
}

/// Request body for taking a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeSnapshotRequest {
    pub sandbox: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Request body for extending a sandbox TTL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendTtlRequest {
    pub by: String,
}

/// Request body for the quick-run endpoint (`POST /run`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickRunRequest {
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

// ---------------------------------------------------------------------------
// Audit / sandbox logs
// ---------------------------------------------------------------------------

/// A single audit log entry returned by `GET /sandboxes/:name/logs`.
///
/// The backend serializes `AuditEntry` with `#[serde(flatten)]` on its event
/// enum, so every entry has top-level `timestamp`, `pid`, `user` plus the
/// event-specific fields. We capture the known top-level keys and stash the
/// rest in `details` so the frontend can display them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub timestamp: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub user: Option<String>,
    /// The audit event type, e.g. "sandbox_created", "command_executed".
    #[serde(rename = "type", default)]
    pub event_type: Option<String>,
    /// All remaining fields from the flattened event (name, image, command, …).
    #[serde(flatten)]
    pub details: std::collections::HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Diagnostics types
// ---------------------------------------------------------------------------

/// System status information from GET /status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    pub version: String,
    pub backend: String,
    pub api_key_configured: bool,
}

/// A single health check result from GET /doctor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: String,
    pub message: String,
}

/// Aggregated doctor results from GET /doctor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorResult {
    pub checks: Vec<HealthCheck>,
    pub healthy: bool,
}

/// Result of garbage collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcResult {
    pub removed: Vec<String>,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

/// A stored secret (name only, not the value).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Agents/Plugins
// ---------------------------------------------------------------------------

/// Agent/plugin integration info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub display_name: String,
    pub enabled: bool,
    pub description: String,
    pub package: Option<String>,
    pub cli_installed: bool,
    pub cli_version: Option<String>,
    pub tested_version: String,
    pub compatibility_status: String,
    pub install_command: String,
    pub integration_supported: bool,
    pub integration_project_installed: bool,
    pub integration_global_installed: bool,
    pub integration_global_supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIntegrationResult {
    pub target: String,
    pub scope: String,
    pub confirmed: bool,
    pub files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Policy (Enterprise)
// ---------------------------------------------------------------------------

/// Enterprise policy status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStatus {
    pub enabled: bool,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub offline_mode: Option<String>,
    #[serde(default)]
    pub policy_server: Option<String>,
}

/// Result of a policy check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCheckResult {
    pub decision: String,
    pub reason: String,
    #[serde(default)]
    pub matched_policies: Vec<String>,
    #[serde(default)]
    pub evaluation_time_us: u64,
}

/// Result of a policy reload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyReloadResult {
    pub reloaded: bool,
    #[serde(default)]
    pub version: u64,
}

/// A policy audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyAuditEntry {
    pub timestamp: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub decision: String,
    #[serde(default)]
    pub matched_policies: Vec<String>,
    #[serde(default)]
    pub evaluation_time_us: u64,
    #[serde(default)]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// LLM Usage
// ---------------------------------------------------------------------------

/// Accumulated LLM usage for a single provider+model combination within a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmUsageEntry {
    pub provider: String,
    pub model: String,
    pub request_count: u64,
    pub streaming_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub last_request: String,
}

// ---------------------------------------------------------------------------
// Durable Objects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableObjectInfo {
    pub id: String,
    pub class: String,
    pub object_id: String,
    pub status: String,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub storage: serde_json::Value,
    pub idle_timeout_seconds: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateObjectRequest {
    pub class: String,
    pub object_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<serde_json::Value>,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_seconds: i64,
}

fn default_idle_timeout() -> i64 {
    300
}

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub fire_at: Option<String>,
    pub method: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub target_class: Option<String>,
    #[serde(default)]
    pub target_object_id: Option<String>,
    #[serde(default)]
    pub target_orchestration: Option<String>,
    pub status: String,
    #[serde(default)]
    pub last_fired_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScheduleRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fire_at: Option<String>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_orchestration: Option<String>,
}

// ---------------------------------------------------------------------------
// Durable Stores
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableStoreInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStoreRequest {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreExecuteResult {
    pub rows_affected: usize,
    #[serde(default)]
    pub last_insert_rowid: Option<i64>,
}

/// Result of a store command (e.g. Redis command).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreCommandResult {
    pub result: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Docker Images
// ---------------------------------------------------------------------------

/// Information about a cached Docker image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImage {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
    pub created: String,
}

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

/// Result from running a hardware benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub create_ms: f64,
    pub exec_ms: f64,
    pub destroy_ms: f64,
    pub total_ms: f64,
    pub image: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Session Recording
// ---------------------------------------------------------------------------

/// A single recorded command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub command: Vec<String>,
    pub output: String,
    pub exit_code: i32,
    pub timestamp: String,
    pub duration_ms: f64,
}

/// Summary of a sandbox session (from list endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub sandbox: String,
    pub entry_count: u64,
}

/// Recorded session for a sandbox (from detail endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSession {
    pub sandbox: String,
    pub entries: Vec<SessionEntry>,
}

// ---------------------------------------------------------------------------
// API response wrapper (internal)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Interactive Permissions
// ---------------------------------------------------------------------------

/// A stored permission grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionGrant {
    pub id: String,
    pub kind: String,
    pub scope: String,
    #[serde(default)]
    pub sandbox: Option<String>,
    pub granted_at: String,
    pub granted_by: String,
}

/// Request body for granting a permission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantPermissionRequest {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
}

/// Result of a permission check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionCheckResult {
    pub permitted: bool,
    pub kind: String,
}

// ---------------------------------------------------------------------------
// API response wrapper (internal)
// ---------------------------------------------------------------------------

/// Envelope returned by the agentkernel HTTP API.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Template info (for the desktop UI)
// ---------------------------------------------------------------------------

/// Describes a built-in sandbox template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub name: String,
    pub description: String,
    pub category: String,
    pub base_image: String,
    pub vcpus: u32,
    pub memory_mb: u64,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub secret_files: Vec<String>,
    #[serde(default)]
    pub init_script: Option<String>,
    /// Plain-text help shown in UI and attached to created sandboxes.
    #[serde(default)]
    pub help_text: Option<String>,
    /// Secret bindings: maps env var name → target host.
    /// Used to auto-configure proxy secret bindings on sandbox creation.
    #[serde(default)]
    pub secrets: std::collections::BTreeMap<String, String>,
}
