use tauri::State;

use crate::state::AppState;
use crate::types::{CreateScheduleRequest, ScheduleInfo};

#[tauri::command(rename_all = "snake_case")]
pub async fn list_schedules(state: State<'_, AppState>) -> Result<Vec<ScheduleInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_schedules().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_schedule(id: String, state: State<'_, AppState>) -> Result<ScheduleInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_schedule(&id).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn create_schedule(
    req: CreateScheduleRequest,
    state: State<'_, AppState>,
) -> Result<ScheduleInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .create_schedule(&req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_schedule(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.delete_schedule(&id).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn trigger_schedule(
    id: String,
    state: State<'_, AppState>,
) -> Result<ScheduleInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .trigger_schedule(&id)
        .await
        .map_err(|e| e.to_string())
}
