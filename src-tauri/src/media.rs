use crate::models::{
    DownloadStatus, DownloadTask, HttpContext, ProgressEvent, SegmentProgress, SegmentStatus,
};
use crate::state::StateManager;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Duration};

const PROGRESS_MARKER: &str = "VELOCITY_PROGRESS";
const FILE_MARKER: &str = "VELOCITY_FILE:";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub filename: String,
    pub filesize: Option<u64>,
    pub content_type: Option<String>,
    pub formats: Vec<MediaFormatOption>,
    pub can_merge: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaFormatOption {
    pub id: String,
    pub label: String,
    pub height: Option<u64>,
    pub ext: Option<String>,
    pub filesize: Option<u64>,
    pub requires_ffmpeg: bool,
}

#[derive(Debug, Clone)]
struct ToolCommand {
    program: String,
    base_args: Vec<String>,
}

#[derive(Debug, Default)]
struct MediaProgress {
    downloaded: Option<u64>,
    total: Option<u64>,
    total_estimate: Option<u64>,
    percent: Option<f64>,
    speed: Option<f64>,
    eta: Option<f64>,
}

pub fn is_likely_media_page_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };

    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    let path = parsed.path().to_lowercase();

    if path.ends_with(".m3u8") || path.ends_with(".mpd") {
        return true;
    }

    [
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "dailymotion.com",
        "tiktok.com",
        "instagram.com",
        "facebook.com",
        "fb.watch",
        "x.com",
        "twitter.com",
        "twitch.tv",
        "soundcloud.com",
        "reddit.com",
        "streamable.com",
        "bilibili.com",
    ]
    .iter()
    .any(|domain| host == *domain || host.ends_with(&format!(".{}", domain)))
}

pub fn is_html_content_type(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .map(|value| value.to_lowercase().contains("text/html"))
        .unwrap_or(false)
}

fn media_probe_contexts(url: &str, ctx: &HttpContext) -> Vec<HttpContext> {
    if is_youtube_url(url) && !http_context_is_empty(ctx) {
        vec![HttpContext::default(), ctx.clone()]
    } else {
        vec![ctx.clone()]
    }
}

fn http_context_is_empty(ctx: &HttpContext) -> bool {
    non_empty(&ctx.cookies).is_none()
        && non_empty(&ctx.referer).is_none()
        && non_empty(&ctx.user_agent).is_none()
}

pub async fn probe_media_url(
    url: &str,
    ctx: &HttpContext,
    app_handle: Option<&AppHandle>,
) -> Result<MediaInfo, String> {
    let can_merge = ffmpeg_available(app_handle);
    let is_youtube = is_youtube_url(url);
    let extractor_attempts: Vec<Option<&str>> = if is_youtube {
        vec![
            None,
            Some("youtube:player_client=web,ios,android,mweb"),
            Some("youtube:player_client=default,web,ios,android"),
            Some("youtube:skip=dash,hls"),
        ]
    } else {
        vec![None]
    };

    let mut best_json: Option<Value> = None;
    let mut best_video_count = 0usize;
    let mut last_error = None;
    let target_video_count = if is_youtube { 3 } else { 1 };
    // 2 rounds is enough; a third round almost never produces better results
    // and adds 5-10 seconds of wait time.
    let max_rounds = if is_youtube { 2 } else { 1 };

    let context_attempts = media_probe_contexts(url, ctx);

    'outer: for round in 0..max_rounds {
        for probe_ctx in &context_attempts {
            for extractor_args in &extractor_attempts {
                match probe_media_json_once(url, probe_ctx, app_handle, *extractor_args).await {
                    Ok(json) => {
                        let video_count = json_video_candidate_count(&json);
                        log::info!(
                            "yt-dlp probe attempt for {} returned {} video format(s), round={}, clean_context={}, extractor_args={:?}",
                            url,
                            video_count,
                            round + 1,
                            probe_ctx.cookies.is_none() && probe_ctx.referer.is_none() && probe_ctx.user_agent.is_none(),
                            extractor_args
                        );
                        if video_count > best_video_count || best_json.is_none() {
                            best_video_count = video_count;
                            best_json = Some(json);
                        }
                        // One YouTube video format is usually only the progressive 360p
                        // fallback. Keep probing so DASH formats such as 720p/1080p are
                        // available in the quality selector.
                        if best_video_count >= target_video_count {
                            break 'outer;
                        }
                    }
                    Err(error) => {
                        log::warn!(
                            "yt-dlp probe attempt failed for {}, round={}, clean_context={}, extractor_args={:?}: {}",
                            url,
                            round + 1,
                            probe_ctx.cookies.is_none() && probe_ctx.referer.is_none() && probe_ctx.user_agent.is_none(),
                            extractor_args,
                            error
                        );
                        last_error = Some(error);
                    }
                }
                if !is_youtube || best_video_count >= target_video_count {
                    break;
                }
            }
            if best_video_count >= target_video_count {
                break;
            }
        }

        if !is_youtube || best_video_count >= target_video_count {
            break;
        }

        log::warn!(
            "yt-dlp returned only {} YouTube video format(s) after round {}. Retrying after a short delay.",
            best_video_count,
            round + 1
        );
        sleep(Duration::from_millis(300)).await;
    }

    if is_youtube && best_video_count < 1 {
        log::warn!(
            "Using limited YouTube format result for {} after {} probe round(s).",
            url,
            max_rounds
        );
    }

    let Some(json) = best_json else {
        return Err(
            last_error.unwrap_or_else(|| "yt-dlp did not return media information".to_string())
        );
    };

    Ok(media_info_from_json(&json, can_merge))
}

