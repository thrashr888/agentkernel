use tauri::State;

use crate::state::AppState;
use crate::types::{SessionRecording, SessionSummary};

/// List all sandbox sessions (recorded exec history).
#[tauri::command(rename_all = "snake_case")]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_sessions().await.map_err(|e| e.to_string())
}

/// Get session recording metadata and parsed events.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_session(
    id: String,
    state: State<'_, AppState>,
) -> Result<SessionRecording, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_session(&id).await.map_err(|e| e.to_string())
}

/// Get the original asciicast v2 artifact for a session.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_session_cast(id: String, state: State<'_, AppState>) -> Result<String, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .get_session_cast(&id)
        .await
        .map_err(|e| e.to_string())
}
