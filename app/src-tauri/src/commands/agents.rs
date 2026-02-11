use tauri::State;

use crate::state::AppState;
use crate::types::AgentInfo;

#[tauri::command(rename_all = "snake_case")]
pub async fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_agents().await.map_err(|e| e.to_string())
}

/// Install an agent CLI tool on the host machine.
#[tauri::command(rename_all = "snake_case")]
pub async fn install_agent(name: String) -> Result<String, String> {
    let (cmd, args): (&str, Vec<&str>) = match name.as_str() {
        "claude" => ("npm", vec!["install", "-g", "@anthropic-ai/claude-code"]),
        "gemini" => ("npm", vec!["install", "-g", "@anthropic-ai/gemini-cli"]),
        "codex" => ("npm", vec!["install", "-g", "@openai/codex"]),
        "opencode" => ("cargo", vec!["install", "opencode"]),
        "amp" => ("npm", vec!["install", "-g", "@sourcegraph/amp"]),
        "pi" => (
            "npm",
            vec!["install", "-g", "@mariozechner/pi-coding-agent"],
        ),
        "copilot" => (
            "npm",
            vec!["install", "-g", "@githubnext/github-copilot-cli"],
        ),
        _ => return Err(format!("Unknown agent: {}", name)),
    };

    let output = std::process::Command::new(cmd)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run {}: {}", cmd, e))?;

    if output.status.success() {
        Ok(format!("Installed {} successfully", name))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Install failed: {}", stderr.trim()))
    }
}