async fn probe_media_json_once(
    url: &str,
    ctx: &HttpContext,
    app_handle: Option<&AppHandle>,
    extractor_args: Option<&str>,
) -> Result<Value, String> {
    let mut args = vec![
        "--dump-single-json".to_string(),
        "--no-playlist".to_string(),
        "--skip-download".to_string(),
        "--no-warnings".to_string(),
        "--no-colors".to_string(),
        "--no-check-certificates".to_string(),
        "--ignore-no-formats-error".to_string(),
        "--no-write-thumbnail".to_string(),
        "--no-write-subs".to_string(),
        "--no-write-auto-subs".to_string(),
        "--no-write-comments".to_string(),
        "--no-write-info-json".to_string(),
    ];
    if let Some(extractor_args) = extractor_args {
        append_youtube_extractor_args(&mut args, extractor_args);
    }
    append_ffmpeg_location(&mut args, app_handle);
    append_http_context(&mut args, ctx);
    args.push(url.to_string());

    let stdout = run_ytdlp_capture(args, app_handle).await?;
    let json_line = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with('{'))
        .ok_or_else(|| "yt-dlp did not return media information".to_string())?;
    let json: Value = serde_json::from_str(json_line)
        .map_err(|e| format!("Could not read yt-dlp media information: {}", e))?;
    log::info!("Raw yt-dlp -J output for {}:\n{}", url, json_line);

    Ok(json)
}

pub fn log_ffmpeg_availability(app_handle: Option<&AppHandle>) {
    if let Some(path) = local_ffmpeg_path(app_handle) {
        log::info!("FFmpeg found at {}", path.display());
    } else if ffmpeg_available(app_handle) {
        log::info!("FFmpeg found on PATH");
    } else {
        log::warn!("FFmpeg was not found. YouTube video-only formats will fail to merge audio.");
    }
}

