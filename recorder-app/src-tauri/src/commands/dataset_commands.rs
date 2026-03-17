use crate::library::EgorecListItem;
use crate::dataset::convert::{convert_dataset_to_lerobot, ConversionProgress};
use crate::dataset::manifest;
use crate::dataset::scan::{scan_datasets, DatasetSummary};
use crate::recorder::status::RecorderState;
use crate::state::{AppState, ConversionStatus, EgorecMetadataDto};
use crate::upload::upload_queue::{load_manifest as load_upload_manifest, QueueStatus, UploadQueueEntry};
use crate::upload::s3_upload::{build_s3_client, compute_sha256, make_object_key, upload_file, UploadProgressEvent};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Emitter;
use tauri::State;

#[tauri::command]
pub async fn list_datasets(state: State<'_, Arc<AppState>>) -> Result<Vec<DatasetSummary>, String> {
    let output_dir = {
        let config = state.config.read();
        config
            .storage
            .output_dir
            .clone()
            .ok_or("No output directory configured")?
    };

    let dir = PathBuf::from(&output_dir);
    tokio::task::spawn_blocking(move || scan_datasets(&dir))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

#[tauri::command]
pub async fn create_dataset(
    name: String,
    target_episodes: Option<u32>,
    state: State<'_, Arc<AppState>>,
) -> Result<DatasetSummary, String> {
    let output_dir = {
        let config = state.config.read();
        config
            .storage
            .output_dir
            .clone()
            .ok_or("No output directory configured")?
    };

    let dir = PathBuf::from(&output_dir);
    let manifest_result =
        tokio::task::spawn_blocking(move || manifest::create_dataset(&dir, &name, target_episodes))
            .await
            .map_err(|e| format!("Task error: {}", e))??;

    // Return a summary for the newly created dataset
    let dir_name = sanitize_dir_name(&manifest_result.name);
    Ok(DatasetSummary {
        name: manifest_result.name,
        dir_name,
        description: manifest_result.description,
        tags: manifest_result.tags,
        file_count: 0,
        total_frames: 0,
        total_duration_s: 0.0,
        total_size_bytes: 0,
        uploaded_count: 0,
        has_lerobot: false,
        created_at: manifest_result.created_at,
        updated_at: manifest_result.updated_at,
        target_episodes: manifest_result.target_episodes,
    })
}

#[tauri::command]
pub async fn update_dataset(
    dir_name: String,
    name: String,
    description: String,
    tags: Vec<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let output_dir = {
        let config = state.config.read();
        config
            .storage
            .output_dir
            .clone()
            .ok_or("No output directory configured")?
    };

    let dataset_dir = PathBuf::from(&output_dir).join(&dir_name);
    tokio::task::spawn_blocking(move || {
        let mut m = manifest::load_manifest(&dataset_dir)
            .ok_or("Dataset manifest not found")?;
        m.name = name;
        m.description = description;
        m.tags = tags;
        m.updated_at = Utc::now().to_rfc3339();
        manifest::save_manifest(&dataset_dir, &m)
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

#[tauri::command]
pub async fn delete_dataset(
    dir_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let output_dir = {
        let config = state.config.read();
        config
            .storage
            .output_dir
            .clone()
            .ok_or("No output directory configured")?
    };

    let dataset_dir = PathBuf::from(&output_dir).join(&dir_name);
    tokio::task::spawn_blocking(move || manifest::delete_dataset(&dataset_dir))
        .await
        .map_err(|e| format!("Task error: {}", e))?
}

#[tauri::command]
pub async fn get_dataset_files(
    dir_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<EgorecListItem>, String> {
    let output_dir = {
        let config = state.config.read();
        config
            .storage
            .output_dir
            .clone()
            .ok_or("No output directory configured")?
    };

    let dataset_dir = PathBuf::from(&output_dir).join(&dir_name);
    tokio::task::spawn_blocking(move || scan_dataset_files(&dataset_dir, &dir_name))
        .await
        .map_err(|e| format!("Task error: {}", e))?
}

#[tauri::command]
pub async fn upload_dataset(
    dir_name: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<usize, String> {
    let (output_dir, upload_config) = {
        let config = state.config.read();
        (
            config
                .storage
                .output_dir
                .clone()
                .ok_or("No output directory configured")?,
            config.upload.clone(),
        )
    };

    let output_path = PathBuf::from(&output_dir);
    let dataset_dir = output_path.join(&dir_name);

    // Find .egorec files not yet uploaded
    let upload_manifest = load_upload_manifest(&output_path);
    let uploaded_files = upload_manifest.uploaded_files();

    let mut files_to_upload = Vec::new();
    for entry in walkdir::WalkDir::new(&dataset_dir)
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
        let rel_path = path
            .strip_prefix(&output_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if !uploaded_files.contains(rel_path.as_str()) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files_to_upload.push((path, rel_path, size));
        }
    }

    // Queue each file for upload
    let bucket = upload_config
        .bucket
        .clone()
        .ok_or("Bucket not configured")?;

    // Add all files to queue as Pending (filter out already-queued)
    let mut queued_files = Vec::new();
    {
        let mut queue = state.upload_queue.write();
        for (file_path, filename, size_bytes) in files_to_upload {
            // Skip files that are currently in-flight (pending, hashing, or uploading)
            let dominated = queue.iter().any(|e| {
                e.filename == filename
                    && matches!(
                        e.status,
                        QueueStatus::Pending
                            | QueueStatus::Hashing { .. }
                            | QueueStatus::Uploading { .. }
                    )
            });
            if dominated {
                continue;
            }
            // Remove stale failed/completed entries so we can re-queue
            queue.retain(|e| e.filename != filename);
            queue.push(UploadQueueEntry {
                filename: filename.clone(),
                path: file_path.to_string_lossy().to_string(),
                size_bytes,
                status: QueueStatus::Pending,
            });
            queued_files.push((file_path, filename, size_bytes));
        }
    }

    let count = queued_files.len();

    if !queued_files.is_empty() {
        let state_clone = state.inner().clone();
        let app_handle_clone = app_handle.clone();

        tauri::async_runtime::spawn(async move {
            upload_dataset_files(
                queued_files, state_clone, app_handle_clone,
                upload_config, bucket, output_path,
            ).await;
        });
    }

    Ok(count)
}

#[tauri::command]
pub async fn convert_dataset(
    dir_name: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    // Check if already running
    if state
        .conversion_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A conversion is already in progress".to_string());
    }

    let output_dir = {
        let config = state.config.read();
        config
            .storage
            .output_dir
            .clone()
            .ok_or_else(|| {
                state.conversion_running.store(false, Ordering::SeqCst);
                "No output directory configured".to_string()
            })?
    };

    let dataset_dir = PathBuf::from(&output_dir).join(&dir_name);

    // Get dataset name from manifest
    let dataset_name = manifest::load_manifest(&dataset_dir)
        .map(|m| m.name)
        .unwrap_or_else(|| dir_name.clone());

    let (tx, rx) = crossbeam_channel::unbounded::<ConversionProgress>();

    // Spawn blocking conversion task
    let dataset_dir_clone = dataset_dir.clone();
    let dataset_name_clone = dataset_name.clone();
    tokio::task::spawn_blocking(move || {
        let result = convert_dataset_to_lerobot(
            &dataset_dir_clone,
            "ego_recording",
            &dataset_name_clone,
            tx,
        );

        if let Err(ref e) = result {
            log::error!("Dataset conversion failed: {}", e);
        }

        result
    });

    // Spawn async task to relay progress from crossbeam channel to Tauri events
    let state_for_relay = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.try_recv() {
                Ok(progress) => {
                    let is_done = progress.phase == "completed" || progress.phase == "error";
                    *state_for_relay.conversion_progress.write() = Some(progress.clone());
                    let _ = app_handle.emit("dataset:convert_progress", &progress);

                    if is_done {
                        state_for_relay
                            .conversion_running
                            .store(false, Ordering::SeqCst);
                        break;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    // Sender dropped — conversion finished (possibly with error)
                    // Check if we already got a completion signal
                    let already_done = state_for_relay
                        .conversion_progress
                        .read()
                        .as_ref()
                        .map(|p| p.phase == "completed" || p.phase == "error")
                        .unwrap_or(false);

                    if !already_done {
                        let error_progress = ConversionProgress {
                            dataset_name: dataset_name.clone(),
                            current_file: String::new(),
                            file_index: 0,
                            total_files: 0,
                            frames_done: 0,
                            total_frames: 0,
                            phase: "error".to_string(),
                            error: Some("Conversion task ended unexpectedly".to_string()),
                        };
                        *state_for_relay.conversion_progress.write() = Some(error_progress.clone());
                        let _ = app_handle.emit("dataset:convert_progress", &error_progress);
                    }

                    state_for_relay
                        .conversion_running
                        .store(false, Ordering::SeqCst);
                    break;
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn get_conversion_status(
    state: State<'_, Arc<AppState>>,
) -> Option<ConversionProgress> {
    state.conversion_progress.read().clone()
}

/// Scan .egorec files in a dataset directory and return as EgorecListItem list.
fn scan_dataset_files(dataset_dir: &std::path::Path, dir_name: &str) -> Result<Vec<EgorecListItem>, String> {
    use egorec::format::*;
    use std::io::{BufReader, Seek, SeekFrom};

    let mut items = Vec::new();

    for entry in walkdir::WalkDir::new(dataset_dir)
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
        let name = path
            .strip_prefix(dataset_dir.parent().unwrap_or(dataset_dir))
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(file);

        let header = match FileHeader::read_from(&mut reader) {
            Ok(h) => h,
            Err(_) => continue,
        };

        let file_size = match reader.get_ref().metadata() {
            Ok(m) => m.len(),
            Err(_) => continue,
        };

        if file_size < (FILE_HEADER_SIZE as u64 + FileFooter::SIZE as u64) {
            continue;
        }

        if reader
            .seek(SeekFrom::End(-(FileFooter::SIZE as i64)))
            .is_err()
        {
            continue;
        }

        let footer = match FileFooter::read_from(&mut reader) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let metadata = EgorecMetadataDto::from_header(&header, footer.total_frames, footer.total_duration_us);

        let conversion_status = if metadata.rgb_codec == 2 {
            ConversionStatus::Streamable
        } else {
            ConversionStatus::Idle
        };

        items.push(EgorecListItem {
            name,
            dataset: Some(dir_name.to_string()),
            session_name: metadata.session_name.clone(),
            rgb_codec: metadata.rgb_codec,
            color_width: metadata.color_width,
            color_height: metadata.color_height,
            fps: metadata.fps,
            total_frames: metadata.total_frames,
            duration_s: metadata.duration_s,
            size_bytes,
            conversion_status,
            has_imu: metadata.has_imu,
        });
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

/// Result of a successful single-file upload (returned to orchestrator for manifest write).
struct UploadResult {
    filename: String,
    r2_key: String,
    size_bytes: u64,
    sha256: String,
}

/// Orchestrate uploading a batch of files with bounded concurrency (max 2 in-flight).
/// Manifest writes happen here (sequentially) to avoid concurrent-write races.
async fn upload_dataset_files(
    files: Vec<(PathBuf, String, u64)>,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    upload_config: crate::config::UploadConfig,
    bucket: String,
    output_path: PathBuf,
) {
    // Build one shared S3 client for the whole batch
    let client = match build_s3_client(&upload_config) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            // Mark all files as failed
            for (_, filename, _) in &files {
                set_queue_status(&state, filename, QueueStatus::Failed { error: e.clone() });
            }
            return;
        }
    };

    const MAX_CONCURRENT: usize = 2;
    let mut join_set = tokio::task::JoinSet::new();
    let mut file_iter = files.into_iter();

    // Seed with initial batch
    for _ in 0..MAX_CONCURRENT {
        if let Some((file_path, filename, size_bytes)) = file_iter.next() {
            let r2_key = make_object_key(upload_config.prefix.as_deref(), &filename);
            join_set.spawn(upload_single_file(
                file_path, filename, size_bytes, r2_key,
                state.clone(), app_handle.clone(), client.clone(),
                upload_config.multipart_chunk_mb, bucket.clone(),
            ));
        }
    }

    // As each completes, record in manifest (sequential — no race), then spawn next
    while let Some(result) = join_set.join_next().await {
        if let Ok(Some(upload)) = result {
            let mut manifest = crate::upload::upload_queue::load_manifest(&output_path);
            crate::upload::upload_queue::record_upload(
                &mut manifest,
                upload.filename,
                upload.r2_key,
                upload.size_bytes,
                upload.sha256,
                1,
            );
            if let Err(e) = crate::upload::upload_queue::save_manifest(&output_path, &manifest) {
                log::error!("Failed to save manifest: {}", e);
            }
        }

        if let Some((file_path, filename, size_bytes)) = file_iter.next() {
            let r2_key = make_object_key(upload_config.prefix.as_deref(), &filename);
            join_set.spawn(upload_single_file(
                file_path, filename, size_bytes, r2_key,
                state.clone(), app_handle.clone(), client.clone(),
                upload_config.multipart_chunk_mb, bucket.clone(),
            ));
        }
    }
}

/// Upload a single file: hash → upload. Returns [UploadResult] on success for manifest recording.
async fn upload_single_file(
    file_path: PathBuf,
    filename: String,
    size_bytes: u64,
    r2_key: String,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    client: Arc<aws_sdk_s3::Client>,
    base_chunk_mb: u32,
    bucket: String,
) -> Option<UploadResult> {
    // Hash
    set_queue_status(&state, &filename, QueueStatus::Hashing { progress: 0.0 });

    let filename_for_hash = filename.clone();
    let app_handle_hash = app_handle.clone();
    let sha256 = match compute_sha256(&file_path, size_bytes, move |bytes_done| {
        let progress = UploadProgressEvent {
            filename: filename_for_hash.clone(),
            bytes_transferred: bytes_done,
            total_bytes: size_bytes,
            speed_bps: 0,
            phase: "hashing".to_string(),
        };
        let _ = app_handle_hash.emit("upload:progress", &progress);
    })
    .await
    {
        Ok(h) => h,
        Err(e) => {
            set_queue_status(&state, &filename, QueueStatus::Failed { error: e });
            return None;
        }
    };

    // Throttle chunk size during recording
    let is_recording = {
        let status = state.recorder_status.read();
        status.state == RecorderState::Recording
    };
    let chunk_mb = if is_recording {
        std::cmp::min(base_chunk_mb, 5)
    } else {
        base_chunk_mb
    };

    set_queue_status(
        &state,
        &filename,
        QueueStatus::Uploading {
            progress: 0.0,
            speed_bps: 0,
        },
    );

    let state_for_progress = state.clone();
    let filename_for_progress = filename.clone();
    match upload_file(
        &client,
        &bucket,
        &r2_key,
        &file_path,
        chunk_mb,
        &app_handle,
        &filename,
        move |bytes_transferred, total_bytes, speed_bps| {
            let progress = if total_bytes > 0 {
                bytes_transferred as f64 / total_bytes as f64
            } else {
                0.0
            };
            set_queue_status(
                &state_for_progress,
                &filename_for_progress,
                QueueStatus::Uploading { progress, speed_bps },
            );
        },
    )
    .await
    {
        Ok(()) => {
            set_queue_status(
                &state,
                &filename,
                QueueStatus::Completed {
                    sha256: sha256.clone(),
                },
            );

            let progress = UploadProgressEvent {
                filename: filename.clone(),
                bytes_transferred: size_bytes,
                total_bytes: size_bytes,
                speed_bps: 0,
                phase: "completed".to_string(),
            };
            let _ = app_handle.emit("upload:progress", &progress);

            Some(UploadResult {
                filename,
                r2_key,
                size_bytes,
                sha256,
            })
        }
        Err(e) => {
            set_queue_status(&state, &filename, QueueStatus::Failed { error: e });
            None
        }
    }
}

fn set_queue_status(state: &AppState, filename: &str, status: QueueStatus) {
    let mut queue = state.upload_queue.write();
    if let Some(entry) = queue.iter_mut().find(|e| e.filename == filename) {
        entry.status = status;
    }
}

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
