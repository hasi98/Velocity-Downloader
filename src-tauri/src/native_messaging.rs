#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Native Messaging host skeleton for browser extension integration.
///
/// This module provides the foundation for a Chrome/Firefox native messaging host
/// that can receive intercepted download URLs from a browser extension.
///
/// To use this:
/// 1. Register the native messaging host manifest with the browser
/// 2. Build the host binary
/// 3. The browser extension sends messages via stdin using the native messaging protocol
/// 4. This host reads messages, processes them, and sends responses via stdout

/// Message received from the browser extension
#[derive(Debug, Serialize, Deserialize)]
pub struct NativeMessage {
    /// Action type: "intercept_download", "get_status", "ping"
    pub action: String,
    /// URL to download (for intercept_download)
    pub url: Option<String>,
    /// Suggested filename
    pub filename: Option<String>,
    /// Referrer URL
    pub referrer: Option<String>,
    /// File size if known
    pub file_size: Option<u64>,
    /// MIME type if known
    pub mime_type: Option<String>,
}

/// Response sent back to the browser extension
#[derive(Debug, Serialize, Deserialize)]
pub struct NativeResponse {
    pub success: bool,
    pub message: String,
    pub download_id: Option<String>,
}

/// Read a native message from stdin (Chrome native messaging protocol)
/// Messages are prefixed with a 4-byte little-endian length
pub fn read_native_message() -> io::Result<NativeMessage> {
    let mut length_bytes = [0u8; 4];
    io::stdin().read_exact(&mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes) as usize;

    if length > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Message too large",
        ));
    }

    let mut buffer = vec![0u8; length];
    io::stdin().read_exact(&mut buffer)?;

    serde_json::from_slice(&buffer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write a native response to stdout (Chrome native messaging protocol)
pub fn write_native_response(response: &NativeResponse) -> io::Result<()> {
    let json = serde_json::to_vec(response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let length = json.len() as u32;
    io::stdout().write_all(&length.to_le_bytes())?;
    io::stdout().write_all(&json)?;
    io::stdout().flush()?;

    Ok(())
}

/// Generate the native messaging host manifest for Chrome
pub fn generate_chrome_manifest(host_path: &str, extension_id: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": "com.myidm.native_host",
        "description": "My IDM Native Messaging Host",
        "path": host_path,
        "type": "stdio",
        "allowed_origins": [
            format!("chrome-extension://{}/", extension_id)
        ]
    }))
    .unwrap_or_default()
}

/// Generate the native messaging host manifest for Firefox
pub fn generate_firefox_manifest(host_path: &str, extension_id: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": "com.myidm.native_host",
        "description": "My IDM Native Messaging Host",
        "path": host_path,
        "type": "stdio",
        "allowed_extensions": [extension_id]
    }))
    .unwrap_or_default()
}

/// Process an incoming native message
/// This is the main handler that would be called in the native host's event loop
pub fn process_message(msg: NativeMessage) -> NativeResponse {
    match msg.action.as_str() {
        "ping" => NativeResponse {
            success: true,
            message: "pong".to_string(),
            download_id: None,
        },
        "intercept_download" => {
            if let Some(url) = msg.url {
                // In a full implementation, this would communicate with the
                // running Tauri app via IPC (named pipe, socket, etc.)
                log::info!(
                    "Intercepted download: {} (filename: {:?}, size: {:?})",
                    url,
                    msg.filename,
                    msg.file_size
                );

                NativeResponse {
                    success: true,
                    message: format!("Download intercepted: {}", url),
                    download_id: Some(uuid::Uuid::new_v4().to_string()),
                }
            } else {
                NativeResponse {
                    success: false,
                    message: "No URL provided".to_string(),
                    download_id: None,
                }
            }
        }
        "get_status" => NativeResponse {
            success: true,
            message: "running".to_string(),
            download_id: None,
        },
        _ => NativeResponse {
            success: false,
            message: format!("Unknown action: {}", msg.action),
            download_id: None,
        },
    }
}

/// Tauri command to generate native host manifest files
#[tauri::command]
pub fn generate_native_manifests(
    extension_id: String,
) -> Result<serde_json::Value, String> {
    let host_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "myidm_native_host.exe".to_string());

    let chrome = generate_chrome_manifest(&host_path, &extension_id);
    let firefox = generate_firefox_manifest(&host_path, &extension_id);

    Ok(serde_json::json!({
        "chrome_manifest": chrome,
        "firefox_manifest": firefox,
        "host_path": host_path,
        "instructions": {
            "chrome": "Save the Chrome manifest to: HKEY_CURRENT_USER\\Software\\Google\\Chrome\\NativeMessagingHosts\\com.myidm.native_host",
            "firefox": "Save the Firefox manifest to: HKEY_CURRENT_USER\\Software\\Mozilla\\NativeMessagingHosts\\com.myidm.native_host"
        }
    }))
}
