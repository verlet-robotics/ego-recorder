use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub recorder: RecorderConfig,
    pub storage: StorageConfig,
    pub upload: UploadConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecorderConfig {
    /// Path to the C++ ego-recorder binary
    pub binary_path: Option<String>,
    /// Default CRF quality (0-51, default 23)
    pub default_crf: u8,
    /// Warmup frames to skip (default 30)
    pub warmup_frames: u32,
    /// H.264 encoder speed preset (ultrafast/superfast/veryfast/fast)
    pub h264_preset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Default output directory for recordings
    pub output_dir: Option<String>,
    /// Minimum free disk space in MB before warning (default 500)
    pub disk_threshold_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadConfig {
    pub endpoint: Option<String>,
    pub bucket: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub region: Option<String>,
    pub auto_upload: bool,
    /// S3 key prefix (e.g. "device-01/")
    pub prefix: Option<String>,
    /// Multipart chunk size in MB (default 32)
    pub multipart_chunk_mb: u32,
    /// Poll interval for auto-upload in seconds (default 30)
    pub poll_interval_s: u64,
    /// Mtime grace period to avoid uploading in-progress files (default 10)
    pub file_settle_s: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            recorder: RecorderConfig::default(),
            storage: StorageConfig::default(),
            upload: UploadConfig::default(),
        }
    }
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            binary_path: locate_binary(),
            default_crf: 23,
            warmup_frames: 30,
            h264_preset: "ultrafast".to_string(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            disk_threshold_mb: 500,
        }
    }
}

/// Returns ~/Documents/ego-recordings, creating it if needed.
fn default_output_dir() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Documents").join("ego-recordings");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.to_string_lossy().to_string())
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            region: None,
            auto_upload: false,
            prefix: None,
            multipart_chunk_mb: 32,
            poll_interval_s: 30,
            file_settle_s: 10,
        }
    }
}

/// Returns the app configuration directory (~/.config/ego-recorder-app/).
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("ego-recorder-app")
}

/// Returns the config file path (~/.config/ego-recorder-app/config.toml).
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Load configuration from TOML file. Returns default config if file is missing.
pub fn load_config() -> AppConfig {
    let path = config_path();
    if !path.exists() {
        return AppConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
            log::warn!("Failed to parse config file: {}. Using defaults.", e);
            AppConfig::default()
        }),
        Err(e) => {
            log::warn!("Failed to read config file: {}. Using defaults.", e);
            AppConfig::default()
        }
    }
}

/// Save configuration to TOML file atomically (write to tmp then rename).
pub fn save_config(config: &AppConfig) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    let toml_str =
        toml::to_string_pretty(config).map_err(|e| format!("Failed to serialize config: {}", e))?;

    let path = config_path();
    let tmp_path = path.with_extension("toml.tmp");

    std::fs::write(&tmp_path, &toml_str)
        .map_err(|e| format!("Failed to write temp config: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to rename config file: {}", e))?;

    Ok(())
}

/// Check if this is the first run (config file does not exist).
pub fn is_first_run() -> bool {
    !config_path().exists()
}

/// Search for the ego-recorder binary in PATH and common relative locations.
pub fn locate_binary() -> Option<String> {
    // Check PATH first
    if let Ok(output) = std::process::Command::new("which")
        .arg("ego-recorder")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // Check common relative locations from the app binary
    let relative_paths = [
        "../build/ego-recorder",
        "../../build/ego-recorder",
        "../ego-recorder",
    ];

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    if let Some(dir) = &exe_dir {
        for rel in &relative_paths {
            let candidate = dir.join(rel);
            if candidate.exists() {
                return candidate.canonicalize().ok().map(|p| p.to_string_lossy().to_string());
            }
        }
    }

    // Also check from cwd
    for rel in &relative_paths {
        let candidate = PathBuf::from(rel);
        if candidate.exists() {
            return candidate
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }
    }

    None
}
