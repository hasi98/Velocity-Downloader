use axum::{
    extract::State,
    http::Method,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tower_http::cors::{Any, CorsLayer};
use crate::models::HttpContext;

#[derive(Clone)]
pub struct ServerState {
    pub app_handle: AppHandle,
}

/// Request body sent by the browser extension.
/// The `url` field is always present; the context fields are optional
/// (absent when the extension is an older build or the download is
/// initiated from the manual Quick-Add bar).
#[derive(Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub cookies: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Serialize)]
pub struct DownloadResponse {
    pub success: bool,
    pub message: String,
}

pub async fn run_server(app_handle: AppHandle) {
    let state = ServerState { app_handle };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/ping", axum::routing::get(|| async { "pong" }))
        .route("/add_download", post(handle_add_download))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:41420").await.unwrap();
    log::info!("Local API Server listening on {:?}", listener.local_addr().unwrap());
    
    axum::serve(listener, app).await.unwrap();
}

async fn handle_add_download(
    State(state): State<ServerState>,
    Json(payload): Json<DownloadRequest>,
) -> Json<DownloadResponse> {
    log::info!(
        "Received download request from extension: {} (cookies={}, referer={})",
        payload.url,
        payload.cookies.as_deref().map(|c| if c.is_empty() { "none" } else { "present" }).unwrap_or("none"),
        payload.referer.as_deref().unwrap_or("none"),
    );
    
    use tauri::Emitter;

    // Build http context from extension-provided fields
    let http_context = HttpContext {
        cookies: payload.cookies,
        referer: payload.referer,
        user_agent: payload.user_agent,
    };
    
    // Forward URL + browser context to the UI for the confirmation dialog
    match state.app_handle.emit("show-add-download", serde_json::json!({
        "url": payload.url,
        "cookies": http_context.cookies,
        "referer": http_context.referer,
        "user_agent": http_context.user_agent,
    })) {
        Ok(_) => {
            Json(DownloadResponse {
                success: true,
                message: "Download triggered in UI".to_string(),
            })
        }
        Err(e) => {
            log::error!("Failed to emit download event: {}", e);
            Json(DownloadResponse {
                success: false,
                message: e.to_string(),
            })
        }
    }
}
