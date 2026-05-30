#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod manager;
mod media;
mod models;
mod state;

use axum::{extract::State, http::Method, routing::post, Json, Router};
use chrono::{Datelike, Local, Timelike};
use engine::DownloadEngineConfig;
use engine::{DownloadEngine, SharedSpeedLimiter};
use futures_util::StreamExt;
use manager::DownloadManager;
use models::{AppSettings, DownloadStatus, DownloadTask, HttpContext};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slint::winit_030::{winit, WinitWindowAccessor};
use slint::CloseRequestResponse;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, PhysicalSize, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tower_http::cors::{Any, CorsLayer};
use tokio::io::AsyncWriteExt;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BringWindowToTop, CallWindowProcW, CreatePopupMenu, DestroyMenu, GetCursorPos,
    GetSystemMetrics, LoadIconW, LoadImageW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TrackPopupMenu, GWLP_WNDPROC, HWND_NOTOPMOST, HWND_TOPMOST,
    IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, MF_SEPARATOR, MF_STRING,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_RESTORE,
    SW_SHOWNORMAL, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WA_ACTIVE, WM_ACTIVATE, WM_APP,
    WM_CLOSE, WM_CONTEXTMENU, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCACTIVATE,
    WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN, WM_NCRBUTTONDOWN, WM_RBUTTONUP,
    WNDPROC,
};
#[cfg(target_os = "windows")]
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

slint::include_modules!();

thread_local! {
    static DOWNLOAD_WINDOWS: RefCell<HashMap<String, DownloadStatusWindow>> = RefCell::new(HashMap::new());
    static COMPLETION_HANDLED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static LAST_PUBLISHED_ROWS: RefCell<Vec<RowView>> = RefCell::new(Vec::new());
    static LAST_PUBLISHED_DETAILS: RefCell<HashMap<String, DownloadDetailView>> = RefCell::new(HashMap::new());
    static LAST_COMPLETION_STATES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static SETTINGS_WINDOW: RefCell<Option<NativeSettingsWindow>> = const { RefCell::new(None) };
    static ABOUT_WINDOW: RefCell<Option<NativeAboutWindow>> = const { RefCell::new(None) };
    static GLOBAL_SPEED_LIMIT_WINDOW: RefCell<Option<NativeGlobalSpeedLimiterWindow>> = const { RefCell::new(None) };
    static ADD_DOWNLOAD_WINDOW: RefCell<Option<NativeAddDownloadWindow>> = const { RefCell::new(None) };
    static ADD_DOWNLOAD_CONTEXT: RefCell<HttpContext> = RefCell::new(HttpContext::default());
    static BATCH_DOWNLOAD_WINDOW: RefCell<Option<NativeBatchDownloadWindow>> = const { RefCell::new(None) };
    static SCHEDULER_WINDOW: RefCell<Option<NativeSchedulerWindow>> = const { RefCell::new(None) };
    static FILE_PROPERTIES_WINDOWS: RefCell<HashMap<String, FilePropertiesWindow>> = RefCell::new(HashMap::new());
    static DOWNLOAD_COMPLETE_WINDOWS: RefCell<HashMap<String, DownloadCompleteWindow>> = RefCell::new(HashMap::new());
    static COMPLETION_DIALOGS_DISABLED: RefCell<bool> = const { RefCell::new(false) };
    static COMPLETION_DIALOG_ELIGIBLE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    #[cfg(target_os = "windows")]
    static MAIN_WINDOW_HOOKS: RefCell<HashMap<isize, MainWindowHook>> = RefCell::new(HashMap::new());
}

static REFRESH_NOTIFY: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();
static PENDING_UPDATE: OnceLock<Arc<Mutex<Option<PendingUpdate>>>> = OnceLock::new();
static SCHEDULER_CONFIG: OnceLock<Arc<Mutex<SchedulerConfig>>> = OnceLock::new();
const UPDATE_ENDPOINT: &str =
    "https://github.com/hasi98/Velocity-Downloader/releases/latest/download/latest.json";
#[cfg(target_os = "windows")]
const TRAY_MESSAGE: u32 = WM_APP + 42;
#[cfg(target_os = "windows")]
const TRAY_OPEN: u32 = 1001;
#[cfg(target_os = "windows")]
const TRAY_ADD_DOWNLOAD: u32 = 1002;
#[cfg(target_os = "windows")]
const TRAY_ADD_BATCH: u32 = 1003;
#[cfg(target_os = "windows")]
const TRAY_EXIT: u32 = 1004;
#[cfg(target_os = "windows")]
const WM_SETICON: u32 = 0x0080;
#[cfg(target_os = "windows")]
const ICON_SMALL: usize = 0;
#[cfg(target_os = "windows")]
const ICON_BIG: usize = 1;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
struct MainWindowHook {
    weak: slint::Weak<NativeMain>,
    menu_open: Arc<Mutex<bool>>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
    original_proc: isize,
}

#[derive(Clone, PartialEq)]
struct RowView {
    id: String,
    filename: String,
    size: String,
    status: String,
    time_left: String,
    rate: String,
    category: String,
    progress: f32,
}

#[derive(Clone, PartialEq)]
struct DownloadDetailView {
    id: String,
    filename: String,
    url: String,
    save_path: String,
    file_size: String,
    downloaded: String,
    status: String,
    rate: String,
    time_left: String,
    resume_capability: String,
    connections: String,
    progress_text: String,
    progress: f32,
    speed_limit_text: String,
    segments: Vec<SegmentView>,
}

#[derive(Clone, PartialEq)]
struct SegmentView {
    number: String,
    downloaded: String,
    status: String,
    progress: f32,
}

#[derive(Clone)]
struct PendingUpdate {
    version: String,
    notes: String,
    url: String,
    sha256: String,
}

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    #[serde(default)]
    notes: String,
    platforms: HashMap<String, UpdatePlatform>,
}

#[derive(Deserialize)]
struct UpdatePlatform {
    url: String,
    #[allow(dead_code)]
    signature: Option<String>,
    #[serde(default)]
    sha256: String,
}

#[derive(Clone)]
struct FilePropertiesView {
    id: String,
    filename: String,
    file_type: String,
    status: String,
    size: String,
    save_path: String,
    url: String,
    last_date: String,
    result: String,
    can_open: bool,
}

#[derive(Clone)]
struct AddAnalysisView {
    filename: String,
    file_size: String,
    save_path: String,
    is_media: bool,
    qualities: Vec<QualityOptionView>,
}

#[derive(Clone)]
struct QualityOptionView {
    id: String,
    label: String,
    size: String,
    size_bytes: String,
}

#[derive(Clone)]
struct ExtensionApiState {
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
}

#[derive(Deserialize)]
struct ExtensionDownloadRequest {
    url: String,
    cookies: Option<String>,
    referer: Option<String>,
    user_agent: Option<String>,
    #[allow(dead_code)]
    source: Option<String>,
}

#[derive(Serialize)]
struct ExtensionDownloadResponse {
    success: bool,
    message: String,
}

#[derive(Clone, PartialEq)]
struct BatchQueueView {
    url: String,
    status: String,
    progress: f32,
    error: String,
    active: bool,
}

#[derive(Clone)]
struct SchedulerConfig {
    enabled: bool,
    start_at_enabled: bool,
    start_time: String,
    stop_enabled: bool,
    stop_time: String,
    last_start_key: String,
    last_stop_key: String,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_at_enabled: true,
            start_time: "08:00".to_string(),
            stop_enabled: true,
            stop_time: "23:00".to_string(),
            last_start_key: String::new(),
            last_stop_key: String::new(),
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    env_logger::init();
    let started_from_windows_startup = std::env::args().any(|arg| arg == "--startup");

    let ui = NativeMain::new()?;
    let manager = Arc::new(DownloadManager::new());
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start async runtime"),
    );
    let selected_category = Arc::new(Mutex::new("All Downloads".to_string()));
    let search_query = Arc::new(Mutex::new(String::new()));
    let sort_mode = Arc::new(Mutex::new("date".to_string()));
    let menu_open = Arc::new(Mutex::new(false));
    let last_row_click: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
    let refresh_notify = Arc::new(tokio::sync::Notify::new());
    let _ = REFRESH_NOTIFY.set(refresh_notify.clone());
    let scheduler_config = Arc::new(Mutex::new(SchedulerConfig::default()));
    let _ = SCHEDULER_CONFIG.set(scheduler_config.clone());
    let weak = ui.as_weak();
    let initial_settings = crate::state::StateManager::load_settings();
    COMPLETION_DIALOGS_DISABLED.with(|disabled| {
        *disabled.borrow_mut() = !initial_settings.show_download_complete_dialog;
    });
    media::log_ffmpeg_availability(None);
    cleanup_downloaded_update_installers();
    sync_startup_setting_on_launch(manager.clone(), runtime.clone());
    sync_extension_files_on_startup();
    start_extension_api(
        manager.clone(),
        runtime.clone(),
        weak.clone(),
        selected_category.clone(),
        search_query.clone(),
        sort_mode.clone(),
    );

    ui.window().on_close_requested({
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                let _ = ui.hide();
            }
            CloseRequestResponse::KeepWindowShown
        }
    });

    refresh_download_rows(
        weak.clone(),
        manager.clone(),
        runtime.clone(),
        selected_category.clone(),
        search_query.clone(),
        sort_mode.clone(),
    );
    start_refresh_loop(
        weak.clone(),
        manager.clone(),
        runtime.clone(),
        selected_category.clone(),
        search_query.clone(),
        sort_mode.clone(),
        refresh_notify,
    );
    start_scheduler_loop(
        scheduler_config,
        manager.clone(),
        runtime.clone(),
        weak.clone(),
        selected_category.clone(),
        search_query.clone(),
        sort_mode.clone(),
    );

    ui.on_add_url({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move || {
            set_status(weak.clone(), "");
            open_add_download_window(
                manager.clone(),
                runtime.clone(),
                weak.clone(),
                selected_category.clone(),
                search_query.clone(),
                sort_mode.clone(),
            );
        }
    });

    ui.on_open_options({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        move || {
            open_settings_window(manager.clone(), runtime.clone(), weak.clone());
        }
    });

    ui.on_install_extension_prompt({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        move || {
            mark_extension_prompt_seen(manager.clone(), runtime.clone());
            if let Some(ui) = weak.upgrade() {
                ui.set_show_extension_install_prompt(false);
            }
            open_settings_window_with_tab(
                manager.clone(),
                runtime.clone(),
                weak.clone(),
                Some("Browser Extension".to_string()),
            );
        }
    });

    ui.on_dismiss_extension_prompt({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        move || {
            mark_extension_prompt_seen(manager.clone(), runtime.clone());
            if let Some(ui) = weak.upgrade() {
                ui.set_show_extension_install_prompt(false);
            }
        }
    });

    ui.on_view_update_prompt({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_update_prompt(false);
            }
            open_settings_window_with_tab(
                manager.clone(),
                runtime.clone(),
                weak.clone(),
                Some("Updates".to_string()),
            );
        }
    });

    ui.on_dismiss_update_prompt({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_update_prompt(false);
            }
        }
    });

    ui.on_context_menu_state_changed({
        let menu_open = menu_open.clone();
        move |open| {
            if let Ok(mut state) = menu_open.lock() {
                *state = open;
            }
        }
    });

    ui.on_app_chrome_pointer_down({
        let weak = weak.clone();
        let menu_open = menu_open.clone();
        move || {
            let was_open = menu_open.lock().map(|state| *state).unwrap_or(false);
            if !was_open {
                return;
            }

            if let Ok(mut state) = menu_open.lock() {
                *state = false;
            }

            if let Some(ui) = weak.upgrade() {
                ui.set_show_download_context_menu(false);
                ui.set_context_popup_pending(false);
                ui.set_context_download_id(SharedString::default());
                ui.set_context_download_status(SharedString::default());
                ui.set_context_menu_x(0.0);
                ui.set_context_menu_y(0.0);
            }
        }
    });

    ui.on_category_selected({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |category| {
            if let Ok(mut selected) = selected_category.lock() {
                *selected = category.to_string();
            }
            refresh_download_rows(
                weak.clone(),
                manager.clone(),
                runtime.clone(),
                selected_category.clone(),
                search_query.clone(),
                sort_mode.clone(),
            );
        }
    });

    ui.on_download_selected({
        let weak = weak.clone();
        move |id| {
            if !id.is_empty() {
                set_status(weak.clone(), "");
            }
        }
    });

    ui.on_selection_request({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let last_row_click = last_row_click.clone();
        move |id, index, ctrl, shift| {
            let id = id.to_string();
            let should_open_properties = if !ctrl && !shift && !id.is_empty() {
                let now = Instant::now();
                if let Ok(mut last) = last_row_click.lock() {
                    let matched = last
                        .as_ref()
                        .map(|(last_id, time)| {
                            last_id == &id && now.duration_since(*time) <= Duration::from_millis(550)
                        })
                        .unwrap_or(false);
                    *last = Some((id.clone(), now));
                    matched
                } else {
                    false
                }
            } else {
                false
            };
            update_selection_from_request(weak.clone(), id.clone(), index, ctrl, shift);
            if should_open_properties {
                open_file_properties_for_id(
                    id,
                    manager.clone(),
                    runtime.clone(),
                    weak.clone(),
                );
            }
        }
    });

    ui.on_context_selection_request({
        let weak = weak.clone();
        let last_row_click = last_row_click.clone();
        move |id, index| {
            if let Ok(mut last) = last_row_click.lock() {
                *last = None;
            }
            update_selection_from_request(weak.clone(), id.to_string(), index, false, false);
        }
    });

    ui.on_keyboard_selection_request({
        let weak = weak.clone();
        move |action, shift, ctrl| {
            update_selection_from_keyboard(weak.clone(), action.to_string(), shift, ctrl);
        }
    });

    ui.on_download_double_clicked({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        move |id| {
            if !id.is_empty() {
                open_file_properties_for_id(
                    id.to_string(),
                    manager.clone(),
                    runtime.clone(),
                    weak.clone(),
                );
            }
        }
    });

    ui.on_resume_selected({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |id| {
            let id = id.to_string();
            if id.is_empty() {
                set_status(weak.clone(), "Select a download first.");
                return;
            }
            let weak_for_task = weak.clone();
            let manager_for_task = manager.clone();
            let selected_for_task = selected_category.clone();
            let search_for_task = search_query.clone();
            let sort_for_task = sort_mode.clone();
            let runtime_for_refresh = runtime.clone();
            runtime.spawn(async move {
                let result = manager_for_task.resume_download(&id, None).await;
                match result {
                    Ok(()) => pulse_refresh_loop(runtime_for_refresh.clone()),
                    Err(error) => {
                        set_status(weak_for_task.clone(), format!("Resume failed: {}", error));
                    }
                }
                refresh_download_rows(
                    weak_for_task,
                    manager_for_task,
                    runtime_for_refresh,
                    selected_for_task,
                    search_for_task,
                    sort_for_task,
                );
            });
        }
    });

    ui.on_stop_selected({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |id| {
            let id = id.to_string();
            if id.is_empty() {
                set_status(weak.clone(), "Select a download first.");
                return;
            }
            let weak_for_task = weak.clone();
            let manager_for_task = manager.clone();
            let selected_for_task = selected_category.clone();
            let search_for_task = search_query.clone();
            let sort_for_task = sort_mode.clone();
            let runtime_for_refresh = runtime.clone();
            runtime.spawn(async move {
                let result = manager_for_task.pause_download(&id).await;
                if let Err(error) = result {
                    set_status(weak_for_task.clone(), format!("Stop failed: {}", error));
                }
                refresh_download_rows(
                    weak_for_task,
                    manager_for_task,
                    runtime_for_refresh,
                    selected_for_task,
                    search_for_task,
                    sort_for_task,
                );
            });
        }
    });

    ui.on_delete_selected({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |id| {
            let id = id.to_string();
            if id.is_empty() {
                set_status(weak.clone(), "Select a download first.");
                return;
            }
            let weak_for_task = weak.clone();
            let manager_for_task = manager.clone();
            let selected_for_task = selected_category.clone();
            let search_for_task = search_query.clone();
            let sort_for_task = sort_mode.clone();
            let runtime_for_refresh = runtime.clone();
            runtime.spawn(async move {
                let result = manager_for_task.remove_download(&id).await;
                match result {
                    Ok(()) => {
                        clear_selected_download(weak_for_task.clone());
                    }
                    Err(error) => {
                        set_status(weak_for_task.clone(), format!("Delete failed: {}", error));
                    }
                }
                refresh_download_rows(
                    weak_for_task,
                    manager_for_task,
                    runtime_for_refresh,
                    selected_for_task,
                    search_for_task,
                    sort_for_task,
                );
            });
        }
    });

    ui.on_delete_selected_list({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |ids| {
            let ids = parse_selected_ids(ids.as_str());
            if ids.is_empty() {
                set_status(weak.clone(), "Select one or more downloads first.");
                return;
            }
            let weak_for_task = weak.clone();
            let manager_for_task = manager.clone();
            let selected_for_task = selected_category.clone();
            let search_for_task = search_query.clone();
            let sort_for_task = sort_mode.clone();
            let runtime_for_refresh = runtime.clone();
            runtime.spawn(async move {
                let mut removed = 0usize;
                for id in &ids {
                    if manager_for_task.remove_download(id).await.is_ok() {
                        removed += 1;
                    }
                }
                clear_selected_download(weak_for_task.clone());
                set_status(
                    weak_for_task.clone(),
                    format!("Deleted {} download(s).", removed),
                );
                refresh_download_rows(
                    weak_for_task,
                    manager_for_task,
                    runtime_for_refresh,
                    selected_for_task,
                    search_for_task,
                    sort_for_task,
                );
            });
        }
    });

    ui.on_search_updated({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |query| {
            if let Ok(mut search) = search_query.lock() {
                *search = query.trim().to_string();
            }
            refresh_download_rows(
                weak.clone(),
                manager.clone(),
                runtime.clone(),
                selected_category.clone(),
                search_query.clone(),
                sort_mode.clone(),
            );
        }
    });

    ui.on_menu_action({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |action| {
            handle_menu_action(
                action.to_string(),
                weak.clone(),
                manager.clone(),
                runtime.clone(),
                selected_category.clone(),
                search_query.clone(),
                sort_mode.clone(),
            );
        }
    });

    ui.on_download_context_action({
        let weak = weak.clone();
        let manager = manager.clone();
        let runtime = runtime.clone();
        let selected_category = selected_category.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        move |action, id| {
            handle_download_context_action(
                action.to_string(),
                id.to_string(),
                weak.clone(),
                manager.clone(),
                runtime.clone(),
                selected_category.clone(),
                search_query.clone(),
                sort_mode.clone(),
            );
        }
    });

    ui.show()?;
    let should_show_extension_prompt = !runtime
        .block_on(manager.get_settings())
        .extension_prompt_seen;
    if should_show_extension_prompt && !started_from_windows_startup {
        ui.set_show_extension_install_prompt(true);
    }
    if started_from_windows_startup {
        let _ = ui.hide();
    }
    start_update_check_on_launch(
        weak.clone(),
        runtime.clone(),
        !should_show_extension_prompt && !started_from_windows_startup,
    );
    #[cfg(target_os = "windows")]
    set_slint_window_icons(&ui.window());
    #[cfg(target_os = "windows")]
    schedule_main_window_titlebar_hook(
        weak.clone(),
        menu_open.clone(),
        manager.clone(),
        runtime.clone(),
        selected_category.clone(),
        search_query.clone(),
        sort_mode.clone(),
    );
    slint::run_event_loop_until_quit()
}

