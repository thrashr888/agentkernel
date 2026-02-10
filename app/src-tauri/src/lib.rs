mod api_client;
mod commands;
mod state;
mod types;

use state::AppState;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
};

/// Navigate the main window to a given path using BrowserRouter pushState.
fn navigate_to(app: &tauri::AppHandle, path: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let js = format!(
            "window.history.pushState({{}}, '', '{path}'); \
             window.dispatchEvent(new PopStateEvent('popstate'));"
        );
        let _ = window.eval(&js);
    }
}

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
        .setup(|app| {
            // -- Status items (disabled — informational only) --
            let status_item =
                MenuItem::with_id(app, "status", "\u{1F534} Connecting\u{2026}", false, None::<&str>)?;
            let sandbox_count =
                MenuItem::with_id(app, "sandbox_count", "\u{2014}", false, None::<&str>)?;

            // -- Navigation items --
            let dashboard =
                MenuItem::with_id(app, "dashboard", "Open Dashboard", true, None::<&str>)?;
            let sandboxes =
                MenuItem::with_id(app, "sandboxes", "Sandboxes", true, None::<&str>)?;
            let secrets = MenuItem::with_id(app, "secrets", "Secrets", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;

            let sep1 = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let quit =
                MenuItem::with_id(app, "quit", "Quit AgentKernel", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &status_item,
                    &sandbox_count,
                    &sep1,
                    &dashboard,
                    &sandboxes,
                    &secrets,
                    &settings,
                    &sep2,
                    &quit,
                ],
            )?;

            // Load tray icon from bundled PNG
            let icon = Image::from_bytes(include_bytes!("../icons/icon.png"))?;

            let _tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .tooltip("AgentKernel")
                .icon(icon)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "dashboard" => navigate_to(app, "/"),
                    "sandboxes" => navigate_to(app, "/sandboxes"),
                    "secrets" => navigate_to(app, "/secrets"),
                    "settings" => navigate_to(app, "/settings"),
                    _ => {}
                })
                .build(app)?;

            // -- Background poller: update tray status every 5 seconds --
            let status_clone = status_item.clone();
            let count_clone = sandbox_count.clone();
            let handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                loop {
                    let client = handle
                        .state::<AppState>()
                        .client
                        .lock()
                        .ok()
                        .map(|c| c.clone());

                    let Some(client) = client else {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    };

                    let connected = client.health().await.is_ok();

                    // Update connection status
                    let _ = status_clone.set_text(if connected {
                        "\u{1F7E2} Connected"
                    } else {
                        "\u{1F534} Disconnected"
                    });

                    // Update sandbox count
                    if connected {
                        match client.list_sandboxes().await {
                            Ok(list) => {
                                let running =
                                    list.iter().filter(|s| s.status == "running").count();
                                let total = list.len();
                                let text = match (total, running) {
                                    (0, _) => "No Sandboxes".to_string(),
                                    (_, 0) => format!("{total} Stopped"),
                                    _ => format!("{running} Running, {total} Total"),
                                };
                                let _ = count_clone.set_text(&text);
                            }
                            Err(_) => {
                                let _ = count_clone.set_text("\u{2014}");
                            }
                        }
                    } else {
                        let _ = count_clone.set_text("\u{2014}");
                    }

                    // Update tray tooltip
                    if let Some(tray) = handle.tray_by_id("main") {
                        let tooltip = if connected {
                            "AgentKernel \u{2014} Connected"
                        } else {
                            "AgentKernel \u{2014} Disconnected"
                        };
                        let _ = tray.set_tooltip(Some(tooltip));
                    }

                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Hide the main window on close instead of quitting, so the tray stays alive.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            // health
            commands::health::check_connection,
            // audit
            commands::audit::get_audit_log,
            // diagnostics
            commands::diagnostics::get_status,
            commands::diagnostics::get_doctor,
            commands::diagnostics::run_gc,
            // sandboxes
            commands::sandboxes::list_sandboxes,
            commands::sandboxes::get_sandbox,
            commands::sandboxes::create_sandbox,
            commands::sandboxes::remove_sandbox,
            commands::sandboxes::start_sandbox,
            commands::sandboxes::stop_sandbox,
            commands::sandboxes::extend_ttl,
            commands::sandboxes::get_sandbox_logs,
            commands::sandboxes::open_terminal,
            commands::sandboxes::quickstart_agent,
            commands::sandboxes::export_sandbox,
            commands::sandboxes::resize_sandbox,
            // files
            commands::files::list_files,
            commands::files::read_file,
            // exec
            commands::exec::exec_command,
            commands::exec::exec_detached,
            commands::exec::list_detached,
            commands::exec::get_detached_logs,
            commands::exec::kill_detached,
            commands::exec::quick_run,
            // snapshots
            commands::snapshots::list_snapshots,
            commands::snapshots::take_snapshot,
            commands::snapshots::delete_snapshot,
            commands::snapshots::restore_snapshot,
            // templates
            commands::templates::list_templates,
            // secrets
            commands::secrets::list_secrets,
            commands::secrets::create_secret,
            commands::secrets::delete_secret,
            // agents/plugins
            commands::agents::list_agents,
            commands::agents::install_agent,
            // policy
            commands::policy::get_policy_status,
            commands::policy::check_policy,
            // settings
            commands::settings::get_settings,
            commands::settings::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
