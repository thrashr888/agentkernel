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

/// Show container-runtime disk usage by resource type.
#[tauri::command(rename_all = "snake_case")]
pub async fn image_disk_usage(
    state: State<'_, AppState>,
) -> Result<Vec<crate::types::DockerImageDiskUsage>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.image_disk_usage().await.map_err(|e| e.to_string())
}

/// Pull a Docker image into the local cache.
#[tauri::command(rename_all = "snake_case")]
pub async fn pull_image(image: String, state: State<'_, AppState>) -> Result<String, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.pull_image(&image).await.map_err(|e| e.to_string())
}

/// Remove unused images from the local cache.
#[tauri::command(rename_all = "snake_case")]
pub async fn prune_images(
    agentkernel_only: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .prune_images(agentkernel_only)
        .await
        .map_err(|e| e.to_string())
}