#[cfg(target_os = "windows")]
fn schedule_main_window_titlebar_hook(
    weak: slint::Weak<NativeMain>,
    menu_open: Arc<Mutex<bool>>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    slint::Timer::single_shot(std::time::Duration::from_millis(150), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        install_main_window_titlebar_hook(
            &ui,
            weak.clone(),
            menu_open,
            manager,
            runtime,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

#[cfg(target_os = "windows")]
fn install_main_window_titlebar_hook(
    ui: &NativeMain,
    weak: slint::Weak<NativeMain>,
    menu_open: Arc<Mutex<bool>>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let slint_handle = ui.window().window_handle();
    let Ok(handle) = slint_handle.window_handle() else {
        log::warn!("Could not install tray/icon hook: Slint window handle is not available.");
        return;
    };

    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        log::warn!("Could not install tray/icon hook: native handle is not Win32.");
        return;
    };

    let hwnd_key = handle.hwnd.get();
    let hwnd = hwnd_key as HWND;

    MAIN_WINDOW_HOOKS.with(|hooks| {
        if hooks.borrow().contains_key(&hwnd_key) {
            return;
        }

        // SAFETY: The HWND belongs to the Slint window on this UI thread. We store the
        // previous procedure and always forward messages to it from the replacement proc.
        let original_proc = unsafe {
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, main_window_proc as *const () as isize)
        };
        if original_proc == 0 {
            let error = unsafe { GetLastError() };
            if error != 0 {
                log::warn!(
                    "Could not subclass main window for tray menu. error={}",
                    error
                );
                return;
            }
        }

        hooks.borrow_mut().insert(
            hwnd_key,
            MainWindowHook {
                weak,
                menu_open,
                manager,
                runtime,
                selected_category,
                search_query,
                sort_mode,
                original_proc,
            },
        );
        log::info!("Installing Velocity tray icon for HWND {hwnd:?}.");
        add_windows_tray_icon(hwnd);
        set_window_icons(hwnd);
    });
}

#[cfg(target_os = "windows")]
fn add_windows_tray_icon(hwnd: HWND) {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = load_icon_at_size(16);
    log::info!(
        "Velocity tray icon handle loaded: {}",
        !data.hIcon.is_null()
    );

    let tip = "Velocity Download Manager\0"
        .encode_utf16()
        .collect::<Vec<u16>>();
    let copy_len = tip.len().min(data.szTip.len());
    data.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);

    unsafe {
        let ok = Shell_NotifyIconW(NIM_ADD, &mut data);
        if ok == 0 {
            log::warn!(
                "Could not add Velocity tray icon. icon_loaded={}, error={}",
                !data.hIcon.is_null(),
                GetLastError()
            );
            return;
        }
        log::info!("Velocity tray icon added.");

        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let version_ok = Shell_NotifyIconW(NIM_SETVERSION, &mut data);
        if version_ok == 0 {
            log::warn!(
                "Could not set Velocity tray icon version. error={}",
                GetLastError()
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn load_icon_at_size(size: i32) -> *mut std::ffi::c_void {
    let Some(path) = find_icon_file_for_tray() else {
        log::warn!("Velocity icon file was not found.");
        return unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
    };
    let mut wide: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    wide.push(0);
    let icon = unsafe {
        LoadImageW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            IMAGE_ICON,
            size,
            size,
            LR_LOADFROMFILE,
        )
    };
    if !icon.is_null() {
        log::debug!("Loaded Velocity icon from {}", path.display());
        return icon;
    }

    let fallback = unsafe {
        LoadImageW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
    };
    if !fallback.is_null() {
        log::debug!("Loaded Velocity default-size icon from {}", path.display());
        return fallback;
    }

    log::warn!(
        "Could not load Velocity icon from {}. error={}",
        path.display(),
        unsafe { GetLastError() }
    );
    unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) }
}

#[cfg(target_os = "windows")]
fn set_slint_window_icons(window: &slint::Window) {
    if let Some(icon) = load_winit_window_icon() {
        window.with_winit_window(|winit_window| {
            winit_window.set_window_icon(Some(icon));
        });
    }

    let slint_handle = window.window_handle();
    let Ok(handle) = slint_handle.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    set_window_icons(handle.hwnd.get() as HWND);
}

#[cfg(target_os = "windows")]
fn force_window_to_front_later(window_weak: slint::Weak<DownloadCompleteWindow>) {
    slint::Timer::single_shot(std::time::Duration::from_millis(80), move || {
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        center_window_on_primary_screen(&window.window());
        force_window_to_front(&window.window());
    });
}

#[cfg(target_os = "windows")]
fn center_window_on_primary_screen(window: &slint::Window) {
    let slint_handle = window.window_handle();
    let Ok(handle) = slint_handle.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let size = window.size();
    let width = if size.width > 0 {
        size.width as i32
    } else {
        520
    };
    let height = if size.height > 0 {
        size.height as i32
    } else {
        250
    };

    unsafe {
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let x = ((screen_width - width) / 2).max(0);
        let y = ((screen_height - height) / 2).max(0);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_SHOWWINDOW,
        );
    }
}

#[cfg(target_os = "windows")]
fn force_window_to_front(window: &slint::Window) {
    let slint_handle = window.window_handle();
    let Ok(handle) = slint_handle.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;

    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        // Briefly use topmost so the completion popup appears above other
        // applications, then immediately return it to normal z-order.
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        SendMessageW(hwnd, WM_ACTIVATE, WA_ACTIVE as usize, 0);
        SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
    }
}

#[cfg(target_os = "windows")]
fn load_winit_window_icon() -> Option<winit::window::Icon> {
    let path = find_icon_file_for_tray()?;
    let file = fs::File::open(path).ok()?;
    let icon_dir = ico::IconDir::read(file).ok()?;
    let entry = icon_dir
        .entries()
        .iter()
        .min_by_key(|entry| entry.width().abs_diff(32) + entry.height().abs_diff(32))?;
    let image = entry.decode().ok()?;
    winit::window::Icon::from_rgba(image.rgba_data().to_vec(), image.width(), image.height()).ok()
}

#[cfg(target_os = "windows")]
fn set_window_icons(hwnd: HWND) {
    let small = load_icon_at_size(16);
    if !small.is_null() {
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, small as isize);
        }
    }

    let big = load_icon_at_size(32);
    if !big.is_null() {
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, big as isize);
        }
    }
}

#[cfg(target_os = "windows")]
fn find_icon_file_for_tray() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("icons").join("icon.ico"));
            candidates.push(dir.join("resources").join("icons").join("icon.ico"));
            candidates.push(dir.join("resources").join("icon.ico"));
            candidates.push(dir.join("..").join("icons").join("icon.ico"));
            // Dev mode: exe is at target/debug/ or target/release/, icons at src-tauri/icons/
            candidates.push(dir.join("..").join("..").join("icons").join("icon.ico"));
        }
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("icons").join("icon.ico"));
        candidates.push(current.join("src-tauri").join("icons").join("icon.ico"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(target_os = "windows")]
fn remove_windows_tray_icon(hwnd: HWND) {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &mut data);
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let hwnd_key = hwnd as isize;

    let should_close_menus = matches!(
        msg,
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN
    ) || msg == WM_KILLFOCUS
        || (msg == WM_ACTIVATE && (wparam & 0xffff) == 0);

    if should_close_menus {
        MAIN_WINDOW_HOOKS.with(|hooks| {
            if let Some(hook) = hooks.borrow().get(&hwnd_key) {
                close_menus_from_hook(&hook.weak, &hook.menu_open);
            }
        });
    }

    if msg == WM_CLOSE {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        return 0;
    }

    if msg == TRAY_MESSAGE {
        let tray_event = tray_notification_event(lparam);
        match tray_event {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                show_main_window_from_tray(hwnd);
                return 0;
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                let command = show_tray_context_menu(hwnd);
                if command != 0 {
                    handle_tray_command(hwnd, command);
                }
                return 0;
            }
            _ => {}
        }
    }

    let original_proc = MAIN_WINDOW_HOOKS.with(|hooks| {
        hooks
            .borrow()
            .get(&hwnd_key)
            .map(|hook| hook.original_proc)
            .unwrap_or_default()
    });

    if original_proc == 0 {
        return 0;
    }

    let original_proc: WNDPROC = std::mem::transmute(original_proc);
    CallWindowProcW(original_proc, hwnd, msg, wparam, lparam)
}

#[cfg(target_os = "windows")]
fn tray_notification_event(lparam: LPARAM) -> u32 {
    // With NOTIFYICON_VERSION_4, Windows stores the notification event in
    // LOWORD(lParam). Older behavior used the full lParam value.
    let low_word = (lparam as u32) & 0xffff;
    if low_word != 0 {
        low_word
    } else {
        lparam as u32
    }
}

#[cfg(target_os = "windows")]
fn show_main_window_from_tray(hwnd: HWND) {
    MAIN_WINDOW_HOOKS.with(|hooks| {
        if let Some(hook) = hooks.borrow().get(&(hwnd as isize)) {
            if let Some(ui) = hook.weak.upgrade() {
                let _ = ui.show();
            }
        }
    });
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

#[cfg(target_os = "windows")]
fn show_tray_context_menu(hwnd: HWND) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }

        let open = wide_menu_text("Open");
        let add_download = wide_menu_text("Add new download");
        let add_batch = wide_menu_text("Add new Batch download");
        let exit = wide_menu_text("Exit");

        AppendMenuW(menu, MF_STRING, TRAY_OPEN as usize, open.as_ptr());
        AppendMenuW(
            menu,
            MF_STRING,
            TRAY_ADD_DOWNLOAD as usize,
            add_download.as_ptr(),
        );
        AppendMenuW(menu, MF_STRING, TRAY_ADD_BATCH as usize, add_batch.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, TRAY_EXIT as usize, exit.as_ptr());

        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            DestroyMenu(menu);
            return 0;
        }

        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        command as u32
    }
}

