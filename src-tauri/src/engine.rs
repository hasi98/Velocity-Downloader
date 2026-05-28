use crate::models::*;
use futures_util::StreamExt;
use reqwest::Client;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};

const MIN_SEGMENT_SIZE: u64 = 1 * 1024 * 1024; // 1 MB minimum per segment

/// Shared per-download throttle. Multi-segment downloads must use one shared
/// limiter so the configured speed is not multiplied by the connection count.
pub struct SharedSpeedLimiter {
    next_available: Mutex<Instant>,
}

impl SharedSpeedLimiter {
    pub fn new() -> Self {
        Self {
            next_available: Mutex::new(Instant::now()),
        }
    }

    pub async fn wait(&self, bytes: u64, task_speed_limit: &Arc<RwLock<Option<u64>>>) {
        let Some(limit_bps) = *task_speed_limit.read().await else {
            return;
        };

        if limit_bps == 0 || bytes == 0 {
            return;
        }

        let now = Instant::now();
        let delay = std::time::Duration::from_secs_f64(bytes as f64 / limit_bps as f64);
        let sleep_for = {
            let mut next_available = self.next_available.lock().await;
            let base = if *next_available > now {
                *next_available
            } else {
                now
            };
            *next_available = base + delay;
            next_available.saturating_duration_since(now)
        };

        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for).await;
        }
    }
}

/// The core download engine
pub struct DownloadEngine {
    client: Client,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DownloadEngineConfig {
    pub large_file_mode: bool,
}

impl DownloadEngine {
    pub fn new() -> Self {
        Self::new_with_config(DownloadEngineConfig::default())
    }

    pub fn new_with_config(config: DownloadEngineConfig) -> Self {
        let _large_file_mode = config.large_file_mode;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"));
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("en-US,en;q=0.5"),
        );
        headers.insert(
            "Sec-Fetch-Dest",
            reqwest::header::HeaderValue::from_static("document"),
        );
        headers.insert(
            "Sec-Fetch-Mode",
            reqwest::header::HeaderValue::from_static("navigate"),
        );
        headers.insert(
            "Sec-Fetch-Site",
            reqwest::header::HeaderValue::from_static("none"),
        );
        headers.insert(
            "Sec-Fetch-User",
            reqwest::header::HeaderValue::from_static("?1"),
        );
        headers.insert(
            "Upgrade-Insecure-Requests",
            reqwest::header::HeaderValue::from_static("1"),
        );

        // Build a client with cookie store and browser-like headers
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .cookie_store(true)
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none()) // We manually handle it to preserve explicit headers
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    /// Apply HttpContext headers to a request builder.
    fn apply_context(
        builder: reqwest::RequestBuilder,
        ctx: &HttpContext,
    ) -> reqwest::RequestBuilder {
        let mut b = builder;
        if let Some(ref cookies) = ctx.cookies {
            if !cookies.is_empty() {
                b = b.header("Cookie", cookies);
            }
        }
        if let Some(ref referer) = ctx.referer {
            if !referer.is_empty() {
                b = b.header("Referer", referer);
            }
        }
        if let Some(ref ua) = ctx.user_agent {
            if !ua.is_empty() {
                // Override the default client-level User-Agent
                b = b.header("User-Agent", ua);
            }
        }
        b
    }

    /// Helper to fetch a request with manual redirect following up to 10 times.
    /// This is necessary because reqwest strips explicit headers (like Cookie, Referer) across domains.
    async fn fetch_with_redirect(
        client: &Client,
        url: &str,
        method: reqwest::Method,
        ctx: &HttpContext,
        range: Option<&str>,
    ) -> Result<reqwest::Response, String> {
        let mut current_url = url.to_string();
        for _ in 0..10 {
            let parsed_url = reqwest::Url::parse(&current_url)
                .map_err(|e| format!("Invalid URL {}: {}", current_url, e))?;
            let mut builder = client.request(method.clone(), parsed_url);
            builder = Self::apply_context(builder, ctx);
            if let Some(r) = range {
                builder = builder.header("Range", r);
            }
            let response = builder
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;

            if response.status().is_redirection() {
                if let Some(loc) = response.headers().get("location") {
                    let loc_str = loc.to_str().unwrap_or_default();
                    if loc_str.starts_with("http") {
                        current_url = loc_str.to_string();
                    } else if loc_str.starts_with('/') {
                        let mut base_url = reqwest::Url::parse(&current_url).unwrap();
                        base_url.set_path(loc_str);
                        current_url = base_url.to_string();
                    } else {
                        // Unhandled relative redirect (rare), return the response to let caller handle failure
                        return Ok(response);
                    }
                    continue;
                }
            }
            return Ok(response);
        }
        Err("Too many redirects".to_string())
    }

