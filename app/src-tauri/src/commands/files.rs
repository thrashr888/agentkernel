use tauri::State;

use crate::state::AppState;
use crate::types::{FileReadResponse, RunOutput};

/// List files in a directory inside a sandbox (via `ls -la`).
#[tauri::command(rename_all = "snake_case")]
pub async fn list_files(
    name: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<RunOutput, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .list_files(&name, &path)
        .await
        .map_err(|e| e.to_string())
}

/// Read a file from a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn read_file(
    name: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<FileReadResponse, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .read_file(&name, &path)
        .await
        .map_err(|e| e.to_string())
}
