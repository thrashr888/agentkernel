use tauri::State;

use crate::state::AppState;
use crate::types::BenchmarkResult;

/// Run a hardware benchmark (timed create/exec/destroy cycle).
#[tauri::command(rename_all = "snake_case")]
pub async fn run_benchmark(state: State<'_, AppState>) -> Result<BenchmarkResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.run_benchmark().await.map_err(|e| e.to_string())
}
