use crate::engine::DownloadEngine;
use crate::models::*;
use crate::state::StateManager;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};

/// The download manager orchestrating all concurrent downloads
pub struct DownloadManager {
    engine: Arc<DownloadEngine>,
    tasks: Arc<RwLock<HashMap<String, Arc<RwLock<DownloadTask>>>>>,
    cancel_tokens: Arc<RwLock<HashMap<String, Arc<Mutex<bool>>>>>,
    settings: Arc<RwLock<AppSettings>>,
    active_count: Arc<Mutex<usize>>,
    speed_limits: Arc<RwLock<HashMap<String, Arc<RwLock<Option<u64>>>>>>,
    hidden_tasks: Arc<RwLock<HashSet<String>>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        let settings = StateManager::load_settings();
        let restored_tasks = Self::load_restored_tasks(&settings);

        Self {
            engine: Arc::new(DownloadEngine::new()),
            tasks: Arc::new(RwLock::new(restored_tasks)),
            cancel_tokens: Arc::new(RwLock::new(HashMap::new())),
            settings: Arc::new(RwLock::new(settings)),
            active_count: Arc::new(Mutex::new(0)),
            speed_limits: Arc::new(RwLock::new(HashMap::new())),
            hidden_tasks: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    fn load_restored_tasks(settings: &AppSettings) -> HashMap<String, Arc<RwLock<DownloadTask>>> {
        let mut restored = HashMap::new();
        let mut seen = HashSet::new();

        for mut task in StateManager::load_history() {
            if !seen.insert(task.id.clone()) {
                continue;
            }
            task.status = DownloadStatus::Completed;
            task.speed_bps = 0.0;
            task.eta_seconds = 0.0;
            for segment in &mut task.segments {
                segment.speed_bps = 0.0;
                segment.status = SegmentStatus::Completed;
            }
            restored.insert(task.id.clone(), Arc::new(RwLock::new(task)));
        }

        let mut roots = vec![settings.default_download_dir.clone()];
        if let Some(temp_dir) = &settings.temp_download_dir {
            roots.push(temp_dir.clone());
        }

        for root in roots {
            for mut task in StateManager::scan_for_resumable_sync(&root) {
                if task.status == DownloadStatus::Completed {
                    continue;
                }
                seen.insert(task.id.clone());
                Self::verify_restored_segments(&mut task);
                restored.insert(task.id.clone(), Arc::new(RwLock::new(task)));
            }
        }

        restored
    }

    fn verify_restored_segments(task: &mut DownloadTask) {
        task.speed_bps = 0.0;
        task.eta_seconds = 0.0;

        if matches!(task.status, DownloadStatus::Downloading | DownloadStatus::Assembling | DownloadStatus::Queued) {
            task.status = DownloadStatus::Paused;
        }

        if task.supports_range && !task.segments.is_empty() {
            let mut total_downloaded = 0;
            for segment in &mut task.segments {
                let actual = std::fs::metadata(&segment.temp_file)
                    .map(|m| m.len().min(segment.total_size()))
                    .unwrap_or(0);
                segment.downloaded = actual;
                segment.speed_bps = 0.0;
                segment.status = if actual >= segment.total_size() {
                    SegmentStatus::Completed
                } else if actual > 0 {
                    SegmentStatus::Paused
                } else {
                    SegmentStatus::Pending
                };
                total_downloaded += actual;
            }
            task.downloaded = total_downloaded;
        }

        task.updated_at = Utc::now();
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

    fn sanitize_filename(filename: &str) -> String {
        let name = std::path::Path::new(filename)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download");
        let sanitized: String = name
            .chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                c if c.is_control() => '_',
                c => c,
            })
            .collect();
        let trimmed = sanitized.trim().trim_matches('.').to_string();
        if trimmed.is_empty() {
            "download".to_string()
        } else {
            trimmed
        }
    }

