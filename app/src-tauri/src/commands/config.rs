use tauri::State;

use crate::state::AppState;
use crate::types::SandboxInfo;

/// Export sandbox configuration as TOML.
#[tauri::command(rename_all = "snake_case")]
pub async fn export_sandbox_config(
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .export_sandbox_config(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Import sandbox configuration from TOML.
#[tauri::command(rename_all = "snake_case")]
pub async fn import_sandbox_config(
    name: Option<String>,
    config: String,
    state: State<'_, AppState>,
) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .import_sandbox_config(name.as_deref(), &config)
        .await
        .map_err(|e| e.to_string())
}
