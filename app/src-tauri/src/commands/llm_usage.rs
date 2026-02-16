use std::collections::HashMap;
use tauri::State;

use crate::state::AppState;
use crate::types::LlmUsageEntry;

/// Get LLM usage data for all sandboxes.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_llm_usage(
    state: State<'_, AppState>,
) -> Result<HashMap<String, Vec<LlmUsageEntry>>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_llm_usage().await.map_err(|e| e.to_string())
}

/// Get LLM usage data for a specific sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_llm_usage_by_sandbox(
    sandbox: String,
    state: State<'_, AppState>,
) -> Result<Vec<LlmUsageEntry>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .get_llm_usage_by_sandbox(&sandbox)
        .await
        .map_err(|e| e.to_string())
}
