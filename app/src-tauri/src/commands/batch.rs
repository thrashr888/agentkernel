use tauri::State;

use crate::state::AppState;
use crate::types::{BatchCommand, BatchRunResponse};

/// Run commands concurrently using the server's batch endpoint.
#[tauri::command(rename_all = "snake_case")]
pub async fn batch_run(
    commands: Vec<BatchCommand>,
    state: State<'_, AppState>,
) -> Result<BatchRunResponse, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.batch_run(commands).await.map_err(|e| e.to_string())
}