#[cfg(target_os = "windows")]
fn wide_menu_text(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn handle_tray_command(hwnd: HWND, command: u32) {
    let context = MAIN_WINDOW_HOOKS.with(|hooks| {
        hooks.borrow().get(&(hwnd as isize)).map(|hook| {
            (
                hook.weak.clone(),
                hook.manager.clone(),
                hook.runtime.clone(),
                hook.selected_category.clone(),
                hook.search_query.clone(),
                hook.sort_mode.clone(),
            )
        })
    });

    match command {
        TRAY_OPEN => show_main_window_from_tray(hwnd),
        TRAY_ADD_DOWNLOAD => {
            if let Some((weak, manager, runtime, selected_category, search_query, sort_mode)) =
                context
            {
                show_main_window_from_tray(hwnd);
                open_add_download_window(
                    manager,
                    runtime,
                    weak,
                    selected_category,
                    search_query,
                    sort_mode,
                );
            }
        }
        TRAY_ADD_BATCH => {
            if let Some((weak, manager, runtime, selected_category, search_query, sort_mode)) =
                context
            {
                show_main_window_from_tray(hwnd);
                open_batch_download_window(
                    manager,
                    runtime,
                    weak,
                    selected_category,
                    search_query,
                    sort_mode,
                );
            }
        }
        TRAY_EXIT => {
            remove_windows_tray_icon(hwnd);
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
}

#[cfg(target_os = "windows")]
fn close_menus_from_hook(weak: &slint::Weak<NativeMain>, menu_open: &Arc<Mutex<bool>>) {
    let was_open = menu_open.lock().map(|state| *state).unwrap_or(false);
    if let Some(ui) = weak.upgrade() {
        ui.set_active_menu(SharedString::default());
        ui.set_menu_hot_tracking(false);

        if was_open {
            if let Ok(mut state) = menu_open.lock() {
                *state = false;
            }

            ui.set_show_download_context_menu(false);
            ui.set_context_popup_pending(false);
            ui.set_context_download_id(SharedString::default());
            ui.set_context_download_status(SharedString::default());
            ui.set_context_menu_x(0.0);
            ui.set_context_menu_y(0.0);
        }
    }
}

fn handle_menu_action(
    action: String,
    weak: slint::Weak<NativeMain>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let selected_id = weak
        .upgrade()
        .map(|ui| ui.get_selected_download_id().to_string())
        .unwrap_or_default();

    match action.as_str() {
        "add-download" => set_status(weak, ""),
        "add-batch" => open_batch_download_window(
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "exit" => std::process::exit(0),
        "resume" | "download-now" => resume_selected_download(
            selected_id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "stop" => stop_selected_download(
            selected_id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "redownload" => redownload_selected(
            selected_id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "delete-completed" => delete_completed_downloads(
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "stop-all" => stop_all_downloads(
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "start-queue" => start_queued_downloads(
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "stop-queue" => stop_all_downloads(
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "scheduler" => open_scheduler_window(
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "find" => set_status(weak, "Search is ready."),
        "speed-limiter" => toggle_speed_limiter(manager, runtime, weak),
        "toggle-categories" => set_status(weak, "Category panel toggled."),
        "arrange-date" | "arrange-updated" | "arrange-name" | "arrange-size" | "arrange-status" => {
            let mode = action.trim_start_matches("arrange-").to_string();
            if let Ok(mut sort) = sort_mode.lock() {
                *sort = mode.clone();
            }
            set_status(weak.clone(), format!("Arranged by {}", mode));
            refresh_download_rows(
                weak,
                manager,
                runtime,
                selected_category,
                search_query,
                sort_mode,
            );
        }
        "check-updates" => {
            open_settings_window_with_tab(
                manager,
                runtime,
                weak.clone(),
                Some("Updates".to_string()),
            );
            set_status(weak, "Open Options > Updates to check for releases.");
        }
        "about" => show_about_window(weak),
        "homepage" => {
            open_url("https://github.com/hasi98/velocity-downloader");
            set_status(weak, "Opening project homepage.");
        }
        "support" => {
            open_url("https://github.com/hasi98/velocity-downloader/issues");
            set_status(weak, "Opening support page.");
        }
        _ => {}
    }
}

fn handle_download_context_action(
    action: String,
    id: String,
    weak: slint::Weak<NativeMain>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(weak, "Select a download first.");
        return;
    }

    select_download(weak.clone(), &id);

    match action.as_str() {
        "open" => open_selected_file(id, manager, runtime, weak),
        "open-folder" => open_selected_folder(id, manager, runtime, weak),
        "show-download-window" => open_download_status_for_id(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "resume" => resume_selected_download(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "stop" => stop_selected_download(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "redownload" => redownload_selected(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "remove" => remove_selected_download(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "move-queue" => move_selected_to_queue(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "delete-queue" => remove_selected_from_queue(
            id,
            manager,
            runtime,
            weak,
            selected_category,
            search_query,
            sort_mode,
        ),
        "refresh-address" => {
            set_status(
                weak,
                "Refresh address will be available after the replace URL flow is added.",
            );
        }
        "properties" => open_file_properties_for_id(id, manager, runtime, weak),
        _ => {}
    }
}

fn show_about_window(main_weak: slint::Weak<NativeMain>) {
    let _ = slint::invoke_from_event_loop(move || {
        ABOUT_WINDOW.with(|about_window| {
            let mut about_window = about_window.borrow_mut();
            if about_window.is_none() {
                let Ok(window) = NativeAboutWindow::new() else {
                    set_status(main_weak.clone(), "Could not open About window.");
                    return;
                };
                window.set_version(env!("CARGO_PKG_VERSION").into());

                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());

                window.on_close_about(|| {
                    ABOUT_WINDOW.with(|about_window| {
                        if let Some(window) = about_window.borrow().as_ref() {
                            let _ = window.hide();
                        }
                    });
                });

                window.on_titlebar_drag_requested({
                    let window_weak = window.as_weak();
                    move || {
                        if let Some(window) = window_weak.upgrade() {
                            window.window().with_winit_window(|winit_window| {
                                let _ = winit_window.drag_window();
                            });
                        }
                    }
                });

                window.on_open_homepage(|| {
                    open_url("https://github.com/hasi98/velocity-downloader");
                });

                *about_window = Some(window);
            }

            if let Some(window) = about_window.as_ref() {
                window.set_version(env!("CARGO_PKG_VERSION").into());
                center_about_window(&main_weak, window);
                let _ = window.show();
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());
            }
        });
    });
}

fn center_about_window(main_weak: &slint::Weak<NativeMain>, window: &NativeAboutWindow) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_window = main.window();
    let about_window = window.window();
    let main_pos = main_window.position();
    let main_size = main_window.size();
    let about_size = about_window.size();

    let about_width = if about_size.width > 0 {
        about_size.width as i32
    } else {
        (430.0 * about_window.scale_factor()) as i32
    };
    let about_height = if about_size.height > 0 {
        about_size.height as i32
    } else {
        (166.0 * about_window.scale_factor()) as i32
    };

    let x = main_pos.x + ((main_size.width as i32 - about_width) / 2).max(0);
    let y = main_pos.y + ((main_size.height as i32 - about_height) / 2).max(0);
    about_window.set_position(PhysicalPosition::new(x, y));
}

fn open_global_speed_limiter_window(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
) {
    let manager_for_settings = manager.clone();
    let runtime_for_settings = runtime.clone();
    let _ = slint::invoke_from_event_loop(move || {
        GLOBAL_SPEED_LIMIT_WINDOW.with(|speed_window| {
            let mut speed_window = speed_window.borrow_mut();
            if speed_window.is_none() {
                let Ok(window) = NativeGlobalSpeedLimiterWindow::new() else {
                    set_status(main_weak.clone(), "Could not open speed limiter.");
                    return;
                };

                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());

                window.on_close_window(|| {
                    GLOBAL_SPEED_LIMIT_WINDOW.with(|speed_window| {
                        if let Some(window) = speed_window.borrow().as_ref() {
                            let _ = window.hide();
                        }
                    });
                });

                window.on_titlebar_drag_requested({
                    let window_weak = window.as_weak();
                    move || {
                        if let Some(window) = window_weak.upgrade() {
                            window.window().with_winit_window(|winit_window| {
                                let _ = winit_window.drag_window();
                            });
                        }
                    }
                });

                window.on_apply_limit({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let window_weak = window.as_weak();
                    move |value| {
                        let limit = match parse_speed_limit_kib(value.as_str()) {
                            Ok(limit) => limit,
                            Err(error) => {
                                if let Some(window) = window_weak.upgrade() {
                                    window.set_message(error.into());
                                }
                                return;
                            }
                        };

                        let manager = manager.clone();
                        let main_weak = main_weak.clone();
                        let window_weak = window_weak.clone();
                        runtime.spawn(async move {
                            let mut settings = manager.get_settings().await;
                            settings.speed_limit_bps = limit;
                            manager.update_settings(settings).await;
                            let message = match limit {
                                Some(limit_bps) => format!("Global limit is on at {} KB/s.", limit_bps / 1024),
                                None => "Global speed limiter is off.".to_string(),
                            };
                            set_status(main_weak, message.clone());
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = window_weak.upgrade() {
                                    window.set_limiter_active(limit.is_some());
                                    window.set_message(message.into());
                                }
                            });
                        });
                    }
                });

                window.on_clear_limit({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let window_weak = window.as_weak();
                    move || {
                        let manager = manager.clone();
                        let main_weak = main_weak.clone();
                        let window_weak = window_weak.clone();
                        runtime.spawn(async move {
                            let mut settings = manager.get_settings().await;
                            settings.speed_limit_bps = None;
                            manager.update_settings(settings).await;
                            set_status(main_weak, "Global speed limiter is off.");
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = window_weak.upgrade() {
                                    window.set_limit_text(SharedString::default());
                                    window.set_limiter_active(false);
                                    window.set_message("Global speed limiter is off.".into());
                                }
                            });
                        });
                    }
                });

                *speed_window = Some(window);
            }

            if let Some(window) = speed_window.as_ref() {
                let window_weak = window.as_weak();
                let manager = manager_for_settings.clone();
                runtime_for_settings.spawn(async move {
                    let settings = manager.get_settings().await;
                    let limit = settings.speed_limit_bps;
                    let text = limit.map(|value| (value / 1024).to_string()).unwrap_or_default();
                    let message = if let Some(limit_bps) = limit {
                        format!("Global limit is on at {} KB/s.", limit_bps / 1024)
                    } else {
                        "Global speed limiter is off.".to_string()
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak.upgrade() {
                            window.set_limit_text(text.into());
                            window.set_limiter_active(limit.is_some());
                            window.set_message(message.into());
                        }
                    });
                });
                center_global_speed_limiter_window(&main_weak, window);
                let _ = window.show();
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());
            }
        });
    });
}

fn center_global_speed_limiter_window(
    main_weak: &slint::Weak<NativeMain>,
    window: &NativeGlobalSpeedLimiterWindow,
) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_window = main.window();
    let speed_window = window.window();
    let main_pos = main_window.position();
    let main_size = main_window.size();
    let speed_size = speed_window.size();

    let speed_width = if speed_size.width > 0 {
        speed_size.width as i32
    } else {
        (430.0 * speed_window.scale_factor()) as i32
    };
    let speed_height = if speed_size.height > 0 {
        speed_size.height as i32
    } else {
        (210.0 * speed_window.scale_factor()) as i32
    };

    let x = main_pos.x + ((main_size.width as i32 - speed_width) / 2).max(0);
    let y = main_pos.y + ((main_size.height as i32 - speed_height) / 2).max(0);
    speed_window.set_position(PhysicalPosition::new(x, y));
}

fn open_selected_file(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
) {
    runtime.spawn(async move {
        match manager.get_download(&id).await {
            Some(task) if task.status == DownloadStatus::Completed => {
                open_file_native(&task.save_path);
                set_status(main_weak, format!("Opening {}", task.filename));
            }
            Some(_) => set_status(main_weak, "Download is not complete yet."),
            None => set_status(main_weak, "Download not found."),
        }
    });
}

fn open_selected_folder(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
) {
    runtime.spawn(async move {
        match manager.get_download(&id).await {
            Some(task) => {
                open_folder_native(&task.save_path);
                set_status(main_weak, "Opening download folder.");
            }
            None => set_status(main_weak, "Download not found."),
        }
    });
}

fn move_selected_to_queue(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        match manager_for_task.move_to_scheduled_queue(&id).await {
            Ok(()) => {
                set_status(main_weak.clone(), "Moved download to scheduler queue.");
                wake_refresh_loop();
            }
            Err(error) => set_status(main_weak.clone(), format!("Move to queue failed: {}", error)),
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn remove_selected_from_queue(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        match manager_for_task.pause_download(&id).await {
            Ok(()) => {
                set_status(main_weak.clone(), "Removed download from scheduler queue.");
                wake_refresh_loop();
            }
            Err(error) => {
                set_status(
                    main_weak.clone(),
                    format!("Delete from queue failed: {}", error),
                );
            }
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn resume_selected_download(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let result = match manager_for_task.get_download(&id).await {
            Some(task)
                if matches!(
                    task.status,
                    DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Queued
                ) =>
            {
                manager_for_task.resume_download(&id, None).await
            }
            Some(task) if task.status == DownloadStatus::Downloading => {
                set_status(main_weak.clone(), "Download is already running.");
                Ok(())
            }
            Some(task) if task.status == DownloadStatus::Completed => {
                set_status(main_weak.clone(), "Download already complete.");
                Ok(())
            }
            Some(_) => Ok(()),
            None => Err("Download not found".to_string()),
        };

        match result {
            Ok(()) => pulse_refresh_loop(runtime_for_refresh.clone()),
            Err(error) => {
                set_status(main_weak.clone(), format!("Resume failed: {}", error));
            }
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn stop_selected_download(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        if let Err(error) = manager_for_task.pause_download(&id).await {
            set_status(main_weak.clone(), format!("Stop failed: {}", error));
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn redownload_selected(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let Some(task) = manager_for_task.get_download(&id).await else {
            set_status(main_weak, "Download not found.");
            return;
        };

        let save_dir = Path::new(&task.save_path)
            .parent()
            .map(|path| path.to_string_lossy().to_string());
        let filename = task.filename.clone();
        let url = task.url.clone();
        let ctx = task.http_context.clone();
        let media_format = task.media_format.clone();
        let is_media_redownload =
            task.download_kind == models::DownloadKind::Media && media_format.is_some();
        let expected_size = if is_media_redownload {
            std::fs::metadata(&task.save_path)
                .ok()
                .map(|metadata| metadata.len())
                .filter(|size| *size > 0)
                .or_else(|| (task.total_size > 0).then_some(task.total_size))
        } else {
            None
        };

        let _ = manager_for_task.remove_download(&id).await;
        let result = if is_media_redownload {
            manager_for_task
                .add_download_with_expected_size(
                    url,
                    save_dir,
                    Some(filename),
                    media_format,
                    expected_size,
                    ctx,
                    None,
                )
                .await
        } else {
            manager_for_task
                .add_download(url, save_dir, Some(filename), media_format, ctx, None)
                .await
        };

        match result {
            Ok(task) => {
                mark_completion_dialog_eligible(&task.id);
                pulse_refresh_loop(runtime_for_refresh.clone());
                select_download(main_weak.clone(), &task.id);
                show_download_status_window(
                    task_to_detail(&task),
                    manager_for_task.clone(),
                    runtime_for_refresh.clone(),
                    main_weak.clone(),
                    selected_category.clone(),
                    search_query.clone(),
                    sort_mode.clone(),
                );
            }
            Err(error) => set_status(main_weak.clone(), format!("Redownload failed: {}", error)),
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn remove_selected_download(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let result = manager_for_task.remove_download(&id).await;
        match result {
            Ok(()) => clear_selected_download(main_weak.clone()),
            Err(error) => set_status(main_weak.clone(), format!("Remove failed: {}", error)),
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn delete_completed_downloads(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let completed: Vec<String> = manager_for_task
            .get_all_downloads()
            .await
            .into_iter()
            .filter(|task| task.status == DownloadStatus::Completed)
            .map(|task| task.id)
            .collect();

        for id in &completed {
            let _ = manager_for_task.remove_download(id).await;
        }

        set_status(
            main_weak.clone(),
            format!("Deleted {} completed download(s).", completed.len()),
        );
        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn stop_all_downloads(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let active: Vec<String> = manager_for_task
            .get_all_downloads()
            .await
            .into_iter()
            .filter(|task| {
                matches!(
                    task.status,
                    DownloadStatus::Downloading
                        | DownloadStatus::Queued
                        | DownloadStatus::Assembling
                )
            })
            .map(|task| task.id)
            .collect();

        for id in &active {
            let _ = manager_for_task.pause_download(id).await;
        }

        set_status(
            main_weak.clone(),
            format!("Stopped {} download(s).", active.len()),
        );
        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn start_queued_downloads(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let queued: Vec<String> = manager_for_task
            .get_all_downloads()
            .await
            .into_iter()
            .filter(|task| task.status == DownloadStatus::Queued)
            .map(|task| task.id)
            .collect();

        for id in &queued {
            let _ = manager_for_task.resume_download(id, None).await;
        }
        pulse_refresh_loop(runtime_for_refresh.clone());

        set_status(
            main_weak.clone(),
            format!("Started {} queued download(s).", queued.len()),
        );
        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn open_scheduler_window(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let config = scheduler_config();
    let snapshot = config
        .lock()
        .map(|config| config.clone())
        .unwrap_or_default();
    let queued_rows = scheduler_queued_rows(manager.clone(), runtime.clone());

    SCHEDULER_WINDOW.with(|scheduler_window| {
        let mut scheduler_window = scheduler_window.borrow_mut();
        if scheduler_window.is_none() {
            let Ok(window) = NativeSchedulerWindow::new() else {
                set_status(main_weak, "Could not open Scheduler.");
                return;
            };
            #[cfg(target_os = "windows")]
            set_slint_window_icons(&window.window());

            window.on_close_window({
                let window_weak = window.as_weak();
                move || {
                    if let Some(window) = window_weak.upgrade() {
                        let _ = window.hide();
                    }
                }
            });

            window.on_save_schedule({
                let config = config.clone();
                let window_weak = window.as_weak();
                let main_weak = main_weak.clone();
                let manager = manager.clone();
                let runtime = runtime.clone();
                let selected_category = selected_category.clone();
                let search_query = search_query.clone();
                let sort_mode = sort_mode.clone();
                move || {
                    let Some(window) = window_weak.upgrade() else {
                        return;
                    };
                    let start_time = time_picker_to_schedule_time(
                        window.get_start_hour(),
                        window.get_start_minute(),
                        window.get_start_pm(),
                    );
                    let stop_time = if window.get_stop_enabled() {
                        time_picker_to_schedule_time(
                            window.get_stop_hour(),
                            window.get_stop_minute(),
                            window.get_stop_pm(),
                        )
                    } else {
                        String::new()
                    };
                    let schedule_active =
                        window.get_start_at_enabled() || window.get_stop_enabled();
                    if let Ok(mut scheduler) = config.lock() {
                        scheduler.enabled = schedule_active;
                        scheduler.start_at_enabled = window.get_start_at_enabled();
                        scheduler.start_time = start_time.clone();
                        scheduler.stop_enabled = window.get_stop_enabled();
                        scheduler.stop_time = stop_time.clone();
                        scheduler.last_start_key.clear();
                        scheduler.last_stop_key.clear();
                    }

                    window.set_schedule_enabled(schedule_active);
                    apply_scheduler_time_to_window(&window, &start_time, &stop_time);
                    let message = if schedule_active {
                        format!(
                            "Schedule applied.{}{}",
                            if window.get_start_at_enabled() {
                                format!(" Starts at {}.", start_time)
                            } else {
                                String::new()
                            },
                            if stop_time.is_empty() {
                                String::new()
                            } else {
                                format!(" Stops at {}.", stop_time)
                            }
                        )
                    } else {
                        "Schedule disabled. Enable start or stop time to use it.".to_string()
                    };
                    window.set_message(message.into());

                    set_status(
                        main_weak.clone(),
                        if schedule_active {
                            "Scheduler applied."
                        } else {
                            "Scheduler disabled."
                        },
                    );
                    refresh_download_rows(
                        main_weak.clone(),
                        manager.clone(),
                        runtime.clone(),
                        selected_category.clone(),
                        search_query.clone(),
                        sort_mode.clone(),
                    );
                }
            });

            *scheduler_window = Some(window);
        }

        if let Some(window) = scheduler_window.as_ref() {
            window.set_schedule_enabled(snapshot.enabled);
            window.set_start_at_enabled(snapshot.start_at_enabled);
            window.set_stop_enabled(snapshot.stop_enabled);
            window.set_queued_downloads(ModelRc::new(VecModel::from(queued_rows)));
            apply_scheduler_time_to_window(window, &snapshot.start_time, &snapshot.stop_time);
            window.set_message(if snapshot.enabled {
                "Scheduler is enabled.".into()
            } else {
                "Choose times, enable the scheduler, then Save.".into()
            });
            center_scheduler_window(&main_weak, window);
            let _ = window.show();
            #[cfg(target_os = "windows")]
            set_slint_window_icons(&window.window());
        }
    });
}

fn scheduler_queued_rows(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Vec<DownloadRow> {
    let tasks = runtime.block_on(manager.get_all_downloads());
    tasks
        .into_iter()
        .filter(|task| task.status == DownloadStatus::Queued && task.scheduled_queue)
        .map(|task| task_to_row(&task))
        .enumerate()
        .map(|(index, row)| row_view_to_download_row(row, index, false))
        .collect()
}

fn apply_scheduler_time_to_window(
    window: &NativeSchedulerWindow,
    start_time: &str,
    stop_time: &str,
) {
    let (start_hour, start_minute, start_pm) =
        schedule_time_to_picker(start_time).unwrap_or((8, 0, false));
    let stop = schedule_time_to_picker(stop_time);
    let (stop_hour, stop_minute, stop_pm) = stop.unwrap_or((11, 0, true));

    window.set_start_hour(start_hour as i32);
    window.set_start_minute(start_minute as i32);
    window.set_start_pm(start_pm);
    window.set_stop_enabled(stop.is_some());
    window.set_stop_hour(stop_hour as i32);
    window.set_stop_minute(stop_minute as i32);
    window.set_stop_pm(stop_pm);
}

fn center_scheduler_window(main_weak: &slint::Weak<NativeMain>, window: &NativeSchedulerWindow) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_window = main.window();
    let scheduler_window = window.window();
    let main_pos = main_window.position();
    let main_size = main_window.size();
    let scheduler_size = scheduler_window.size();

    let scheduler_width = if scheduler_size.width > 0 {
        scheduler_size.width as i32
    } else {
        430
    };
    let scheduler_height = if scheduler_size.height > 0 {
        scheduler_size.height as i32
    } else {
        260
    };

    let x = main_pos.x + ((main_size.width as i32 - scheduler_width) / 2).max(0);
    let y = main_pos.y + ((main_size.height as i32 - scheduler_height) / 2).max(0);
    scheduler_window.set_position(PhysicalPosition::new(x, y));
}

fn scheduler_config() -> Arc<Mutex<SchedulerConfig>> {
    SCHEDULER_CONFIG
        .get_or_init(|| Arc::new(Mutex::new(SchedulerConfig::default())))
        .clone()
}

fn time_picker_to_schedule_time(hour: i32, minute: i32, pm: bool) -> String {
    let hour = hour.clamp(1, 12) as u32;
    let minute = minute.clamp(0, 59) as u32;
    let mut hour_24 = hour % 12;
    if pm {
        hour_24 += 12;
    }
    format!("{:02}:{:02}", hour_24, minute)
}

fn schedule_time_to_picker(value: &str) -> Option<(u32, u32, bool)> {
    let (hour, minute) = parse_schedule_time(value)?;
    let pm = hour >= 12;
    let hour_12 = match hour % 12 {
        0 => 12,
        value => value,
    };
    Some((hour_12, minute, pm))
}

fn parse_schedule_time(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour = hour.trim().parse::<u32>().ok()?;
    let minute = minute.trim().parse::<u32>().ok()?;
    if hour < 24 && minute < 60 {
        Some((hour, minute))
    } else {
        None
    }
}

fn start_scheduler_loop(
    config: Arc<Mutex<SchedulerConfig>>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let now = Local::now();
            let today = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
            let current_time = format!("{:02}:{:02}", now.hour(), now.minute());

            let action = {
                let Ok(mut scheduler) = config.lock() else {
                    continue;
                };
                if !scheduler.enabled {
                    None
                } else if scheduler.start_at_enabled
                    && scheduler.start_time == current_time
                    && scheduler.last_start_key != format!("{} {}", today, current_time)
                {
                    scheduler.last_start_key = format!("{} {}", today, current_time);
                    Some("start")
                } else if scheduler.stop_enabled
                    && !scheduler.stop_time.is_empty()
                    && scheduler.stop_time == current_time
                    && scheduler.last_stop_key != format!("{} {}", today, current_time)
                {
                    scheduler.last_stop_key = format!("{} {}", today, current_time);
                    Some("stop")
                } else {
                    None
                }
            };

            match action {
                Some("start") => {
                    let count = scheduler_start_queue(manager.clone()).await;
                    set_status(
                        main_weak.clone(),
                        format!("Scheduler started {} download(s).", count),
                    );
                    refresh_download_rows(
                        main_weak.clone(),
                        manager.clone(),
                        runtime_for_refresh.clone(),
                        selected_category.clone(),
                        search_query.clone(),
                        sort_mode.clone(),
                    );
                }
                Some("stop") => {
                    let count = scheduler_stop_queue(manager.clone()).await;
                    set_status(
                        main_weak.clone(),
                        format!("Scheduler stopped {} download(s).", count),
                    );
                    refresh_download_rows(
                        main_weak.clone(),
                        manager.clone(),
                        runtime_for_refresh.clone(),
                        selected_category.clone(),
                        search_query.clone(),
                        sort_mode.clone(),
                    );
                }
                _ => {}
            }
        }
    });
}

async fn scheduler_start_queue(manager: Arc<DownloadManager>) -> usize {
    let mut immediate = Vec::new();
    let mut sequential_groups: HashMap<String, Vec<(usize, String)>> = HashMap::new();

    for task in manager
        .get_all_downloads()
        .await
        .into_iter()
        .filter(|task| task.status == DownloadStatus::Queued && task.scheduled_queue)
    {
        if task.batch_sequential {
            let group_id = task
                .batch_group_id
                .clone()
                .unwrap_or_else(|| task.id.clone());
            sequential_groups
                .entry(group_id)
                .or_default()
                .push((task.batch_queue_index, task.id));
        } else {
            immediate.push(task.id);
        }
    }

    let mut started = 0usize;
    for id in &immediate {
        let _ = manager.resume_download(id, None).await;
        started += 1;
    }

    for (_group_id, mut indexed_ids) in sequential_groups {
        indexed_ids.sort_by_key(|(index, _)| *index);
        let ids = indexed_ids
            .into_iter()
            .map(|(_, id)| id)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            continue;
        }
        started += 1;
        let manager_for_group = manager.clone();
        tokio::spawn(async move {
            run_scheduled_sequential_group(manager_for_group, ids).await;
        });
    }

    started
}

async fn run_scheduled_sequential_group(manager: Arc<DownloadManager>, ids: Vec<String>) {
    for id in ids {
        if manager.resume_download(&id, None).await.is_err() {
            continue;
        }

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let Some(task) = manager.get_download(&id).await else {
                break;
            };

            match task.status {
                DownloadStatus::Completed | DownloadStatus::Failed => break,
                DownloadStatus::Paused => return,
                DownloadStatus::Queued
                | DownloadStatus::Downloading
                | DownloadStatus::Assembling => {}
            }
        }
    }
}

async fn scheduler_stop_queue(manager: Arc<DownloadManager>) -> usize {
    let active: Vec<String> = manager
        .get_all_downloads()
        .await
        .into_iter()
        .filter(|task| {
            task.scheduled_queue
                && matches!(
                    task.status,
                    DownloadStatus::Downloading
                        | DownloadStatus::Queued
                        | DownloadStatus::Assembling
                )
        })
        .map(|task| task.id)
        .collect();
    for id in &active {
        let _ = manager.pause_download(id).await;
    }
    active.len()
}

fn toggle_speed_limiter(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
) {
    open_global_speed_limiter_window(manager, runtime, main_weak);
}

fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn open_file_native(path: &str) {
    let Some(path) = normalize_existing_path(path) else {
        return;
    };
    if !path.exists() {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        let file = wide_menu_text(&path.to_string_lossy());
        let verb = wide_menu_text("open");
        let directory = path
            .parent()
            .map(|parent| wide_menu_text(&parent.to_string_lossy()));
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                directory
                    .as_ref()
                    .map(|value| value.as_ptr())
                    .unwrap_or(std::ptr::null()),
                SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn open_with_native(path: &str) {
    let Some(path) = normalize_existing_path(path) else {
        return;
    };
    if !path.exists() {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        let operation = wide_menu_text("openas");
        let file = wide_menu_text(&path.to_string_lossy());
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        open_file_native(&path.to_string_lossy());
    }
}

fn open_browser_extensions_page(browser: &str) -> Result<(), String> {
    let url = match browser {
        "edge" => "edge://extensions",
        "brave" => "brave://extensions",
        _ => "chrome://extensions",
    };

    #[cfg(target_os = "windows")]
    {
        for candidate in browser_launch_candidates(browser) {
            if Command::new(&candidate).arg(url).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(format!(
            "{} was not found on this PC",
            browser_display_name(browser)
        ))
    }

    #[cfg(not(target_os = "windows"))]
    {
        open_url(url);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn browser_launch_candidates(browser: &str) -> Vec<PathBuf> {
    let exe_name = match browser {
        "edge" => "msedge.exe",
        "brave" => "brave.exe",
        _ => "chrome.exe",
    };
    let vendor_path = match browser {
        "edge" => ["Microsoft", "Edge", "Application", exe_name],
        "brave" => ["BraveSoftware", "Brave-Browser", "Application", exe_name],
        _ => ["Google", "Chrome", "Application", exe_name],
    };

    let mut candidates = vec![PathBuf::from(exe_name)];
    for env_name in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(base) = std::env::var_os(env_name) {
            let mut path = PathBuf::from(base);
            for part in vendor_path {
                path.push(part);
            }
            candidates.push(path);
        }
    }
    candidates
}

fn browser_display_name(browser: &str) -> &'static str {
    match browser {
        "edge" => "Microsoft Edge",
        "brave" => "Brave",
        _ => "Chrome",
    }
}

fn open_folder_native(path: &str) {
    let Some(folder) = normalize_path_for_folder_action(path) else {
        return;
    };

    #[cfg(target_os = "windows")]
    {
        let file = wide_menu_text(&folder.to_string_lossy());
        let verb = wide_menu_text("open");
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(&folder)
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&folder).spawn();
    }
}

fn normalize_existing_path(path: &str) -> Option<PathBuf> {
    let path = path.trim().trim_matches('"');
    if path.is_empty() {
        return None;
    }

    let candidate = PathBuf::from(path);
    if candidate.exists() {
        return fs::canonicalize(&candidate).ok().or(Some(candidate));
    }

    if candidate.is_absolute() {
        return Some(candidate);
    }

    std::env::current_dir().ok().map(|dir| dir.join(candidate))
}

fn normalize_path_for_folder_action(path: &str) -> Option<PathBuf> {
    let target = normalize_existing_path(path)?;
    if target.is_dir() {
        return Some(target);
    }

    target
        .parent()
        .map(Path::to_path_buf)
        .or_else(|| Some(target))
}

fn start_refresh_loop(
    weak: slint::Weak<NativeMain>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
    refresh_notify: Arc<tokio::sync::Notify>,
) {
    runtime.spawn(async move {
        loop {
            let (rows, details, has_active_downloads) =
                download_snapshot(&manager, &selected_category, &search_query, &sort_mode).await;
            publish_rows(weak.clone(), rows);
            publish_download_window_updates(details);

            if has_active_downloads {
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            } else {
                refresh_notify.notified().await;
            }
        }
    });
}

fn refresh_download_rows(
    weak: slint::Weak<NativeMain>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    runtime.spawn(async move {
        let (rows, details, _) =
            download_snapshot(&manager, &selected_category, &search_query, &sort_mode).await;
        publish_rows(weak, rows);
        publish_download_window_updates(details);
    });
}

async fn download_snapshot(
    manager: &DownloadManager,
    selected_category: &Arc<Mutex<String>>,
    search_query: &Arc<Mutex<String>>,
    sort_mode: &Arc<Mutex<String>>,
) -> (Vec<RowView>, Vec<DownloadDetailView>, bool) {
    let selected = selected_category
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "All Downloads".to_string());
    let query = search_query
        .lock()
        .map(|value| value.trim().to_lowercase())
        .unwrap_or_default();
    let sort = sort_mode
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "date".to_string());

    let mut tasks = manager.get_all_downloads().await;
    sort_tasks(&mut tasks, &sort);
    let has_active_downloads = tasks.iter().any(is_active_download);
    let rows = tasks
        .iter()
        .filter(|task| category_matches(task, &selected))
        .filter(|task| search_matches(task, &query))
        .map(task_to_row)
        .collect();
    let details = tasks.iter().map(task_to_detail).collect();

    (rows, details, has_active_downloads)
}

fn is_active_download(task: &DownloadTask) -> bool {
    matches!(
        task.status,
        DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Assembling
    )
}

fn wake_refresh_loop() {
    if let Some(refresh_notify) = REFRESH_NOTIFY.get() {
        refresh_notify.notify_waiters();
    }
}

fn pulse_refresh_loop(runtime: Arc<tokio::runtime::Runtime>) {
    wake_refresh_loop();
    runtime.spawn(async {
        for _ in 0..8 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            wake_refresh_loop();
        }
    });
}

fn publish_download_window_updates(details: Vec<DownloadDetailView>) {
    let _ = slint::invoke_from_event_loop(move || {
        for detail in &details {
            maybe_show_download_complete_window(detail);
        }

        DOWNLOAD_WINDOWS.with(|windows| {
            let windows = windows.borrow();
            for detail in details {
                if let Some(window) = windows.get(&detail.id) {
                    let changed = LAST_PUBLISHED_DETAILS.with(|last| {
                        let mut last = last.borrow_mut();
                        if last.get(&detail.id) == Some(&detail) {
                            false
                        } else {
                            last.insert(detail.id.clone(), detail.clone());
                            true
                        }
                    });

                    if !changed {
                        continue;
                    }

                    apply_download_detail(window, &detail);
                    handle_completion_options(window, &detail);
                }
            }
        });
    });
}

fn maybe_show_download_complete_window(detail: &DownloadDetailView) {
    let disabled = COMPLETION_DIALOGS_DISABLED.with(|disabled| *disabled.borrow());
    if disabled {
        return;
    }

    let should_show = LAST_COMPLETION_STATES.with(|states| {
        let mut states = states.borrow_mut();
        let previous = states.insert(detail.id.clone(), detail.status.clone());
        if detail.status != "Complete" {
            COMPLETION_DIALOG_ELIGIBLE.with(|eligible| {
                eligible.borrow_mut().insert(detail.id.clone());
            });
            return false;
        }

        let eligible =
            COMPLETION_DIALOG_ELIGIBLE.with(|eligible| eligible.borrow_mut().remove(&detail.id));
        eligible || (previous.is_some() && previous.as_deref() != Some("Complete"))
    });

    if should_show {
        show_download_complete_window(detail.clone());
    }
}

fn mark_completion_dialog_eligible(id: &str) {
    let id = id.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        COMPLETION_DIALOG_ELIGIBLE.with(|eligible| {
            eligible.borrow_mut().insert(id);
        });
    });
}

fn show_download_complete_window(detail: DownloadDetailView) {
    DOWNLOAD_COMPLETE_WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        if !windows.contains_key(&detail.id) {
            let Ok(window) = DownloadCompleteWindow::new() else {
                return;
            };
            #[cfg(target_os = "windows")]
            set_slint_window_icons(&window.window());

            window.on_open_file({
                let id = detail.id.clone();
                move |path| {
                    open_file_native(&path);
                    hide_download_complete_window(&id);
                }
            });
            window.on_open_with({
                let id = detail.id.clone();
                move |path| {
                    open_with_native(&path);
                    hide_download_complete_window(&id);
                }
            });
            window.on_open_folder({
                let id = detail.id.clone();
                move |path| {
                    open_folder_native(&path);
                    hide_download_complete_window(&id);
                }
            });
            window.on_close_window({
                let id = detail.id.clone();
                move |dont_show_again| {
                    if dont_show_again {
                        COMPLETION_DIALOGS_DISABLED.with(|disabled| {
                            *disabled.borrow_mut() = true;
                        });
                        let mut settings = crate::state::StateManager::load_settings();
                        settings.show_download_complete_dialog = false;
                        if let Err(error) = crate::state::StateManager::save_settings(&settings) {
                            log::warn!("Could not save completion dialog preference: {}", error);
                        }
                    }
                    hide_download_complete_window(&id);
                }
            });

            windows.insert(detail.id.clone(), window);
        }

        DOWNLOAD_WINDOWS.with(|download_windows| {
            if let Some(progress_window) = download_windows.borrow().get(&detail.id) {
                let _ = progress_window.hide();
            }
        });

        if let Some(window) = windows.get(&detail.id) {
            window.set_filename(detail.filename.clone().into());
            window.set_url(detail.url.clone().into());
            window.set_save_path(detail.save_path.clone().into());
            window.set_downloaded(detail.downloaded.clone().into());
            window.set_dont_show_again(false);
            let _ = window.show();
            #[cfg(target_os = "windows")]
            {
                set_slint_window_icons(&window.window());
                force_window_to_front_later(window.as_weak());
            }
        }
    });
}

fn hide_download_complete_window(id: &str) {
    DOWNLOAD_COMPLETE_WINDOWS.with(|windows| {
        if let Some(window) = windows.borrow().get(id) {
            let _ = window.hide();
        }
    });
}

fn open_download_status_for_id(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_window = runtime.clone();
    runtime.spawn(async move {
        match manager_for_task.get_download(&id).await {
            Some(task) => show_download_status_window(
                task_to_detail(&task),
                manager,
                runtime_for_window,
                main_weak,
                selected_category,
                search_query,
                sort_mode,
            ),
            None => set_status(main_weak, "Download not found."),
        }
    });
}

fn open_file_properties_for_id(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    runtime.spawn(async move {
        match manager.get_download(&id).await {
            Some(task) => show_file_properties_window(task_to_properties(&task), main_weak),
            None => set_status(main_weak, "Download not found."),
        }
    });
}

fn show_file_properties_window(view: FilePropertiesView, main_weak: slint::Weak<NativeMain>) {
    let _ = slint::invoke_from_event_loop(move || {
        FILE_PROPERTIES_WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            if !windows.contains_key(&view.id) {
                let Ok(window) = FilePropertiesWindow::new() else {
                    set_status(main_weak.clone(), "Could not open file properties.");
                    return;
                };
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());

                window.on_open_file(|path| {
                    open_file_native(&path);
                });

                window.on_copy_address({
                    let main_weak = main_weak.clone();
                    move |url| match arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(url.to_string()))
                    {
                        Ok(()) => set_status(main_weak.clone(), "Download address copied."),
                        Err(error) => set_status(
                            main_weak.clone(),
                            format!("Could not copy address: {}", error),
                        ),
                    }
                });

                let id_for_close = view.id.clone();
                window.on_close_window(move || {
                    FILE_PROPERTIES_WINDOWS.with(|windows| {
                        if let Some(window) = windows.borrow().get(&id_for_close) {
                            let _ = window.hide();
                        }
                    });
                });

                windows.insert(view.id.clone(), window);
            }

            if let Some(window) = windows.get(&view.id) {
                apply_file_properties(window, &view);
                center_file_properties(&main_weak, window);
                let _ = window.show();
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());
            }
        });
    });
}

fn apply_file_properties(window: &FilePropertiesWindow, view: &FilePropertiesView) {
    window.set_download_id(view.id.clone().into());
    window.set_prop_filename(view.filename.clone().into());
    window.set_prop_file_type(view.file_type.clone().into());
    window.set_prop_status(view.status.clone().into());
    window.set_prop_size(view.size.clone().into());
    window.set_prop_save_path(view.save_path.clone().into());
    window.set_prop_url(view.url.clone().into());
    window.set_prop_last_date(view.last_date.clone().into());
    window.set_prop_result(view.result.clone().into());
    window.set_prop_can_open(view.can_open);
}

fn center_file_properties(main_weak: &slint::Weak<NativeMain>, window: &FilePropertiesWindow) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_window = main.window();
    let props_window = window.window();
    let main_pos = main_window.position();
    let main_size = main_window.size();
    let props_size = props_window.size();

    let props_width = if props_size.width > 0 {
        props_size.width as i32
    } else {
        (480.0 * props_window.scale_factor()) as i32
    };
    let props_height = if props_size.height > 0 {
        props_size.height as i32
    } else {
        (390.0 * props_window.scale_factor()) as i32
    };

    let x = main_pos.x + ((main_size.width as i32 - props_width) / 2).max(0);
    let y = main_pos.y + ((main_size.height as i32 - props_height) / 2).max(0);
    props_window.set_position(PhysicalPosition::new(x, y));
}

fn show_download_status_window(
    detail: DownloadDetailView,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let _ = slint::invoke_from_event_loop(move || {
        DOWNLOAD_WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            if !windows.contains_key(&detail.id) {
                let Ok(window) = DownloadStatusWindow::new() else {
                    set_status(main_weak.clone(), "Could not open download status window.");
                    return;
                };
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());

                window.on_pause_or_resume({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let selected_category = selected_category.clone();
                    let search_query = search_query.clone();
                    let sort_mode = sort_mode.clone();
                    move |id| {
                        pause_or_resume_download(
                            id.to_string(),
                            manager.clone(),
                            runtime.clone(),
                            main_weak.clone(),
                            selected_category.clone(),
                            search_query.clone(),
                            sort_mode.clone(),
                        );
                    }
                });

                window.on_cancel_download({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let selected_category = selected_category.clone();
                    let search_query = search_query.clone();
                    let sort_mode = sort_mode.clone();
                    move |id| {
                        cancel_download_from_window(
                            id.to_string(),
                            manager.clone(),
                            runtime.clone(),
                            main_weak.clone(),
                            selected_category.clone(),
                            search_query.clone(),
                            sort_mode.clone(),
                        );
                    }
                });

                window.on_close_window(|id| {
                    let id = id.to_string();
                    DOWNLOAD_WINDOWS.with(|windows| {
                        if let Some(window) = windows.borrow().get(&id) {
                            let _ = window.hide();
                        }
                    });
                });

                window.on_open_folder(|path| {
                    open_folder_native(&path);
                });

                window.on_set_speed_limit({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let window_weak = window.as_weak();
                    move |id, value| {
                        set_speed_limit_from_window(
                            id.to_string(),
                            value.to_string(),
                            manager.clone(),
                            runtime.clone(),
                            window_weak.clone(),
                        );
                    }
                });

                windows.insert(detail.id.clone(), window);
            }

            if let Some(window) = windows.get(&detail.id) {
                apply_download_detail(window, &detail);
                handle_completion_options(window, &detail);
                let _ = window.show();
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());
            }
        });
    });
}

fn pause_or_resume_download(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        set_status(main_weak, "Select a download first.");
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let result = match manager_for_task.get_download(&id).await {
            Some(task)
                if matches!(
                    task.status,
                    DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Queued
                ) =>
            {
                manager_for_task.resume_download(&id, None).await
            }
            Some(task) if task.status == DownloadStatus::Completed => {
                set_status(main_weak.clone(), "Download already complete.");
                Ok(())
            }
            Some(_) => manager_for_task.pause_download(&id).await,
            None => Err("Download not found".to_string()),
        };

        match result {
            Ok(()) => pulse_refresh_loop(runtime_for_refresh.clone()),
            Err(error) => {
                set_status(
                    main_weak.clone(),
                    format!("Download action failed: {}", error),
                );
            }
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn cancel_download_from_window(
    id: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    if id.is_empty() {
        return;
    }

    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let result = manager_for_task.pause_download(&id).await;
        if let Err(error) = result {
            set_status(main_weak.clone(), format!("Cancel failed: {}", error));
        } else {
            clear_selected_download(main_weak.clone());
            let id_for_hide = id.clone();
            let _ = slint::invoke_from_event_loop(move || {
                DOWNLOAD_WINDOWS.with(|windows| {
                    if let Some(window) = windows.borrow().get(&id_for_hide) {
                        let _ = window.hide();
                    }
                });
            });
            set_status(main_weak.clone(), "Download cancelled and kept in history.");
        }

        refresh_download_rows(
            main_weak,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn set_speed_limit_from_window(
    id: String,
    value: String,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    window_weak: slint::Weak<DownloadStatusWindow>,
) {
    let limit = match parse_speed_limit_kib(&value) {
        Ok(limit) => limit,
        Err(error) => {
            set_download_window_message(window_weak, error);
            return;
        }
    };

    runtime.spawn(async move {
        let result = manager.set_task_speed_limit(&id, limit).await;
        let message = match result {
            Ok(()) => match limit {
                Some(limit_bps) => format!("Limited to {} KB/s.", limit_bps / 1024),
                None => "Speed limit removed.".to_string(),
            },
            Err(error) => format!("Could not update speed limit: {}", error),
        };
        set_download_window_message(window_weak, message);
    });
}

fn parse_speed_limit_kib(value: &str) -> Result<Option<u64>, String> {
    let value = value.trim();
    if value.is_empty() || value == "0" {
        return Ok(None);
    }

    let kib = value
        .parse::<u64>()
        .map_err(|_| "Enter a number in KB/s.".to_string())?;
    Ok(Some(kib.saturating_mul(1024)))
}

fn set_download_window_message(
    window_weak: slint::Weak<DownloadStatusWindow>,
    message: impl Into<String>,
) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window_weak.upgrade() {
            window.set_speed_limit_message(message.into());
        }
    });
}

fn sync_extension_files_on_startup() {
    if let Err(error) = sync_extension_files() {
        log::warn!("Could not update local extension files: {}", error);
    }
}

fn sync_extension_files() -> Result<PathBuf, String> {
    let Some(source) = find_extension_source_dir() else {
        return Err("Extension source folder was not found.".to_string());
    };
    let dest = extension_dest_dir()
        .ok_or_else(|| "Could not locate local app data folder.".to_string())?;

    replace_directory(&source, &dest).map_err(|error| {
        format!(
            "Could not copy extension files from {} to {}: {}",
            source.display(),
            dest.display(),
            error
        )
    })?;
    log::info!("Browser extension files synced to {}", dest.display());
    Ok(dest)
}

fn extension_dest_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("VelocityDownloader").join("extension"))
}

