use tauri::State;

use crate::state::AppState;
use crate::types::{
    CreateStoreRequest, DurableStoreInfo, StoreCommandResult, StoreExecuteResult, StoreQueryResult,
};

#[tauri::command(rename_all = "snake_case")]
pub async fn list_stores(state: State<'_, AppState>) -> Result<Vec<DurableStoreInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_stores().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_store(id: String, state: State<'_, AppState>) -> Result<DurableStoreInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_store(&id).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn create_store(
    req: CreateStoreRequest,
    state: State<'_, AppState>,
) -> Result<DurableStoreInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.create_store(&req).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_store(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.delete_store(&id).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn query_store(
    id: String,
    sql: String,
    params: Vec<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<StoreQueryResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .query_store(&id, &sql, params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn execute_store(
    id: String,
    sql: String,
    params: Vec<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<StoreExecuteResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .execute_store(&id, &sql, params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn command_store(
    id: String,
    command: Vec<String>,
    state: State<'_, AppState>,
) -> Result<StoreCommandResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .command_store(&id, command)
        .await
        .map_err(|e| e.to_string())
}
