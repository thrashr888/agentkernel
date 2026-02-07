mod api_client;
mod commands;
mod state;
mod types;

use state::AppState;

/// Entry point for the Tauri application.
///
/// Registers all IPC commands and injects shared `AppState`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            // health
            commands::health::check_connection,
            // sandboxes
            commands::sandboxes::list_sandboxes,
            commands::sandboxes::get_sandbox,
            commands::sandboxes::create_sandbox,
            commands::sandboxes::remove_sandbox,
            commands::sandboxes::extend_ttl,
            // exec
            commands::exec::exec_command,
            commands::exec::exec_detached,
            commands::exec::list_detached,
            commands::exec::get_detached_logs,
            commands::exec::kill_detached,
            // snapshots
            commands::snapshots::list_snapshots,
            commands::snapshots::take_snapshot,
            commands::snapshots::delete_snapshot,
            commands::snapshots::restore_snapshot,
            // templates
            commands::templates::list_templates,
            // settings
            commands::settings::get_settings,
            commands::settings::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
