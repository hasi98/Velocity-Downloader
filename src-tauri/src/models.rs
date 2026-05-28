use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a download
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    Assembling,
}

/// Status of an individual segment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SegmentStatus {
    Pending,
    Downloading,
    Paused,
    Completed,
    Failed,
}

/// Download engine used for the task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadKind {
    Direct,
    Media,
}

fn default_download_kind() -> DownloadKind {
    DownloadKind::Direct
}

/// Represents a single download segment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: usize,
    pub start_byte: u64,
    pub end_byte: u64,
    pub downloaded: u64,
    pub status: SegmentStatus,
    pub speed_bps: f64,
    pub temp_file: String,
}

impl Segment {
    pub fn new(id: usize, start_byte: u64, end_byte: u64, temp_dir: &str) -> Self {
        let temp_file = std::path::PathBuf::from(temp_dir)
            .join(format!("segment_{}.part", id))
            .to_string_lossy()
            .to_string();
        Self {
            id,
            start_byte,
            end_byte,
            downloaded: 0,
            status: SegmentStatus::Pending,
            speed_bps: 0.0,
            temp_file,
        }
    }

    pub fn total_size(&self) -> u64 {
        self.end_byte - self.start_byte + 1
    }

    pub fn progress(&self) -> f64 {
        if self.total_size() == 0 {
            return 0.0;
        }
        (self.downloaded as f64 / self.total_size() as f64) * 100.0
    }
}

/// HTTP context forwarded from the browser extension.
/// All fields are optional — manual/quick-add downloads won't have them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpContext {
    /// Cookie header value, e.g. "session=abc; token=xyz"
    pub cookies: Option<String>,
    /// Referer URL of the page that triggered the download
    pub referer: Option<String>,
    /// Browser User-Agent string
    pub user_agent: Option<String>,
}

/// Core download task metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub save_path: String,
    pub total_size: u64,
    pub downloaded: u64,
    pub status: DownloadStatus,
    pub segments: Vec<Segment>,
    pub supports_range: bool,
    pub num_segments: usize,
    pub speed_bps: f64,
    pub eta_seconds: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error: Option<String>,
    pub content_type: Option<String>,
    #[serde(default = "default_download_kind")]
    pub download_kind: DownloadKind,
    #[serde(default)]
    pub media_format: Option<String>,
    /// Browser context forwarded by the extension (cookies, referer, user-agent)
    pub http_context: HttpContext,
    /// Per-download speed limit in bytes per second
    pub speed_limit_bps: Option<u64>,
    /// Temporary directory override
    #[serde(default)]
    pub temp_dir_override: Option<String>,
    #[serde(default)]
    pub scheduled_queue: bool,
    #[serde(default)]
    pub batch_group_id: Option<String>,
    #[serde(default)]
    pub batch_sequential: bool,
    #[serde(default)]
    pub batch_queue_index: usize,
}

impl DownloadTask {
    pub fn new(url: String, filename: String, save_path: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            url,
            filename,
            save_path,
            total_size: 0,
            downloaded: 0,
            status: DownloadStatus::Queued,
            segments: Vec::new(),
            supports_range: false,
            num_segments: 1,
            speed_bps: 0.0,
            eta_seconds: 0.0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error: None,
            content_type: None,
            download_kind: DownloadKind::Direct,
            media_format: None,
            http_context: HttpContext::default(),
            speed_limit_bps: None,
            temp_dir_override: None,
            scheduled_queue: false,
            batch_group_id: None,
            batch_sequential: false,
            batch_queue_index: 0,
        }
    }

    pub fn progress(&self) -> f64 {
        if self.total_size == 0 {
            return 0.0;
        }
        (self.downloaded as f64 / self.total_size as f64) * 100.0
    }

    pub fn temp_dir(&self) -> String {
        let parent = if let Some(ref dir) = self.temp_dir_override {
            std::path::PathBuf::from(dir)
        } else {
            std::path::Path::new(&self.save_path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf()
        };
        parent
            .join(format!(".myidm_temp_{}", self.id))
            .to_string_lossy()
            .to_string()
    }

    pub fn meta_file_path(&self) -> String {
        let parent = std::path::Path::new(&self.save_path)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        parent
            .join(format!(".{}.meta", self.filename))
            .to_string_lossy()
            .to_string()
    }
}

/// Progress event sent to the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub download_id: String,
    pub total_size: u64,
    pub downloaded: u64,
    pub speed_bps: f64,
    pub eta_seconds: f64,
    pub status: DownloadStatus,
    pub segments: Vec<SegmentProgress>,
    pub speed_limit_bps: Option<u64>,
}

/// Individual segment progress
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentProgress {
    pub id: usize,
    pub downloaded: u64,
    pub total_size: u64,
    pub speed_bps: f64,
    pub status: SegmentStatus,
    pub progress: f64,
}

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_segments")]
    pub default_segments: usize,
    #[serde(default = "default_download_dir")]
    pub default_download_dir: String,
    #[serde(default)]
    pub temp_download_dir: Option<String>,
    #[serde(default)]
    pub speed_limit_bps: Option<u64>,
    #[serde(default)]
    pub start_on_boot: bool,
    #[serde(default)]
    pub extension_prompt_seen: bool,
}

fn default_segments() -> usize {
    8
}

fn default_download_dir() -> String {
    dirs::download_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_segments: 8,
            default_download_dir: default_download_dir(),
            temp_download_dir: None,
            speed_limit_bps: None,
            start_on_boot: false,
            extension_prompt_seen: false,
        }
    }
}
