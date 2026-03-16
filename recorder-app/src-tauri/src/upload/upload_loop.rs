use crate::recorder::status::RecorderState;
use crate::state::AppState;
use crate::upload::s3_upload::{
    build_s3_client, compute_sha256, make_object_key, upload_file, UploadProgressEvent,
};
use crate::upload::upload_queue::{
    load_manifest, record_upload, save_manifest, scan_pending, QueueStatus, UploadQueueEntry,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Emitter;

/// Spawn the background upload loop.
pub fn spawn_upload_loop(app_handle: tauri::AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        upload_loop(app_handle, state).await;
    });
}

async fn upload_loop(app_handle: tauri::AppHandle, state: Arc<AppState>) {
    let mut s3_client: Option<aws_sdk_s3::Client> = None;
    let mut backoff_map: HashMap<String, (u32, std::time::Instant)> = HashMap::new();

    loop {
        let poll_interval_s = {
            let config = state.config.read();
            config.upload.poll_interval_s
        };

        tokio::time::sleep(std::time::Duration::from_secs(poll_interval_s)).await;

        // Check if auto-upload is enabled
        if !state.upload_enabled.load(Ordering::Relaxed) {
            continue;
        }

        // Get config snapshot
        let (output_dir, upload_config) = {
            let config = state.config.read();
            let output_dir = config.storage.output_dir.clone();
            let upload_config = config.upload.clone();
            (output_dir, upload_config)
        };

        let output_dir = match output_dir {
            Some(d) => d,
            None => continue,
        };

        // Check if credentials are configured
        if upload_config.endpoint.is_none()
            || upload_config.bucket.is_none()
            || upload_config.access_key.is_none()
            || upload_config.secret_key.is_none()
        {
            continue;
        }

        let dir = Path::new(&output_dir);
        if !dir.exists() {
            continue;
        }

        // Load manifest and scan for pending files
        let manifest = load_manifest(dir);
        let pending = scan_pending(dir, &manifest, upload_config.file_settle_s);

        if pending.is_empty() {
            continue;
        }

        // Determine throttling based on recording state
        let is_recording = {
            let status = state.recorder_status.read();
            status.state == RecorderState::Recording
        };
        let chunk_mb = if is_recording {
            std::cmp::min(upload_config.multipart_chunk_mb, 5)
        } else {
            upload_config.multipart_chunk_mb
        };

        // Ensure S3 client exists
        if s3_client.is_none() {
            match build_s3_client(&upload_config) {
                Ok(c) => s3_client = Some(c),
                Err(e) => {
                    log::error!("Failed to create S3 client: {}", e);
                    continue;
                }
            }
        }

        // Reload manifest (in case another process updated it)
        let mut manifest = load_manifest(dir);
        let bucket = upload_config.bucket.as_deref().unwrap();

        // Process one file at a time
        for entry in &pending {
            // Check if we're still enabled
            if !state.upload_enabled.load(Ordering::Relaxed) {
                break;
            }

            // Check backoff (saturating to prevent panic on large attempt counts)
            if let Some((attempts, last_attempt)) = backoff_map.get(&entry.filename) {
                let backoff_secs = 2u64
                    .saturating_pow(*attempts)
                    .saturating_mul(10)
                    .min(300);
                if last_attempt.elapsed().as_secs() < backoff_secs {
                    continue;
                }
            }

            let file_path = Path::new(&entry.path);
            if !file_path.exists() {
                continue;
            }

            let r2_key = make_object_key(
                upload_config.prefix.as_deref(),
                &entry.filename,
            );

            // Update in-memory queue: hashing
            update_queue_status(
                &state,
                &entry.filename,
                entry,
                QueueStatus::Hashing { progress: 0.0 },
            );

            // Compute SHA-256
            let filename_for_event = entry.filename.clone();
            let total_size = entry.size_bytes;
            let app_handle_hash = app_handle.clone();
            let sha256 = match compute_sha256(file_path, total_size, move |bytes_done| {
                let progress = UploadProgressEvent {
                    filename: filename_for_event.clone(),
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
                    log::error!("Failed to hash {}: {}", entry.filename, e);
                    update_queue_status(
                        &state,
                        &entry.filename,
                        entry,
                        QueueStatus::Failed {
                            error: e.to_string(),
                        },
                    );
                    record_backoff(&mut backoff_map, &entry.filename);
                    continue;
                }
            };

            // Update in-memory queue: uploading
            update_queue_status(
                &state,
                &entry.filename,
                entry,
                QueueStatus::Uploading {
                    progress: 0.0,
                    speed_bps: 0,
                },
            );

            // Upload
            let client = s3_client.as_ref().unwrap();
            match upload_file(
                client,
                bucket,
                &r2_key,
                file_path,
                chunk_mb,
                &app_handle,
                &entry.filename,
            )
            .await
            {
                Ok(()) => {
                    log::info!("Uploaded {} -> {}", entry.filename, r2_key);

                    // Record in manifest and persist — if save fails, don't mark
                    // completed so the file will be retried next cycle instead of
                    // silently re-uploaded forever.
                    let attempt_count = backoff_map
                        .get(&entry.filename)
                        .map(|(a, _)| *a + 1)
                        .unwrap_or(1);
                    record_upload(
                        &mut manifest,
                        entry.filename.clone(),
                        r2_key.clone(),
                        entry.size_bytes,
                        sha256.clone(),
                        attempt_count,
                    );
                    if let Err(e) = save_manifest(dir, &manifest) {
                        log::error!("Failed to save manifest after upload of {}: {} — will retry", entry.filename, e);
                        // Remove the in-memory record so next cycle re-adds it
                        manifest.uploads.retain(|r| r.filename != entry.filename);
                        record_backoff(&mut backoff_map, &entry.filename);
                        update_queue_status(
                            &state,
                            &entry.filename,
                            entry,
                            QueueStatus::Failed {
                                error: format!("Upload succeeded but manifest save failed: {}", e),
                            },
                        );
                        continue;
                    }

                    // Clear backoff
                    backoff_map.remove(&entry.filename);

                    // Update queue status
                    update_queue_status(
                        &state,
                        &entry.filename,
                        entry,
                        QueueStatus::Completed {
                            sha256: sha256.clone(),
                        },
                    );

                    // Emit completion event
                    let progress = UploadProgressEvent {
                        filename: entry.filename.clone(),
                        bytes_transferred: entry.size_bytes,
                        total_bytes: entry.size_bytes,
                        speed_bps: 0,
                        phase: "completed".to_string(),
                    };
                    let _ = app_handle.emit("upload:progress", &progress);
                }
                Err(e) => {
                    log::error!("Failed to upload {}: {}", entry.filename, e);

                    // Re-create S3 client on next attempt (connection may be stale)
                    s3_client = None;

                    update_queue_status(
                        &state,
                        &entry.filename,
                        entry,
                        QueueStatus::Failed {
                            error: e.to_string(),
                        },
                    );
                    record_backoff(&mut backoff_map, &entry.filename);

                    // Stop processing more files after a failure
                    break;
                }
            }
        }
    }
}

fn update_queue_status(
    state: &AppState,
    filename: &str,
    entry: &UploadQueueEntry,
    status: QueueStatus,
) {
    let mut queue = state.upload_queue.write();
    if let Some(existing) = queue.iter_mut().find(|e| e.filename == filename) {
        existing.status = status;
    } else {
        queue.push(UploadQueueEntry {
            filename: filename.to_string(),
            path: entry.path.clone(),
            size_bytes: entry.size_bytes,
            status,
        });
    }
}

fn record_backoff(backoff_map: &mut HashMap<String, (u32, std::time::Instant)>, filename: &str) {
    let attempts = backoff_map
        .get(filename)
        .map(|(a, _)| a + 1)
        .unwrap_or(1);
    backoff_map.insert(filename.to_string(), (attempts, std::time::Instant::now()));
}
