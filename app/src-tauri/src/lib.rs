mod api_client;
mod commands;
mod state;
mod types;

use state::AppState;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, SubmenuBuilder},
    tray::{TrayIconBuilder, TrayIconId},
    Manager, WindowEvent,
};
use types::SandboxInfo;

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

/// Compute a fingerprint of the sandbox list so we can skip menu rebuilds
/// when nothing has changed (rebuilding replaces the menu and closes it).
fn sandbox_fingerprint(sandboxes: &[SandboxInfo]) -> String {
    sandboxes
        .iter()
        .take(5)
        .map(|s| {
            format!(
                "{}:{}:{}:{}:{}",
                s.name,
                s.status,
                s.ip.as_deref().unwrap_or(""),
                s.vcpus.unwrap_or(0),
                s.memory_mb.unwrap_or(0),
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// Rebuild the tray menu with an up-to-date "Recent Sandboxes" submenu.
#[allow(clippy::too_many_arguments)]
fn rebuild_tray_menu(
    handle: &tauri::AppHandle,
    tray_id: &TrayIconId,
    sandboxes: &[SandboxInfo],
    status_item: &MenuItem<tauri::Wry>,
    sandbox_count: &MenuItem<tauri::Wry>,
    quick_create: &MenuItem<tauri::Wry>,
    dashboard: &MenuItem<tauri::Wry>,
    sandboxes_nav: &MenuItem<tauri::Wry>,
    secrets: &MenuItem<tauri::Wry>,
    settings: &MenuItem<tauri::Wry>,
    quit: &MenuItem<tauri::Wry>,
) -> anyhow::Result<()> {
    // Build the Recent Sandboxes submenu (up to 5 entries, with stats)
    let mut builder = SubmenuBuilder::new(handle, "Recent Sandboxes");
    let recent: Vec<_> = sandboxes.iter().take(5).collect();
    if recent.is_empty() {
        builder = builder.text("no_sandboxes", "No sandboxes");
    } else {
        for sb in &recent {
            // Per-sandbox submenu with stats and actions
            let icon = if sb.status == "running" {
                "\u{1F7E2}"
            } else {
                "\u{26AA}"
            };

            let mut sb_sub = SubmenuBuilder::new(handle, format!("{icon} {}", sb.name));

            // Stats line (disabled — informational)
            let mut stats_parts = Vec::new();
            if let Some(ip) = &sb.ip {
                stats_parts.push(ip.clone());
            }
            if let (Some(vcpus), Some(mem)) = (sb.vcpus, sb.memory_mb) {
                stats_parts.push(format!("{vcpus}vCPU / {mem}MB"));
            }
            if !stats_parts.is_empty() {
                let stats_text = stats_parts.join(" \u{2014} ");
                sb_sub = sb_sub.item(&MenuItem::with_id(
                    handle,
                    format!("stats:{}", sb.name),
                    &stats_text,
                    false,
                    None::<&str>,
                )?);
            }

            // Backend + image info
            let image_text = sb.image.as_deref().unwrap_or("unknown");
            sb_sub = sb_sub.item(&MenuItem::with_id(
                handle,
                format!("info:{}", sb.name),
                format!("{} \u{2014} {image_text}", sb.backend),
                false,
                None::<&str>,
            )?);

            sb_sub = sb_sub.separator();

            // Actions
            sb_sub = sb_sub
                .text(format!("sandbox:{}", sb.name), "Open in Dashboard")
                .text(format!("terminal:{}", sb.name), "Open Terminal")
                .text(format!("logs:{}", sb.name), "View Logs\u{2026}");

            builder = builder.item(&sb_sub.build()?);
        }
    }
    let recent_sub = builder.build()?;

    // Resource summary line (disabled — informational)
    let running: Vec<_> = sandboxes.iter().filter(|s| s.status == "running").collect();
    let total_vcpus: u32 = running.iter().filter_map(|s| s.vcpus).sum();
    let total_mem: u64 = running.iter().filter_map(|s| s.memory_mb).sum();
    let resource_text = if running.is_empty() {
        None
    } else {
        Some(format!(
            "\u{2699}\u{FE0F} {total_vcpus} vCPUs, {} MB allocated",
            total_mem
        ))
    };
    let resource_item = resource_text
        .as_ref()
        .map(|text| MenuItem::with_id(handle, "resources", text.as_str(), false, None::<&str>))
        .transpose()?;

    let sep1 = PredefinedMenuItem::separator(handle)?;
    let sep2 = PredefinedMenuItem::separator(handle)?;
    let sep3 = PredefinedMenuItem::separator(handle)?;

    // Build menu — conditionally include resource summary
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![status_item, sandbox_count];
    if let Some(ref ri) = resource_item {
        items.push(ri);
    }
    items.extend_from_slice(&[
        &sep1,
        quick_create,
        &recent_sub,
        &sep2,
        dashboard,
        sandboxes_nav,
        secrets,
        settings,
        &sep3,
        quit,
    ]);

    let menu = Menu::with_items(handle, &items)?;

    if let Some(tray) = handle.tray_by_id(tray_id) {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

/// Entry point for the Tauri application.
///
/// Registers all IPC commands and injects shared `AppState`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(feature = "debug-bridge")]
    let builder = builder.plugin(tauri_plugin_debug_bridge::init());

    builder
        .manage(AppState::default())
        .setup(|app| {
            // -- Status items (disabled — informational only) --
            let status_item = MenuItem::with_id(
                app,
                "status",
                "\u{1F534} Connecting\u{2026}",
                false,
                None::<&str>,
            )?;
            let sandbox_count =
                MenuItem::with_id(app, "sandbox_count", "\u{2014}", false, None::<&str>)?;

            // -- Quick actions --
            let quick_create =
                MenuItem::with_id(app, "quick_create", "New Sandbox\u{2026}", true, None::<&str>)?;
            let recent_sandboxes = SubmenuBuilder::new(app, "Recent Sandboxes")
                .text("no_sandboxes", "No sandboxes")
                .build()?;

            // -- Navigation items --
            let dashboard =
                MenuItem::with_id(app, "dashboard", "Open Dashboard", true, None::<&str>)?;
            let sandboxes = MenuItem::with_id(app, "sandboxes", "Sandboxes", true, None::<&str>)?;
            let secrets = MenuItem::with_id(app, "secrets", "Secrets", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;

            let sep1 = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let sep3 = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit AgentKernel", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &status_item,
                    &sandbox_count,
                    &sep1,
                    &quick_create,
                    &recent_sandboxes,
                    &sep2,
                    &dashboard,
                    &sandboxes,
                    &secrets,
                    &settings,
                    &sep3,
                    &quit,
                ],
            )?;

            // Load tray icon — 44x44 template icon for macOS menu bar
            let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

            let tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .icon_as_template(true)
                .tooltip("AgentKernel")
                .icon(icon)
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    match id {
                        "quit" => app.exit(0),
                        "quick_create" => navigate_to(app, "/sandboxes?action=create"),
                        "dashboard" => navigate_to(app, "/"),
                        "sandboxes" => navigate_to(app, "/sandboxes"),
                        "secrets" => navigate_to(app, "/secrets"),
                        "settings" => navigate_to(app, "/settings"),
                        _ if id.starts_with("sandbox:") => {
                            let name = &id["sandbox:".len()..];
                            navigate_to(app, &format!("/sandboxes/{name}"));
                        }
                        _ if id.starts_with("terminal:") => {
                            let name = &id["terminal:".len()..];
                            let exec_cmd = format!(
                                "container exec -it agentkernel-{name} /bin/sh"
                            );
                            let _ = std::process::Command::new("osascript")
                                .args([
                                    "-e",
                                    &format!(
                                        "tell application \"Terminal\"\n    activate\n    do script \"{exec_cmd}\"\nend tell"
                                    ),
                                ])
                                .spawn();
                        }
                        _ if id.starts_with("logs:") => {
                            let name = &id["logs:".len()..];
                            navigate_to(app, &format!("/sandboxes/{name}?tab=logs"));
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // -- Background poller: update tray status every 5 seconds --
            let status_clone = status_item.clone();
            let count_clone = sandbox_count.clone();
            let quick_create_clone = quick_create.clone();
            let dashboard_clone = dashboard.clone();
            let sandboxes_clone = sandboxes.clone();
            let secrets_clone = secrets.clone();
            let settings_clone = settings.clone();
            let quit_clone = quit.clone();
            let tray_id = tray.id().clone();
            let handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                // Track last sandbox state to avoid rebuilding (and closing) the
                // tray menu when nothing has changed.
                let mut last_fingerprint = String::new();

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

                    // Update sandbox count + recent sandboxes submenu
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

                                // Sort: running first, then alphabetically by name
                                let mut sorted = list;
                                sorted.sort_by(|a, b| {
                                    let a_running = a.status == "running";
                                    let b_running = b.status == "running";
                                    b_running.cmp(&a_running).then(a.name.cmp(&b.name))
                                });

                                // Only rebuild the menu when sandbox data actually changes
                                let fingerprint = sandbox_fingerprint(&sorted);
                                if fingerprint != last_fingerprint {
                                    last_fingerprint = fingerprint;
                                    if let Err(e) = rebuild_tray_menu(
                                        &handle,
                                        &tray_id,
                                        &sorted,
                                        &status_clone,
                                        &count_clone,
                                        &quick_create_clone,
                                        &dashboard_clone,
                                        &sandboxes_clone,
                                        &secrets_clone,
                                        &settings_clone,
                                        &quit_clone,
                                    ) {
                                        eprintln!("Failed to rebuild tray menu: {e}");
                                    }
                                }
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
            commands::sandboxes::update_sandbox,
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
            // llm usage
            commands::llm_usage::get_llm_usage,
            commands::llm_usage::get_llm_usage_by_sandbox,
            // policy
            commands::policy::get_policy_status,
            commands::policy::check_policy,
            commands::policy::reload_policy,
            commands::policy::get_policy_audit,
            // settings
            commands::settings::get_settings,
            commands::settings::save_settings,
            // durable objects
            commands::objects::list_objects,
            commands::objects::get_object,
            commands::objects::create_object,
            commands::objects::delete_object,
            commands::objects::patch_object,
            commands::objects::call_object,
            // schedules
            commands::schedules::list_schedules,
            commands::schedules::get_schedule,
            commands::schedules::create_schedule,
            commands::schedules::delete_schedule,
            commands::schedules::trigger_schedule,
            // durable stores
            commands::stores::list_stores,
            commands::stores::get_store,
            commands::stores::create_store,
            commands::stores::delete_store,
            commands::stores::query_store,
            commands::stores::execute_store,
            commands::stores::command_store,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