pub async fn download_media(
    task_arc: Arc<RwLock<DownloadTask>>,
    cancel_token: Arc<Mutex<bool>>,
    app_handle: Option<AppHandle>,
    task_speed_limit: Arc<RwLock<Option<u64>>>,
) -> Result<(), String> {
    let (url, ctx, save_path, media_format) = {
        let task = task_arc.read().await;
        (
            task.url.clone(),
            task.http_context.clone(),
            PathBuf::from(task.save_path.clone()),
            task.media_format.clone(),
        )
    };

    let save_dir = save_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    tokio::fs::create_dir_all(&save_dir)
        .await
        .map_err(|e| format!("Could not create save folder: {}", e))?;

    let stem = save_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("media");
    let output_template = save_dir.join(format!("{}.%(ext)s", stem));

    let app_handle_ref = app_handle.as_ref();
    let can_merge = ffmpeg_available(app_handle_ref);
    let selected_format = media_format
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let is_audio_only = selected_format
        .as_deref()
        .map(|value| value.starts_with("audio:"))
        .unwrap_or(false);
    let format_selector = selected_format
        .as_deref()
        .map(|value| {
            if let Some(format_id) = value.strip_prefix("audio:") {
                if format_id.trim().is_empty() {
                    "bestaudio".to_string()
                } else {
                    format_id.to_string()
                }
            } else if let Some((format_id, height)) = selected_video_format(value) {
                build_video_fallback_selector(format_id, height)
            } else if value.contains("best") || value.contains('+') || value.contains('/') {
                value.to_string()
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| {
            if can_merge {
                "bestvideo[vcodec^=avc1]+bestaudio[ext=m4a]/bestvideo+bestaudio/best".to_string()
            } else {
                media_format_selector(can_merge).to_string()
            }
        });
    let mut args = vec![
        "--newline".to_string(),
        "--progress".to_string(),
        "--no-colors".to_string(),
        "--continue".to_string(),
        "--no-playlist".to_string(),
        "--no-warnings".to_string(),
        "--no-check-certificates".to_string(),
        "--no-write-thumbnail".to_string(),
        "--no-write-subs".to_string(),
        "--no-write-auto-subs".to_string(),
        "--no-write-comments".to_string(),
        "--no-write-info-json".to_string(),
        "--progress-template".to_string(),
        format!(
            "download:{} downloaded=%(progress.downloaded_bytes)s total=%(progress.total_bytes)s total_estimate=%(progress.total_bytes_estimate)s speed=%(progress.speed)s eta=%(progress.eta)s",
            PROGRESS_MARKER
        ),
        "--print".to_string(),
        format!("after_move:{}%(filepath)s", FILE_MARKER),
        "-f".to_string(),
        format_selector,
        "-o".to_string(),
        output_template.to_string_lossy().to_string(),
    ];
    append_youtube_extractor_args(&mut args, "youtube:player_client=web,ios,android");

    if is_audio_only {
        args.push("--extract-audio".to_string());
        args.push("--audio-format".to_string());
        args.push("mp3".to_string());
    } else {
        args.push("--merge-output-format".to_string());
        args.push("mp4".to_string());
    }
    append_ffmpeg_location(&mut args, app_handle_ref);

    if let Some(limit_bps) = *task_speed_limit.read().await {
        if limit_bps > 0 {
            let limit_kib = std::cmp::max(1, limit_bps / 1024);
            args.push("--limit-rate".to_string());
            args.push(format!("{}K", limit_kib));
        }
    }

    append_http_context(&mut args, &ctx);
    args.push(url);
    log::info!("Spawning yt-dlp with args: {:?}", args);

    emit_current_progress(&task_arc, app_handle_ref).await;

    let mut child = spawn_ytdlp(args, app_handle_ref).await?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read yt-dlp output".to_string())?;
    let stderr = child.stderr.take();
    let stderr_lines = Arc::new(Mutex::new(Vec::<String>::new()));

    if let Some(stderr) = stderr {
        let errors_for_stderr = stderr_lines.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    log::warn!("yt-dlp stderr: {}", trimmed);
                    let mut errors = errors_for_stderr.lock().await;
                    errors.push(trimmed.to_string());
                    if errors.len() > 20 {
                        errors.remove(0);
                    }
                }
            }
        });
    }

    let mut stdout_lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        process_output_line(&line, &task_arc, app_handle.as_ref()).await;
                    }
                    Ok(None) => break,
                    Err(e) => return Err(format!("Could not read yt-dlp output: {}", e)),
                }
            }
            _ = sleep(Duration::from_millis(200)) => {
                if *cancel_token.lock().await {
                    let _ = child.kill().await;
                    pause_media_task(&task_arc, app_handle.as_ref()).await;
                    return Ok(());
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("yt-dlp did not exit cleanly: {}", e))?;

    if *cancel_token.lock().await {
        pause_media_task(&task_arc, app_handle.as_ref()).await;
        return Ok(());
    }

    if !status.success() {
        let errors = stderr_lines.lock().await;
        let detail = errors
            .last()
            .cloned()
            .unwrap_or_else(|| format!("yt-dlp exited with {}", status));
        if detail
            .to_lowercase()
            .contains("requested format is not available")
        {
            return Err("Format unavailable, try a different quality".to_string());
        }
        return Err(detail);
    }

    finalize_media_task(&task_arc).await;
    Ok(())
}

fn media_info_from_json(json: &Value, can_merge: bool) -> MediaInfo {
    let title = json
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| json.get("fulltitle").and_then(Value::as_str))
        .unwrap_or("media");
    let ext = preferred_extension(json, can_merge);
    let filename = sanitize_component(&format!("{}.{}", title, ext));
    let filesize = requested_download_size(json)
        .or_else(|| read_u64(json.get("filesize")))
        .or_else(|| read_u64(json.get("filesize_approx")));

    MediaInfo {
        filename,
        filesize,
        content_type: Some("video/media".to_string()),
        formats: media_quality_options(json, can_merge),
        can_merge,
    }
}

