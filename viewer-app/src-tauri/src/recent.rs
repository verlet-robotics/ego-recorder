use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 20;
const FILENAME: &str = "recent_workspaces.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentWorkspace {
    pub path: String,
    pub alias: Option<String>,
    pub last_opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentFile {
    version: u8,
    workspaces: Vec<RecentWorkspace>,
}

impl Default for RecentFile {
    fn default() -> Self {
        Self {
            version: 1,
            workspaces: Vec::new(),
        }
    }
}

fn storage_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(FILENAME)
}

fn read_file(app_data_dir: &Path) -> RecentFile {
    let path = storage_path(app_data_dir);
    if !path.exists() {
        return RecentFile::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_file(app_data_dir: &Path, data: &RecentFile) -> Result<(), String> {
    let path = storage_path(app_data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create app data dir: {}", e))?;
    }
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| format!("Failed to serialize recent workspaces: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write recent workspaces: {}", e))
}

pub fn list(app_data_dir: &Path) -> Vec<RecentWorkspace> {
    read_file(app_data_dir).workspaces
}

pub fn touch(app_data_dir: &Path, workspace_path: &str) -> Result<(), String> {
    let mut data = read_file(app_data_dir);
    let now = Utc::now();

    if let Some(existing) = data.workspaces.iter_mut().find(|w| w.path == workspace_path) {
        existing.last_opened_at = now;
    } else {
        data.workspaces.push(RecentWorkspace {
            path: workspace_path.to_string(),
            alias: None,
            last_opened_at: now,
        });
    }

    data.workspaces
        .sort_by(|a, b| b.last_opened_at.cmp(&a.last_opened_at));
    data.workspaces.truncate(MAX_RECENT);

    write_file(app_data_dir, &data)
}

pub fn remove(app_data_dir: &Path, workspace_path: &str) -> Result<(), String> {
    let mut data = read_file(app_data_dir);
    data.workspaces.retain(|w| w.path != workspace_path);
    write_file(app_data_dir, &data)
}

pub fn set_alias(
    app_data_dir: &Path,
    workspace_path: &str,
    alias: Option<String>,
) -> Result<(), String> {
    let mut data = read_file(app_data_dir);
    let entry = data
        .workspaces
        .iter_mut()
        .find(|w| w.path == workspace_path)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_path))?;

    entry.alias = alias.filter(|s| !s.trim().is_empty());
    write_file(app_data_dir, &data)
}
