use tauri::State;

use crate::api_client::ApiClient;
use crate::state::{AppState, Settings};

/// Return the current settings.
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.clone())
}

/// Persist new settings and rebuild the API client.
#[tauri::command]
pub async fn save_settings(
    new_settings: Settings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Persist to disk first so we fail fast on IO errors.
    new_settings.save().map_err(|e| e.to_string())?;

    // Rebuild the HTTP client with potentially new URL / key.
    let new_client = ApiClient::new(&new_settings.api_url, new_settings.api_key.as_deref());

    // Update shared state.
    {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        *settings = new_settings;
    }
    {
        let mut client = state.client.lock().map_err(|e| e.to_string())?;
        *client = new_client;
    }

    Ok(())
}