fn media_quality_options(json: &Value, can_merge: bool) -> Vec<MediaFormatOption> {
    let mut options = Vec::new();
    options.push(MediaFormatOption {
        id: media_format_selector(can_merge).to_string(),
        label: if can_merge {
            "Best available".to_string()
        } else {
            "Best single-file".to_string()
        },
        height: None,
        ext: None,
        filesize: requested_download_size(json),
        requires_ffmpeg: false,
    });

    let mut formats = json
        .get("formats")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    formats.sort_by(|a, b| {
        read_u64(b.get("height"))
            .cmp(&read_u64(a.get("height")))
            .then_with(|| read_u64(b.get("fps")).cmp(&read_u64(a.get("fps"))))
            .then_with(|| format_size_value(b).cmp(&format_size_value(a)))
    });

    if can_merge {
        let best_audio_size = best_audio_size(&formats);
        let mut video_options = formats
            .iter()
            .filter(|format| read_u64(format.get("height")).is_some())
            .filter(|format| has_video(format))
            .filter(|format| !is_unavailable_placeholder(format))
            .filter_map(|format| video_option_from_format(format, best_audio_size))
            .collect::<Vec<_>>();
        dedupe_video_options_by_height(&mut video_options);
        options.extend(video_options);

        let mut audio_options = formats
            .iter()
            .filter(|format| !has_video(format) && has_audio(format))
            .filter(|format| !is_unavailable_placeholder(format))
            .filter_map(audio_option_from_format)
            .collect::<Vec<_>>();
        dedupe_format_options(&mut audio_options);
        options.extend(audio_options);
    } else {
        let mut combined = formats
            .iter()
            .filter(|format| read_u64(format.get("height")).is_some())
            .filter(|format| has_video(format))
            .filter(|format| !is_unavailable_placeholder(format))
            .filter_map(|format| video_option_from_format(format, None))
            .collect::<Vec<_>>();
        dedupe_video_options_by_height(&mut combined);
        options.extend(combined);

        let mut audio_options = formats
            .iter()
            .filter(|format| !has_video(format) && has_audio(format))
            .filter(|format| !is_unavailable_placeholder(format))
            .filter_map(audio_option_from_format)
            .collect::<Vec<_>>();
        dedupe_format_options(&mut audio_options);
        options.extend(audio_options);
    }

    let total_formats = formats.len();
    let video_candidates = formats
        .iter()
        .filter(|format| read_u64(format.get("height")).is_some())
        .filter(|format| has_video(format))
        .count();
    log::info!(
        "yt-dlp format parse: total_formats={}, video_candidates={}, output_options={}, can_merge={}",
        total_formats,
        video_candidates,
        options.len(),
        can_merge
    );
    if video_candidates <= 1 {
        log::warn!(
            "yt-dlp returned only {} video format(s). Format summary: {}",
            video_candidates,
            summarize_formats(&formats)
        );
    }

    options
}

fn video_option_from_format(
    format: &Value,
    best_audio_size: Option<u64>,
) -> Option<MediaFormatOption> {
    let id = format.get("format_id")?.as_str()?.to_string();
    let height = read_u64(format.get("height"))?;
    let ext = format
        .get("ext")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let own_size = format_size_value(format);
    let filesize = if !has_audio(format) {
        own_size.and_then(|video_size| {
            best_audio_size.map(|audio_size| video_size.saturating_add(audio_size))
        })
    } else {
        own_size
    };

    let vcodec = codec_label(format.get("vcodec"));

    let mut label = format!("{}p ({})", height, vcodec);
    if let Some(filesize) = filesize {
        label.push_str(&format!(" - {}", format_size(filesize)));
    } else {
        label.push_str(" - Unknown");
    }

    Some(MediaFormatOption {
        id: format!("video:{}:{}", id, height),
        label,
        height: Some(height),
        ext,
        filesize,
        requires_ffmpeg: !has_audio(format),
    })
}

fn audio_option_from_format(format: &Value) -> Option<MediaFormatOption> {
    let id = format.get("format_id")?.as_str()?.to_string();
    let ext = format
        .get("ext")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let filesize = format_size_value(format);
    let acodec = codec_label(format.get("acodec"));
    let mut label = format!("Audio only ({})", acodec);
    if let Some(filesize) = filesize {
        label.push_str(&format!(" - {}", format_size(filesize)));
    } else {
        label.push_str(" - Unknown");
    }

    Some(MediaFormatOption {
        id: format!("audio:{}", id),
        label,
        height: None,
        ext,
        filesize,
        requires_ffmpeg: false,
    })
}

fn format_size_value(format: &Value) -> Option<u64> {
    read_u64(format.get("filesize")).or_else(|| read_u64(format.get("filesize_approx")))
}

fn is_unavailable_placeholder(format: &Value) -> bool {
    read_u64(format.get("filesize")) == Some(0) && read_u64(format.get("filesize_approx")).is_none()
}

fn has_video(format: &Value) -> bool {
    format
        .get("vcodec")
        .and_then(Value::as_str)
        .map(|codec| codec != "none")
        .unwrap_or(false)
}

fn has_audio(format: &Value) -> bool {
    format
        .get("acodec")
        .and_then(Value::as_str)
        .map(|codec| codec != "none")
        .unwrap_or(false)
}