    /// Probe a URL to detect Range support and get file metadata.
    /// Strategy: Use GET with Range: bytes=0-0 instead of HEAD (more reliable).
    /// Falls back to HEAD if GET probe fails.
    pub async fn probe_url(
        &self,
        url: &str,
        ctx: &HttpContext,
    ) -> Result<(u64, bool, Option<String>, String), String> {
        // Strategy 1: GET with Range: bytes=0-0
        // This is far more reliable than HEAD for file hosts like GoFile, MediaFire, etc.
        let probe_result = self.probe_with_range_get(url, ctx).await;

        if let Ok(result) = probe_result {
            return Ok(result);
        }

        // Strategy 2: Fallback to HEAD
        let head_result = self.probe_with_head(url, ctx).await;

        if let Ok(result) = head_result {
            return Ok(result);
        }

        // Strategy 3: Plain GET to at least get filename and content-type
        // (we won't know size or range support, but download can still proceed)
        let get_result = self.probe_with_get(url, ctx).await;

        if let Ok(result) = get_result {
            return Ok(result);
        }

        Err(format!(
            "Failed to probe URL. Range GET: {:?}, HEAD: {:?}, GET: {:?}",
            probe_result.err(),
            head_result.err(),
            get_result.err()
        ))
    }

    /// Probe using GET with Range: bytes=0-0
    /// This returns Content-Range: bytes 0-0/TOTAL_SIZE which gives us the real file size
    async fn probe_with_range_get(
        &self,
        url: &str,
        ctx: &HttpContext,
    ) -> Result<(u64, bool, Option<String>, String), String> {
        let response = Self::fetch_with_redirect(
            &self.client,
            url,
            reqwest::Method::GET,
            ctx,
            Some("bytes=0-0"),
        )
        .await?;

        let status = response.status().as_u16();
        let final_url = response.url().to_string();

        // 206 = server supports Range! Parse Content-Range for total size
        if status == 206 {
            let total_size = Self::parse_content_range_total(response.headers())
                .or_else(|| {
                    response
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .unwrap_or(0);

            let content_type = Self::get_content_type(&response);
            let filename = Self::extract_filename(&response, &final_url);

            log::info!(
                "Range GET probe success: size={}, supports_range=true, filename={}",
                total_size,
                filename
            );

            return Ok((total_size, total_size > 0, content_type, filename));
        }

        // 200 = server ignored Range header (doesn't support it or chunky stream)
        if status == 200 {
            let content_length = response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);

            let content_type = Self::get_content_type(&response);
            let filename = Self::extract_filename(&response, &final_url);

            log::info!(
                "Range GET probe: server returned 200 (no range support), size={}, filename={}",
                content_length,
                filename
            );

            return Ok((content_length, false, content_type, filename));
        }

        Err(format!("Range GET returned status {}", status))
    }

    /// Probe using HEAD request (fallback)
    async fn probe_with_head(
        &self,
        url: &str,
        ctx: &HttpContext,
    ) -> Result<(u64, bool, Option<String>, String), String> {
        let response =
            Self::fetch_with_redirect(&self.client, url, reqwest::Method::HEAD, ctx, None).await?;

        if !response.status().is_success() {
            return Err(format!("HEAD returned status: {}", response.status()));
        }

        let final_url = response.url().to_string();

        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let accept_ranges = response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_lowercase())
            .unwrap_or_default();

        let supports_range = accept_ranges == "bytes" && content_length > 0;
        let content_type = Self::get_content_type(&response);
        let filename = Self::extract_filename(&response, &final_url);

        log::info!(
            "HEAD probe: size={}, supports_range={}, filename={}",
            content_length,
            supports_range,
            filename
        );

        Ok((content_length, supports_range, content_type, filename))
    }

    /// Probe using plain GET (last resort - just grab headers, abort body)
    async fn probe_with_get(
        &self,
        url: &str,
        ctx: &HttpContext,
    ) -> Result<(u64, bool, Option<String>, String), String> {
        let response =
            Self::fetch_with_redirect(&self.client, url, reqwest::Method::GET, ctx, None).await?;

        if !response.status().is_success() {
            return Err(format!("GET returned status: {}", response.status()));
        }

        let final_url = response.url().to_string();

        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let content_type = Self::get_content_type(&response);
        let filename = Self::extract_filename(&response, &final_url);

        log::info!(
            "Plain GET probe: size={}, filename={}",
            content_length,
            filename
        );

        // Don't download the body, just abort
        drop(response);

        Ok((content_length, false, content_type, filename))
    }

