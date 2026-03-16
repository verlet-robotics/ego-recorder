use crate::state::AppState;
use crate::upload::s3_upload::{
    self, build_s3_client, compute_sha256, make_object_key, upload_file, UploadProgressEvent,
};
use crate::upload::upload_queue::{
    load_manifest, record_upload, save_manifest, QueueStatus, UploadQueueEntry, UploadManifest,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Emitter;
use tauri::State;

/// Manually queue a file for upload.
#[tauri::command]
pub async fn queue_upload(
    path: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let file_path = PathBuf::from(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {}", path));
    }

    let metadata = std::fs::metadata(&file_path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let (output_dir, upload_config) = {
        let config = state.config.read();
        (config.storage.output_dir.clone(), config.upload.clone())
    };

    let output_dir = output_dir.ok_or("No output directory configured")?;
    let dir = PathBuf::from(&output_dir);

    // Compute relative filename
    let filename = file_path
        .strip_prefix(&dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            file_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone())
        });

    let r2_key = make_object_key(upload_config.prefix.as_deref(), &filename);
    let bucket = upload_config
        .bucket
        .clone()
        .ok_or("Bucket not configured")?;

    // Add to in-memory queue
    {
        let mut queue = state.upload_queue.write();
        if queue.iter().any(|e| e.filename == filename) {
            return Err("File is already in the upload queue".to_string());
        }
        queue.push(UploadQueueEntry {
            filename: filename.clone(),
            path: path.clone(),
            size_bytes: metadata.len(),
            status: QueueStatus::Hashing { progress: 0.0 },
        });
    }

    // Spawn upload task — all values moved into the task must be owned
    let state = state.inner().clone();
    let filename_clone = filename.clone();
    let file_path_owned = file_path.clone();
    let dir_owned = dir.clone();
    tauri::async_runtime::spawn(async move {
        let total_size = metadata.len();

        // Hash
        let filename_for_hash = filename_clone.clone();
        let app_handle_hash = app_handle.clone();
        let sha256 = match compute_sha256(&file_path_owned, total_size, move |bytes_done| {
            let progress = UploadProgressEvent {
                filename: filename_for_hash.clone(),
                bytes_transferred: bytes_done,
                total_bytes: total_size,
                speed_bps: 0,
                phase: "hashing".to_string(),
            };
            let _ = app_handle_hash.emit("upload:progress", &progress);
        })
        .await
        {
            Ok(h) => h,
            Err(e) => {
                set_queue_status(&state, &filename_clone, QueueStatus::Failed { error: e });
                return;
            }
        };

        // Update status to uploading
        set_queue_status(
            &state,
            &filename_clone,
            QueueStatus::Uploading {
                progress: 0.0,
                speed_bps: 0,
            },
        );

        // Upload
        let client = match build_s3_client(&upload_config) {
            Ok(c) => c,
            Err(e) => {
                set_queue_status(&state, &filename_clone, QueueStatus::Failed { error: e });
                return;
            }
        };

        match upload_file(
            &client,
            &bucket,
            &r2_key,
            &file_path_owned,
            upload_config.multipart_chunk_mb,
            &app_handle,
            &filename_clone,
        )
        .await
        {
            Ok(()) => {
                // Record in manifest
                let mut manifest = load_manifest(&dir_owned);
                record_upload(
                    &mut manifest,
                    filename_clone.clone(),
                    r2_key,
                    total_size,
                    sha256.clone(),
                    1,
                );
                if let Err(e) = save_manifest(&dir_owned, &manifest) {
                    log::error!("Failed to save manifest: {}", e);
                }

                set_queue_status(
                    &state,
                    &filename_clone,
                    QueueStatus::Completed {
                        sha256: sha256.clone(),
                    },
                );

                let progress = UploadProgressEvent {
                    filename: filename_clone,
                    bytes_transferred: total_size,
                    total_bytes: total_size,
                    speed_bps: 0,
                    phase: "completed".to_string(),
                };
                let _ = app_handle.emit("upload:progress", &progress);
            }
            Err(e) => {
                set_queue_status(
                    &state,
                    &filename_clone,
                    QueueStatus::Failed { error: e },
                );
            }
        }
    });

    Ok(())
}

/// Get current upload queue state.
#[tauri::command]
pub fn get_upload_queue(state: State<'_, Arc<AppState>>) -> Vec<UploadQueueEntry> {
    let config = state.config.read();
    let output_dir = config.storage.output_dir.clone();
    drop(config);

    let mut queue = state.upload_queue.read().clone();

    // Merge with manifest data for completed uploads not in memory
    if let Some(dir) = &output_dir {
        let manifest = load_manifest(Path::new(dir));
        let in_queue: std::collections::HashSet<String> =
            queue.iter().map(|e| e.filename.clone()).collect();

        for record in &manifest.uploads {
            if record.success && !in_queue.contains(&record.filename) {
                queue.push(UploadQueueEntry {
                    filename: record.filename.clone(),
                    path: String::new(),
                    size_bytes: record.size_bytes,
                    status: QueueStatus::Completed {
                        sha256: record.sha256.clone(),
                    },
                });
            }
        }
    }

    queue
}

/// Read manifest from a directory.
#[tauri::command]
pub fn get_upload_manifest(dir: String) -> UploadManifest {
    load_manifest(Path::new(&dir))
}

/// Retry all failed uploads.
#[tauri::command]
pub fn retry_failed(state: State<'_, Arc<AppState>>) {
    let mut queue = state.upload_queue.write();
    for entry in queue.iter_mut() {
        if matches!(entry.status, QueueStatus::Failed { .. }) {
            entry.status = QueueStatus::Pending;
        }
    }
}

/// Cancel an in-progress upload by marking it failed.
#[tauri::command]
pub fn cancel_upload(filename: String, state: State<'_, Arc<AppState>>) {
    let mut queue = state.upload_queue.write();
    if let Some(entry) = queue.iter_mut().find(|e| e.filename == filename) {
        entry.status = QueueStatus::Failed {
            error: "Cancelled by user".to_string(),
        };
    }
}

/// Test S3 connection with current config.
#[tauri::command]
pub async fn test_upload_connection(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let upload_config = {
        let config = state.config.read();
        config.upload.clone()
    };
    s3_upload::test_connection(&upload_config).await
}

/// Toggle auto-upload on/off.
#[tauri::command]
pub fn toggle_auto_upload(enable: bool, state: State<'_, Arc<AppState>>) {
    state.upload_enabled.store(enable, Ordering::Relaxed);
    log::info!("Auto-upload {}", if enable { "enabled" } else { "disabled" });
}

fn set_queue_status(state: &AppState, filename: &str, status: QueueStatus) {
    let mut queue = state.upload_queue.write();
    if let Some(entry) = queue.iter_mut().find(|e| e.filename == filename) {
        entry.status = status;
    }
}