fn dedupe_format_options(options: &mut Vec<MediaFormatOption>) {
    let mut seen = Vec::<String>::new();
    options.retain(|option| {
        let key = option.id.clone();
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn dedupe_video_options_by_height(options: &mut Vec<MediaFormatOption>) {
    let mut seen = Vec::<u64>::new();
    options.retain(|option| {
        let Some(height) = option.height else {
            return true;
        };

        if seen.contains(&height) {
            false
        } else {
            seen.push(height);
            true
        }
    });
}

fn best_audio_size(formats: &[Value]) -> Option<u64> {
    formats
        .iter()
        .filter(|format| has_audio(format) && !has_video(format))
        .filter_map(format_size_value)
        .max()
}

fn codec_label(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .filter(|codec| !codec.trim().is_empty() && *codec != "none")
        .map(|codec| codec.split('.').next().unwrap_or(codec).to_uppercase())
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

fn format_size(bytes: u64) -> String {
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
        format!("{:.1} {}", size, UNITS[unit])
    }
}

fn preferred_extension(json: &Value, can_merge: bool) -> String {
    let vcodec = json
        .get("vcodec")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if can_merge && vcodec != "none" {
        return "mp4".to_string();
    }

    json.get("ext")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("mp4")
        .to_string()
}

fn requested_download_size(json: &Value) -> Option<u64> {
    let downloads = json.get("requested_downloads")?.as_array()?;
    let mut total = 0u64;
    for item in downloads {
        if let Some(size) =
            read_u64(item.get("filesize")).or_else(|| read_u64(item.get("filesize_approx")))
        {
            total = total.saturating_add(size);
        }
    }
    if total > 0 {
        Some(total)
    } else {
        None
    }
}

fn read_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|v| v as u64)),
        Value::String(text) => parse_number(text).map(|v| v as u64),
        _ => None,
    }
}

fn sanitize_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = sanitized.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "media.mp4".to_string()
    } else {
        trimmed
    }
}

fn append_http_context(args: &mut Vec<String>, ctx: &HttpContext) {
    if let Some(value) = non_empty(&ctx.cookies) {
        args.push("--add-header".to_string());
        args.push(format!("Cookie: {}", value));
    }

    if let Some(value) = non_empty(&ctx.referer) {
        args.push("--referer".to_string());
        args.push(value.to_string());
        args.push("--add-header".to_string());
        args.push(format!("Referer: {}", value));
    }

    if let Some(value) = non_empty(&ctx.user_agent) {
        args.push("--user-agent".to_string());
        args.push(value.to_string());
    }
}

fn media_format_selector(can_merge: bool) -> &'static str {
    if can_merge {
        "bestvideo+bestaudio/best"
    } else {
        "best[ext=mp4]/best"
    }
}

fn selected_video_format(value: &str) -> Option<(&str, Option<u64>)> {
    let value = value
        .strip_prefix("video:")
        .or_else(|| value.strip_prefix("video-only:"))?;
    let Some((format_id, height)) = value.rsplit_once(':') else {
        return Some((value, None));
    };
    let height = height.trim().parse::<u64>().ok();
    Some((format_id, height))
}

fn build_video_fallback_selector(format_id: &str, height: Option<u64>) -> String {
    let format_id = format_id.trim();
    let requested = if format_id.is_empty() {
        "bestvideo+bestaudio".to_string()
    } else {
        format!("{}+bestaudio[ext=m4a]/{}+bestaudio", format_id, format_id)
    };

    if let Some(height) = height {
        format!(
            "{}/bestvideo[height<={}][vcodec^=avc1]+bestaudio[ext=m4a]/bestvideo[height<={}]+bestaudio/bestvideo+bestaudio/best",
            requested, height, height
        )
    } else {
        format!("{}/bestvideo+bestaudio/best", requested)
    }
}

fn append_youtube_extractor_args(args: &mut Vec<String>, value: &str) {
    args.push("--extractor-args".to_string());
    args.push(value.to_string());
}

fn is_youtube_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    host == "youtube.com"
        || host.ends_with(".youtube.com")
        || host == "youtu.be"
        || host.ends_with(".youtu.be")
}

fn json_video_candidate_count(json: &Value) -> usize {
    json.get("formats")
        .and_then(Value::as_array)
        .map(|formats| {
            formats
                .iter()
                .filter(|format| read_u64(format.get("height")).is_some())
                .filter(|format| has_video(format))
                .count()
        })
        .unwrap_or_default()
}

fn append_ffmpeg_location(args: &mut Vec<String>, app_handle: Option<&AppHandle>) {
    if let Some(path) = local_ffmpeg_path(app_handle) {
        let location = path
            .parent()
            .unwrap_or_else(|| path.as_path())
            .to_string_lossy()
            .to_string();
        args.push("--ffmpeg-location".to_string());
        args.push(location);
    }
}

