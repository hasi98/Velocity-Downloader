use crate::manager::DownloadManager;
use crate::models::*;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::RwLock;

pub type ManagerState = Arc<RwLock<DownloadManager>>;

/// Probe a URL and return file metadata
#[tauri::command]
pub async fn probe_url(
    url: String,
    cookies: Option<String>,
    referer: Option<String>,
    user_agent: Option<String>,
    manager: State<'_, ManagerState>,
) -> Result<serde_json::Value, String> {
    let _mgr = manager.read().await;
    let ctx = HttpContext { cookies, referer, user_agent };
    let engine = crate::engine::DownloadEngine::new();
    let (size, supports_range, content_type, filename) = engine.probe_url(&url, &ctx).await?;

    Ok(serde_json::json!({
        "size": size,
        "supports_range": supports_range,
        "content_type": content_type,
        "filename": filename
    }))
}

/// Add a new download
#[tauri::command]
pub async fn add_download(
    url: String,
    save_path: Option<String>,
    cookies: Option<String>,
    referer: Option<String>,
    user_agent: Option<String>,
    app_handle: AppHandle,
    manager: State<'_, ManagerState>,
) -> Result<DownloadTask, String> {
    let ctx = HttpContext { cookies, referer, user_agent };
    let mgr = manager.read().await;
    mgr.add_download(url, save_path, ctx, app_handle).await
}

/// Pause a download
#[tauri::command]
pub async fn pause_download(
    download_id: String,
    manager: State<'_, ManagerState>,
) -> Result<(), String> {
    let mgr = manager.read().await;
    mgr.pause_download(&download_id).await
}

/// Resume a download
#[tauri::command]
pub async fn resume_download(
    download_id: String,
    app_handle: AppHandle,
    manager: State<'_, ManagerState>,
) -> Result<(), String> {
    let mgr = manager.read().await;
    mgr.resume_download(&download_id, app_handle).await
}

/// Remove a download
#[tauri::command]
pub async fn remove_download(
    download_id: String,
    manager: State<'_, ManagerState>,
) -> Result<(), String> {
    let mgr = manager.read().await;
    mgr.remove_download(&download_id).await
}

/// Get all downloads
#[tauri::command]
pub async fn get_all_downloads(
    manager: State<'_, ManagerState>,
) -> Result<Vec<DownloadTask>, String> {
    let mgr = manager.read().await;
    Ok(mgr.get_all_downloads().await)
}

/// Get a specific download
#[tauri::command]
pub async fn get_download(
    download_id: String,
    manager: State<'_, ManagerState>,
) -> Result<Option<DownloadTask>, String> {
    let mgr = manager.read().await;
    Ok(mgr.get_download(&download_id).await)
}

/// Update settings
#[tauri::command]
pub async fn update_settings(
    settings: AppSettings,
    manager: State<'_, ManagerState>,
) -> Result<(), String> {
    let mgr = manager.read().await;
    mgr.update_settings(settings).await;
    Ok(())
}

/// Get current settings
#[tauri::command]
pub async fn get_settings(
    manager: State<'_, ManagerState>,
) -> Result<AppSettings, String> {
    let mgr = manager.read().await;
    Ok(mgr.get_settings().await)
}

/// Set personal speed limit for a specific task
#[tauri::command]
pub async fn set_task_speed_limit(
    download_id: String,
    limit_bps: Option<u64>,
    manager: State<'_, ManagerState>,
) -> Result<(), String> {
    let mgr = manager.read().await;
    mgr.set_task_speed_limit(&download_id, limit_bps).await
}

/// Get default download directory
#[tauri::command]
pub async fn get_default_download_dir() -> Result<String, String> {
    dirs::download_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "Could not determine download directory".to_string())
}

