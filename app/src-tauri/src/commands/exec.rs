use tauri::State;

use crate::state::AppState;
use crate::types::{DetachedCommand, DetachedLogsResponse, RunOutput};

/// Execute a command inside an existing sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn exec_command(
    name: String,
    command: Vec<String>,
    env: Vec<String>,
    workdir: Option<String>,
    state: State<'_, AppState>,
) -> Result<RunOutput, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .exec_in_sandbox(&name, command, env, workdir)
        .await
        .map_err(|e| e.to_string())
}

/// Start a detached (background) command in a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn exec_detached(
    name: String,
    command: Vec<String>,
    state: State<'_, AppState>,
) -> Result<DetachedCommand, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .exec_detached(&name, command)
        .await
        .map_err(|e| e.to_string())
}

/// List detached commands in a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_detached(
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<DetachedCommand>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_detached(&name).await.map_err(|e| e.to_string())
}

/// Get logs from a detached command.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_detached_logs(
    name: String,
    cmd_id: String,
    state: State<'_, AppState>,
) -> Result<DetachedLogsResponse, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .detached_logs(&name, &cmd_id)
        .await
        .map_err(|e| e.to_string())
}

/// Kill a detached command.
#[tauri::command(rename_all = "snake_case")]
pub async fn kill_detached(
    name: String,
    cmd_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .kill_detached(&name, &cmd_id)
        .await
        .map_err(|e| e.to_string())
}
