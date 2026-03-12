use crate::config;
use crate::state::AppState;
use rfd::FileDialog;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub async fn open_directory(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<String>, String> {
    let dir = FileDialog::new()
        .set_title("Open Recordings Directory")
        .pick_folder();

    if let Some(ref path) = dir {
        *state.recordings_dir.write() = Some(path.clone());
        state.file_index.write().clear();
        *state.analysis_cache.write() = None;

        if let Ok(data_dir) = app.path().app_data_dir() {
            let _ = config::save_recordings_dir(&data_dir, path);
        }
    }

    Ok(dir.map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn get_recordings_dir(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    Ok(state
        .recordings_dir
        .read()
        .as_ref()
        .map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn set_recordings_dir(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    dir: String,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(&dir);
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", dir));
    }
    *state.recordings_dir.write() = Some(path.clone());
    state.file_index.write().clear();
    *state.analysis_cache.write() = None;

    if let Ok(data_dir) = app.path().app_data_dir() {
        let _ = config::save_recordings_dir(&data_dir, &path);
    }

    Ok(())
}
