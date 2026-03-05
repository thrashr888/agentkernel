use tauri::State;

use crate::state::AppState;
use crate::types::DockerImage;

/// List cached Docker images.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_images(state: State<'_, AppState>) -> Result<Vec<DockerImage>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_images().await.map_err(|e| e.to_string())
}

/// Remove a Docker image.
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_image(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.remove_image(&id).await.map_err(|e| e.to_string())
}
