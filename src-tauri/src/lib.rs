mod commands;
mod engine;
mod manager;
mod models;
mod native_messaging;
mod state;
mod server;

use commands::ManagerState;
use manager::DownloadManager;
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let download_manager: ManagerState = Arc::new(RwLock::new(DownloadManager::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(download_manager)
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                server::run_server(handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe_url,
            commands::add_download,
            commands::pause_download,
            commands::resume_download,
            commands::remove_download,
            commands::get_all_downloads,
            commands::get_download,
            commands::update_settings,
            commands::get_settings,
            commands::get_default_download_dir,
            commands::bring_window_to_front,
            commands::check_extension_installed,
            commands::install_extension,
            commands::set_task_speed_limit,
            commands::open_file,
            commands::open_folder,
            native_messaging::generate_native_manifests,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
