use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

const MANIFEST_FILE: &str = "dataset.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetManifest {
    pub version: u32,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub task: String,
    pub created_at: String,
    pub updated_at: String,
    /// Optional target number of episodes for this dataset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_episodes: Option<u32>,
}

impl DatasetManifest {
    pub fn new(name: &str, target_episodes: Option<u32>) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            version: 1,
            name: name.to_string(),
            description: String::new(),
            tags: Vec::new(),
            task: "ego_recording".to_string(),
            created_at: now.clone(),
            updated_at: now,
            target_episodes,
        }
    }
}

/// Load dataset.json from a dataset directory, returns None if missing or invalid.
pub fn load_manifest(dataset_dir: &Path) -> Option<DatasetManifest> {
    let path = dataset_dir.join(MANIFEST_FILE);
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Save dataset.json atomically via tmp+rename.
pub fn save_manifest(dataset_dir: &Path, manifest: &DatasetManifest) -> Result<(), String> {
    let path = dataset_dir.join(MANIFEST_FILE);
    let tmp_path = dataset_dir.join("dataset.json.tmp");

    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("Failed to serialize manifest: {}", e))?;

    std::fs::write(&tmp_path, &json)
        .map_err(|e| format!("Failed to write temp manifest: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to rename manifest: {}", e))?;

    Ok(())
}

/// Create a new dataset directory with dataset.json.
pub fn create_dataset(output_dir: &Path, name: &str, target_episodes: Option<u32>) -> Result<DatasetManifest, String> {
    // Sanitize name for filesystem use
    let dir_name = sanitize_dir_name(name);
    let dataset_dir = output_dir.join(&dir_name);

    if dataset_dir.exists() {
        return Err(format!("Directory already exists: {}", dir_name));
    }

    std::fs::create_dir_all(&dataset_dir)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    let manifest = DatasetManifest::new(name, target_episodes);
    save_manifest(&dataset_dir, &manifest)?;

    Ok(manifest)
}

/// Delete a dataset directory recursively.
pub fn delete_dataset(dataset_dir: &Path) -> Result<(), String> {
    if !dataset_dir.exists() {
        return Err("Dataset directory does not exist".to_string());
    }
    std::fs::remove_dir_all(dataset_dir)
        .map_err(|e| format!("Failed to delete dataset: {}", e))
}

/// Convert a dataset name to a filesystem-safe directory name.
fn sanitize_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
