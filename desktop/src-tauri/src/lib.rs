pub mod wsl_manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            wsl_manager::open_folder,
            wsl_manager::open_site_folder,
            wsl_manager::open_vscode,
            wsl_manager::check_wsl_status,
            wsl_manager::restart_engine,
            wsl_manager::bootstrap_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
