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
    /// Prevent display from turning off due to idle while the app is running.
    /// Manual screen lock (Super+L) still works.
    pub keep_display_on: bool,
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
            keep_display_on: true,
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

/// Search for the ego-recorder binary in PATH, well-known locations, and
/// by walking up from the app binary / cwd looking for `build/ego-recorder`.
pub fn locate_binary() -> Option<String> {
    let binary_name = "ego-recorder";

    // 1. Check PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg(binary_name)
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // 2. Check well-known install locations
    let home = std::env::var("HOME").unwrap_or_default();
    let fixed_paths: Vec<PathBuf> = vec![
        PathBuf::from(&home).join(".local/bin").join(binary_name),
        PathBuf::from("/usr/local/bin").join(binary_name),
    ];
    for candidate in &fixed_paths {
        if candidate.is_file() {
            return candidate.canonicalize().ok().map(|p| p.to_string_lossy().to_string());
        }
    }

    // 3. Walk up from the app binary directory looking for build/ego-recorder.
    //    The Tauri binary lives at recorder-app/src-tauri/target/{debug,release}/
    //    while the C++ binary is at <repo>/build/ego-recorder, so we need to
    //    walk up several levels.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    if let Some(found) = exe_dir.as_ref().and_then(|dir| walk_up_for_binary(dir, binary_name)) {
        return Some(found);
    }

    // 4. Walk up from cwd (covers running from recorder-app/ or repo root)
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = walk_up_for_binary(&cwd, binary_name) {
            return Some(found);
        }
    }

    None
}

/// Walk up from `start` checking `build/<name>` at each ancestor, up to 8 levels.
fn walk_up_for_binary(start: &std::path::Path, name: &str) -> Option<String> {
    let mut dir = start.to_path_buf();
    for _ in 0..8 {
        let candidate = dir.join("build").join(name);
        if candidate.is_file() {
            return candidate.canonicalize().ok().map(|p| p.to_string_lossy().to_string());
        }
        if !dir.pop() {
            break;
        }
    }
    None
}
