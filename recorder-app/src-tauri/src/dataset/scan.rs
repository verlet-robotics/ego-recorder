use crate::dataset::manifest::load_manifest;
use crate::state::EgorecMetadataDto;
use crate::upload::upload_queue;
use egorec::format::*;
use serde::Serialize;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetSummary {
    pub name: String,
    pub dir_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub file_count: usize,
    pub total_frames: u64,
    pub total_duration_s: f64,
    pub total_size_bytes: u64,
    pub uploaded_count: usize,
    pub has_lerobot: bool,
    pub created_at: String,
    pub updated_at: String,
    pub target_episodes: Option<u32>,
}

/// Scan output_dir for immediate subdirectories containing dataset.json.
pub fn scan_datasets(output_dir: &Path) -> Vec<DatasetSummary> {
    let read_dir = match std::fs::read_dir(output_dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut summaries = Vec::new();

    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest = match load_manifest(&path) {
            Some(m) => m,
            None => continue,
        };

        let dir_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Scan .egorec files in this dataset directory
        let (file_count, total_frames, total_duration_s, total_size_bytes) =
            scan_egorec_stats(&path);

        // Check upload manifest for uploaded count
        let upload_manifest = upload_queue::load_manifest(output_dir);
        let uploaded_count = count_uploaded_files(&upload_manifest, &dir_name);

        // Check if _lerobot conversion exists
        let has_lerobot = path.join("_lerobot/meta/info.json").exists();

        summaries.push(DatasetSummary {
            name: manifest.name,
            dir_name,
            description: manifest.description,
            tags: manifest.tags,
            file_count,
            total_frames,
            total_duration_s,
            total_size_bytes,
            uploaded_count,
            has_lerobot,
            created_at: manifest.created_at,
            updated_at: manifest.updated_at,
            target_episodes: manifest.target_episodes,
        });
    }

    summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    summaries
}

/// Scan .egorec files in a directory and compute aggregate stats.
fn scan_egorec_stats(dir: &Path) -> (usize, u64, f64, u64) {
    let mut file_count = 0usize;
    let mut total_frames = 0u64;
    let mut total_duration_s = 0f64;
    let mut total_size_bytes = 0u64;

    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "egorec")
                && !e.path().to_string_lossy().contains(".pruned")
        })
    {
        let path = entry.path();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        if let Ok(meta) = parse_egorec_metadata_quick(path) {
            file_count += 1;
            total_frames += meta.total_frames;
            total_duration_s += meta.duration_s;
            total_size_bytes += size;
        }
    }

    (file_count, total_frames, total_duration_s, total_size_bytes)
}

/// Quick metadata parse — reads header + footer only (same pattern as library_commands).
fn parse_egorec_metadata_quick(path: &Path) -> Result<EgorecMetadataDto, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {}", e))?;
    let mut reader = BufReader::new(file);

    let header = FileHeader::read_from(&mut reader).map_err(|e| format!("header: {}", e))?;

    let file_size = reader
        .get_ref()
        .metadata()
        .map_err(|e| format!("metadata: {}", e))?
        .len();

    if file_size < (FILE_HEADER_SIZE as u64 + FileFooter::SIZE as u64) {
        return Err("File too small".into());
    }

    reader
        .seek(SeekFrom::End(-(FileFooter::SIZE as i64)))
        .map_err(|e| format!("seek: {}", e))?;
    let footer = FileFooter::read_from(&mut reader).map_err(|e| format!("footer: {}", e))?;

    Ok(EgorecMetadataDto::from_header(
        &header,
        footer.total_frames,
        footer.total_duration_us,
    ))
}

/// Count how many files from this dataset have been uploaded.
fn count_uploaded_files(
    manifest: &upload_queue::UploadManifest,
    dataset_dir_name: &str,
) -> usize {
    let prefix = format!("{}/", dataset_dir_name);
    manifest
        .uploads
        .iter()
        .filter(|r| r.success && r.filename.starts_with(&prefix))
        .count()
}