fn find_extension_source_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("extension"));
            candidates.push(exe_dir.join("resources").join("extension"));
            candidates.push(exe_dir.join("..").join("extension"));
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("extension"));
        candidates.push(current_dir.join("..").join("extension"));
        candidates.push(current_dir.join("..").join("..").join("extension"));
    }

    candidates
        .into_iter()
        .find(|path| path.join("manifest.json").is_file())
}

fn replace_directory(source: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(dest).map_err(|error| error.to_string())?;
    copy_dir_recursive(source, dest)
}

fn copy_dir_recursive(source: &Path, dest: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir_all(&dest_path).map_err(|error| error.to_string())?;
            copy_dir_recursive(&source_path, &dest_path)?;
        } else {
            fs::copy(&source_path, &dest_path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn start_extension_api(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let state = ExtensionApiState {
        manager,
        runtime: runtime.clone(),
        main_weak,
        selected_category,
        search_query,
        sort_mode,
    };

    runtime.spawn(async move {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers(Any);
        let app = Router::new()
            .route("/ping", axum::routing::get(|| async { "pong" }))
            .route("/add_download", post(handle_extension_add_download))
            .layer(cors)
            .with_state(state);

        match tokio::net::TcpListener::bind("127.0.0.1:41420").await {
            Ok(listener) => {
                log::info!("Extension API listening on 127.0.0.1:41420");
                if let Err(error) = axum::serve(listener, app).await {
                    log::error!("Extension API stopped: {}", error);
                }
            }
            Err(error) => {
                log::warn!(
                    "Could not start extension API on 127.0.0.1:41420: {}",
                    error
                );
            }
        }
    });
}

async fn handle_extension_add_download(
    State(state): State<ExtensionApiState>,
    Json(payload): Json<ExtensionDownloadRequest>,
) -> Json<ExtensionDownloadResponse> {
    let url = payload.url.trim().to_string();
    if url.is_empty() {
        return Json(ExtensionDownloadResponse {
            success: false,
            message: "URL is empty".to_string(),
        });
    }

    let context = HttpContext {
        cookies: payload.cookies,
        referer: payload.referer,
        user_agent: payload.user_agent,
    };

    let settings = state.manager.get_settings().await;
    let should_open_dialog =
        settings.show_add_dialog_for_extension_downloads || media::is_likely_media_page_url(&url);
    if !should_open_dialog {
        let manager = state.manager.clone();
        let runtime = state.runtime.clone();
        let main_weak = state.main_weak.clone();
        let selected_category = state.selected_category.clone();
        let search_query = state.search_query.clone();
        let sort_mode = state.sort_mode.clone();
        match manager
            .add_download(url.clone(), None, None, None, context, None)
            .await
        {
            Ok(task) => {
                mark_completion_dialog_eligible(&task.id);
                pulse_refresh_loop(runtime.clone());
                set_status(
                    main_weak.clone(),
                    format!("Started {} from browser.", task.filename),
                );
                refresh_download_rows(
                    main_weak,
                    manager,
                    runtime,
                    selected_category,
                    search_query,
                    sort_mode,
                );
                return Json(ExtensionDownloadResponse {
                    success: true,
                    message: "Download started in Velocity Download Manager".to_string(),
                });
            }
            Err(error) => {
                set_status(state.main_weak, "Browser download could not be started.");
                return Json(ExtensionDownloadResponse {
                    success: false,
                    message: error,
                });
            }
        }
    }

    open_add_download_window_with_initial(
        state.manager,
        state.runtime,
        state.main_weak,
        state.selected_category,
        state.search_query,
        state.sort_mode,
        Some((url, context)),
    );

    Json(ExtensionDownloadResponse {
        success: true,
        message: "Download opened in Velocity Download Manager".to_string(),
    })
}

fn open_add_download_window(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    open_add_download_window_with_initial(
        manager,
        runtime,
        main_weak,
        selected_category,
        search_query,
        sort_mode,
        None,
    );
}

fn open_add_download_window_with_initial(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
    initial: Option<(String, HttpContext)>,
) {
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            let mut add_window = add_window.borrow_mut();
            if add_window.is_none() {
                let Ok(window) = NativeAddDownloadWindow::new() else {
                    set_status(main_weak.clone(), "Could not open Add Download window.");
                    return;
                };
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());

                window.on_close_window(|| {
                    ADD_DOWNLOAD_WINDOW.with(|add_window| {
                        if let Some(window) = add_window.borrow().as_ref() {
                            let _ = window.hide();
                        }
                    });
                });

                window.on_analyze_url({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let window_weak = window.as_weak();
                    move |url| {
                        let url = url.trim().to_string();
                        if url.is_empty() {
                            return;
                        }

                        let manager = manager.clone();
                        let window_weak = window_weak.clone();
                        let context = current_add_download_context();
                        runtime.spawn(async move {
                            let result = manager
                                .analyze_download(url.clone(), context, None)
                                .await
                                .map(|analysis| analysis_to_add_view(url.clone(), analysis));
                            apply_add_analysis(window_weak, url, result);
                        });
                    }
                });

                window.on_start_download({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let selected_category = selected_category.clone();
                    let search_query = search_query.clone();
                    let sort_mode = sort_mode.clone();
                    move |url, media_format, filename, save_path, selected_size_bytes| {
                        start_download_from_add_window(
                            url.to_string(),
                            media_format.to_string(),
                            filename.to_string(),
                            save_path.to_string(),
                            selected_size_bytes.to_string(),
                            current_add_download_context(),
                            false,
                            manager.clone(),
                            runtime.clone(),
                            main_weak.clone(),
                            selected_category.clone(),
                            search_query.clone(),
                            sort_mode.clone(),
                            true,
                        );
                    }
                });

                window.on_queue_download({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let selected_category = selected_category.clone();
                    let search_query = search_query.clone();
                    let sort_mode = sort_mode.clone();
                    move |url, media_format, filename, save_path, selected_size_bytes| {
                        start_download_from_add_window(
                            url.to_string(),
                            media_format.to_string(),
                            filename.to_string(),
                            save_path.to_string(),
                            selected_size_bytes.to_string(),
                            current_add_download_context(),
                            false,
                            manager.clone(),
                            runtime.clone(),
                            main_weak.clone(),
                            selected_category.clone(),
                            search_query.clone(),
                            sort_mode.clone(),
                            false,
                        );
                    }
                });

                window.on_download_ffmpeg({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    let selected_category = selected_category.clone();
                    let search_query = search_query.clone();
                    let sort_mode = sort_mode.clone();
                    let window_weak = window.as_weak();
                    move |url, media_format, filename, save_path, selected_size_bytes| {
                        install_ffmpeg_then_start(
                            url.to_string(),
                            media_format.to_string(),
                            filename.to_string(),
                            save_path.to_string(),
                            selected_size_bytes.to_string(),
                            current_add_download_context(),
                            window_weak.clone(),
                            manager.clone(),
                            runtime.clone(),
                            main_weak.clone(),
                            selected_category.clone(),
                            search_query.clone(),
                            sort_mode.clone(),
                        );
                    }
                });

                window.on_choose_folder({
                    let window_weak = window.as_weak();
                    move |current_path, filename| {
                        let current_path = current_path.to_string();
                        let filename = filename.to_string();
                        if let Some(path) = choose_download_folder(&current_path, &filename) {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_save_path(path.into());
                            }
                        }
                    }
                });

                window.on_quality_open_changed({
                    let window_weak = window.as_weak();
                    move |open| {
                        if let Some(window) = window_weak.upgrade() {
                            let height =
                                if window.get_ffmpeg_required() || window.get_ffmpeg_installing() {
                                    if open {
                                        470
                                    } else {
                                        380
                                    }
                                } else if open {
                                    425
                                } else {
                                    310
                                };
                            set_add_window_size(&window, 590, height);
                        }
                    }
                });

                *add_window = Some(window);
            }

            if let Some(window) = add_window.as_ref() {
                reset_add_window(window);
                match initial.as_ref() {
                    Some((url, context)) => {
                        set_add_download_context(context.clone());
                        window.set_download_url(url.clone().into());
                        window.set_analyzing(true);
                        window.set_analysis_pending(false);
                        set_add_window_size(window, 590, 170);
                        let manager = manager.clone();
                        let window_weak = window.as_weak();
                        let url_for_analysis = url.clone();
                        let context = context.clone();
                        runtime.spawn(async move {
                            let result = manager
                                .analyze_download(url_for_analysis.clone(), context, None)
                                .await
                                .map(|analysis| {
                                    analysis_to_add_view(url_for_analysis.clone(), analysis)
                                });
                            apply_add_analysis(window_weak, url_for_analysis, result);
                        });
                    }
                    None => set_add_download_context(HttpContext::default()),
                }
                center_add_window(&main_weak, window);
                let _ = window.show();
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());
            }
        });
    });
}

