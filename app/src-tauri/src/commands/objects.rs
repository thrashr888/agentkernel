use tauri::State;

use crate::state::AppState;
use crate::types::{CreateObjectRequest, DurableObjectInfo};

#[tauri::command(rename_all = "snake_case")]
pub async fn list_objects(state: State<'_, AppState>) -> Result<Vec<DurableObjectInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_objects().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_object(
    id: String,
    state: State<'_, AppState>,
) -> Result<DurableObjectInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_object(&id).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn create_object(
    req: CreateObjectRequest,
    state: State<'_, AppState>,
) -> Result<DurableObjectInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.create_object(&req).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_object(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.delete_object(&id).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn patch_object(
    id: String,
    storage: Option<serde_json::Value>,
    status: Option<String>,
    state: State<'_, AppState>,
) -> Result<DurableObjectInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .patch_object(&id, storage, status)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn call_object(
    class: String,
    object_id: String,
    method: String,
    args: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .call_object(&class, &object_id, &method, args)
        .await
        .map_err(|e| e.to_string())
}
