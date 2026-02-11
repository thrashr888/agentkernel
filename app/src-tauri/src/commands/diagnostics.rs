use tauri::State;

use crate::state::AppState;
use crate::types::{DoctorResult, GcResult, StatusInfo};

/// Get system status information from the API.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_status(state: State<'_, AppState>) -> Result<StatusInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_status().await.map_err(|e| e.to_string())
}

/// Run health checks via the API.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_doctor(state: State<'_, AppState>) -> Result<DoctorResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_doctor().await.map_err(|e| e.to_string())
}

/// Run garbage collection to remove expired sandboxes.
#[tauri::command(rename_all = "snake_case")]
pub async fn run_gc(state: State<'_, AppState>) -> Result<GcResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.run_gc().await.map_err(|e| e.to_string())
}