fn set_add_download_context(context: HttpContext) {
    ADD_DOWNLOAD_CONTEXT.with(|stored| {
        *stored.borrow_mut() = context;
    });
}

fn current_add_download_context() -> HttpContext {
    ADD_DOWNLOAD_CONTEXT.with(|stored| stored.borrow().clone())
}

fn choose_download_folder(current_path: &str, filename: &str) -> Option<String> {
    let mut dialog = rfd::FileDialog::new();
    if let Some(parent) = Path::new(current_path).parent() {
        dialog = dialog.set_directory(parent);
    }

    let folder = dialog.pick_folder()?;
    let file_name = Path::new(current_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            Path::new(filename)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.trim().is_empty())
        })
        .unwrap_or("download");

    Some(folder.join(file_name).to_string_lossy().to_string())
}

fn reset_add_window(window: &NativeAddDownloadWindow) {
    set_add_window_size(window, 590, 170);
    window.set_download_url(SharedString::default());
    window.set_filename(SharedString::default());
    window.set_file_size(SharedString::default());
    window.set_save_path(SharedString::default());
    window.set_analysis_message(SharedString::default());
    window.set_analyzing(false);
    window.set_can_start(false);
    window.set_is_media(false);
    window.set_selected_quality_id(SharedString::default());
    window.set_selected_quality_label(SharedString::default());
    window.set_selected_quality_size_bytes(SharedString::default());
    window.set_quality_open(false);
    window.set_analysis_pending(false);
    window.set_ffmpeg_required(false);
    window.set_ffmpeg_installing(false);
    window.set_ffmpeg_failed(false);
    window.set_ffmpeg_progress(0.0);
    window.set_ffmpeg_message(SharedString::default());
    window.set_qualities(ModelRc::new(VecModel::from(Vec::<QualityOption>::new())));
}

fn analysis_to_add_view(_url: String, analysis: manager::DownloadAnalysis) -> AddAnalysisView {
    AddAnalysisView {
        filename: analysis.filename,
        file_size: if analysis.size > 0 {
            format_bytes(analysis.size).to_string()
        } else {
            "Unknown".to_string()
        },
        save_path: analysis.save_path,
        is_media: analysis.is_media,
        qualities: analysis
            .formats
            .into_iter()
            .map(|format| QualityOptionView {
                id: format.id,
                label: format.label,
                size: format
                    .filesize
                    .map(|size| format_bytes(size).to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                size_bytes: format
                    .filesize
                    .map(|size| size.to_string())
                    .unwrap_or_default(),
            })
            .collect(),
    }
}

fn apply_add_analysis(
    window_weak: slint::Weak<NativeAddDownloadWindow>,
    url: String,
    result: Result<AddAnalysisView, String>,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window_weak.upgrade() {
            if window.get_download_url().trim() != url {
                return;
            }

            window.set_analyzing(false);
            match result {
                Ok(view) => {
                    let expanded_height = if view.is_media && !view.qualities.is_empty() {
                        310
                    } else {
                        265
                    };
                    set_add_window_size(&window, 590, expanded_height);
                    let qualities: Vec<QualityOption> = view
                        .qualities
                        .into_iter()
                        .map(|option| QualityOption {
                            id: option.id.into(),
                            label: option.label.into(),
                            size: option.size.into(),
                            size_bytes: option.size_bytes.into(),
                        })
                        .collect();
                    let first_quality = qualities.first().cloned();

                    window.set_filename(view.filename.into());
                    window.set_file_size(view.file_size.into());
                    window.set_save_path(view.save_path.into());
                    window.set_is_media(view.is_media);
                    window.set_can_start(true);
                    window.set_analysis_message(SharedString::default());
                    window.set_qualities(ModelRc::new(VecModel::from(qualities)));

                    if let Some(option) = first_quality {
                        window.set_selected_quality_id(option.id);
                        window.set_selected_quality_label(option.label);
                        window.set_selected_quality_size_bytes(option.size_bytes);
                        window.set_file_size(option.size);
                    } else {
                        window.set_selected_quality_id(SharedString::default());
                        window.set_selected_quality_label(SharedString::default());
                        window.set_selected_quality_size_bytes(SharedString::default());
                    }
                }
                Err(error) => {
                    set_add_window_size(&window, 590, 210);
                    window.set_can_start(false);
                    window.set_is_media(false);
                    window.set_qualities(ModelRc::new(VecModel::from(Vec::<QualityOption>::new())));
                    window.set_selected_quality_id(SharedString::default());
                    window.set_selected_quality_label(SharedString::default());
                    window.set_selected_quality_size_bytes(SharedString::default());
                    window.set_analysis_message(format!("Failed to analyze URL. {}", error).into());
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn start_download_from_add_window(
    url: String,
    media_format: String,
    filename: String,
    save_path: String,
    selected_size_bytes: String,
    http_context: HttpContext,
    ffmpeg_checked: bool,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
    auto_start: bool,
) {
    let url = url.trim().to_string();
    if url.is_empty() {
        set_status(main_weak, "Enter a download URL.");
        return;
    }
    if !ffmpeg_checked
        && !media_format.trim().is_empty()
        && media::local_ffmpeg_path(None).is_none()
    {
        show_ffmpeg_required_prompt();
        set_status(main_weak, "FFmpeg is required for this video quality.");
        return;
    }

    let save_dir = Path::new(&save_path)
        .parent()
        .map(|path| path.to_string_lossy().to_string());
    let filename = (!filename.trim().is_empty()).then_some(filename);
    let media_format = (!media_format.trim().is_empty()).then_some(media_format);
    let selected_size = selected_size_bytes
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|size| *size > 0);
    log::info!(
        "Add Download start: url={}, selected_media_format={:?}, selected_size={:?}, save_path={}",
        url,
        media_format,
        selected_size,
        save_path
    );

    set_status(
        main_weak.clone(),
        if auto_start {
            "Starting download..."
        } else {
            "Adding to queue..."
        },
    );
    let manager_for_task = manager.clone();
    let runtime_for_refresh = runtime.clone();
    let manager_for_window = manager.clone();
    let runtime_for_window = runtime.clone();
    let main_weak_for_task = main_weak.clone();
    let main_weak_for_window = main_weak.clone();
    let selected_for_window = selected_category.clone();
    let search_for_window = search_query.clone();
    let sort_for_window = sort_mode.clone();

    runtime.spawn(async move {
        let result = if auto_start {
            manager_for_task
                .add_download_with_expected_size(
                    url,
                    save_dir,
                    filename,
                    media_format,
                    selected_size,
                    http_context,
                    None,
                )
                .await
        } else {
            manager_for_task
                .queue_download_with_expected_size(
                    url,
                    save_dir,
                    filename,
                    media_format,
                    selected_size,
                    http_context,
                    None,
                )
                .await
        };

        match result {
            Ok(task) => {
                if auto_start {
                    mark_completion_dialog_eligible(&task.id);
                }
                pulse_refresh_loop(runtime_for_refresh.clone());
                select_download(main_weak_for_task.clone(), &task.id);
                set_status(
                    main_weak_for_task.clone(),
                    if auto_start {
                        format!("Started {}", task.filename)
                    } else {
                        format!("Queued {}", task.filename)
                    },
                );
                hide_add_download_window();
                if auto_start {
                    show_download_status_window(
                        task_to_detail(&task),
                        manager_for_window,
                        runtime_for_window,
                        main_weak_for_window,
                        selected_for_window,
                        search_for_window,
                        sort_for_window,
                    );
                }
            }
            Err(error) => {
                set_status(
                    main_weak_for_task.clone(),
                    format!("Download failed: {}", error),
                );
                set_add_window_error(format!("Download failed. {}", error));
            }
        }

        refresh_download_rows(
            main_weak_for_task,
            manager_for_task,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

fn hide_add_download_window() {
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            if let Some(window) = add_window.borrow().as_ref() {
                let _ = window.hide();
            }
        });
    });
}

fn set_add_window_error(message: impl Into<String>) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            if let Some(window) = add_window.borrow().as_ref() {
                set_add_window_size(window, 590, 210);
                window.set_analyzing(false);
                window.set_can_start(true);
                window.set_analysis_message(message.into());
            }
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn install_ffmpeg_then_start(
    url: String,
    media_format: String,
    filename: String,
    save_path: String,
    selected_size_bytes: String,
    http_context: HttpContext,
    window_weak: slint::Weak<NativeAddDownloadWindow>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    set_ffmpeg_install_state(true, 0.0, "Downloading FFmpeg...");
    set_status(main_weak.clone(), "Downloading FFmpeg...");
    let runtime_for_start = runtime.clone();
    runtime.spawn(async move {
        let result = download_and_install_ffmpeg(window_weak).await;
        match result {
            Ok(()) => {
                clear_ffmpeg_install_prompt();
                start_download_from_add_window(
                    url,
                    media_format,
                    filename,
                    save_path,
                    selected_size_bytes,
                    http_context,
                    true,
                    manager,
                    runtime_for_start,
                    main_weak,
                    selected_category,
                    search_query,
                    sort_mode,
                    true,
                );
            }
            Err(error) => {
                set_ffmpeg_install_state(false, 0.0, format!("FFmpeg install failed. {}", error));
                set_ffmpeg_failed(true);
                set_status(main_weak, "FFmpeg install failed.");
            }
        }
    });
}

async fn download_and_install_ffmpeg(
    window_weak: slint::Weak<NativeAddDownloadWindow>,
) -> Result<(), String> {
    if media::local_ffmpeg_path(None).is_some() {
        return Ok(());
    }

    const FFMPEG_ZIP_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
    let install_dir = ffmpeg_install_dir()?;
    fs::create_dir_all(&install_dir).map_err(|error| error.to_string())?;
    let temp_dir = std::env::temp_dir()
        .join("VelocityDownloadManager")
        .join("ffmpeg");
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
    let zip_path = temp_dir.join("ffmpeg.zip");

    let engine = DownloadEngine::new_with_config(DownloadEngineConfig {
        large_file_mode: true,
    });
    let cancel_token = Arc::new(tokio::sync::Mutex::new(false));
    let speed_limit = Arc::new(tokio::sync::RwLock::new(None));
    let speed_limiter = Arc::new(SharedSpeedLimiter::new());
    let progress = {
        let window_weak = window_weak.clone();
        let downloaded_total = Arc::new(Mutex::new(0_u64));
        Arc::new(move |downloaded_delta: u64, total: u64, _speed: f64| {
            if total == 0 {
                return;
            }
            let downloaded = downloaded_total
                .lock()
                .map(|mut value| {
                    *value = value.saturating_add(downloaded_delta);
                    *value
                })
                .unwrap_or(downloaded_delta);
            let progress = ((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32;
            let _ = slint::invoke_from_event_loop({
                let window_weak = window_weak.clone();
                move || {
                    if let Some(window) = window_weak.upgrade() {
                        let current = window.get_ffmpeg_progress();
                        window.set_ffmpeg_progress(current.max(progress));
                        window.set_ffmpeg_message(
                            format!(
                                "Downloading FFmpeg... {} / {} ({:.0}%)",
                                format_bytes(downloaded).to_string(),
                                format_bytes(total).to_string(),
                                current.max(progress)
                            )
                            .into(),
                        );
                    }
                }
            });
        }) as Arc<dyn Fn(u64, u64, f64) + Send + Sync>
    };

    DownloadEngine::download_single(
        engine.client(),
        FFMPEG_ZIP_URL.to_string(),
        HttpContext::default(),
        zip_path.to_string_lossy().to_string(),
        0,
        cancel_token,
        progress,
        speed_limit,
        speed_limiter,
        true,
    )
    .await
    .map_err(|error| {
        let _ = fs::remove_file(&zip_path);
        let _ = fs::remove_file(format!("{}.part", zip_path.to_string_lossy()));
        error
    })?;

    set_ffmpeg_install_state(true, 100.0, "Extracting FFmpeg...");
    let extract_dir = temp_dir.join("extract");
    fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
    extract_zip_with_powershell(&zip_path, &extract_dir)?;
    let ffmpeg = find_file_recursive(&extract_dir, "ffmpeg.exe")
        .ok_or_else(|| "Downloaded archive did not contain ffmpeg.exe.".to_string())?;
    fs::copy(&ffmpeg, install_dir.join("ffmpeg.exe")).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&temp_dir);
    Ok(())
}

fn show_ffmpeg_required_prompt() {
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            if let Some(window) = add_window.borrow().as_ref() {
                window.set_ffmpeg_required(true);
                window.set_ffmpeg_installing(false);
                window.set_ffmpeg_failed(false);
                window.set_ffmpeg_progress(0.0);
                window.set_ffmpeg_message(
                    "FFmpeg is required for video merging. Download it now? (~60MB)".into(),
                );
                set_add_window_size(window, 590, 380);
            }
        });
    });
}

fn set_ffmpeg_install_state(installing: bool, progress: f32, message: impl Into<String>) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            if let Some(window) = add_window.borrow().as_ref() {
                window.set_ffmpeg_required(true);
                window.set_ffmpeg_installing(installing);
                if installing {
                    window.set_ffmpeg_failed(false);
                }
                window.set_ffmpeg_progress(progress);
                window.set_ffmpeg_message(message.clone().into());
                set_add_window_size(window, 590, 380);
            }
        });
    });
}

