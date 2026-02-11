use tauri::State;

use crate::state::AppState;

/// Check connectivity to the agentkernel API.
#[tauri::command(rename_all = "snake_case")]
pub async fn check_connection(state: State<'_, AppState>) -> Result<String, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.health().await.map_err(|e| e.to_string())
}