fn summarize_formats(formats: &[Value]) -> String {
    formats
        .iter()
        .take(80)
        .map(|format| {
            let id = format
                .get("format_id")
                .and_then(Value::as_str)
                .unwrap_or("-");
            let height = read_u64(format.get("height"))
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let ext = format.get("ext").and_then(Value::as_str).unwrap_or("-");
            let vcodec = format.get("vcodec").and_then(Value::as_str).unwrap_or("-");
            let acodec = format.get("acodec").and_then(Value::as_str).unwrap_or("-");
            format!(
                "id={}, height={}, ext={}, vcodec={}, acodec={}",
                id, height, ext, vcodec, acodec
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn ffmpeg_available(app_handle: Option<&AppHandle>) -> bool {
    if local_ffmpeg_path(app_handle).is_some() {
        return true;
    }

    let mut command = std::process::Command::new("ffmpeg");
    apply_no_window_std(&mut command);
    command
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn local_ffmpeg_path(app_handle: Option<&AppHandle>) -> Option<PathBuf> {
    for dir in tool_dirs(app_handle) {
        for name in ["ffmpeg.exe", "ffmpeg"] {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

async fn run_ytdlp_capture(
    args: Vec<String>,
    app_handle: Option<&AppHandle>,
) -> Result<String, String> {
    let mut last_error = None;
    for tool in tool_candidates(app_handle) {
        let mut command = command_from_tool(&tool);
        command.args(&args);

        match command.output().await {
            Ok(output) if output.status.success() => {
                return String::from_utf8(output.stdout)
                    .map_err(|e| format!("yt-dlp returned invalid text: {}", e));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return Err(if detail.is_empty() {
                    format!("yt-dlp exited with {}", output.status)
                } else {
                    detail
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last_error = Some(e.to_string());
                continue;
            }
            Err(e) => {
                return Err(format!("Could not start yt-dlp: {}", e));
            }
        }
    }

    Err(format!(
        "yt-dlp was not found. Install yt-dlp, place yt-dlp.exe next to Velocity Downloader, or set VELOCITY_YTDLP.{}",
        last_error
            .map(|e| format!(" Last error: {}", e))
            .unwrap_or_default()
    ))
}

async fn spawn_ytdlp(args: Vec<String>, app_handle: Option<&AppHandle>) -> Result<Child, String> {
    let mut last_error = None;
    for tool in tool_candidates(app_handle) {
        let mut command = command_from_tool(&tool);
        command.args(&args);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last_error = Some(e.to_string());
                continue;
            }
            Err(e) => return Err(format!("Could not start yt-dlp: {}", e)),
        }
    }

    Err(format!(
        "yt-dlp was not found. Install yt-dlp, place yt-dlp.exe next to Velocity Downloader, or set VELOCITY_YTDLP.{}",
        last_error
            .map(|e| format!(" Last error: {}", e))
            .unwrap_or_default()
    ))
}

fn command_from_tool(tool: &ToolCommand) -> Command {
    let mut command = Command::new(&tool.program);
    apply_no_window(&mut command);
    command.args(&tool.base_args);
    command
}

fn apply_no_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

fn apply_no_window_std(command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

fn tool_candidates(app_handle: Option<&AppHandle>) -> Vec<ToolCommand> {
    let mut tools = Vec::new();

    if let Ok(path) = std::env::var("VELOCITY_YTDLP") {
        let path = path.trim().trim_matches('"');
        if !path.is_empty() {
            tools.push(ToolCommand {
                program: path.to_string(),
                base_args: Vec::new(),
            });
        }
    }

    for path in local_ytdlp_paths(app_handle) {
        tools.push(ToolCommand {
            program: path.to_string_lossy().to_string(),
            base_args: Vec::new(),
        });
    }

    #[cfg(target_os = "windows")]
    tools.push(ToolCommand {
        program: "yt-dlp.exe".to_string(),
        base_args: Vec::new(),
    });

    #[cfg(not(target_os = "windows"))]
    tools.push(ToolCommand {
        program: "yt-dlp".to_string(),
        base_args: Vec::new(),
    });

    tools.push(ToolCommand {
        program: "python".to_string(),
        base_args: vec!["-m".to_string(), "yt_dlp".to_string()],
    });

    #[cfg(target_os = "windows")]
    tools.push(ToolCommand {
        program: "py".to_string(),
        base_args: vec!["-m".to_string(), "yt_dlp".to_string()],
    });

    tools
}

fn local_ytdlp_paths(app_handle: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in tool_dirs(app_handle) {
        for name in ["yt-dlp.exe", "yt-dlp"] {
            let candidate = dir.join(name);
            if candidate.exists() {
                paths.push(candidate);
            }
        }
    }
    paths
}

fn tool_dirs(app_handle: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
            dirs.push(parent.join("bin"));
            if let Some(project_dir) = parent
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
            {
                dirs.push(project_dir.to_path_buf());
                dirs.push(project_dir.join("bin"));
            }
        }
    }

    if let Ok(current) = std::env::current_dir() {
        dirs.push(current.clone());
        dirs.push(current.join("bin"));
        if let Some(parent) = current.parent() {
            dirs.push(parent.to_path_buf());
            dirs.push(parent.join("bin"));
        }
    }

    if let Some(app) = app_handle {
        if let Ok(resource_dir) = app.path().resource_dir() {
            dirs.push(resource_dir.clone());
            dirs.push(resource_dir.join("bin"));
        }
    }

    dirs
}

async fn process_output_line(
    line: &str,
    task_arc: &Arc<RwLock<DownloadTask>>,
    app_handle: Option<&AppHandle>,
) -> bool {
    if let Some(index) = line.find(FILE_MARKER) {
        let path = line[index + FILE_MARKER.len()..].trim();
        if !path.is_empty() {
            update_media_file_path(task_arc, path).await;
            return true;
        }
    }

    if let Some(progress) = parse_progress_line(line) {
        update_media_progress(task_arc, app_handle, progress).await;
        return true;
    }

    false
}

async fn update_media_file_path(task_arc: &Arc<RwLock<DownloadTask>>, path: &str) {
    let mut task = task_arc.write().await;
    task.save_path = path.to_string();
    if let Some(name) = Path::new(path).file_name().and_then(|value| value.to_str()) {
        task.filename = name.to_string();
    }
}

fn parse_progress_line(line: &str) -> Option<MediaProgress> {
    if let Some(progress) = parse_ytdlp_download_line(line) {
        return Some(progress);
    }

    let marker_index = line.find(PROGRESS_MARKER)?;
    let values = &line[marker_index + PROGRESS_MARKER.len()..];
    let mut progress = MediaProgress::default();

    for part in values.split_whitespace() {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };

        match key {
            "downloaded" => progress.downloaded = parse_number(value).map(|value| value as u64),
            "total" => progress.total = parse_number(value).map(|value| value as u64),
            "total_estimate" => {
                progress.total_estimate = parse_number(value).map(|value| value as u64)
            }
            "speed" => progress.speed = parse_number(value),
            "eta" => progress.eta = parse_number(value),
            _ => {}
        }
    }

    Some(progress)
}

fn parse_ytdlp_download_line(line: &str) -> Option<MediaProgress> {
    let line = line.trim();
    let rest = line.strip_prefix("[download]")?.trim();
    let percent_end = rest.find('%')?;
    let percent = parse_number(&rest[..percent_end])?;

    let mut progress = MediaProgress {
        percent: Some(percent),
        ..MediaProgress::default()
    };

    if let Some(of_index) = rest.find(" of ") {
        let after_of = &rest[of_index + 4..];
        let size_text = after_of
            .split(" at ")
            .next()
            .or_else(|| after_of.split(" ETA ").next())
            .unwrap_or("")
            .trim();
        progress.total = parse_human_bytes(size_text);
    }

    if let Some(at_index) = rest.find(" at ") {
        let after_at = &rest[at_index + 4..];
        let speed_text = after_at
            .split(" ETA ")
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches("/s")
            .trim_end_matches("/sec");
        progress.speed = parse_human_bytes(speed_text).map(|value| value as f64);
    }

    if let Some(eta_index) = rest.find(" ETA ") {
        let eta_text = rest[eta_index + 5..].trim();
        progress.eta = parse_eta_text(eta_text);
    }

    Some(progress)
}

fn parse_human_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") || value == "~" {
        return None;
    }

    let split_index = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(value.len());
    let number = parse_number(&value[..split_index])?;
    let unit = value[split_index..].trim().to_lowercase();
    let multiplier = if unit.starts_with("kib") || unit.starts_with("kb") {
        1024.0
    } else if unit.starts_with("mib") || unit.starts_with("mb") {
        1024.0 * 1024.0
    } else if unit.starts_with("gib") || unit.starts_with("gb") {
        1024.0 * 1024.0 * 1024.0
    } else if unit.starts_with("tib") || unit.starts_with("tb") {
        1024.0 * 1024.0 * 1024.0 * 1024.0
    } else {
        1.0
    };

    Some((number * multiplier) as u64)
}

fn parse_eta_text(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") || value == "--:--" {
        return None;
    }

    let parts = value
        .split(':')
        .filter_map(|part| part.parse::<u64>().ok())
        .collect::<Vec<_>>();

    match parts.as_slice() {
        [seconds] => Some(*seconds as f64),
        [minutes, seconds] => Some((minutes * 60 + seconds) as f64),
        [hours, minutes, seconds] => Some((hours * 3600 + minutes * 60 + seconds) as f64),
        _ => None,
    }
}

fn parse_number(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("na")
        || value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("null")
        || value == "?"
    {
        return None;
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
}

async fn update_media_progress(
    task_arc: &Arc<RwLock<DownloadTask>>,
    app_handle: Option<&AppHandle>,
    progress: MediaProgress,
) {
    let event = {
        let mut task = task_arc.write().await;
        let reported_total = progress.total.or(progress.total_estimate);
        if let Some(total) = reported_total {
            if total > 0 && task.total_size > 0 && total < task.total_size {
                log::info!(
                    "Ignoring smaller yt-dlp progress phase total: {} bytes while task total is {} bytes",
                    total,
                    task.total_size
                );
            } else if total > 0 {
                task.total_size = total;
            }
        }

        let next_downloaded = if let Some(downloaded) = progress.downloaded {
            Some(downloaded)
        } else if let Some(percent) = progress.percent {
            let total = task.total_size;
            if total > 0 {
                Some(
                    ((percent / 100.0) * total as f64)
                        .round()
                        .clamp(0.0, total as f64) as u64,
                )
            } else {
                None
            }
        } else {
            None
        };

        if let Some(downloaded) = next_downloaded {
            if downloaded < task.downloaded {
                log::info!(
                    "Ignoring decreasing yt-dlp progress: {} bytes after {} bytes",
                    downloaded,
                    task.downloaded
                );
                return;
            }
            task.downloaded = downloaded;
        }
        if let Some(speed) = progress.speed {
            task.speed_bps = speed;
        }
        if let Some(eta) = progress.eta {
            task.eta_seconds = eta;
        } else if task.speed_bps > 0.0 && task.total_size > 0 {
            let remaining = task.total_size.saturating_sub(task.downloaded);
            task.eta_seconds = remaining as f64 / task.speed_bps;
        }
        task.status = DownloadStatus::Downloading;

        let total_size = task.total_size;
        let downloaded = task.downloaded;
        let speed_bps = task.speed_bps;
        if let Some(segment) = task.segments.get_mut(0) {
            segment.end_byte = total_size.saturating_sub(1);
            segment.downloaded = downloaded;
            segment.speed_bps = speed_bps;
            segment.status = SegmentStatus::Downloading;
        }

        task.updated_at = chrono::Utc::now();
        let event = progress_event_from_task(&task);
        let _ = StateManager::save_state(&task).await;
        event
    };

    if let Some(app_handle) = app_handle {
        let _ = app_handle.emit("download-progress", &event);
    }
}

async fn pause_media_task(task_arc: &Arc<RwLock<DownloadTask>>, app_handle: Option<&AppHandle>) {
    let event = {
        let mut task = task_arc.write().await;
        task.status = DownloadStatus::Paused;
        task.speed_bps = 0.0;
        task.eta_seconds = 0.0;
        for segment in &mut task.segments {
            if segment.status != SegmentStatus::Completed {
                segment.status = SegmentStatus::Paused;
                segment.speed_bps = 0.0;
            }
        }
        task.updated_at = chrono::Utc::now();
        let event = progress_event_from_task(&task);
        let _ = StateManager::save_state(&task).await;
        event
    };

    if let Some(app_handle) = app_handle {
        let _ = app_handle.emit("download-progress", &event);
    }
}

async fn emit_current_progress(
    task_arc: &Arc<RwLock<DownloadTask>>,
    app_handle: Option<&AppHandle>,
) {
    let event = {
        let task = task_arc.read().await;
        progress_event_from_task(&task)
    };
    if let Some(app_handle) = app_handle {
        let _ = app_handle.emit("download-progress", &event);
    }
}

async fn finalize_media_task(task_arc: &Arc<RwLock<DownloadTask>>) {
    let mut task = task_arc.write().await;
    if let Ok(metadata) = tokio::fs::metadata(&task.save_path).await {
        let len = metadata.len();
        if len > 0 {
            task.total_size = len;
            task.downloaded = len;
        }
    } else if task.total_size == 0 {
        task.total_size = task.downloaded;
    }

    let total_size = task.total_size;
    let downloaded = task.downloaded;
    if let Some(segment) = task.segments.get_mut(0) {
        segment.end_byte = total_size.saturating_sub(1);
        segment.downloaded = downloaded;
        segment.speed_bps = 0.0;
        segment.status = SegmentStatus::Completed;
    }
    task.speed_bps = 0.0;
    task.eta_seconds = 0.0;
    task.updated_at = chrono::Utc::now();
}

fn progress_event_from_task(task: &DownloadTask) -> ProgressEvent {
    ProgressEvent {
        download_id: task.id.clone(),
        total_size: task.total_size,
        downloaded: task.downloaded,
        speed_bps: task.speed_bps,
        eta_seconds: task.eta_seconds,
        status: task.status.clone(),
        speed_limit_bps: task.speed_limit_bps,
        segments: task
            .segments
            .iter()
            .map(|segment| SegmentProgress {
                id: segment.id,
                downloaded: segment.downloaded,
                total_size: segment.total_size(),
                speed_bps: segment.speed_bps,
                status: segment.status.clone(),
                progress: segment.progress(),
            })
            .collect(),
    }
}
