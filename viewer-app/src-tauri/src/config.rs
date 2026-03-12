use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FILENAME: &str = "viewer_config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigFile {
    version: u8,
    recordings_dir: Option<String>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            version: 1,
            recordings_dir: None,
        }
    }
}

fn storage_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(FILENAME)
}

fn read_file(app_data_dir: &Path) -> ConfigFile {
    let path = storage_path(app_data_dir);
    if !path.exists() {
        return ConfigFile::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_file(app_data_dir: &Path, data: &ConfigFile) -> Result<(), String> {
    let path = storage_path(app_data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create app data dir: {}", e))?;
    }
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write config: {}", e))
}

/// Load the persisted recordings directory from config.
pub fn load_recordings_dir(app_data_dir: &Path) -> Option<PathBuf> {
    read_file(app_data_dir)
        .recordings_dir
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Persist the recordings directory to config.
pub fn save_recordings_dir(app_data_dir: &Path, path: &Path) -> Result<(), String> {
    let mut data = read_file(app_data_dir);
    data.recordings_dir = Some(path.to_string_lossy().to_string());
    write_file(app_data_dir, &data)
}
