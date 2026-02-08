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
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_window_state::Builder::new().build());

    #[cfg(feature = "debug-bridge")]
    let builder = builder.plugin(tauri_plugin_debug_bridge::init());

    builder
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            // health
            commands::health::check_connection,
            // sandboxes
            commands::sandboxes::list_sandboxes,
            commands::sandboxes::get_sandbox,
            commands::sandboxes::create_sandbox,
            commands::sandboxes::remove_sandbox,
            commands::sandboxes::start_sandbox,
            commands::sandboxes::stop_sandbox,
            commands::sandboxes::extend_ttl,
            commands::sandboxes::get_sandbox_logs,
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
