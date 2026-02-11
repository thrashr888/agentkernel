use tauri::State;

use crate::state::AppState;
use crate::types::AuditLogEntry;

/// Get the global audit log, optionally limited to the last N entries.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_audit_log(
    last: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<AuditLogEntry>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_audit_log(last).await.map_err(|e| e.to_string())
}