    fn unique_save_path(save_dir: &str, filename: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(save_dir);
        let candidate = dir.join(filename);
        let meta_candidate = dir.join(format!(".{}.meta", filename));
        if !candidate.exists() && !meta_candidate.exists() {
            return candidate;
        }

        let path = std::path::Path::new(filename);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("download");
        let ext = path.extension().and_then(|e| e.to_str());

        for index in 1..10_000 {
            let next_name = match ext {
                Some(ext) if !ext.is_empty() => format!("{} ({}).{}", stem, index, ext),
                _ => format!("{} ({})", stem, index),
            };
            let next_path = dir.join(&next_name);
            let next_meta = dir.join(format!(".{}.meta", next_name));
            if !next_path.exists() && !next_meta.exists() {
                return next_path;
            }
        }

        dir.join(format!("{}-{}", uuid::Uuid::new_v4(), filename))
    }

    fn friendly_error(error: &str) -> String {
        let lower = error.to_lowercase();
        if lower.contains("got html page instead of file") || lower.contains("text/html") {
            "The server returned a web page instead of the file. The link may require a fresh login, cookies, or a refreshed URL.".to_string()
        } else if lower.contains("returned 200 instead of 206") || lower.contains("does not support range") {
            "This server does not support multi-part resume downloads. Try again as a single connection or refresh the link.".to_string()
        } else if lower.contains("no space") || lower.contains("disk full") {
            "Not enough disk space for this download.".to_string()
        } else if lower.contains("permission denied") || lower.contains("access is denied") {
            "Velocity cannot write to the selected folder. Choose another save location or check folder permissions.".to_string()
        } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("stalled") {
            "The connection stalled or timed out. Use Retry to continue from the saved progress.".to_string()
        } else if lower.contains("too many redirects") {
            "The link redirected too many times. The URL may be expired or protected.".to_string()
        } else {
            error.to_string()
        }
    }

    /// Add a new download to the queue
    pub async fn add_download(
        &self,
        url: String,
        save_path: Option<String>,
        filename_override: Option<String>,
        ctx: HttpContext,
        app_handle: AppHandle,
    ) -> Result<DownloadTask, String> {
        self.add_download_internal(url, save_path, filename_override, ctx, app_handle, true).await
    }

    /// Add a download that starts immediately but stays hidden until explicitly revealed.
    pub async fn prefetch_download(
        &self,
        url: String,
        save_path: Option<String>,
        filename_override: Option<String>,
        ctx: HttpContext,
        app_handle: AppHandle,
    ) -> Result<DownloadTask, String> {
        self.add_download_internal(url, save_path, filename_override, ctx, app_handle, false).await
    }

    async fn add_download_internal(
        &self,
        url: String,
        save_path: Option<String>,
        filename_override: Option<String>,
        ctx: HttpContext,
        app_handle: AppHandle,
        visible: bool,
    ) -> Result<DownloadTask, String> {
        // Probe the URL with the browser context so auth cookies are applied
        let (total_size, supports_range, content_type, filename) =
            self.engine.probe_url(&url, &ctx).await?;
        let filename = filename_override
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(filename);
        let filename = Self::sanitize_filename(&filename);

        let settings = self.settings.read().await;
        let save_dir = save_path.unwrap_or_else(|| {
            let base = settings.default_download_dir.clone();
            let category = Self::category_for_filename(&filename);
            
            let path = std::path::PathBuf::from(base).join(category);
            let _ = std::fs::create_dir_all(&path); // Ensure directory exists
            path.to_string_lossy().to_string()
        });
        let temp_dir_override = settings.temp_download_dir.clone();
        let global_speed_limit = settings.speed_limit_bps;
        drop(settings);

        // Use proper OS path joining
        let full_path_buf = Self::unique_save_path(&save_dir, &filename);
        let filename = full_path_buf
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&filename)
            .to_string();
        let full_path = full_path_buf.to_string_lossy().to_string();

        let mut task = DownloadTask::new(url, filename, full_path);
        task.total_size = total_size;
        task.supports_range = supports_range;
        task.content_type = content_type;
        task.http_context = ctx;
        task.temp_dir_override = temp_dir_override;
        task.speed_limit_bps = global_speed_limit;

        if supports_range && total_size > 0 {
            let settings = self.settings.read().await;
            let num_segments =
                DownloadEngine::calculate_segments(total_size, settings.default_segments);
            drop(settings);

            task.num_segments = num_segments;
            let temp_dir = task.temp_dir();
            task.segments = DownloadEngine::create_segments(total_size, num_segments, &temp_dir);

            log::info!(
                "Download configured: {} segments, total_size={}, range=true",
                num_segments, total_size
            );
        } else {
            task.num_segments = 1;
            let temp_dir = task.temp_dir();
            // For single downloads, segment covers the whole file
            task.segments = vec![Segment::new(0, 0, total_size.saturating_sub(1).max(0), &temp_dir)];
            
            log::info!(
                "Download configured: single stream (no range), total_size={}",
                total_size
            );
        }

