mod builtin;
mod commands;
mod config_store;
mod logcat;
mod model;
mod rules;
mod session;

use session::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::open_log_file,
            commands::query_logs,
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
            commands::import_rules_json
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
