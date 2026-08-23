use tauri::State;

use crate::state::AppState;
use crate::types::QuotaStatus;

/// Fetch tenant-scoped resource quota usage for the dashboard.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_quotas(state: State<'_, AppState>) -> Result<QuotaStatus, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_quotas().await.map_err(|e| e.to_string())
}