        // Save initial state
        StateManager::save_state(&task).await?;

        let task_arc = Arc::new(RwLock::new(task.clone()));
        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(task.id.clone(), task_arc);
        }

        if visible {
            // Emit task added event
            let _ = app_handle.emit("download-added", &task);
        } else {
            let mut hidden = self.hidden_tasks.write().await;
            hidden.insert(task.id.clone());
        }

        // Try to start the download immediately
        self.try_start_download(&task.id, app_handle).await?;

        Ok(task)
    }

    /// Reveal a hidden prefetch so it appears in the UI with its current progress.
    pub async fn reveal_download(
        &self,
        download_id: &str,
        app_handle: AppHandle,
    ) -> Result<DownloadTask, String> {
        let task = {
            let tasks = self.tasks.read().await;
            tasks
                .get(download_id)
                .cloned()
                .ok_or("Download not found")?
        };

        {
            let mut hidden = self.hidden_tasks.write().await;
            hidden.remove(download_id);
        }

        let task_snapshot = task.read().await.clone();
        let _ = app_handle.emit("download-added", &task_snapshot);

        let progress = ProgressEvent {
            download_id: task_snapshot.id.clone(),
            total_size: task_snapshot.total_size,
            downloaded: task_snapshot.downloaded,
            speed_bps: task_snapshot.speed_bps,
            eta_seconds: task_snapshot.eta_seconds,
            status: task_snapshot.status.clone(),
            speed_limit_bps: task_snapshot.speed_limit_bps,
            segments: task_snapshot
                .segments
                .iter()
                .map(|s| SegmentProgress {
                    id: s.id,
                    downloaded: s.downloaded,
                    total_size: s.total_size(),
                    speed_bps: s.speed_bps,
                    status: s.status.clone(),
                    progress: s.progress(),
                })
                .collect(),
        };
        let _ = app_handle.emit("download-progress", &progress);

        Ok(task_snapshot)
    }

    /// Try to start a download if we're under the concurrent limit
    async fn try_start_download(
        &self,
        download_id: &str,
        app_handle: AppHandle,
    ) -> Result<(), String> {
        self.start_download(download_id, app_handle).await
    }

    /// Start downloading a specific task
    pub async fn start_download(
        &self,
        download_id: &str,
        app_handle: AppHandle,
    ) -> Result<(), String> {
        let task_arc = {
            let tasks = self.tasks.read().await;
            tasks
                .get(download_id)
                .cloned()
                .ok_or("Download not found")?
        };

        // Create cancel token
        let cancel_token = Arc::new(Mutex::new(false));
        {
            let mut tokens = self.cancel_tokens.write().await;
            tokens.insert(download_id.to_string(), cancel_token.clone());
        }

        // Update status
        {
            let mut task = task_arc.write().await;
            task.status = DownloadStatus::Downloading;
            task.updated_at = Utc::now();
        }

        *self.active_count.lock().await += 1;

        // Create speed limit tracker
        let limit_arc = {
            let task = task_arc.read().await;
            Arc::new(RwLock::new(task.speed_limit_bps))
        };
        {
            let mut limits = self.speed_limits.write().await;
            limits.insert(download_id.to_string(), limit_arc.clone());
        }

        let engine = self.engine.clone();
        let cancel_tokens = self.cancel_tokens.clone();
        let speed_limits = self.speed_limits.clone();
        let active_count = self.active_count.clone();
        let hidden_tasks = self.hidden_tasks.clone();
        let download_id = download_id.to_string();
        let app_handle_clone = app_handle.clone();
        let all_tasks = self.tasks.clone();

        // Spawn the download process
        tokio::spawn(async move {
            let result = Self::run_download(
                engine,
                task_arc.clone(),
                cancel_token,
                app_handle_clone.clone(),
                limit_arc,
                hidden_tasks,
            )
            .await;

            // Update final status
            {
                let mut task = task_arc.write().await;
                match &result {
                    Ok(()) => {
                        if task.status != DownloadStatus::Paused {
                            task.status = DownloadStatus::Completed;
                            task.speed_bps = 0.0;
                            task.eta_seconds = 0.0;
                            // Set downloaded to total_size only if we know total_size
                            if task.total_size > 0 {
                                task.downloaded = task.total_size;
                            }
                            // Clean up temp files and meta
                            let _ = DownloadEngine::cleanup_temp(&task.temp_dir()).await;
                            let _ = StateManager::delete_state(&task).await;
                            let _ = StateManager::upsert_history(&task);
                        }
                    }
                    Err(e) => {
                        if task.status != DownloadStatus::Paused {
                            task.status = DownloadStatus::Failed;
                            task.error = Some(Self::friendly_error(e));
                        }
                        task.speed_bps = 0.0;
                        // Save state for resume
                        let _ = StateManager::save_state(&task).await;
                    }
                }
                task.updated_at = Utc::now();

                // Emit final status
                let progress = ProgressEvent {
                    download_id: download_id.clone(),
                    total_size: task.total_size,
                    downloaded: task.downloaded,
                    speed_bps: task.speed_bps,
                    eta_seconds: task.eta_seconds,
                    status: task.status.clone(),
                    speed_limit_bps: task.speed_limit_bps,
                    segments: task
                        .segments
                        .iter()
                        .map(|s| SegmentProgress {
                            id: s.id,
                            downloaded: s.downloaded,
                            total_size: s.total_size(),
                            speed_bps: s.speed_bps,
                            status: s.status.clone(),
                            progress: s.progress(),
                        })
                        .collect(),
                };
                let _ = app_handle_clone.emit("download-progress", &progress);
            }

            // Clean up cancel token
            {
                let mut tokens = cancel_tokens.write().await;
                tokens.remove(&download_id);
            }

            {
                let mut limits = speed_limits.write().await;
                limits.remove(&download_id);
            }

            // Decrement active count
            {
                let mut count = active_count.lock().await;
                *count = count.saturating_sub(1);
            }

            // Try to start next queued download
            let tasks = all_tasks.read().await;
            for (id, task) in tasks.iter() {
                let t = task.read().await;
                if t.status == DownloadStatus::Queued {
                    let id_clone = id.clone();
                    drop(t);
                    drop(tasks);
                    let _ = app_handle_clone.emit("check-queue", &id_clone);
                    break;
                }
            }
        });

        Ok(())
    }

    /// Internal: run the segment download process
    async fn run_download(
        engine: Arc<DownloadEngine>,
        task_arc: Arc<RwLock<DownloadTask>>,
        cancel_token: Arc<Mutex<bool>>,
        app_handle: AppHandle,
        task_speed_limit: Arc<RwLock<Option<u64>>>,
        hidden_tasks: Arc<RwLock<HashSet<String>>>,
    ) -> Result<(), String> {
        let (url, supports_range, save_path, segments_data, total_size, ctx) = {
            let task = task_arc.read().await;
            (
                task.url.clone(),
                task.supports_range,
                task.save_path.clone(),
                task.segments.clone(),
                task.total_size,
                task.http_context.clone(),
            )
        };

        if !supports_range || segments_data.len() <= 1 {
            // Single-file download (no segmentation)
            let task_for_cb = task_arc.clone();
            let app_for_cb = app_handle.clone();
            let dl_id = task_arc.read().await.id.clone();

            // Callback now receives: (bytes_this_chunk, actual_total_size, speed)
            let callback = Arc::new(move |bytes: u64, actual_total: u64, speed: f64| {
                let task_for_update = task_for_cb.clone();
                let app = app_for_cb.clone();
                let id = dl_id.clone();
                tokio::spawn(async move {
                    let mut task = task_for_update.write().await;
                    task.downloaded += bytes;
                    task.speed_bps = speed;
                    
                    // Update total_size if we now know it from the actual response
                    if actual_total > 0 && task.total_size != actual_total {
                        log::info!(
                            "Updating total_size from {} to {} (from actual response)",
                            task.total_size, actual_total
                        );
                        task.total_size = actual_total;
                        // Update segment info too
                        if !task.segments.is_empty() {
                            task.segments[0].end_byte = actual_total.saturating_sub(1);
                        }
                    }

                    if speed > 0.0 && task.total_size > 0 {
                        let remaining = task.total_size.saturating_sub(task.downloaded);
                        task.eta_seconds = remaining as f64 / speed;
                    }
                    if !task.segments.is_empty() {
                        task.segments[0].downloaded = task.downloaded;
                        task.segments[0].speed_bps = speed;
                        task.segments[0].status = SegmentStatus::Downloading;
                    }

                    let progress = ProgressEvent {
                        download_id: id,
                        total_size: task.total_size,
                        downloaded: task.downloaded,
                        speed_bps: task.speed_bps,
                        eta_seconds: task.eta_seconds,
                        status: task.status.clone(),
                        speed_limit_bps: task.speed_limit_bps,
                        segments: task
                            .segments
                            .iter()
                            .map(|s| SegmentProgress {
                                id: s.id,
                                downloaded: s.downloaded,
                                total_size: s.total_size(),
                                speed_bps: s.speed_bps,
                                status: s.status.clone(),
                                progress: s.progress(),
                            })
                            .collect(),
                    };
                    let _ = app.emit("download-progress", &progress);
                });
            });

            let mut retries = 0;
            let max_retries = 50;
            let actual_downloaded = loop {
                let res = DownloadEngine::download_single(
                    engine.client(),
                    url.clone(),
                    ctx.clone(),
                    save_path.clone(),
                    total_size,
                    cancel_token.clone(),
                    callback.clone(),
                    task_speed_limit.clone(),
                )
                .await;

                if res.is_ok() || *cancel_token.lock().await || retries >= max_retries {
                    break res;
                }

                if let Err(e) = &res {
                    if e.contains("got HTML page instead of file") || e.contains("returned 200 instead of 206") {
                        break res; // Hard error, don't spam requests
                    }
                }

                retries += 1;
                // Exponential backoff, capped at 20 seconds. 1, 2, 4, 8, 16, 20...
                let sleep_s = std::cmp::min(20, 2u64.pow(retries.min(6) as u32));
                log::warn!("Single download failed ({:?}), retrying in {}s ({}/{})...", res.unwrap_err(), sleep_s, retries, max_retries);
                tokio::time::sleep(tokio::time::Duration::from_secs(sleep_s)).await;
            }?;

            // Update the task with actual downloaded bytes
            {
                let mut task = task_arc.write().await;
                if actual_downloaded > 0 {
                    task.downloaded = actual_downloaded;
                    // If total size was 0 initially, or if we got more bytes than expected, update it
                    if task.total_size == 0 || actual_downloaded > task.total_size {
                        task.total_size = actual_downloaded;
                    }
                }
            }
        } else {
            // Multi-segment download
            let segment_arcs: Vec<Arc<RwLock<Segment>>> = segments_data
                .into_iter()
                .map(|s| Arc::new(RwLock::new(s)))
                .collect();

            let mut handles = Vec::new();

            for seg_arc in &segment_arcs {
                let client = engine.client();
                let url = url.clone();
                let ctx = ctx.clone();
                let seg = seg_arc.clone();
                let token = cancel_token.clone();
                let task_for_cb = task_arc.clone();
                let app_for_cb = app_handle.clone();
                let all_segs = segment_arcs.clone();

                let callback = Arc::new(move |_seg_id: usize, bytes: u64, _speed: f64| {
                    let task_for_update = task_for_cb.clone();
                    let app = app_for_cb.clone();
                    let segs = all_segs.clone();
                    tokio::spawn(async move {
                        let mut task = task_for_update.write().await;
                        task.downloaded += bytes;

                        let mut total_speed = 0.0;
                        let mut updated_segments = Vec::new();
                        let mut total_downloaded = 0;
                        for (i, s) in segs.iter().enumerate() {
                            let seg = s.read().await;
                            total_speed += seg.speed_bps;
                            total_downloaded += seg.downloaded;
                            
                            if i < task.segments.len() {
                                task.segments[i] = seg.clone();
                            }
                            
                            updated_segments.push(SegmentProgress {
                                id: seg.id,
                                downloaded: seg.downloaded,
                                total_size: seg.total_size(),
                                speed_bps: seg.speed_bps,
                                status: seg.status.clone(),
                                progress: seg.progress(),
                            });
                        }

                        // Re-sync global task downloaded with true sum of segments 
                        // to prevent drift across resumes.
                        task.downloaded = total_downloaded;
                        
                        task.speed_bps = total_speed;
                        if total_speed > 0.0 && task.total_size > 0 {
                            let remaining = task.total_size.saturating_sub(task.downloaded);
                            task.eta_seconds = remaining as f64 / total_speed;
                        }

                        let progress = ProgressEvent {
                            download_id: task.id.clone(),
                            total_size: task.total_size,
                            downloaded: task.downloaded,
                            speed_bps: task.speed_bps,
                            eta_seconds: task.eta_seconds,
                            status: task.status.clone(),
                            speed_limit_bps: task.speed_limit_bps,
                            segments: updated_segments,
                        };
                        let _ = app.emit("download-progress", &progress);

                        // Periodically save state
                        let _ = StateManager::save_state(&task).await;
                    });
                });

                let limit_arc_for_seg = task_speed_limit.clone();
                let handle = tokio::spawn(async move {
                    let mut retries = 0;
                    let max_retries = 50;
                    loop {
                        let res = DownloadEngine::download_segment(
                            client.clone(), 
                            url.clone(), 
                            ctx.clone(), 
                            seg.clone(), 
                            token.clone(), 
                            callback.clone(), 
                            limit_arc_for_seg.clone()
                        ).await;

                        if res.is_ok() || *token.lock().await || retries >= max_retries {
                            return res;
                        }

                        if let Err(e) = &res {
                            if e.contains("got HTML page instead of file") || e.contains("returned 200 instead of 206") {
                                return res; // Hard error, retrying won't help expired links
                            }
                        }

                        retries += 1;
                        let seg_id = seg.read().await.id;
                        let sleep_s = std::cmp::min(20, 2u64.pow(retries.min(6) as u32));
                        log::warn!("Segment {} failed ({:?}), retrying in {}s ({}/{})...", seg_id, res.unwrap_err(), sleep_s, retries, max_retries);
                        tokio::time::sleep(tokio::time::Duration::from_secs(sleep_s)).await;
                    }
                });

                handles.push(handle);
            }

            // Wait for all segments to complete
            let mut errors = Vec::new();
            for handle in handles {
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => errors.push(e),
                    Err(e) => errors.push(format!("Task join error: {}", e)),
                }
            }

            if !errors.is_empty() && !*cancel_token.lock().await {
                // Ensure state is saved properly when an error happens mid-download
                let mut task = task_arc.write().await;
                let mut total_downloaded = 0;
                for (i, seg_arc) in segment_arcs.iter().enumerate() {
                    let seg = seg_arc.read().await;
                    total_downloaded += seg.downloaded;
                    if i < task.segments.len() {
                        task.segments[i] = seg.clone();
                    }
                }
                task.downloaded = total_downloaded;
                task.status = DownloadStatus::Failed;
                task.speed_bps = 0.0;
                let _ = StateManager::save_state(&task).await;
                
                return Err(format!("Segment errors: {}", errors.join(", ")));
            }

            if *cancel_token.lock().await {
                // Copy segment state back to task for persistence
                let mut task = task_arc.write().await;
                for (i, seg_arc) in segment_arcs.iter().enumerate() {
                    let seg = seg_arc.read().await;
                    if i < task.segments.len() {
                        task.segments[i] = seg.clone();
                    }
                }
                task.status = DownloadStatus::Paused;
                let _ = StateManager::save_state(&task).await;
                return Ok(());
            }

            let download_id = task_arc.read().await.id.clone();
            while hidden_tasks.read().await.contains(&download_id) {
                if *cancel_token.lock().await {
                    let mut task = task_arc.write().await;
                    task.status = DownloadStatus::Paused;
                    let _ = StateManager::save_state(&task).await;
                    return Ok(());
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }

            // Assemble the final file
            {
                let mut task = task_arc.write().await;
                task.status = DownloadStatus::Assembling;
                let _ = app_handle.emit(
                    "download-progress",
                    &ProgressEvent {
                        download_id: task.id.clone(),
                        total_size: task.total_size,
                        downloaded: task.downloaded,
                        speed_bps: 0.0,
                        eta_seconds: 0.0,
                        status: DownloadStatus::Assembling,
                        speed_limit_bps: task.speed_limit_bps,
                        segments: vec![],
                    },
                );
            }

            // Collect final segments
            let final_segments: Vec<Segment> = {
                let mut segs = Vec::new();
                for seg_arc in &segment_arcs {
                    segs.push(seg_arc.read().await.clone());
                }
                segs
            };

            DownloadEngine::assemble_file(&final_segments, &save_path).await?;

            // Clean up temp dir
            let temp_dir = task_arc.read().await.temp_dir();
            let _ = DownloadEngine::cleanup_temp(&temp_dir).await;
        }

        Ok(())
    }

    /// Pause a download
    pub async fn pause_download(&self, download_id: &str) -> Result<(), String> {
        // Set cancel token
        let tokens = self.cancel_tokens.read().await;
        if let Some(token) = tokens.get(download_id) {
            *token.lock().await = true;
        }

        // Update task status
        let tasks = self.tasks.read().await;
        if let Some(task_arc) = tasks.get(download_id) {
            let mut task = task_arc.write().await;
            task.status = DownloadStatus::Paused;
            task.speed_bps = 0.0;
            task.updated_at = Utc::now();
            StateManager::save_state(&task).await?;
        }

        Ok(())
    }

    /// Resume a paused download
    pub async fn resume_download(
        &self,
        download_id: &str,
        app_handle: AppHandle,
    ) -> Result<(), String> {
        // Update status to queued and restart
        {
            let tasks = self.tasks.read().await;
            if let Some(task_arc) = tasks.get(download_id) {
                let mut task = task_arc.write().await;
                task.status = DownloadStatus::Queued;
                task.updated_at = Utc::now();
            }
        }

        self.start_download(download_id, app_handle).await
    }

    /// Remove a download from the queue
    pub async fn remove_download(&self, download_id: &str) -> Result<(), String> {
        // Cancel if running
        {
            let tokens = self.cancel_tokens.read().await;
            if let Some(token) = tokens.get(download_id) {
                *token.lock().await = true;
            }
        }

        // Remove from tasks
        let task = {
            let mut tasks = self.tasks.write().await;
            tasks.remove(download_id)
        };

        {
            let mut hidden = self.hidden_tasks.write().await;
            hidden.remove(download_id);
        }

        // Clean up files
        if let Some(task_arc) = task {
            let task = task_arc.read().await;
            let _ = DownloadEngine::cleanup_temp(&task.temp_dir()).await;
            let _ = StateManager::delete_state(&task).await;
            let _ = StateManager::remove_history(&task.id);
        }

        Ok(())
    }

    /// Get all downloads
    pub async fn get_all_downloads(&self) -> Vec<DownloadTask> {
        let tasks = self.tasks.read().await;
        let hidden = self.hidden_tasks.read().await;
        let mut result = Vec::new();
        for (id, task_arc) in tasks.iter() {
            if hidden.contains(id) {
                continue;
            }
            result.push(task_arc.read().await.clone());
        }
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result
    }

    /// Get a specific download
    pub async fn get_download(&self, download_id: &str) -> Option<DownloadTask> {
        let tasks = self.tasks.read().await;
        if let Some(task_arc) = tasks.get(download_id) {
            Some(task_arc.read().await.clone())
        } else {
            None
        }
    }

    /// Set personal speed limit for a specific task
    pub async fn set_task_speed_limit(&self, download_id: &str, limit_bps: Option<u64>) -> Result<(), String> {
        // Update task model
        {
            let tasks = self.tasks.read().await;
            if let Some(task_arc) = tasks.get(download_id) {
                let mut task = task_arc.write().await;
                task.speed_limit_bps = limit_bps;
                StateManager::save_state(&task).await?;
            }
        }

        // Update active tracker if running
        {
            let limits = self.speed_limits.read().await;
            if let Some(limit_arc) = limits.get(download_id) {
                let mut limit = limit_arc.write().await;
                *limit = limit_bps;
            }
        }

        Ok(())
    }

    /// Update settings
    pub async fn update_settings(&self, new_settings: AppSettings) {
        let mut settings = self.settings.write().await;
        *settings = new_settings.clone();
        let _ = StateManager::save_settings(&new_settings);
    }

    /// Get current settings
    pub async fn get_settings(&self) -> AppSettings {
        self.settings.read().await.clone()
    }
}
