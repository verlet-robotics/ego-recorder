use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;
use walkdir::WalkDir;

/// Persistent manifest matching Python's `.upload_manifest.json` format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadManifest {
    pub version: u32,
    pub uploads: Vec<UploadRecord>,
}

/// A single upload record in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadRecord {
    pub filename: String,
    pub r2_key: String,
    pub uploaded_at: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub attempt_count: u32,
    pub success: bool,
}

/// In-memory queue item for tracking upload state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadQueueEntry {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub status: QueueStatus,
}

/// Upload status enum with progress and error info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum QueueStatus {
    Pending,
    Hashing { progress: f64 },
    Uploading { progress: f64, speed_bps: u64 },
    Completed { sha256: String },
    Failed { error: String },
}

const MANIFEST_FILE: &str = ".upload_manifest.json";

impl Default for UploadManifest {
    fn default() -> Self {
        Self {
            version: 1,
            uploads: Vec::new(),
        }
    }
}

impl UploadManifest {
    /// Set of filenames already uploaded.
    pub fn uploaded_files(&self) -> HashSet<&str> {
        self.uploads
            .iter()
            .filter(|r| r.success)
            .map(|r| r.filename.as_str())
            .collect()
    }
}

/// Load manifest from directory, return empty if missing or unparseable.
pub fn load_manifest(dir: &Path) -> UploadManifest {
    let path = dir.join(MANIFEST_FILE);
    if !path.exists() {
        return UploadManifest::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
            log::warn!("Failed to parse upload manifest: {}. Starting fresh.", e);
            UploadManifest::default()
        }),
        Err(e) => {
            log::warn!("Failed to read upload manifest: {}. Starting fresh.", e);
            UploadManifest::default()
        }
    }
}

/// Save manifest atomically (tmp + rename).
pub fn save_manifest(dir: &Path, manifest: &UploadManifest) -> Result<(), String> {
    let path = dir.join(MANIFEST_FILE);
    let tmp_path = dir.join(".upload_manifest.json.tmp");

    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("Failed to serialize manifest: {}", e))?;

    std::fs::write(&tmp_path, &json)
        .map_err(|e| format!("Failed to write temp manifest: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to rename manifest file: {}", e))?;

    Ok(())
}

/// Check if a file has been successfully uploaded.
pub fn is_uploaded(manifest: &UploadManifest, filename: &str) -> bool {
    manifest
        .uploads
        .iter()
        .any(|r| r.filename == filename && r.success)
}

/// Record a successful upload in the manifest.
pub fn record_upload(
    manifest: &mut UploadManifest,
    filename: String,
    r2_key: String,
    size_bytes: u64,
    sha256: String,
    attempt_count: u32,
) {
    let uploaded_at = Utc::now().to_rfc3339();
    manifest.uploads.push(UploadRecord {
        filename,
        r2_key,
        uploaded_at,
        size_bytes,
        sha256,
        attempt_count,
        success: true,
    });
}

/// Scan directory for .egorec files not yet in the manifest.
/// Skips files modified within `settle_s`, files under `.pruned/`, and files < 1024 bytes.
/// Returns sorted by mtime (oldest first).
pub fn scan_pending(
    dir: &Path,
    manifest: &UploadManifest,
    settle_s: u64,
) -> Vec<UploadQueueEntry> {
    let uploaded = manifest.uploaded_files();
    let now = SystemTime::now();
    let mut entries = Vec::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();

        // Must be .egorec
        if path.extension().and_then(|e| e.to_str()) != Some("egorec") {
            continue;
        }

        // Skip .pruned/ directories
        if path
            .components()
            .any(|c| c.as_os_str() == ".pruned")
        {
            continue;
        }

        // Get relative path
        let rel_path = match path.strip_prefix(dir) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };

        // Skip already uploaded
        if uploaded.contains(rel_path.as_str()) {
            continue;
        }

        // Check size (skip < 1024 bytes)
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = metadata.len();
        if size < 1024 {
            continue;
        }

        // Check settle time (skip if modified too recently)
        if let Ok(mtime) = metadata.modified() {
            if let Ok(age) = now.duration_since(mtime) {
                if age.as_secs() < settle_s {
                    continue;
                }
            }
        }

        entries.push((
            path.to_path_buf(),
            rel_path,
            size,
            metadata.modified().unwrap_or(now),
        ));
    }

    // Sort by mtime ascending (oldest first)
    entries.sort_by_key(|(_, _, _, mtime)| *mtime);

    entries
        .into_iter()
        .map(|(path, filename, size_bytes, _)| UploadQueueEntry {
            filename,
            path: path.to_string_lossy().to_string(),
            size_bytes,
            status: QueueStatus::Pending,
        })
        .collect()
}
