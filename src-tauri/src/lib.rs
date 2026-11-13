mod adb;
mod builtin;
mod cdp;
mod commands;
mod config_store;
mod logcat;
mod model;
mod netcapture;
mod netprobe;
mod network;
mod rules;
mod session;
mod stream;

use session::AppState;
use tauri::Manager;

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
            commands::query_logs,
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
            commands::poll_network_capture,
            commands::query_network_requests,
            commands::stop_network_capture,
            commands::close_network_capture,
            commands::import_network_capture,
            commands::configure_device_proxy,
            commands::restore_device_proxy
        ])
        // A live `adb logcat` child must not outlive the window.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<AppState>().stop_all_streams();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
