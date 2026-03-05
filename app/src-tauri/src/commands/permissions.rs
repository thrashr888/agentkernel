use tauri::State;

use crate::state::AppState;
use crate::types::{GrantPermissionRequest, PermissionCheckResult, PermissionGrant};

#[tauri::command(rename_all = "snake_case")]
pub async fn list_permissions(
    state: State<'_, AppState>,
) -> Result<Vec<PermissionGrant>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_permissions().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn grant_permission(
    req: GrantPermissionRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .grant_permission(&req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn revoke_permission(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .revoke_permission(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn check_permission(
    kind: String,
    sandbox: Option<String>,
    state: State<'_, AppState>,
) -> Result<PermissionCheckResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .check_permission(&kind, sandbox.as_deref())
        .await
        .map_err(|e| e.to_string())
}
