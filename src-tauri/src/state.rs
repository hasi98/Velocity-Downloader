use crate::models::DownloadTask;
use std::path::Path;
use tokio::fs;

/// Manages persistent state via .meta files for pause/resume support
pub struct StateManager;

impl StateManager {
    /// Save download state to a .meta file
    pub async fn save_state(task: &DownloadTask) -> Result<(), String> {
        let meta_path = task.meta_file_path();

        // Ensure parent directory exists
        if let Some(parent) = Path::new(&meta_path).parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create meta dir: {}", e))?;
        }

        let json = serde_json::to_string_pretty(task)
            .map_err(|e| format!("Failed to serialize state: {}", e))?;

        fs::write(&meta_path, json)
            .await
            .map_err(|e| format!("Failed to write meta file: {}", e))?;

        Ok(())
    }

    /// Load download state from a .meta file
    pub async fn load_state(meta_path: &str) -> Result<DownloadTask, String> {
        let content = fs::read_to_string(meta_path)
            .await
            .map_err(|e| format!("Failed to read meta file: {}", e))?;

        let task: DownloadTask = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to deserialize state: {}", e))?;

        Ok(task)
    }

    /// Delete a .meta file (after completion or cancellation)
    pub async fn delete_state(task: &DownloadTask) -> Result<(), String> {
        let meta_path = task.meta_file_path();
        if Path::new(&meta_path).exists() {
            fs::remove_file(&meta_path)
                .await
                .map_err(|e| format!("Failed to delete meta file: {}", e))?;
        }
        Ok(())
    }

    /// Scan a directory for .meta files to find resumable downloads
    pub async fn scan_for_resumable(directory: &str) -> Result<Vec<DownloadTask>, String> {
        let mut tasks = Vec::new();
        let dir = Path::new(directory);

        if !dir.exists() {
            return Ok(tasks);
        }

        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| format!("Failed to read dir: {}", e))?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| format!("Dir entry error: {}", e))? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("meta") {
                if let Ok(task) = Self::load_state(&path.to_string_lossy()).await {
                    tasks.push(task);
                }
            }
        }

        Ok(tasks)
    }

    /// Synchronously scan a directory tree for .meta files during app startup.
    pub fn scan_for_resumable_sync(directory: &str) -> Vec<DownloadTask> {
        let mut tasks = Vec::new();
        let root = std::path::PathBuf::from(directory);
        Self::scan_meta_dir_sync(&root, &mut tasks, 0);
        tasks
    }

    fn scan_meta_dir_sync(dir: &Path, tasks: &mut Vec<DownloadTask>, depth: usize) {
        if depth > 5 || !dir.exists() {
            return;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::scan_meta_dir_sync(&path, tasks, depth + 1);
                continue;
            }

            if path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(_) => continue,
            };

            if let Ok(task) = serde_json::from_str::<DownloadTask>(&content) {
                tasks.push(task);
            }
        }
    }

    fn settings_path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("VelocityDownloader").join("settings.json"))
    }

    fn history_path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("VelocityDownloader").join("history.json"))
    }

    /// Load completed download history from disk.
    pub fn load_history() -> Vec<DownloadTask> {
        let path = match Self::history_path() {
            Some(path) => path,
            None => return Vec::new(),
        };

        if !path.exists() {
            return Vec::new();
        }

        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };

        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Insert or replace a task in completed download history.
    pub fn upsert_history(task: &DownloadTask) -> Result<(), String> {
        let path = Self::history_path().ok_or("Failed to determine history path")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let mut history = Self::load_history();
        history.retain(|item| item.id != task.id);
        history.insert(0, task.clone());
        history.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        history.truncate(1000);

        let json = serde_json::to_string_pretty(&history).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Remove a task from completed download history.
    pub fn remove_history(download_id: &str) -> Result<(), String> {
        let path = Self::history_path().ok_or("Failed to determine history path")?;
        let mut history = Self::load_history();
        let original_len = history.len();
        history.retain(|item| item.id != download_id);

        if history.len() == original_len {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let json = serde_json::to_string_pretty(&history).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Save app settings to disk
    pub fn save_settings(settings: &crate::models::AppSettings) -> Result<(), String> {
        let path = Self::settings_path().ok_or("Failed to determine settings path")?;
        
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())?;
        
        Ok(())
    }

    /// Load app settings from disk
    pub fn load_settings() -> crate::models::AppSettings {
        let path = match Self::settings_path() {
            Some(p) => p,
            None => return crate::models::AppSettings::default(),
        };

        if !path.exists() {
            return crate::models::AppSettings::default();
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return crate::models::AppSettings::default(),
        };

        serde_json::from_str(&content).unwrap_or_else(|_| crate::models::AppSettings::default())
    }
}
