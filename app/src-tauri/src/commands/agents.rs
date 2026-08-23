use tauri::State;

use crate::state::AppState;
use crate::types::{AgentInfo, AgentIntegrationResult};

#[tauri::command(rename_all = "snake_case")]
pub async fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_agents().await.map_err(|e| e.to_string())
}

/// Preview or confirm an AgentKernel integration install on the configured server.
#[tauri::command(rename_all = "snake_case")]
pub async fn install_agent(
    state: State<'_, AppState>,
    name: String,
    scope: String,
    confirm: bool,
) -> Result<AgentIntegrationResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .install_agent_integration(&name, &scope, confirm)
        .await
        .map_err(|e| e.to_string())
}