fn set_ffmpeg_failed(failed: bool) {
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            if let Some(window) = add_window.borrow().as_ref() {
                window.set_ffmpeg_failed(failed);
            }
        });
    });
}

fn clear_ffmpeg_install_prompt() {
    let _ = slint::invoke_from_event_loop(move || {
        ADD_DOWNLOAD_WINDOW.with(|add_window| {
            if let Some(window) = add_window.borrow().as_ref() {
                window.set_ffmpeg_required(false);
                window.set_ffmpeg_installing(false);
                window.set_ffmpeg_failed(false);
                window.set_ffmpeg_progress(0.0);
                window.set_ffmpeg_message(SharedString::default());
                let height = if window.get_quality_open() { 425 } else { 310 };
                set_add_window_size(window, 590, height);
            }
        });
    });
}

fn ffmpeg_install_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .ok_or_else(|| "Could not find app directory.".to_string())
}

fn extract_zip_with_powershell(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let mut tar_command = Command::new("tar");
    tar_command.arg("-xf").arg(zip_path).arg("-C").arg(dest);
    #[cfg(target_os = "windows")]
    tar_command.creation_flags(CREATE_NO_WINDOW);
    let tar_status = tar_command.status();
    if let Ok(status) = tar_status {
        if status.success() {
            return Ok(());
        }
        log::warn!("tar extraction failed with {}", status);
    }

    let mut powershell = Command::new("powershell");
    powershell
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-Command")
        .arg(format!(
            "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
            zip_path.to_string_lossy().replace('\'', "''"),
            dest.to_string_lossy().replace('\'', "''")
        ));
    #[cfg(target_os = "windows")]
    powershell.creation_flags(CREATE_NO_WINDOW);
    let status = powershell.status().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Archive extraction failed with {}", status))
    }
}

fn find_file_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, file_name) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(file_name))
            .unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

fn set_add_window_size(window: &NativeAddDownloadWindow, width: u32, height: u32) {
    set_window_logical_size(&window.window(), width, height);
}

fn center_add_window(main_weak: &slint::Weak<NativeMain>, window: &NativeAddDownloadWindow) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_pos = main.window().position();
    let main_size = main.window().size();
    let size = window.window().size();
    let x = main_pos.x + (main_size.width as i32 - size.width as i32).max(0) / 2;
    let y = main_pos.y + (main_size.height as i32 - size.height as i32).max(0) / 2;
    window.window().set_position(PhysicalPosition::new(x, y));
}

#[allow(clippy::too_many_arguments)]
fn open_batch_download_window(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let manager_for_settings = manager.clone();
    let runtime_for_settings = runtime.clone();
    runtime.spawn(async move {
        let settings = manager_for_settings.get_settings().await;
        let default_dir = settings.default_download_dir;
        let _ = slint::invoke_from_event_loop(move || {
            BATCH_DOWNLOAD_WINDOW.with(|batch_window| {
                let mut batch_window = batch_window.borrow_mut();
                if batch_window.is_none() {
                    let Ok(window) = NativeBatchDownloadWindow::new() else {
                        set_status(main_weak.clone(), "Could not open Batch Download window.");
                        return;
                    };
                    #[cfg(target_os = "windows")]
                    set_slint_window_icons(&window.window());

                    window.on_close_window(|| {
                        BATCH_DOWNLOAD_WINDOW.with(|batch_window| {
                            if let Some(window) = batch_window.borrow().as_ref() {
                                let _ = window.hide();
                            }
                        });
                    });

                    window.on_choose_folder({
                        let window_weak = window.as_weak();
                        move |current_dir| {
                            let mut dialog = rfd::FileDialog::new();
                            let current_dir = current_dir.to_string();
                            if !current_dir.trim().is_empty() {
                                dialog = dialog.set_directory(current_dir);
                            }
                            if let Some(folder) = dialog.pick_folder() {
                                if let Some(window) = window_weak.upgrade() {
                                    window
                                        .set_save_dir(folder.to_string_lossy().to_string().into());
                                }
                            }
                        }
                    });

                    window.on_import_list({
                        let window_weak = window.as_weak();
                        move || {
                            let Some(path) = rfd::FileDialog::new()
                                .add_filter("Text files", &["txt", "csv"])
                                .pick_file()
                            else {
                                return;
                            };
                            let Ok(text) = std::fs::read_to_string(path) else {
                                if let Some(window) = window_weak.upgrade() {
                                    window
                                        .set_batch_message("Could not read selected file.".into());
                                }
                                return;
                            };
                            if let Some(window) = window_weak.upgrade() {
                                let current = window.get_urls_text().to_string();
                                let next = if current.trim().is_empty() {
                                    text
                                } else {
                                    format!("{}\n{}", current.trim_end(), text)
                                };
                                window.set_urls_text(next.into());
                            }
                        }
                    });

                    window.on_start_batch({
                        let manager = manager.clone();
                        let runtime = runtime_for_settings.clone();
                        let main_weak = main_weak.clone();
                        let selected_category = selected_category.clone();
                        let search_query = search_query.clone();
                        let sort_mode = sort_mode.clone();
                        let window_weak = window.as_weak();
                        move |urls, save_dir, extension_filter, sequential| {
                            start_batch_download(
                                urls.to_string(),
                                save_dir.to_string(),
                                extension_filter.to_string(),
                                sequential,
                                window_weak.clone(),
                                manager.clone(),
                                runtime.clone(),
                                main_weak.clone(),
                                selected_category.clone(),
                                search_query.clone(),
                                sort_mode.clone(),
                            );
                        }
                    });

                    window.on_queue_batch({
                        let manager = manager.clone();
                        let runtime = runtime_for_settings.clone();
                        let main_weak = main_weak.clone();
                        let selected_category = selected_category.clone();
                        let search_query = search_query.clone();
                        let sort_mode = sort_mode.clone();
                        let window_weak = window.as_weak();
                        move |urls, save_dir, extension_filter, sequential| {
                            queue_batch_download(
                                urls.to_string(),
                                save_dir.to_string(),
                                extension_filter.to_string(),
                                sequential,
                                window_weak.clone(),
                                manager.clone(),
                                runtime.clone(),
                                main_weak.clone(),
                                selected_category.clone(),
                                search_query.clone(),
                                sort_mode.clone(),
                            );
                        }
                    });

                    *batch_window = Some(window);
                }

                if let Some(window) = batch_window.as_ref() {
                    reset_batch_window(window, &default_dir);
                    center_batch_window(&main_weak, window);
                    let _ = window.show();
                    #[cfg(target_os = "windows")]
                    set_slint_window_icons(&window.window());
                }
            });
        });
    });
}

fn reset_batch_window(window: &NativeBatchDownloadWindow, default_dir: &str) {
    window.set_urls_text(SharedString::default());
    window.set_save_dir(default_dir.into());
    window.set_extension_filter(SharedString::default());
    window.set_sequential_mode(false);
    window.set_started(false);
    window.set_batch_message(SharedString::default());
    window.set_queue_rows(ModelRc::new(VecModel::from(Vec::<BatchQueueRow>::new())));
}

fn center_batch_window(main_weak: &slint::Weak<NativeMain>, window: &NativeBatchDownloadWindow) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_pos = main.window().position();
    let main_size = main.window().size();
    let size = window.window().size();
    let x = main_pos.x + (main_size.width as i32 - size.width as i32).max(0) / 2;
    let y = main_pos.y + (main_size.height as i32 - size.height as i32).max(0) / 2;
    window.window().set_position(PhysicalPosition::new(x, y));
}

#[allow(clippy::too_many_arguments)]
fn start_batch_download(
    urls_text: String,
    save_dir: String,
    extension_filter: String,
    sequential: bool,
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let urls = parse_batch_urls(&urls_text, &extension_filter);
    if urls.is_empty() {
        set_batch_message(window_weak, "No valid URLs found.");
        return;
    }

    let rows = urls
        .iter()
        .map(|url| BatchQueueView {
            url: url.clone(),
            status: "Pending".to_string(),
            progress: 0.0,
            error: String::new(),
            active: false,
        })
        .collect::<Vec<_>>();
    let rows = Arc::new(tokio::sync::Mutex::new(rows));
    publish_batch_rows(window_weak.clone(), rows.clone());
    set_batch_started(
        window_weak.clone(),
        true,
        format!("Queued {} URL(s).", urls.len()),
    );

    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        if sequential {
            for index in 0..urls.len() {
                run_batch_item(
                    index,
                    urls[index].clone(),
                    save_dir.clone(),
                    rows.clone(),
                    window_weak.clone(),
                    manager.clone(),
                )
                .await;
            }
        } else {
            let mut handles = Vec::new();
            for (index, url) in urls.into_iter().enumerate() {
                let manager = manager.clone();
                let rows = rows.clone();
                let window_weak = window_weak.clone();
                let save_dir = save_dir.clone();
                handles.push(tokio::spawn(async move {
                    run_batch_item(index, url, save_dir, rows, window_weak, manager).await;
                }));
            }
            for handle in handles {
                let _ = handle.await;
            }
        }

        set_batch_message(window_weak.clone(), "Batch started.");
        refresh_download_rows(
            main_weak,
            manager,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn queue_batch_download(
    urls_text: String,
    save_dir: String,
    extension_filter: String,
    sequential: bool,
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    selected_category: Arc<Mutex<String>>,
    search_query: Arc<Mutex<String>>,
    sort_mode: Arc<Mutex<String>>,
) {
    let urls = parse_batch_urls(&urls_text, &extension_filter);
    if urls.is_empty() {
        set_batch_message(window_weak, "No valid URLs found.");
        return;
    }

    let rows = urls
        .iter()
        .map(|url| BatchQueueView {
            url: url.clone(),
            status: "Pending".to_string(),
            progress: 0.0,
            error: String::new(),
            active: false,
        })
        .collect::<Vec<_>>();
    let rows = Arc::new(tokio::sync::Mutex::new(rows));
    publish_batch_rows(window_weak.clone(), rows.clone());
    set_batch_started(
        window_weak.clone(),
        true,
        format!(
            "Adding {} URL(s) to scheduled queue as {}.",
            urls.len(),
            if sequential {
                "one by one"
            } else {
                "all at once"
            }
        ),
    );

    let runtime_for_refresh = runtime.clone();
    runtime.spawn(async move {
        let batch_group_id = uuid::Uuid::new_v4().to_string();
        let mut queued_count = 0usize;
        for (index, url) in urls.into_iter().enumerate() {
            update_batch_row(
                rows.clone(),
                window_weak.clone(),
                index,
                "Queuing",
                0.0,
                "",
                true,
            )
            .await;

            let save_path = (!save_dir.trim().is_empty()).then_some(save_dir.clone());
            let result = manager
                .queue_batch_download_with_expected_size(
                    url,
                    save_path,
                    None,
                    None,
                    None,
                    HttpContext::default(),
                    None,
                    batch_group_id.clone(),
                    sequential,
                    index,
                )
                .await;

            match result {
                Ok(_) => {
                    queued_count += 1;
                    update_batch_row(
                        rows.clone(),
                        window_weak.clone(),
                        index,
                        "Queued",
                        0.0,
                        "",
                        false,
                    )
                    .await;
                }
                Err(error) => {
                    update_batch_row(
                        rows.clone(),
                        window_weak.clone(),
                        index,
                        "Failed",
                        0.0,
                        &error,
                        false,
                    )
                    .await;
                }
            }
        }

        set_batch_message(
            window_weak.clone(),
            format!(
                "Added {} URL(s) to the scheduler queue. They will run {}.",
                queued_count,
                if sequential {
                    "one by one"
                } else {
                    "all at once"
                }
            ),
        );
        refresh_download_rows(
            main_weak,
            manager,
            runtime_for_refresh,
            selected_category,
            search_query,
            sort_mode,
        );
        wake_refresh_loop();
    });
}

async fn run_batch_item(
    index: usize,
    url: String,
    save_dir: String,
    rows: Arc<tokio::sync::Mutex<Vec<BatchQueueView>>>,
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    manager: Arc<DownloadManager>,
) {
    update_batch_row(
        rows.clone(),
        window_weak.clone(),
        index,
        "Starting",
        0.0,
        "",
        true,
    )
    .await;

    let save_path = (!save_dir.trim().is_empty()).then_some(save_dir);
    let result = manager
        .add_download(url, save_path, None, None, HttpContext::default(), None)
        .await;

    match result {
        Ok(task) => {
            mark_completion_dialog_eligible(&task.id);
            wake_refresh_loop();
            update_batch_row(
                rows.clone(),
                window_weak.clone(),
                index,
                "Downloading",
                progress_percent(&task),
                "",
                true,
            )
            .await;
            monitor_batch_task(index, task.id, rows, window_weak, manager).await;
        }
        Err(error) => {
            update_batch_row(rows, window_weak, index, "Failed", 0.0, &error, false).await;
        }
    }
}

async fn monitor_batch_task(
    index: usize,
    task_id: String,
    rows: Arc<tokio::sync::Mutex<Vec<BatchQueueView>>>,
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    manager: Arc<DownloadManager>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let Some(task) = manager.get_download(&task_id).await else {
            update_batch_row(rows, window_weak, index, "Removed", 0.0, "", false).await;
            return;
        };

        let status = status_label(&task.status, task.total_size, task.downloaded);
        let progress = progress_percent(&task);
        let finished = matches!(
            task.status,
            DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Paused
        );
        let error = task.error.clone().unwrap_or_default();
        update_batch_row(
            rows.clone(),
            window_weak.clone(),
            index,
            &status,
            progress,
            &error,
            !finished,
        )
        .await;

        if finished {
            return;
        }
    }
}

async fn update_batch_row(
    rows: Arc<tokio::sync::Mutex<Vec<BatchQueueView>>>,
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    index: usize,
    status: &str,
    progress: f32,
    error: &str,
    active: bool,
) {
    {
        let mut rows = rows.lock().await;
        if let Some(row) = rows.get_mut(index) {
            row.status = status.to_string();
            row.progress = progress;
            row.error = error.to_string();
            row.active = active;
        }
    }
    publish_batch_rows(window_weak, rows);
}

fn publish_batch_rows(
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    rows: Arc<tokio::sync::Mutex<Vec<BatchQueueView>>>,
) {
    let Ok(rows) = rows.try_lock() else {
        return;
    };
    let rows = rows.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window_weak.upgrade() {
            let rows = rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| BatchQueueRow {
                    index: (index + 1).to_string().into(),
                    url: row.url.into(),
                    status: row.status.into(),
                    progress: row.progress,
                    error: row.error.into(),
                    active: row.active,
                })
                .collect::<Vec<_>>();
            window.set_queue_rows(ModelRc::new(VecModel::from(rows)));
        }
    });
}

fn set_batch_started(
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    started: bool,
    message: impl Into<String>,
) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window_weak.upgrade() {
            window.set_started(started);
            window.set_batch_message(message.into());
            if started {
                set_batch_window_size(&window, 620, 260);
            } else {
                set_batch_window_size(&window, 620, 540);
            }
        }
    });
}

fn set_batch_message(
    window_weak: slint::Weak<NativeBatchDownloadWindow>,
    message: impl Into<String>,
) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window_weak.upgrade() {
            window.set_batch_message(message.into());
        }
    });
}

