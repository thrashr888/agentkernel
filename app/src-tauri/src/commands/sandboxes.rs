use tauri::State;

use crate::state::AppState;
use crate::types::{AuditLogEntry, CreateSandboxRequest, ExtendTtlResponse, SandboxInfo};

/// List all sandboxes.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_sandboxes(state: State<'_, AppState>) -> Result<Vec<SandboxInfo>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.list_sandboxes().await.map_err(|e| e.to_string())
}

/// Get details for a single sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_sandbox(name: String, state: State<'_, AppState>) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Create a new sandbox.
///
/// Resolves `secret_mappings` (env_var → host) from host environment variables
/// and merges them into the `secrets` vec before forwarding to the API.
#[tauri::command(rename_all = "snake_case")]
pub async fn create_sandbox(
    mut req: CreateSandboxRequest,
    state: State<'_, AppState>,
) -> Result<SandboxInfo, String> {
    // Resolve template secret_mappings from host env vars
    if !req.secret_mappings.is_empty() {
        let existing_keys: std::collections::HashSet<String> = req
            .secrets
            .iter()
            .filter_map(|s| s.split('=').next().map(String::from))
            .collect();
        for (env_var, target_host) in &req.secret_mappings {
            // Skip if already provided as an explicit secret binding
            if existing_keys.contains(env_var.as_str()) {
                continue;
            }
            if let Ok(val) = std::env::var(env_var) {
                req.secrets
                    .push(format!("{}={}:{}", env_var, val, target_host));
            }
        }
        // Clear mappings so they aren't sent to the API (it doesn't know about them)
        req.secret_mappings.clear();
    }

    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.create_sandbox(&req).await.map_err(|e| e.to_string())
}

/// Remove a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .remove_sandbox(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Start a stopped sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn start_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.start_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Stop a running sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn stop_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.stop_sandbox(&name).await.map_err(|e| e.to_string())
}

