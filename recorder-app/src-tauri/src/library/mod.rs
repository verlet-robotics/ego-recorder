pub mod watcher;

pub use watcher::{spawn_file_watcher, WatcherCommand};

use crate::state::{ConversionStatus, EgorecMetadataDto, FileEntry};
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EgorecListItem {
    pub name: String,
    pub dataset: Option<String>,
    pub session_name: String,
    pub rgb_codec: u8,
    pub color_width: u32,
    pub color_height: u32,
    pub fps: f64,
    pub total_frames: u64,
    pub duration_s: f64,
    pub size_bytes: u64,
    pub conversion_status: ConversionStatus,
    pub has_imu: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesResponse {
    pub dir: String,
    pub files: Vec<EgorecListItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDetailResponse {
    pub name: String,
    pub metadata: EgorecMetadataDto,
    pub size_bytes: u64,
    pub conversion_status: ConversionStatus,
}

pub fn scan_egorec_files(dir: &std::path::Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();

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
        let path = entry.path().to_path_buf();
        let name = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

        let metadata = match parse_egorec_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let status = if metadata.rgb_codec == 2 {
            ConversionStatus::Streamable
        } else {
            ConversionStatus::Idle
        };

        entries.push(FileEntry {
            name,
            path,
            size_bytes,
            metadata,
            conversion_status: status,
        });
    }

    entries
}

pub fn parse_egorec_metadata(path: &std::path::Path) -> Result<EgorecMetadataDto, String> {
    use egorec::format::*;
    use std::io::{BufReader, Seek, SeekFrom};

    let file = std::fs::File::open(path).map_err(|e| format!("open: {}", e))?;
    let mut reader = BufReader::new(file);

    let header =
        FileHeader::read_from(&mut reader).map_err(|e| format!("header: {}", e))?;

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
    let footer =
        FileFooter::read_from(&mut reader).map_err(|e| format!("footer: {}", e))?;

    Ok(EgorecMetadataDto::from_header(
        &header,
        footer.total_frames,
        footer.total_duration_us,
    ))
}

pub fn extract_dataset(name: &str) -> Option<String> {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("/"))
    } else {
        None
    }
}