    /// Parse Content-Range header to extract total file size
    /// Format: "bytes 0-0/12345" or "bytes 0-1048575/12345678"
    fn parse_content_range_total(headers: &reqwest::header::HeaderMap) -> Option<u64> {
        let cr = headers.get("content-range")?;
        let cr_str = cr.to_str().ok()?;
        // "bytes 0-0/TOTAL" -> extract TOTAL
        let slash_pos = cr_str.rfind('/')?;
        let total_str = &cr_str[slash_pos + 1..];
        if total_str == "*" {
            return None; // Unknown total
        }
        total_str.trim().parse::<u64>().ok()
    }

    fn get_content_type(response: &reqwest::Response) -> Option<String> {
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    fn extract_filename(response: &reqwest::Response, url: &str) -> String {
        // Try Content-Disposition header first
        if let Some(cd) = response.headers().get("content-disposition") {
            if let Ok(cd_str) = cd.to_str() {
                if let Some(fname) = cd_str.split(';').find_map(|part| {
                    let trimmed = part.trim();
                    if trimmed.starts_with("filename*=") {
                        // RFC 5987 encoded filename (higher priority)
                        let name = trimmed.splitn(2, '=').nth(1)?;
                        // Handle UTF-8''filename format
                        if let Some(utf8_name) = name
                            .strip_prefix("UTF-8''")
                            .or_else(|| name.strip_prefix("utf-8''"))
                        {
                            return Some(
                                urlencoding::decode(utf8_name)
                                    .unwrap_or_else(|_| utf8_name.into())
                                    .to_string(),
                            );
                        }
                        Some(name.trim_matches('"').trim_matches('\'').to_string())
                    } else if trimmed.starts_with("filename=") {
                        let name = trimmed
                            .splitn(2, '=')
                            .nth(1)?
                            .trim_matches('"')
                            .trim_matches('\'');
                        Some(name.to_string())
                    } else {
                        None
                    }
                }) {
                    if !fname.is_empty() {
                        return fname;
                    }
                }
            }
        }

        // Fall back to URL path (use final URL after redirects)
        let final_url = response.url().as_str();
        let url_to_parse = if !final_url.is_empty() {
            final_url
        } else {
            url
        };

        url::Url::parse(url_to_parse)
            .ok()
            .and_then(|u| {
                u.path_segments()?.last().map(|s| {
                    urlencoding::decode(s)
                        .unwrap_or_else(|_| s.into())
                        .to_string()
                })
            })
            .filter(|s| !s.is_empty() && s != "/")
            .unwrap_or_else(|| "download".to_string())
    }

    /// Calculate optimal number of segments
    pub fn calculate_segments(total_size: u64, preferred: usize) -> usize {
        if total_size == 0 {
            return 1;
        }

        let max_by_size = (total_size / MIN_SEGMENT_SIZE) as usize;
        let segments = preferred.min(16).max(1);
        segments.min(max_by_size).max(1)
    }

    /// Create segments for a download task
    pub fn create_segments(total_size: u64, num_segments: usize, temp_dir: &str) -> Vec<Segment> {
        let segment_size = total_size / num_segments as u64;
        let mut segments = Vec::with_capacity(num_segments);

        for i in 0..num_segments {
            let start = i as u64 * segment_size;
            let end = if i == num_segments - 1 {
                total_size - 1
            } else {
                (i as u64 + 1) * segment_size - 1
            };
            segments.push(Segment::new(i, start, end, temp_dir));
        }

        segments
    }

    /// Download a single segment with Range header
    pub async fn download_segment(
        client: Client,
        url: String,
        ctx: HttpContext,
        segment: Arc<RwLock<Segment>>,
        cancel_token: Arc<Mutex<bool>>,
        progress_callback: Arc<dyn Fn(usize, u64, f64) + Send + Sync>,
        task_speed_limit: Arc<RwLock<Option<u64>>>,
        speed_limiter: Arc<SharedSpeedLimiter>,
    ) -> Result<(), String> {
        let (start_byte, end_byte, already_downloaded, temp_file, seg_id) = {
            let seg = segment.read().await;
            (
                seg.start_byte + seg.downloaded,
                seg.end_byte,
                seg.downloaded,
                seg.temp_file.clone(),
                seg.id,
            )
        };

        let expected_total = end_byte - (start_byte - already_downloaded) + 1;

        // Skip completed segments
        if already_downloaded >= expected_total {
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Completed;
            log::info!(
                "Segment {} already completed ({}/{})",
                seg_id,
                already_downloaded,
                expected_total
            );
            return Ok(());
        }

        {
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Downloading;
        }

        let range = format!("bytes={}-{}", start_byte, end_byte);
        log::info!("Segment {} requesting range: {}", seg_id, range);

        let response =
            Self::fetch_with_redirect(&client, &url, reqwest::Method::GET, &ctx, Some(&range))
                .await
                .map_err(|e| format!("Segment {} download failed: {}", seg_id, e))?;

        let status = response.status().as_u16();

        // Check for proper response
        if status == 206 {
            // Perfect - server supports Range
            log::info!("Segment {} got 206 Partial Content", seg_id);
        } else if status == 200 {
            // Server ignored Range header and is sending the full file
            // This is a problem for multi-segment downloads
            log::warn!(
                "Segment {} got 200 instead of 206 - server ignoring Range header!",
                seg_id
            );
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Failed;
            return Err(format!(
                "Server does not support Range requests (returned 200 instead of 206). Cannot do multi-segment download."
            ));
        } else if !response.status().is_success() {
            let body_preview = response.text().await.unwrap_or_default();
            let preview = if body_preview.len() > 200 {
                &body_preview[..200]
            } else {
                &body_preview
            };
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Failed;
            return Err(format!(
                "Segment {} server returned status {}: {}",
                seg_id, status, preview
            ));
        }

        // --- Content-Type guard: abort early if server returns an HTML page ---
        let response_content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if response_content_type.contains("text/html") {
            log::error!(
                "Segment {} server returned text/html for URL {} — Content-Type: {}",
                seg_id,
                url,
                response_content_type
            );
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Failed;
            return Err(
                "Failed: got HTML page instead of file, possible session or cookie issue"
                    .to_string(),
            );
        }

        // Verify the Content-Length of the response matches expected segment size
        let response_content_length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());

        let expected_segment_size = end_byte - start_byte + 1;
        if let Some(rcl) = response_content_length {
            if rcl != expected_segment_size && status == 206 {
                log::warn!(
                    "Segment {} content-length mismatch: got {} expected {}",
                    seg_id,
                    rcl,
                    expected_segment_size
                );
            }
        }

        // Create temp directory if needed
        if let Some(parent) = std::path::Path::new(&temp_file).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create temp dir: {}", e))?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temp_file)
            .await
            .map_err(|e| format!("Failed to open temp file: {}", e))?;

        let mut stream = response.bytes_stream();
        let mut last_speed_calc = Instant::now();
        let mut bytes_since_calc: u64 = 0;
        let mut total_received: u64 = 0;
        let mut last_real_data = Instant::now(); // Track time since last non-zero chunk

        loop {
            // Check for cancellation
            if *cancel_token.lock().await {
                let mut seg = segment.write().await;
                seg.status = SegmentStatus::Paused;
                return Ok(());
            }

            // Secondary stall detector: if 15s passed since last REAL data, force-kill
            if last_real_data.elapsed().as_secs() >= 15 {
                log::warn!(
                    "Segment {} stalled: no real data for 15 seconds, forcing reconnect",
                    seg_id
                );
                let mut seg = segment.write().await;
                seg.status = SegmentStatus::Failed;
                return Err(format!("Segment {} stalled (no data for 15s)", seg_id));
            }

            let chunk_or_timeout =
                tokio::time::timeout(std::time::Duration::from_secs(10), stream.next()).await;

            let chunk_result = match chunk_or_timeout {
                Ok(Some(res)) => res,
                Ok(None) => break, // Stream completed successfully
                Err(_) => {
                    log::warn!("Segment {} read timed out after 10 seconds", seg_id);
                    let mut seg = segment.write().await;
                    seg.status = SegmentStatus::Failed;
                    return Err(format!("Segment {} read timeout", seg_id));
                }
            };

            let chunk = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
            let chunk_len = chunk.len() as u64;

            // Skip empty chunks (CDN keep-alives) — they shouldn't reset our stall timer
            if chunk_len == 0 {
                continue;
            }

            last_real_data = Instant::now(); // Got real data, reset stall timer

            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Write error: {}", e))?;

            bytes_since_calc += chunk_len;
            total_received += chunk_len;

            {
                let mut seg = segment.write().await;
                seg.downloaded += chunk_len;
            }

            speed_limiter.wait(chunk_len, &task_speed_limit).await;

            // Calculate speed every 500ms for UI using EWMA to prevent jitter
            let elapsed = last_speed_calc.elapsed();
            if elapsed.as_millis() >= 500 {
                let current_raw_speed = bytes_since_calc as f64 / elapsed.as_secs_f64();
                let smoothed_speed = {
                    let mut seg = segment.write().await;
                    if seg.speed_bps == 0.0 {
                        seg.speed_bps = current_raw_speed;
                    } else {
                        seg.speed_bps = (current_raw_speed * 0.3) + (seg.speed_bps * 0.7);
                    }
                    seg.speed_bps
                };

                progress_callback(seg_id, bytes_since_calc, smoothed_speed);
                bytes_since_calc = 0;
                last_speed_calc = Instant::now();
            }
        }

        // Final flush
        file.flush()
            .await
            .map_err(|e| format!("Flush error: {}", e))?;

        if total_received < expected_segment_size {
            log::warn!(
                "Segment {} connection dropped prematurely! Received {}/{}",
                seg_id,
                total_received,
                expected_segment_size
            );
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Failed;
            return Err(format!(
                "Premature EOF ({} of {} bytes)",
                total_received, expected_segment_size
            ));
        }

        log::info!(
            "Segment {} completed normally: received {} bytes",
            seg_id,
            total_received
        );

        {
            let mut seg = segment.write().await;
            seg.status = SegmentStatus::Completed;
            seg.speed_bps = 0.0;
        }

        // Final progress callback
        if bytes_since_calc > 0 {
            progress_callback(seg_id, bytes_since_calc, 0.0);
        }

        Ok(())
    }