fn parse_batch_urls(urls_text: &str, extension_filter: &str) -> Vec<String> {
    let allowed = extension_filter
        .split(',')
        .map(|ext| ext.trim().trim_start_matches('.').to_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect::<Vec<_>>();

    urls_text
        .lines()
        .map(str::trim)
        .filter(|url| {
            url.starts_with("http://") || url.starts_with("https://") || url.starts_with("ftp://")
        })
        .filter(|url| {
            if allowed.is_empty() {
                return true;
            }

            url::Url::parse(url)
                .ok()
                .and_then(|parsed| {
                    Path::new(parsed.path())
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.to_lowercase())
                })
                .map(|ext| allowed.contains(&ext))
                .unwrap_or(false)
        })
        .map(str::to_string)
        .collect()
}

fn set_batch_window_size(window: &NativeBatchDownloadWindow, width: u32, height: u32) {
    set_window_logical_size(&window.window(), width, height);
}

fn set_window_logical_size(window: &slint::Window, width: u32, height: u32) {
    let scale = window.scale_factor().max(1.0);
    let physical_width = ((width as f32) * scale).round().max(width as f32) as u32;
    let physical_height = ((height as f32) * scale).round().max(height as f32) as u32;
    window.set_size(PhysicalSize::new(physical_width, physical_height));
}

fn pending_update_store() -> Arc<Mutex<Option<PendingUpdate>>> {
    PENDING_UPDATE
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
}

fn set_pending_update(update: Option<PendingUpdate>) {
    if let Ok(mut pending) = pending_update_store().lock() {
        *pending = update;
    }
}

fn get_pending_update() -> Option<PendingUpdate> {
    pending_update_store()
        .lock()
        .ok()
        .and_then(|pending| pending.clone())
}

async fn check_for_native_update() -> Result<Option<PendingUpdate>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let manifest_text = client
        .get(UPDATE_ENDPOINT)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    let manifest: UpdateManifest =
        serde_json::from_str(&manifest_text).map_err(|error| error.to_string())?;

    if compare_versions(&manifest.version, env!("CARGO_PKG_VERSION")) <= 0 {
        return Ok(None);
    }

    let platform = select_update_platform(&manifest)
        .ok_or_else(|| "No Windows x64 installer was found in latest.json.".to_string())?;
    if platform.sha256.trim().is_empty() {
        return Err("Update manifest is missing sha256 verification.".to_string());
    }
    Ok(Some(PendingUpdate {
        version: manifest.version.clone(),
        notes: manifest.notes.clone(),
        url: platform.url.clone(),
        sha256: platform.sha256.clone(),
    }))
}

fn select_update_platform(manifest: &UpdateManifest) -> Option<&UpdatePlatform> {
    manifest
        .platforms
        .get("windows-x86_64")
        .or_else(|| manifest.platforms.get("windows-x86_64-msvc"))
        .or_else(|| {
            manifest
                .platforms
                .iter()
                .find(|(key, _)| key.contains("windows") && key.contains("x86_64"))
                .map(|(_, platform)| platform)
        })
}

fn start_update_check_on_launch(
    main_weak: slint::Weak<NativeMain>,
    runtime: Arc<tokio::runtime::Runtime>,
    can_show_prompt: bool,
) {
    runtime.spawn(async move {
        match check_for_native_update().await {
            Ok(Some(update)) => {
                let version = update.version.clone();
                let notes = update.notes.clone();
                set_pending_update(Some(update));
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = main_weak.upgrade() {
                        ui.set_update_prompt_version(version.into());
                        ui.set_update_prompt_notes(notes.into());
                        if can_show_prompt {
                            ui.set_show_update_prompt(true);
                        }
                    }
                });
            }
            Ok(None) => {
                set_pending_update(None);
            }
            Err(error) => {
                log::warn!("Startup update check failed: {}", error);
            }
        }
    });
}

async fn download_and_launch_update(update: PendingUpdate) -> Result<PathBuf, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(&update.url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let total_size = response.content_length().unwrap_or(0);
    let installer_path = std::env::temp_dir().join(format!(
        "Velocity_Downloader_{}_x64-setup.exe",
        sanitize_version_for_filename(&update.version)
    ));
    let partial_path = installer_path.with_extension("exe.part");
    let mut file = tokio::fs::File::create(&partial_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&partial_path).await;
                return Err(error.to_string());
            }
        };
        file.write_all(&chunk).await.map_err(|error| {
            let _ = std::fs::remove_file(&partial_path);
            error.to_string()
        })?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        set_update_progress(downloaded, total_size);
    }
    file.flush().await.map_err(|error| error.to_string())?;
    drop(file);

    verify_update_sha256_hex(&format!("{:x}", hasher.finalize()), &update.sha256)?;
    if tokio::fs::try_exists(&installer_path).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&installer_path).await;
    }
    tokio::fs::rename(&partial_path, &installer_path)
        .await
        .map_err(|error| {
            let _ = std::fs::remove_file(&partial_path);
            error.to_string()
        })?;
    set_update_progress(total_size, total_size);
    set_update_state("Installing update...", &update.version, &update.notes, true, true);
    schedule_silent_update_install(&installer_path)?;
    Ok(installer_path)
}

fn schedule_silent_update_install(installer_path: &Path) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let pid = std::process::id();

    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "$ErrorActionPreference='SilentlyContinue'; \
             Wait-Process -Id {pid}; \
             $installer={installer}; \
             $app={app}; \
             Start-Process -FilePath $installer -ArgumentList '/S' -Wait; \
             Remove-Item -LiteralPath $installer -Force; \
             Start-Process -FilePath $app",
            pid = pid,
            installer = powershell_single_quoted(installer_path),
            app = powershell_single_quoted(&current_exe),
        );
        let mut command = Command::new("powershell");
        command
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-Command")
            .arg(script)
            .creation_flags(CREATE_NO_WINDOW);
        command.spawn().map_err(|error| error.to_string())?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new(installer_path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn powershell_single_quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn verify_update_sha256_hex(actual: &str, expected: &str) -> Result<(), String> {
    let expected = expected
        .trim()
        .trim_start_matches("sha256:")
        .replace([' ', '\n', '\r', '\t'], "")
        .to_lowercase();
    if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("Update manifest contains an invalid sha256 value.".to_string());
    }

    if actual.eq_ignore_ascii_case(&expected) {
        Ok(())
    } else {
        Err(
            "Update verification failed. The downloaded installer hash did not match latest.json."
                .to_string(),
        )
    }
}

fn sanitize_version_for_filename(version: &str) -> String {
    version
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn cleanup_downloaded_update_installers() {
    let temp_dir = std::env::temp_dir();
    let Ok(entries) = fs::read_dir(temp_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("Velocity_Downloader_")
            && (name.ends_with("_x64-setup.exe") || name.ends_with("_x64-setup.exe.part"))
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn compare_versions(left: &str, right: &str) -> i32 {
    let left_parts = version_parts(left);
    let right_parts = version_parts(right);
    let len = left_parts.len().max(right_parts.len());
    for index in 0..len {
        let left_value = *left_parts.get(index).unwrap_or(&0);
        let right_value = *right_parts.get(index).unwrap_or(&0);
        if left_value > right_value {
            return 1;
        }
        if left_value < right_value {
            return -1;
        }
    }
    0
}

fn version_parts(version: &str) -> Vec<u32> {
    version
        .trim_start_matches('v')
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok())
        .collect()
}

fn open_settings_window(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
) {
    open_settings_window_with_tab(manager, runtime, main_weak, None);
}

fn open_settings_window_with_tab(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    active_tab: Option<String>,
) {
    let manager_for_settings = manager.clone();
    let runtime_for_settings = runtime.clone();
    runtime.spawn(async move {
        let settings = manager_for_settings.get_settings().await;
        show_settings_window(
            settings,
            manager,
            runtime_for_settings,
            main_weak,
            active_tab,
        );
    });
}

fn show_settings_window(
    settings: AppSettings,
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
    main_weak: slint::Weak<NativeMain>,
    active_tab: Option<String>,
) {
    let _ = slint::invoke_from_event_loop(move || {
        SETTINGS_WINDOW.with(|settings_window| {
            let mut settings_window = settings_window.borrow_mut();
            if settings_window.is_none() {
                let Ok(window) = NativeSettingsWindow::new() else {
                    set_status(main_weak.clone(), "Could not open settings window.");
                    return;
                };
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());

                window.on_close_settings(|| {
                    SETTINGS_WINDOW.with(|settings_window| {
                        if let Some(window) = settings_window.borrow().as_ref() {
                            let _ = window.hide();
                        }
                    });
                });

                window.on_titlebar_drag_requested({
                    let window_weak = window.as_weak();
                    move || {
                        if let Some(window) = window_weak.upgrade() {
                            window.window().with_winit_window(|winit_window| {
                                let _ = winit_window.drag_window();
                            });
                        }
                    }
                });

                window.on_choose_default_folder({
                    let window_weak = window.as_weak();
                    move || {
                        let Some(window) = window_weak.upgrade() else {
                            return;
                        };
                        let current_dir = window.get_default_dir().to_string();
                        let mut dialog = rfd::FileDialog::new();
                        if !current_dir.trim().is_empty() {
                            dialog = dialog.set_directory(current_dir);
                        }
                        if let Some(folder) = dialog.pick_folder() {
                            window.set_default_dir(folder.to_string_lossy().to_string().into());
                        }
                    }
                });

                window.on_choose_temp_folder({
                    let window_weak = window.as_weak();
                    move || {
                        let Some(window) = window_weak.upgrade() else {
                            return;
                        };
                        let current_dir = window.get_temp_dir().to_string();
                        let mut dialog = rfd::FileDialog::new();
                        if !current_dir.trim().is_empty() {
                            dialog = dialog.set_directory(current_dir);
                        }
                        if let Some(folder) = dialog.pick_folder() {
                            window.set_temp_dir(folder.to_string_lossy().to_string().into());
                        }
                    }
                });

                window.on_save_settings({
                    let manager = manager.clone();
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    move |default_dir,
                          temp_dir,
                          segments,
                          speed_limit,
                          start_on_boot,
                          show_extension_add_dialog,
                          show_download_complete_dialog| {
                        match settings_from_inputs(
                            default_dir.as_str(),
                            temp_dir.as_str(),
                            segments.as_str(),
                            speed_limit.as_str(),
                            start_on_boot,
                            show_extension_add_dialog,
                            show_download_complete_dialog,
                            crate::state::StateManager::load_settings().extension_prompt_seen,
                        ) {
                            Ok(settings) => {
                                set_settings_message("Saving...");
                                let manager = manager.clone();
                                let main_weak = main_weak.clone();
                                runtime.spawn(async move {
                                    if let Err(error) =
                                        set_windows_startup_enabled(settings.start_on_boot)
                                    {
                                        set_settings_message(format!(
                                            "Startup setting could not be updated: {}",
                                            error
                                        ));
                                        set_status(main_weak.clone(), "Startup setting failed.");
                                        return;
                                    }
                                    let completion_disabled =
                                        !settings.show_download_complete_dialog;
                                    let _ = slint::invoke_from_event_loop(move || {
                                        COMPLETION_DIALOGS_DISABLED.with(|disabled| {
                                            *disabled.borrow_mut() = completion_disabled;
                                        });
                                    });
                                    manager.update_settings(settings).await;
                                    set_settings_message("Settings saved.");
                                    set_status(main_weak, "Settings saved.");
                                });
                            }
                            Err(error) => set_settings_message(error),
                        }
                    }
                });

                window.on_check_updates({
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    move || {
                        set_update_state("Checking for updates...", "", "", false, true);
                        let main_weak = main_weak.clone();
                        runtime.spawn(async move {
                            match check_for_native_update().await {
                                Ok(Some(update)) => {
                                    let version = update.version.clone();
                                    let notes = update.notes.clone();
                                    set_pending_update(Some(update));
                                    set_update_state(
                                        "Update available",
                                        version.clone(),
                                        notes,
                                        true,
                                        false,
                                    );
                                    set_status(main_weak, format!("Velocity {} is available.", version));
                                }
                                Ok(None) => {
                                    set_pending_update(None);
                                    set_update_state(
                                        "Up to date",
                                        "",
                                        "You are running the latest version.",
                                        false,
                                        false,
                                    );
                                    set_status(main_weak, "Velocity is up to date.");
                                }
                                Err(error) => {
                                    set_pending_update(None);
                                    set_update_state(
                                        format!("Update check failed: {}", error),
                                        "",
                                        "",
                                        false,
                                        false,
                                    );
                                    set_status(main_weak, "Update check failed.");
                                }
                            }
                        });
                    }
                });

                window.on_install_update({
                    let runtime = runtime.clone();
                    let main_weak = main_weak.clone();
                    move || {
                        let update = get_pending_update();
                        let Some(update) = update else {
                            set_update_state(
                                "No update is ready to install.",
                                "",
                                "",
                                false,
                                false,
                            );
                            return;
                        };
                        set_update_state("Downloading update...", &update.version, &update.notes, true, true);
                        let main_weak = main_weak.clone();
                        runtime.spawn(async move {
                            match download_and_launch_update(update).await {
                                Ok(path) => {
                                    set_update_state(
                                        "Installing update. Velocity will restart automatically.",
                                        "",
                                        path.display().to_string(),
                                        false,
                                        false,
                                    );
                                    set_status(main_weak, "Installing update...");
                                    let _ = slint::invoke_from_event_loop(|| {
                                        let _ = slint::quit_event_loop();
                                    });
                                }
                                Err(error) => {
                                    set_update_state(
                                        format!("Install failed: {}", error),
                                        "",
                                        "",
                                        true,
                                        false,
                                    );
                                    set_status(main_weak, "Update install failed.");
                                }
                            }
                        });
                    }
                });

                window.on_open_extension_folder({
                    let main_weak = main_weak.clone();
                    move || {
                        match sync_extension_files() {
                            Ok(path) => {
                                open_file_native(&path.to_string_lossy());
                                set_settings_message("Extension folder opened. Enable Developer mode and load this folder as unpacked.");
                                set_status(main_weak.clone(), "Extension folder opened.");
                            }
                            Err(error) => {
                                set_settings_message(format!("Could not prepare extension files: {}", error));
                                set_status(main_weak.clone(), "Could not open extension folder.");
                            }
                        }
                    }
                });

                window.on_open_extension_page({
                    let main_weak = main_weak.clone();
                    move |browser| {
                        match open_browser_extensions_page(browser.as_str()) {
                            Ok(()) => {
                                set_settings_message("Extensions page opened. Turn on Developer mode, click Load unpacked, then choose the VDM extension folder.");
                                set_status(main_weak.clone(), "Browser extensions page opened.");
                            }
                            Err(error) => {
                                set_settings_message(format!("Could not open browser extensions page: {}", error));
                                set_status(main_weak.clone(), "Could not open browser extensions page.");
                            }
                        }
                    }
                });

                *settings_window = Some(window);
            }

            if let Some(window) = settings_window.as_ref() {
                apply_settings(window, &settings);
                if let Some(active_tab) = active_tab.as_ref() {
                    window.set_active_tab(active_tab.clone().into());
                }
                center_settings_window(&main_weak, window);
                let _ = window.show();
                #[cfg(target_os = "windows")]
                set_slint_window_icons(&window.window());
            }
        });
    });
}

fn center_settings_window(main_weak: &slint::Weak<NativeMain>, window: &NativeSettingsWindow) {
    let Some(main) = main_weak.upgrade() else {
        return;
    };

    let main_window = main.window();
    let settings_window = window.window();
    let main_pos = main_window.position();
    let main_size = main_window.size();
    let settings_size = settings_window.size();

    let settings_width = if settings_size.width > 0 {
        settings_size.width as i32
    } else {
        (660.0 * settings_window.scale_factor()) as i32
    };
    let settings_height = if settings_size.height > 0 {
        settings_size.height as i32
    } else {
        (390.0 * settings_window.scale_factor()) as i32
    };

    let x = main_pos.x + ((main_size.width as i32 - settings_width) / 2).max(0);
    let y = main_pos.y + ((main_size.height as i32 - settings_height) / 2).max(0);
    settings_window.set_position(PhysicalPosition::new(x, y));
}

fn apply_download_detail(window: &DownloadStatusWindow, detail: &DownloadDetailView) {
    let title = match detail.status.as_str() {
        "Complete" => format!("Download Complete - {}", detail.filename),
        "Failed" => format!("Download Failed - {}", detail.filename),
        "Paused" => format!("Download Paused - {}", detail.filename),
        _ => format!("{} - {}", detail.progress_text, detail.filename),
    };
    window.set_window_title(title.into());
    window.set_download_id(detail.id.clone().into());
    window.set_filename(detail.filename.clone().into());
    window.set_url(detail.url.clone().into());
    window.set_save_path(detail.save_path.clone().into());
    window.set_file_size(detail.file_size.clone().into());
    window.set_downloaded(detail.downloaded.clone().into());
    window.set_status(detail.status.clone().into());
    window.set_rate(detail.rate.clone().into());
    window.set_time_left(detail.time_left.clone().into());
    window.set_resume_capability(detail.resume_capability.clone().into());
    window.set_connections(detail.connections.clone().into());
    window.set_progress_text(detail.progress_text.clone().into());
    window.set_progress(detail.progress);
    if window.get_speed_limit_text().is_empty() && !detail.speed_limit_text.is_empty() {
        window.set_speed_limit_text(detail.speed_limit_text.clone().into());
    }
    let segments: Vec<SegmentRow> = detail
        .segments
        .iter()
        .map(|segment| SegmentRow {
            number: segment.number.clone().into(),
            downloaded: segment.downloaded.clone().into(),
            status: segment.status.clone().into(),
            progress: segment.progress,
        })
        .collect();
    window.set_segments(ModelRc::new(VecModel::from(segments)));
}

fn handle_completion_options(window: &DownloadStatusWindow, detail: &DownloadDetailView) {
    if detail.status != "Complete" {
        return;
    }

    let open_file = window.get_open_file_on_complete();
    let open_folder = window.get_open_folder_on_complete();
    let close_window = window.get_close_on_complete();
    if !open_file && !open_folder && !close_window {
        return;
    }

    let already_handled = COMPLETION_HANDLED.with(|handled| {
        let mut handled = handled.borrow_mut();
        if handled.contains(&detail.id) {
            true
        } else {
            handled.insert(detail.id.clone());
            false
        }
    });
    if already_handled {
        return;
    }

    if open_file {
        open_file_native(&detail.save_path);
    }
    if open_folder {
        open_folder_native(&detail.save_path);
    }
    if close_window {
        let _ = window.hide();
    }
}

fn apply_settings(window: &NativeSettingsWindow, settings: &AppSettings) {
    window.set_default_dir(settings.default_download_dir.clone().into());
    window.set_temp_dir(
        settings
            .temp_download_dir
            .clone()
            .unwrap_or_default()
            .into(),
    );
    window.set_segments_text(settings.default_segments.to_string().into());
    window.set_speed_limit_text(
        settings
            .speed_limit_bps
            .map(|value| (value / 1024).to_string())
            .unwrap_or_else(|| "0".to_string())
            .into(),
    );
    window.set_start_on_boot(settings.start_on_boot);
    window.set_show_extension_add_dialog(settings.show_add_dialog_for_extension_downloads);
    let completion_dialog_enabled =
        COMPLETION_DIALOGS_DISABLED.with(|disabled| !*disabled.borrow());
    window.set_show_download_complete_dialog(completion_dialog_enabled);
    window.set_settings_message(SharedString::default());
}

fn set_settings_message(message: impl Into<String>) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        SETTINGS_WINDOW.with(|settings_window| {
            if let Some(window) = settings_window.borrow().as_ref() {
                window.set_settings_message(message.into());
            }
        });
    });
}

