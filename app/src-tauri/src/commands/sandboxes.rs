use tauri::State;

use crate::state::AppState;
use crate::types::{AuditLogEntry, CreateSandboxRequest, ExtendTtlResponse, SandboxInfo};

/// List all sandboxes.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_sandboxes(state: State<'_, AppState>) -> Result<Vec<SandboxInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_sandboxes().await.map_err(|e| e.to_string())
}

/// Get details for a single sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_sandbox(name: String, state: State<'_, AppState>) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Create a new sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn create_sandbox(
    req: CreateSandboxRequest,
    state: State<'_, AppState>,
) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.create_sandbox(&req).await.map_err(|e| e.to_string())
}

/// Remove a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.remove_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Start a stopped sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn start_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.start_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Stop a running sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn stop_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.stop_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Get audit logs for a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_sandbox_logs(
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<AuditLogEntry>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .get_sandbox_logs(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Extend a sandbox's time-to-live.
#[tauri::command(rename_all = "snake_case")]
pub async fn extend_ttl(
    name: String,
    by: String,
    state: State<'_, AppState>,
) -> Result<ExtendTtlResponse, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.extend_ttl(&name, &by).await.map_err(|e| e.to_string())
}