    /// Assemble segments into the final file
    pub async fn assemble_file(segments: &[Segment], output_path: &str) -> Result<(), String> {
        log::info!(
            "Assembling {} segments into {}",
            segments.len(),
            output_path
        );

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(output_path).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create output dir: {}", e))?;
        }

        let mut output_file = tokio::fs::File::create(output_path)
            .await
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        let mut total_assembled: u64 = 0;
        for segment in segments {
            let part_path = &segment.temp_file;

            // Verify segment file exists
            if !std::path::Path::new(part_path).exists() {
                return Err(format!(
                    "Segment {} temp file missing: {}",
                    segment.id, part_path
                ));
            }

            let part_meta = tokio::fs::metadata(part_path)
                .await
                .map_err(|e| format!("Failed to read segment {} metadata: {}", segment.id, e))?;

            log::info!(
                "Assembling segment {}: {} bytes from {}",
                segment.id,
                part_meta.len(),
                part_path
            );

            let mut part_file = tokio::fs::File::open(part_path)
                .await
                .map_err(|e| format!("Failed to open segment {}: {}", segment.id, e))?;

            let copied = tokio::io::copy(&mut part_file, &mut output_file)
                .await
                .map_err(|e| format!("Failed to copy segment {}: {}", segment.id, e))?;

            total_assembled += copied;
        }

