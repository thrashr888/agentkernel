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
        "claude" => (
            "npm",
            vec!["install", "-g", "@anthropic-ai/claude-code@2.1.239"],
        ),
        "gemini" => ("npm", vec!["install", "-g", "@google/gemini-cli@0.56.0"]),
        "codex" => ("npm", vec!["install", "-g", "@openai/codex@0.149.0"]),
        "opencode" => ("npm", vec!["install", "-g", "opencode-ai@1.18.21"]),
        "amp" => (
            "npm",
            vec![
                "install",
                "-g",
                "--allow-scripts=@ampcode/cli",
                "@ampcode/cli@0.0.1787342526-gc11bfb",
            ],
        ),
        "pi" => (
            "npm",
            vec!["install", "-g", "@earendil-works/pi-coding-agent@0.84.2"],
        ),
        "copilot" => ("npm", vec!["install", "-g", "@github/copilot@1.0.80"]),
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