/// Get audit logs for a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_sandbox_logs(
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<AuditLogEntry>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .get_sandbox_logs(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Open an interactive terminal session to a running sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn open_terminal(name: String, _state: State<'_, AppState>) -> Result<(), String> {
    // Use `container exec` for Apple Containers (interactive shell)
    // Container names are prefixed with "agentkernel-"
    let exec_cmd = format!("container exec -it agentkernel-{} /bin/sh", name);

    // Open Terminal.app with the command on macOS
    std::process::Command::new("osascript")
        .args([
            "-e",
            &format!(
                "tell application \"Terminal\"\n    activate\n    do script \"{}\"\nend tell",
                exec_cmd
            ),
        ])
        .spawn()
        .map_err(|e| format!("Failed to open terminal: {}", e))?;

    Ok(())
}

/// Quickstart: create a sandbox with an agent, install it, and open a terminal running the agent.
#[tauri::command(rename_all = "snake_case")]
pub async fn quickstart_agent(
    agent: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let sandbox_name = name;

    let client = state.client.lock().map_err(|e| e.to_string())?.clone();

    // Map agent → CLI command and secret bindings (env_var, host)
    // The proxy intercepts HTTPS to these hosts and injects the real API key,
    // which also enables LLM usage tracking via the intercept layer.
    let (agent_cmd, secret_defs): (&str, &[(&str, &str)]) = match agent.as_str() {
        "claude" => ("claude", &[("ANTHROPIC_API_KEY", "api.anthropic.com")]),
        "gemini" => ("gemini", &[
            ("GOOGLE_API_KEY", "generativelanguage.googleapis.com"),
            ("GEMINI_API_KEY", "generativelanguage.googleapis.com"),
        ]),
        "codex" => ("codex", &[("OPENAI_API_KEY", "api.openai.com")]),
        "opencode" => ("opencode", &[
            ("ANTHROPIC_API_KEY", "api.anthropic.com"),
            ("OPENAI_API_KEY", "api.openai.com"),
        ]),
        "amp" => ("amp", &[("ANTHROPIC_API_KEY", "api.anthropic.com")]),
        "pi" => ("pi", &[
            ("ANTHROPIC_API_KEY", "api.anthropic.com"),
            ("OPENAI_API_KEY", "api.openai.com"),
        ]),
        "copilot" => ("github-copilot", &[("GITHUB_TOKEN", "api.github.com")]),
        _ => return Err(format!("Unknown agent: {}", agent)),
    };

    // Build secret bindings from host env vars (format: KEY=value:host)
    let mut secrets = Vec::new();
    for &(var, host) in secret_defs {
        if let Ok(val) = std::env::var(var) {
            secrets.push(format!("{}={}:{}", var, val, host));
        }
    }

    // Create sandbox with agent field + secret bindings for LLM proxy
    let req = CreateSandboxRequest {
        name: sandbox_name.clone(),
        image: Some("node:22-alpine".to_string()),
        vcpus: Some(2),
        memory_mb: Some(1024),
        profile: Some(crate::types::SecurityProfile::Moderate),
        source_url: None,
        source_ref: None,
        volumes: None,
        agent: Some(agent.clone()),
        secrets,
        init_script: None,
        secret_mappings: std::collections::BTreeMap::new(),
    };

    client
        .create_sandbox(&req)
        .await
        .map_err(|e| format!("Failed to create sandbox: {}", e))?;

    let container_name = format!("agentkernel-{}", sandbox_name);
    let exec_cmd = format!(
        "container exec -it {} {}",
        container_name, agent_cmd
    );

    // Open Terminal.app with the agent command
    std::process::Command::new("osascript")
        .args([
            "-e",
            &format!(
                "tell application \"Terminal\"\n    activate\n    do script \"{}\"\nend tell",
                exec_cmd
            ),
        ])
        .spawn()
        .map_err(|e| format!("Failed to open terminal: {}", e))?;

    Ok(sandbox_name)
}

/// Export a sandbox filesystem as a tar.gz archive.
#[tauri::command(rename_all = "snake_case")]
pub async fn export_sandbox(name: String, state: State<'_, AppState>) -> Result<String, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();

    // Create tar.gz inside the container
    let archive_path = "/tmp/sandbox-export.tar.gz";
    let cmd = vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "tar czf {} --exclude=/proc --exclude=/sys --exclude=/dev --exclude={} -C / .",
            archive_path, archive_path
        ),
    ];
    client
        .exec_in_sandbox(&name, cmd, vec![], None)
        .await
        .map_err(|e| format!("Failed to create archive: {}", e))?;

    // Read the archive out via the files API
    let data = client
        .read_file(&name, archive_path)
        .await
        .map_err(|e| format!("Failed to read archive: {}", e))?;

    // Save to Downloads directory
    let downloads = dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let dest = downloads.join(format!("{}.tar.gz", name));

    // Decode base64 content if needed
    if data.encoding == "base64" {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = STANDARD
            .decode(&data.content)
            .map_err(|e| format!("Failed to decode: {}", e))?;
        std::fs::write(&dest, bytes).map_err(|e| format!("Failed to write: {}", e))?;
    } else {
        std::fs::write(&dest, data.content.as_bytes())
            .map_err(|e| format!("Failed to write: {}", e))?;
    }

    // Clean up inside container
    let _ = client
        .exec_in_sandbox(
            &name,
            vec!["rm".to_string(), "-f".to_string(), archive_path.to_string()],
            vec![],
            None,
        )
        .await;

    Ok(dest.to_string_lossy().to_string())
}

/// Extend a sandbox's time-to-live.
#[tauri::command(rename_all = "snake_case")]
pub async fn extend_ttl(
    name: String,
    by: String,
    state: State<'_, AppState>,
) -> Result<ExtendTtlResponse, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .extend_ttl(&name, &by)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn resize_sandbox(
    name: String,
    vcpus: Option<u32>,
    memory_mb: Option<u64>,
    state: State<'_, AppState>,
) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .resize_sandbox(&name, vcpus, memory_mb)
        .await
        .map_err(|e| e.to_string())
}