        output_file
            .flush()
            .await
            .map_err(|e| format!("Failed to flush output: {}", e))?;

        log::info!(
            "Assembly complete: {} total bytes written to {}",
            total_assembled,
            output_path
        );

        Ok(())
    }

    /// Clean up temporary segment files
    pub async fn cleanup_temp(temp_dir: &str) -> Result<(), String> {
        if std::path::Path::new(temp_dir).exists() {
            fs::remove_dir_all(temp_dir)
                .await
                .map_err(|e| format!("Failed to cleanup temp dir: {}", e))?;
        }
        Ok(())
    }

    /// Download a single file without segmentation (fallback for servers without Range support).
    /// Aborts with an error if the server returns text/html (session/cookie issue guard).
    pub async fn download_single(
        client: Client,
        url: String,
        ctx: HttpContext,
        output_path: String,
        total_size_hint: u64,
        cancel_token: Arc<Mutex<bool>>,
        progress_callback: Arc<dyn Fn(u64, u64, f64) + Send + Sync>,
        task_speed_limit: Arc<RwLock<Option<u64>>>,
        speed_limiter: Arc<SharedSpeedLimiter>,
        large_file_mode: bool,
    ) -> Result<u64, String> {
        let response = Self::fetch_with_redirect(&client, &url, reqwest::Method::GET, &ctx, None)
            .await
            .map_err(|e| format!("Download failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let preview = if body.len() > 500 {
                &body[..500]
            } else {
                &body
            };
            return Err(format!("Server returned status {}: {}", status, preview));
        }

        // --- Content-Type guard: abort early if server returns an HTML page ---
        // This catches session errors (GoFile, 1Fichier, etc.) before we write garbage to disk.
        let response_content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if response_content_type.contains("text/html") {
            log::error!(
                "Server returned text/html for URL {} — likely a login redirect or session error. Content-Type: {}",
                url,
                response_content_type
            );
            return Err(
                "Failed: got HTML page instead of file, possible session or cookie issue"
                    .to_string(),
            );
        }

        // Get the actual content-length from the response
        let actual_size = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(total_size_hint);

        log::info!(
            "Single download: status={}, content-length={}, hint was={}, content-type={}",
            status,
            actual_size,
            total_size_hint,
            response_content_type
        );

        // Create parent directories if needed
        if let Some(parent) = std::path::Path::new(&output_path).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create dir: {}", e))?;
        }

        let temp_output_path = format!("{}.part", output_path);
        let mut file = tokio::fs::File::create(&temp_output_path)
            .await
            .map_err(|e| format!("Failed to create file: {}", e))?;

        let mut stream = response.bytes_stream();
        let mut last_speed_calc = Instant::now();
        let mut bytes_since_calc: u64 = 0;
        let mut total_downloaded: u64 = 0;
        let mut current_speed: f64 = 0.0;
        let mut last_real_data = Instant::now();
        let read_timeout_secs = if large_file_mode { 120 } else { 45 };
        let stall_timeout_secs = if large_file_mode { 90 } else { 60 };

        loop {
            if *cancel_token.lock().await {
                file.flush().await.ok();
                return Ok(total_downloaded);
            }

            // Secondary stall detector
            if last_real_data.elapsed().as_secs() >= stall_timeout_secs {
                log::warn!(
                    "Single download stalled: no real data for {} seconds",
                    stall_timeout_secs
                );
                return Err(format!("Stalled (no data for {}s)", stall_timeout_secs));
            }

            let chunk_or_timeout = tokio::time::timeout(
                std::time::Duration::from_secs(read_timeout_secs),
                stream.next(),
            )
            .await;

            let chunk_result = match chunk_or_timeout {
                Ok(Some(res)) => res,
                Ok(None) => break, // Stream completed successfully
                Err(_) => {
                    log::warn!(
                        "Single download read timed out after {} seconds",
                        read_timeout_secs
                    );
                    return Err(format!(
                        "Read timeout after {}s of no data",
                        read_timeout_secs
                    ));
                }
            };

            let chunk = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
            let chunk_len = chunk.len() as u64;

            // Skip empty chunks (CDN keep-alives)
            if chunk_len == 0 {
                continue;
            }

            last_real_data = Instant::now();

            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Write error: {}", e))?;

            bytes_since_calc += chunk_len;
            total_downloaded += chunk_len;

            speed_limiter.wait(chunk_len, &task_speed_limit).await;

            let elapsed = last_speed_calc.elapsed();
            if elapsed.as_millis() >= 500 {
                let raw_speed = bytes_since_calc as f64 / elapsed.as_secs_f64();
                if current_speed == 0.0 {
                    current_speed = raw_speed;
                } else {
                    current_speed = (raw_speed * 0.3) + (current_speed * 0.7);
                }

                progress_callback(
                    bytes_since_calc,
                    actual_size.max(total_downloaded),
                    current_speed,
                );

                bytes_since_calc = 0;
                last_speed_calc = Instant::now();
            }
        }

        file.flush()
            .await
            .map_err(|e| format!("Flush error: {}", e))?;

        if bytes_since_calc > 0 {
            progress_callback(bytes_since_calc, actual_size.max(total_downloaded), 0.0);
        }

        if actual_size > 0 && total_downloaded < actual_size {
            log::warn!(
                "Single download dropped prematurely: {}/{}",
                total_downloaded,
                actual_size
            );
            return Err(format!(
                "Premature EOF ({} of {} bytes)",
                total_downloaded, actual_size
            ));
        }

        if std::path::Path::new(&output_path).exists() {
            fs::remove_file(&output_path)
                .await
                .map_err(|e| format!("Failed to replace existing file: {}", e))?;
        }

        fs::rename(&temp_output_path, &output_path)
            .await
            .map_err(|e| format!("Failed to move completed download into place: {}", e))?;

        log::info!(
            "Single download complete: {} bytes written to {}",
            total_downloaded,
            output_path
        );

        Ok(total_downloaded)
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }
}
