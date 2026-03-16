use crate::library::{
    extract_dataset, scan_egorec_files, EgorecListItem, FileDetailResponse,
    FilesResponse, WatcherCommand,
};
use crate::state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn discover_files(
    dir: String,
    state: State<'_, Arc<AppState>>,
) -> Result<FilesResponse, String> {
    let dir_path = std::path::PathBuf::from(&dir);
    if !dir_path.is_dir() {
        return Err(format!("Not a directory: {}", dir));
    }

    // Scan in blocking task BEFORE touching the index.
    // This avoids a race where the index is cleared but the scan hasn't completed yet.
    let entries =
        tokio::task::spawn_blocking(move || scan_egorec_files(&dir_path))
            .await
            .map_err(|e| format!("Task error: {}", e))?;

    // Build items and new index from scan results, then swap atomically
    let mut items = Vec::new();
    let mut new_index = std::collections::HashMap::new();

    for entry in entries {
        let list_item = EgorecListItem {
            name: entry.name.clone(),
            dataset: extract_dataset(&entry.name),
            session_name: entry.metadata.session_name.clone(),
            rgb_codec: entry.metadata.rgb_codec,
            color_width: entry.metadata.color_width,
            color_height: entry.metadata.color_height,
            fps: entry.metadata.fps,
            total_frames: entry.metadata.total_frames,
            duration_s: entry.metadata.duration_s,
            size_bytes: entry.size_bytes,
            conversion_status: entry.conversion_status,
            has_imu: entry.metadata.has_imu,
        };
        new_index.insert(entry.name.clone(), entry);
        items.push(list_item);
    }

    // Atomic swap — index is never empty during the scan
    *state.file_index.write() = new_index;

    items.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(FilesResponse { dir, files: items })
}

#[tauri::command]
pub fn get_file_metadata(
    file_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<FileDetailResponse, String> {
    let index = state.file_index.read();
    let entry = index
        .get(&file_name)
        .ok_or_else(|| format!("File not found: {}", file_name))?;
    Ok(FileDetailResponse {
        name: entry.name.clone(),
        metadata: entry.metadata.clone(),
        size_bytes: entry.size_bytes,
        conversion_status: entry.conversion_status,
    })
}

#[tauri::command]
pub fn get_video_server_port(state: State<'_, Arc<AppState>>) -> Option<u16> {
    *state.video_server_port.read()
}

#[tauri::command]
pub fn get_stream_url(
    file_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let port = state
        .video_server_port
        .read()
        .ok_or("Video server not running")?;
    let encoded = urlencoding::encode(&file_name);
    Ok(format!("http://localhost:{}/stream/{}", port, encoded))
}

#[tauri::command]
pub async fn watch_directory(
    dir: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let path = PathBuf::from(&dir);
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", dir));
    }

    let tx = state.watcher_cmd_tx.lock().await;
    if let Some(ref tx) = *tx {
        tx.send(WatcherCommand::Watch(path))
            .await
            .map_err(|_| "Watcher task not running".to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub fn get_watched_dir(state: State<'_, Arc<AppState>>) -> Option<String> {
    state.watched_dir.read().clone()
}
