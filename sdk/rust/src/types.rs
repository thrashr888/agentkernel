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
}

/// Exec request body (internal).
#[derive(Serialize)]
pub(crate) struct ExecRequest {
    pub command: Vec<String>,
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
