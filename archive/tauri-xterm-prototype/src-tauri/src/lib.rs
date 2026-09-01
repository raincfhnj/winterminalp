pub mod commands;
pub mod domain;
pub mod error;
pub mod terminal;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(commands::AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::split_pane,
            commands::focus_pane,
            commands::resize_pane,
            commands::close_pane,
            commands::toggle_zoom,
            commands::create_tab,
            commands::activate_tab,
            commands::close_tab,
            commands::start_terminal,
            commands::write_terminal,
            commands::resize_terminal,
            commands::restart_terminal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
