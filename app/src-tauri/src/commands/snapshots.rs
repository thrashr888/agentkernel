use tauri::State;

use crate::state::AppState;
use crate::types::{SandboxInfo, SnapshotMeta};

/// List all snapshots.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_snapshots(state: State<'_, AppState>) -> Result<Vec<SnapshotMeta>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_snapshots().await.map_err(|e| e.to_string())
}

/// Take a snapshot of a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn take_snapshot(
    sandbox: String,
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<SnapshotMeta, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .take_snapshot(&sandbox, name.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Delete a snapshot.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_snapshot(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.delete_snapshot(&name).await.map_err(|e| e.to_string())
}

/// Restore a sandbox from a snapshot.
#[tauri::command(rename_all = "snake_case")]
pub async fn restore_snapshot(
    name: String,
    as_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.restore_snapshot(&name, as_name.as_deref()).await.map_err(|e| e.to_string())
}
