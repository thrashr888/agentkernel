use tauri::State;

use crate::state::AppState;
use crate::types::SandboxSession;

/// List all sandbox sessions (recorded exec history).
#[tauri::command(rename_all = "snake_case")]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SandboxSession>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_sessions().await.map_err(|e| e.to_string())
}

/// Get session recording for a specific sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_sandbox_session(
    name: String,
    state: State<'_, AppState>,
) -> Result<SandboxSession, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .get_sandbox_session(&name)
        .await
        .map_err(|e| e.to_string())
}
