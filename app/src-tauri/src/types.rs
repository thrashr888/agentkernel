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
