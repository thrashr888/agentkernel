use tauri::State;

use crate::commands::tunnels::{connect_entry, TunnelManager};
use crate::state::{AppState, Settings};

/// Return the current settings.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.clone())
}

/// Persist new settings and rebuild the API client.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_settings(
    new_settings: Settings,
    state: State<'_, AppState>,
    tunnel_manager: State<'_, TunnelManager>,
) -> Result<(), String> {
    let old_settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let active_entry = new_settings.active().cloned();
    let tunnel_enabled = active_entry.as_ref().is_some_and(|entry| {
        entry
            .ssh_tunnel
            .as_ref()
            .is_some_and(|config| config.enabled)
    });

    // Persist the candidate before changing the active process/client. If a
    // tunnel cannot be established, the old settings can be restored while
    // the previous tunnel and API client remain untouched.
    new_settings.save().map_err(|error| error.to_string())?;

    // A tunnel is considered connected only after its local endpoint passes
    // the AgentKernel health check. The API client is not switched to the
    // loopback URL until that check succeeds.
    if let Some(entry) = active_entry.as_ref().filter(|_| tunnel_enabled) {
        if let Err(error) = connect_entry(&state, &tunnel_manager, entry.clone()).await {
            if let Err(restore_error) = old_settings.save() {
                return Err(format!(
                    "{error}; also failed to restore previous settings: {restore_error}"
                ));
            }
            return Err(error);
        }
    } else {
        tunnel_manager.stop().await?;
    }

    // Rebuild the HTTP client from the active server.
    let new_client = AppState::client_from_settings(&new_settings);

    // Update shared state.
    {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        *settings = new_settings;
    }
    {
        let mut client = state.client.lock().map_err(|e| e.to_string())?;
        // `connect_entry` already installed the health-checked loopback
        // client for tunnel entries. Direct URL entries use the regular
        // settings-derived client.
        if active_entry
            .as_ref()
            .and_then(|entry| entry.ssh_tunnel.as_ref())
            .is_none_or(|config| !config.enabled)
        {
            *client = new_client;
        }
    }

    Ok(())
}
