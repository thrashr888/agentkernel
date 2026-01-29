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
}

/// Exec request body (internal).
#[derive(Serialize)]
pub(crate) struct ExecRequest {
    pub command: Vec<String>,
}
