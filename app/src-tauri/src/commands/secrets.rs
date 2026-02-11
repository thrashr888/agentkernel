use tauri::State;

use crate::state::AppState;
use crate::types::SecretEntry;

#[tauri::command(rename_all = "snake_case")]
pub async fn list_secrets(state: State<'_, AppState>) -> Result<Vec<SecretEntry>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_secrets().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn create_secret(
    name: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .create_secret(&name, &value)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_secret(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.delete_secret(&name).await.map_err(|e| e.to_string())
}
