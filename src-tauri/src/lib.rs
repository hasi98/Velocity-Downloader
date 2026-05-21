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
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::RwLock;

fn velocity_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("./icons/64x64.png")
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let download_manager: ManagerState = Arc::new(RwLock::new(DownloadManager::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("velocity_icons")
                .on_window_ready(|window| {
                    let _ = window.set_icon(velocity_icon());
                })
                .build(),
        )
        .manage(download_manager)
        .setup(|app| {
            let autostart = app.autolaunch();
            let settings = state::StateManager::load_settings();
            let autostart_enabled = autostart.is_enabled().unwrap_or(false);
            if settings.start_on_boot && !autostart_enabled {
                let _ = autostart.enable();
            } else if !settings.start_on_boot && autostart_enabled {
                let _ = autostart.disable();
            }

            let show = MenuItem::with_id(app, "show", "Show Velocity Downloader", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_icon(velocity_icon());
            }

            TrayIconBuilder::new()
                .tooltip("Velocity Downloader")
                .icon(velocity_icon())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            if std::env::args().any(|arg| arg == "--minimized") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                server::run_server(handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe_url,
            commands::add_download,
            commands::prefetch_download,
            commands::reveal_download,
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
