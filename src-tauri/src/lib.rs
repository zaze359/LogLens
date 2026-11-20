mod adb;
mod builtin;
mod cdp;
mod commands;
mod config_store;
mod error_log;
mod logcat;
mod model;
mod netcapture;
mod netprobe;
mod network;
mod proxy;
mod rules;
mod session;
mod stream;

use session::AppState;
use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::open_log_file,
            commands::cancel_open_log,
            commands::adb_status,
            commands::list_adb_devices,
            commands::start_device_stream,
            commands::poll_device_stream,
            commands::stop_device_stream,
            commands::release_device_stream,
            commands::query_logs,
            commands::log_timeline,
            commands::search_hits,
            commands::close_session,
            commands::export_filtered,
            commands::list_builtin_templates,
            commands::list_rule_bundle,
            commands::rules_location,
            commands::save_custom_rule,
            commands::delete_custom_rule,
            commands::save_rule_category,
            commands::delete_rule_category,
            commands::export_rules_json,
            commands::export_rules_to_file,
            commands::import_rules_from_file,
            commands::import_rules_json,
            commands::list_capture_packages,
            commands::probe_network_capture,
            commands::start_cdp_capture,
            commands::start_proxy_capture,
            commands::poll_network_capture,
            commands::query_network_requests,
            commands::clear_network_capture,
            commands::stop_network_capture,
            commands::close_network_capture,
            commands::import_network_capture,
            commands::configure_device_proxy,
            commands::restore_device_proxy,
            commands::start_proxy_server,
            commands::stop_proxy_server,
            commands::proxy_server_status,
            commands::poll_proxy_server,
            commands::list_error_log,
            commands::clear_error_log,
            commands::report_client_error,
            commands::error_log_location,
            commands::export_error_log,
            commands::list_search_history,
            commands::remember_search,
            commands::delete_search_history,
            commands::clear_search_history
        ])
        // A device-wide global proxy is persistent while this process is not. Clean
        // up on *every* shutdown path, not just the one Tauri happens to deliver.
        .setup(|app| {
            // Install the panic hook + load previous records before anything else can fail.
            error_log::init(app.handle());
            error_log::record_flow("app.start", "应用启动", "");
            let healed = netcapture::heal_stale_proxies();
            if !healed.is_empty() {
                error_log::flow_echo("proxy.heal", "启动自愈", &healed);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // `Destroyed` alone is not enough: on Cmd+Q / main-window close Tauri 2
            // can go straight to RunEvent::Exit, and a forced quit runs no hook at
            // all. The on-disk proxy journal covers that last case on next launch.
            if matches!(
                event,
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
            ) {
                window.state::<AppState>().stop_all_streams();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<AppState>().stop_all_streams();
            }
        });
}