fn set_update_state(
    status: impl Into<String>,
    version: impl Into<String>,
    notes: impl Into<String>,
    available: bool,
    working: bool,
) {
    let status = status.into();
    let version = version.into();
    let notes = notes.into();
    let _ = slint::invoke_from_event_loop(move || {
        SETTINGS_WINDOW.with(|settings_window| {
            if let Some(window) = settings_window.borrow().as_ref() {
                window.set_update_status(status.clone().into());
                window.set_update_version(version.clone().into());
                window.set_update_notes(notes.clone().into());
                window.set_update_available(available);
                window.set_update_working(working);
                if !working {
                    window.set_update_progress(0.0);
                    window.set_update_progress_text("".into());
                }
                window.set_active_tab("Updates".into());
            }
        });
    });
}

fn set_update_progress(downloaded: u64, total: u64) {
    let progress = if total > 0 {
        ((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    };
    let text = if total > 0 {
        format!(
            "Downloading update... {} / {} ({:.0}%)",
            format_bytes(downloaded),
            format_bytes(total),
            progress
        )
    } else {
        format!("Downloading update... {}", format_bytes(downloaded))
    };
    let _ = slint::invoke_from_event_loop(move || {
        SETTINGS_WINDOW.with(|settings_window| {
            if let Some(window) = settings_window.borrow().as_ref() {
                window.set_update_progress(progress);
                window.set_update_progress_text(text.clone().into());
            }
        });
    });
}

fn sync_startup_setting_on_launch(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
) {
    runtime.spawn(async move {
        let settings = manager.get_settings().await;
        if let Err(error) = set_windows_startup_enabled(settings.start_on_boot) {
            log::warn!("Could not sync startup setting: {}", error);
        }
    });
}

fn mark_extension_prompt_seen(
    manager: Arc<DownloadManager>,
    runtime: Arc<tokio::runtime::Runtime>,
) {
    runtime.spawn(async move {
        let mut settings = manager.get_settings().await;
        if settings.extension_prompt_seen {
            return;
        }
        settings.extension_prompt_seen = true;
        manager.update_settings(settings).await;
    });
}

#[cfg(target_os = "windows")]
fn set_windows_startup_enabled(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|error| error.to_string())?;
    let value_name = "Velocity Download Manager";
    if enabled {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let command = format!("\"{}\" --startup", exe.display());
        run_key
            .set_value(value_name, &command)
            .map_err(|error| error.to_string())?;
    } else {
        let _ = run_key.delete_value(value_name);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_windows_startup_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}

fn settings_from_inputs(
    default_dir: &str,
    temp_dir: &str,
    segments: &str,
    speed_limit: &str,
    start_on_boot: bool,
    show_add_dialog_for_extension_downloads: bool,
    show_download_complete_dialog: bool,
    extension_prompt_seen: bool,
) -> Result<AppSettings, String> {
    let default_dir = default_dir.trim();
    if default_dir.is_empty() {
        return Err("Default save folder is required.".to_string());
    }

    let default_segments = segments
        .trim()
        .parse::<usize>()
        .map_err(|_| "Connections must be a number.".to_string())?
        .clamp(1, 16);

    let speed_limit_bps = match speed_limit.trim() {
        "" | "0" => None,
        value => Some(
            value
                .parse::<u64>()
                .map_err(|_| "Speed limit must be a number.".to_string())?
                .saturating_mul(1024),
        ),
    };

    let temp_download_dir = temp_dir
        .trim()
        .is_empty()
        .then(|| None)
        .unwrap_or_else(|| Some(temp_dir.trim().to_string()));

    Ok(AppSettings {
        default_segments,
        default_download_dir: default_dir.to_string(),
        temp_download_dir,
        speed_limit_bps,
        start_on_boot,
        extension_prompt_seen,
        show_add_dialog_for_extension_downloads,
        show_download_complete_dialog,
    })
}

fn select_download(weak: slint::Weak<NativeMain>, id: &str) {
    let id = id.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            let index = LAST_PUBLISHED_ROWS.with(|rows| {
                rows.borrow()
                    .iter()
                    .position(|row| row.id == id)
                    .map(|index| index as i32)
                    .unwrap_or(-1)
            });
            ui.set_selected_download_id(id.clone().into());
            ui.set_selected_download_ids(encode_selected_ids(&[id]));
            ui.set_selected_row_index(index);
            ui.set_selection_anchor_index(index);
        }
    });
}

fn update_selection_from_request(
    weak: slint::Weak<NativeMain>,
    id: String,
    index: i32,
    ctrl: bool,
    shift: bool,
) {
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let rows = LAST_PUBLISHED_ROWS.with(|rows| rows.borrow().clone());
        if rows.is_empty() {
            clear_selection_on_ui(&ui);
            return;
        }

        let index = clamp_row_index(index, rows.len());
        let id = rows
            .get(index as usize)
            .map(|row| row.id.clone())
            .unwrap_or(id);

        if shift {
            let anchor = if ui.get_selection_anchor_index() >= 0 {
                ui.get_selection_anchor_index()
            } else if ui.get_selected_row_index() >= 0 {
                ui.get_selected_row_index()
            } else {
                index
            };
            let ids = ids_for_range(&rows, anchor, index);
            apply_selection_to_ui(&ui, ids, id, index, anchor);
        } else if ctrl {
            let selected_ids = ui.get_selected_download_ids();
            let mut ids = parse_selected_ids(selected_ids.as_str());
            if ids.iter().any(|selected| selected == &id) {
                ids.retain(|selected| selected != &id);
            } else {
                ids.push(id.clone());
            }

            if ids.is_empty() {
                clear_selection_on_ui(&ui);
            } else {
                let primary = if ids.iter().any(|selected| selected == &id) {
                    id.clone()
                } else {
                    ids.last().cloned().unwrap_or_default()
                };
                apply_selection_to_ui(&ui, ids, primary, index, index);
            }
        } else {
            apply_selection_to_ui(&ui, vec![id.clone()], id, index, index);
        }
    });
}

fn update_selection_from_keyboard(
    weak: slint::Weak<NativeMain>,
    action: String,
    shift: bool,
    _ctrl: bool,
) {
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let rows = LAST_PUBLISHED_ROWS.with(|rows| rows.borrow().clone());
        if rows.is_empty() {
            clear_selection_on_ui(&ui);
            return;
        }

        if action == "clear" {
            clear_selection_on_ui(&ui);
            return;
        }

        if action == "select-all" {
            let ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
            let primary_index = ui.get_selected_row_index().max(0);
            let primary_index = clamp_row_index(primary_index, rows.len());
            let primary = rows[primary_index as usize].id.clone();
            apply_selection_to_ui(&ui, ids, primary, primary_index, primary_index);
            return;
        }

        let current = if ui.get_selected_row_index() >= 0 {
            ui.get_selected_row_index()
        } else {
            0
        };

        let target = if let Some(index) = action.strip_prefix("select-index:") {
            index.trim().parse::<i32>().unwrap_or(current)
        } else {
            match action.as_str() {
                "down" => current + 1,
                "up" => current - 1,
                "page-down" => current + 10,
                "page-up" => current - 10,
                "home" => 0,
                "end" => rows.len() as i32 - 1,
                _ => current,
            }
        };

        let target = clamp_row_index(target, rows.len());
        let primary = rows[target as usize].id.clone();
        if shift {
            let anchor = if ui.get_selection_anchor_index() >= 0 {
                ui.get_selection_anchor_index()
            } else {
                current
            };
            let ids = ids_for_range(&rows, anchor, target);
            apply_selection_to_ui(&ui, ids, primary, target, anchor);
        } else {
            apply_selection_to_ui(&ui, vec![primary.clone()], primary, target, target);
        }
    });
}

fn clamp_row_index(index: i32, len: usize) -> i32 {
    if len == 0 {
        -1
    } else if index < 0 {
        0
    } else if index as usize >= len {
        len as i32 - 1
    } else {
        index
    }
}

fn ids_for_range(rows: &[RowView], a: i32, b: i32) -> Vec<String> {
    let start = a.min(b).max(0) as usize;
    let end = a.max(b).min(rows.len() as i32 - 1) as usize;
    rows[start..=end].iter().map(|row| row.id.clone()).collect()
}

fn apply_selection_to_ui(
    ui: &NativeMain,
    ids: Vec<String>,
    primary: String,
    row_index: i32,
    anchor_index: i32,
) {
    ui.set_selected_download_id(primary.clone().into());
    ui.set_selected_download_ids(encode_selected_ids(&ids));
    ui.set_selected_row_index(row_index);
    ui.set_selection_anchor_index(anchor_index);
    update_download_row_selection(ui, &ids);
    ui.invoke_download_selected(primary.into());
}

fn clear_selection_on_ui(ui: &NativeMain) {
    ui.set_selected_download_id(SharedString::default());
    ui.set_selected_download_ids(SharedString::default());
    ui.set_selected_row_index(-1);
    ui.set_selection_anchor_index(-1);
    update_download_row_selection(ui, &[]);
}

fn update_download_row_selection(ui: &NativeMain, selected_ids: &[String]) {
    LAST_PUBLISHED_ROWS.with(|rows| {
        let rows = rows
            .borrow()
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, row)| {
                let selected = selected_ids.iter().any(|id| id == &row.id);
                row_view_to_download_row(row, index, selected)
            })
            .collect::<Vec<_>>();
        ui.set_downloads(ModelRc::new(VecModel::from(rows)));
    });
}

fn encode_selected_ids(ids: &[String]) -> SharedString {
    if ids.is_empty() {
        return SharedString::default();
    }

    format!("\n{}\n", ids.join("\n")).into()
}

fn parse_selected_ids(ids: &str) -> Vec<String> {
    ids.lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn publish_rows(weak: slint::Weak<NativeMain>, rows: Vec<RowView>) {
    let _ = slint::invoke_from_event_loop(move || {
        let changed = LAST_PUBLISHED_ROWS.with(|last| {
            let mut last = last.borrow_mut();
            if *last == rows {
                false
            } else {
                *last = rows.clone();
                true
            }
        });

        if !changed {
            return;
        }

        if let Some(ui) = weak.upgrade() {
            let selected_ids = parse_selected_ids(ui.get_selected_download_ids().as_str());
            let rows: Vec<DownloadRow> = rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| {
                    let selected = selected_ids.iter().any(|id| id == &row.id);
                    row_view_to_download_row(row, index, selected)
                })
                .collect();
            ui.set_downloads(ModelRc::new(VecModel::from(rows)));
        }
    });
}

fn set_status(weak: slint::Weak<NativeMain>, message: impl Into<String>) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status_message(message.into());
        }
    });
}

fn clear_selected_download(weak: slint::Weak<NativeMain>) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            clear_selection_on_ui(&ui);
        }
    });
}

fn row_view_to_download_row(row: RowView, index: usize, selected: bool) -> DownloadRow {
    DownloadRow {
        id: row.id.into(),
        filename: row.filename.into(),
        size: row.size.into(),
        status: row.status.into(),
        time_left: row.time_left.into(),
        rate: row.rate.into(),
        category: row.category.into(),
        progress: row.progress,
        stripe: index % 2 == 1,
        selected,
    }
}

fn task_to_row(task: &DownloadTask) -> RowView {
    let category = category_for_filename(&task.filename).to_string();
    let progress = progress_percent(task);
    RowView {
        id: task.id.clone(),
        filename: task.filename.clone(),
        size: format_bytes(task.total_size.max(task.downloaded)),
        status: task_status_label(task),
        time_left: format_eta(task.eta_seconds).to_string(),
        rate: format_speed(task.speed_bps).to_string(),
        category,
        progress,
    }
}

fn task_to_detail(task: &DownloadTask) -> DownloadDetailView {
    let progress = progress_percent(task);
    DownloadDetailView {
        id: task.id.clone(),
        filename: task.filename.clone(),
        url: task.url.clone(),
        save_path: task.save_path.clone(),
        file_size: format_bytes(task.total_size.max(task.downloaded)),
        downloaded: format_bytes(task.downloaded),
        status: task_status_label(task),
        rate: format_speed(task.speed_bps).to_string(),
        time_left: format_eta(task.eta_seconds).to_string(),
        resume_capability: if task.supports_range
            || task.download_kind == models::DownloadKind::Media
        {
            "Supported".to_string()
        } else {
            "Not supported".to_string()
        },
        connections: task.num_segments.to_string(),
        progress_text: format!("{:.1}%", progress),
        progress,
        speed_limit_text: task
            .speed_limit_bps
            .map(|value| (value / 1024).to_string())
            .unwrap_or_default(),
        segments: task
            .segments
            .iter()
            .map(|segment| SegmentView {
                number: (segment.id + 1).to_string(),
                downloaded: format_bytes(segment.downloaded),
                status: segment_status_label(&segment.status),
                progress: segment.progress() as f32,
            })
            .collect(),
    }
}

fn sort_tasks(tasks: &mut [DownloadTask], sort: &str) {
    match sort {
        "name" => tasks.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase())),
        "size" => tasks.sort_by(|a, b| {
            b.total_size
                .max(b.downloaded)
                .cmp(&a.total_size.max(a.downloaded))
        }),
        "status" => tasks.sort_by(|a, b| {
            status_label(&a.status, a.total_size, a.downloaded).cmp(&status_label(
                &b.status,
                b.total_size,
                b.downloaded,
            ))
        }),
        "updated" => tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        _ => tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
    }
}

fn search_matches(task: &DownloadTask, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let category = category_for_filename(&task.filename).to_lowercase();
    let status = status_label(&task.status, task.total_size, task.downloaded).to_lowercase();
    task.filename.to_lowercase().contains(query)
        || task.url.to_lowercase().contains(query)
        || task.save_path.to_lowercase().contains(query)
        || category.contains(query)
        || status.contains(query)
}

fn segment_status_label(status: &models::SegmentStatus) -> String {
    match status {
        models::SegmentStatus::Pending => "Waiting".to_string(),
        models::SegmentStatus::Downloading => "Receiving data".to_string(),
        models::SegmentStatus::Paused => "Paused".to_string(),
        models::SegmentStatus::Completed => "Complete".to_string(),
        models::SegmentStatus::Failed => "Failed".to_string(),
    }
}

fn category_matches(task: &DownloadTask, selected: &str) -> bool {
    match selected {
        "All Downloads" => true,
        "Finished" => task.status == DownloadStatus::Completed,
        "Unfinished" => task.status != DownloadStatus::Completed,
        "Queues" => task.status == DownloadStatus::Queued,
        "Grabber projects" => false,
        category => category_for_filename(&task.filename) == category,
    }
}

fn progress_percent(task: &DownloadTask) -> f32 {
    if task.status == DownloadStatus::Completed {
        return 100.0;
    }

    if task.total_size == 0 {
        return 0.0;
    }

    ((task.downloaded as f64 / task.total_size as f64) * 100.0).clamp(0.0, 100.0) as f32
}

fn status_label(status: &DownloadStatus, total_size: u64, downloaded: u64) -> String {
    match status {
        DownloadStatus::Completed => "Complete".to_string(),
        DownloadStatus::Downloading => {
            if total_size > 0 {
                format!("{:.2}%", (downloaded as f64 / total_size as f64) * 100.0)
            } else {
                "Receiving data".to_string()
            }
        }
        DownloadStatus::Queued => "Queued".to_string(),
        DownloadStatus::Paused => "Paused".to_string(),
        DownloadStatus::Failed => "Failed".to_string(),
        DownloadStatus::Assembling => "Assembling".to_string(),
    }
}

fn task_status_label(task: &DownloadTask) -> String {
    if task.download_kind == models::DownloadKind::Media
        && task.status == DownloadStatus::Downloading
        && task.downloaded == 0
        && task.speed_bps <= 0.0
    {
        return "Preparing".to_string();
    }

    status_label(&task.status, task.total_size, task.downloaded)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", size, UNITS[unit])
    }
}

fn format_speed(speed_bps: f64) -> SharedString {
    if speed_bps <= 0.0 {
        return SharedString::default();
    }
    format!("{}/sec", format_bytes(speed_bps as u64)).into()
}

fn format_eta(seconds: f64) -> SharedString {
    if seconds <= 0.0 || !seconds.is_finite() {
        return SharedString::default();
    }

    let seconds = seconds.round() as u64;
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{} d {} h", days, hours).into()
    } else if hours > 0 {
        format!("{} h {} m", hours, minutes).into()
    } else if minutes > 0 {
        format!("{} m {} s", minutes, secs).into()
    } else {
        format!("{} s", secs).into()
    }
}

fn category_for_filename(filename: &str) -> &'static str {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "ts" | "m3u8" => "Video",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => "Music",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" => "Compressed",
        "exe" | "msi" | "apk" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" => "Programs",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "csv" => "Documents",
        _ => "General",
    }
}

fn task_to_properties(task: &DownloadTask) -> FilePropertiesView {
    let ext = std::path::Path::new(&task.filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_uppercase();
    let file_type = if ext.is_empty() {
        "File".to_string()
    } else {
        format!("{} File", ext)
    };

    let progress = progress_percent(task);
    let status = match task.status {
        DownloadStatus::Completed => "Complete".to_string(),
        DownloadStatus::Downloading => format!("{:.2}% complete", progress),
        DownloadStatus::Paused => format!("Paused ({:.2}%)", progress),
        DownloadStatus::Failed => "Failed".to_string(),
        DownloadStatus::Queued => "Queued".to_string(),
        DownloadStatus::Assembling => "Assembling file".to_string(),
    };

    let total = task.total_size.max(task.downloaded);
    let size = if total > 0 {
        format!("{} ({} Bytes)", format_bytes(total), total)
    } else {
        "Unknown".to_string()
    };

    let last_date = task.updated_at.format("%b %d %H:%M:%S %Y").to_string();

    let result = match task.status {
        DownloadStatus::Completed => "Complete".to_string(),
        DownloadStatus::Downloading => "Downloading".to_string(),
        DownloadStatus::Paused => "Download has been paused".to_string(),
        DownloadStatus::Failed => task
            .error
            .as_deref()
            .unwrap_or("Download failed")
            .to_string(),
        DownloadStatus::Queued => "Queued for download".to_string(),
        DownloadStatus::Assembling => "Assembling downloaded segments".to_string(),
    };

    FilePropertiesView {
        id: task.id.clone(),
        filename: task.filename.clone(),
        file_type,
        status,
        size,
        save_path: task.save_path.clone(),
        url: task.url.clone(),
        last_date,
        result,
        can_open: task.status == DownloadStatus::Completed,
    }
}
