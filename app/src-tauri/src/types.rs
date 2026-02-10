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
    pub status: String,
    pub backend: String,
    #[serde(default)]
    pub ip: Option<String>,
    pub image: Option<String>,
    pub vcpus: Option<u32>,
    pub memory_mb: Option<u64>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub ports: Vec<String>,
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
}
