use crate::state::{AppState, ConversionStatus, EgorecMetadataDto, FileEntry};
use egorec::FileHeader;
use serde::Serialize;
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, State};
use walkdir::WalkDir;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileListItem {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesResponse {
    pub dir: String,
    pub files: Vec<FileListItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDetailResponse {
    pub name: String,
    pub metadata: EgorecMetadataDto,
    pub size_bytes: u64,
    pub conversion_status: ConversionStatus,
}

pub fn parse_egorec_metadata(path: &Path) -> Result<(EgorecMetadataDto, u64), String> {
    let file_size = std::fs::metadata(path)
        .map(|m| m.len())
        .unwrap_or(0);

    let mut file = BufReader::new(
        File::open(path).map_err(|e| format!("Failed to open: {}", e))?,
    );

    let header = FileHeader::read_from(&mut file)
        .map_err(|e| format!("Failed to read header: {}", e))?;

    let (total_frames, duration_us) = if file_size >= (egorec::FILE_HEADER_SIZE as u64 + 36) {
        file.seek(SeekFrom::End(-36))
            .map_err(|e| format!("seek footer: {}", e))?;
        match egorec::FileFooter::read_from(&mut file) {
            Ok(footer) => (footer.total_frames, footer.total_duration_us),
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    let metadata = EgorecMetadataDto::from_header(&header, total_frames, duration_us);
    Ok((metadata, file_size))
}

#[tauri::command]
pub async fn discover_files(
    _app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<FilesResponse, String> {
    let recordings_dir = state
        .recordings_dir
        .read()
        .clone()
        .ok_or_else(|| "No recordings directory set".to_string())?;

    let dir_str = recordings_dir.to_string_lossy().to_string();

    let dir_clone = recordings_dir.clone();
    let state_clone = Arc::clone(&state);

    tokio::task::spawn_blocking(move || {
        let mut index = state_clone.file_index.write();
        index.clear();

        let mut count = 0u32;
        for entry in WalkDir::new(&dir_clone)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("egorec") {
                continue;
            }

            let rel_path = match path.strip_prefix(&dir_clone) {
                Ok(r) => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            match parse_egorec_metadata(path) {
                Ok((metadata, file_size)) => {
                    let status = if metadata.rgb_codec == 2 {
                        ConversionStatus::Streamable
                    } else {
                        ConversionStatus::Idle
                    };

                    index.insert(
                        rel_path.clone(),
                        FileEntry {
                            name: rel_path,
                            path: path.to_path_buf(),
                            size_bytes: file_size,
                            metadata,
                            conversion_status: status,
                        },
                    );
                    count += 1;
                }
                Err(e) => {
                    log::warn!("Skipping {}: {}", rel_path, e);
                }
            }
        }

        log::info!("Discovered {} .egorec files in {}", count, dir_clone.display());
    })
    .await
    .map_err(|e| format!("Discovery task failed: {}", e))?;

    list_files(state).await.map(|files| FilesResponse {
        dir: dir_str,
        files,
    })
}

#[tauri::command]
pub async fn list_files(state: State<'_, Arc<AppState>>) -> Result<Vec<FileListItem>, String> {
    let index = state.file_index.read();
    let mut files: Vec<FileListItem> = index
        .values()
        .map(|f| {
            let dir = Path::new(&f.name)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .filter(|s| s != ".");
            FileListItem {
                name: f.name.clone(),
                dataset: dir,
                session_name: f.metadata.session_name.clone(),
                rgb_codec: f.metadata.rgb_codec,
                color_width: f.metadata.color_width,
                color_height: f.metadata.color_height,
                fps: f.metadata.fps,
                total_frames: f.metadata.total_frames,
                duration_s: f.metadata.duration_s,
                size_bytes: f.size_bytes,
                conversion_status: f.conversion_status,
                has_imu: f.metadata.has_imu,
            }
        })
        .collect();

    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

#[tauri::command]
pub async fn get_file_metadata(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<FileDetailResponse, String> {
    let index = state.file_index.read();
    let entry = index
        .get(&name)
        .ok_or_else(|| format!("File not found: {}", name))?;

    Ok(FileDetailResponse {
        name: entry.name.clone(),
        metadata: entry.metadata.clone(),
        size_bytes: entry.size_bytes,
        conversion_status: entry.conversion_status,
    })
}