/// Bring window to front
#[tauri::command]
pub fn bring_window_to_front(app_handle: AppHandle) {
    use tauri::Manager;
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Get the path where the extension is stored locally (in AppData)
fn get_local_extension_dest() -> Option<std::path::PathBuf> {
    dirs::data_local_dir().map(|d| d.join("VelocityDownloader").join("extension"))
}

/// Check if the extension files have been copied to the local AppData folder
#[tauri::command]
pub fn check_extension_installed(_browser: String) -> bool {
    if let Some(dest) = get_local_extension_dest() {
        dest.join("manifest.json").exists()
    } else {
        false
    }
}

/// Copy the extension to AppData, then open the browser's extensions page.
/// Returns the folder path so the UI can show it to the user.
#[tauri::command]
pub fn install_extension(browser: String, app_handle: AppHandle) -> Result<String, String> {
    use tauri::Manager;
    use std::fs;

    // 1. Find source: either resource_dir/extension (production) or the dev workspace path
    let resource_dir = app_handle
        .path()
        .resource_dir()
        .map_err(|e| format!("Cannot find resource dir: {}", e))?;

    let source = resource_dir.join("extension");

    // Fallback for dev mode: look for extension next to the project root
    let source = if source.exists() {
        source
    } else {
        // In dev mode Tauri runs from src-tauri, so go up to project root
        let exe_path = std::env::current_exe()
            .map_err(|e| e.to_string())?;
        // Walk up until we find a folder with manifest.json inside extension/
        let mut candidate = exe_path.parent();
        let mut found: Option<std::path::PathBuf> = None;
        for _ in 0..8 {
            if let Some(dir) = candidate {
                let ext_path = dir.join("extension");
                if ext_path.join("manifest.json").exists() {
                    found = Some(ext_path);
                    break;
                }
                candidate = dir.parent();
            }
        }
        found.ok_or_else(|| {
            format!(
                "Extension folder not found at '{}'. In dev mode, run 'npm run tauri build' first.",
                source.display()
            )
        })?
    };

    // 2. Copy extension to a permanent location in AppData
    let dest = get_local_extension_dest()
        .ok_or("Cannot determine AppData directory")?;

    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("Failed to clean old extension: {}", e))?;
    }
    fs::create_dir_all(&dest).map_err(|e| format!("Failed to create extension dir: {}", e))?;

    copy_dir_all(&source, &dest)
        .map_err(|e| format!("Failed to copy extension files: {}", e))?;

    let dest_str = dest.to_string_lossy().to_string();

    // 3. Open the browser's extensions page
    let ext_url = get_browser_ext_url(&browser);
    if !ext_url.is_empty() {
        open_browser_url(&browser, ext_url);
    }

    Ok(dest_str)
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    use std::fs;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&dest_path)?;
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

fn get_browser_ext_url(browser: &str) -> &'static str {
    match browser {
        "chrome"  => "chrome://extensions",
        "edge"    => "edge://extensions",
        "brave"   => "brave://extensions",
        "opera"   => "opera://extensions",
        "vivaldi" => "vivaldi://extensions",
        _ => "",
    }
}

fn open_browser_url(browser: &str, url: &str) {
    #[cfg(target_os = "windows")]
    {
        let exe_name = match browser {
            "chrome"  => "chrome.exe",
            "edge"    => "msedge.exe",
            "brave"   => "brave.exe",
            "opera"   => "opera.exe",
            "vivaldi" => "vivaldi.exe",
            _ => return,
        };

        // Look up the real executable path from the Windows App Paths registry.
        // This is the only reliable way to find browser executables not in PATH.
        let reg_key = format!(
            "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
            exe_name
        );
        let output = std::process::Command::new("reg")
            .args(["query", &reg_key, "/ve"])
            .output();

        let exe_path = if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Parse the default value – the line ends with the path after "REG_SZ    "
            stdout
                .lines()
                .find(|l| l.contains("REG_SZ"))
                .and_then(|l| l.split("REG_SZ").nth(1))
                .map(|s| s.trim().to_string())
        } else {
            None
        };

        // If we found the path, launch it directly with the URL; otherwise fall back to name
        if let Some(path) = exe_path {
            let _ = std::process::Command::new(&path).arg(url).spawn();
        } else {
            let _ = std::process::Command::new(exe_name).arg(url).spawn();
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Open a downloaded file natively
#[tauri::command]
pub fn open_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn().map_err(|e| e.to_string())?;
        
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&path).spawn().map_err(|e| e.to_string())?;
    
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
    
    Ok(())
}

/// Reveal a downloaded file in folder
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer").args(["/select,", &path]).spawn().map_err(|e| e.to_string())?;
    
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").args(["-R", &path]).spawn().map_err(|e| e.to_string())?;
    
    #[cfg(target_os = "linux")]
    {
        let dir = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(&path)).to_string_lossy().to_string();
        std::process::Command::new("xdg-open").arg(dir).spawn().map_err(|e| e.to_string())?;
    }
    
    Ok(())
}
