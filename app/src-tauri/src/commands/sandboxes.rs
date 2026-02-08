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
#[tauri::command(rename_all = "snake_case")]
pub async fn create_sandbox(
    req: CreateSandboxRequest,
    state: State<'_, AppState>,
) -> Result<SandboxInfo, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.create_sandbox(&req).await.map_err(|e| e.to_string())
}

/// Remove a sandbox.
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_sandbox(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.remove_sandbox(&name).await.map_err(|e| e.to_string())
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
    let exec_cmd = format!("container exec -it {} /bin/sh", name);

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
    let downloads =
        dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
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
    client.extend_ttl(&name, &by).await.map_err(|e| e.to_string())
}
