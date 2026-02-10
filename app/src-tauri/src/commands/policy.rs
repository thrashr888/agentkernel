use tauri::State;

use crate::state::AppState;
use crate::types::{PolicyCheckResult, PolicyStatus};

#[tauri::command(rename_all = "snake_case")]
pub async fn get_policy_status(state: State<'_, AppState>) -> Result<PolicyStatus, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_policy_status().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn check_policy(
    state: State<'_, AppState>,
    action: String,
    sandbox: String,
) -> Result<PolicyCheckResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .check_policy(&action, &sandbox)
        .await
        .map_err(|e| e.to_string())
}
