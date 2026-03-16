use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::state::{AppState, ConversionStatus, FileEntry};

use super::{extract_dataset, parse_egorec_metadata, EgorecListItem};

/// How long a file must be unchanged before we consider it "settled" (done writing).
const SETTLE_DURATION: Duration = Duration::from_secs(2);

/// Commands sent from Tauri commands to the watcher task.
pub enum WatcherCommand {
    /// Start watching a new directory (stops the previous watch).
    Watch(PathBuf),
    /// Stop watching entirely.
    Stop,
}

/// Spawn the file watcher background task. Returns a sender for commands.
pub fn spawn_file_watcher(
    app_handle: AppHandle,
    state: Arc<AppState>,
) -> mpsc::Sender<WatcherCommand> {
    let (cmd_tx, cmd_rx) = mpsc::channel(16);

    tauri::async_runtime::spawn(async move {
        watcher_loop(app_handle, state, cmd_rx).await;
    });

    cmd_tx
}

/// Holds the active notify watcher and its event channel.
struct ActiveWatcher {
    _watcher: RecommendedWatcher,
    event_rx: mpsc::Receiver<Event>,
    pending: HashMap<PathBuf, Instant>,
}

async fn watcher_loop(
    app_handle: AppHandle,
    state: Arc<AppState>,
    mut cmd_rx: mpsc::Receiver<WatcherCommand>,
) {
    // Start with configured output_dir
    let initial_dir = state.config.read().storage.output_dir.clone();

    let mut current_dir: Option<PathBuf> = initial_dir.map(PathBuf::from);
    let mut active: Option<ActiveWatcher> = None;

    // Start watching initial dir if configured
    if let Some(ref dir) = current_dir {
        if dir.is_dir() {
            active = create_watcher(dir);
            if active.is_some() {
                *state.watched_dir.write() = Some(dir.to_string_lossy().to_string());
                log::info!("File watcher started on: {:?}", dir);
            }
        }
    }

    loop {
        if let Some(ref mut w) = active {
            let watch_dir = current_dir.as_ref().unwrap();

            tokio::select! {
                biased;

                // Commands take priority
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(WatcherCommand::Watch(dir)) => {
                            active = None;
                            current_dir = Some(dir.clone());
                            if dir.is_dir() {
                                active = create_watcher(&dir);
                                if active.is_some() {
                                    *state.watched_dir.write() =
                                        Some(dir.to_string_lossy().to_string());
                                    log::info!("File watcher switched to: {:?}", dir);
                                }
                            } else {
                                *state.watched_dir.write() = None;
                            }
                        }
                        Some(WatcherCommand::Stop) | None => return,
                    }
                }

                // Process filesystem events
                event = w.event_rx.recv() => {
                    match event {
                        Some(ev) => {
                            handle_event(ev, watch_dir, &mut w.pending, &app_handle, &state);
                        }
                        None => {
                            // Watcher channel closed unexpectedly
                            log::warn!("File watcher channel closed, stopping");
                            active = None;
                            *state.watched_dir.write() = None;
                        }
                    }
                }

                // Periodic settle check
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    process_settled(&mut w.pending, watch_dir, &app_handle, &state);
                }
            }
        } else {
            // No active watcher — wait for a command
            match cmd_rx.recv().await {
                Some(WatcherCommand::Watch(dir)) => {
                    current_dir = Some(dir.clone());
                    if dir.is_dir() {
                        active = create_watcher(&dir);
                        if active.is_some() {
                            *state.watched_dir.write() =
                                Some(dir.to_string_lossy().to_string());
                            log::info!("File watcher started on: {:?}", dir);
                        }
                    }
                }
                Some(WatcherCommand::Stop) | None => return,
            }
        }
    }
}

fn create_watcher(dir: &PathBuf) -> Option<ActiveWatcher> {
    let (tx, rx) = mpsc::channel(256);

    let mut watcher = match RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.blocking_send(event);
            }
        },
        Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            log::error!("Failed to create file watcher: {}", e);
            return None;
        }
    };

    if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
        log::error!("Failed to watch {:?}: {}", dir, e);
        return None;
    }

    Some(ActiveWatcher {
        _watcher: watcher,
        event_rx: rx,
        pending: HashMap::new(),
    })
}

fn is_egorec_file(path: &std::path::Path) -> bool {
    path.extension().map_or(false, |ext| ext == "egorec")
        && !path.to_string_lossy().contains(".pruned")
}

fn handle_event(
    event: Event,
    watch_dir: &PathBuf,
    pending: &mut HashMap<PathBuf, Instant>,
    app_handle: &AppHandle,
    state: &Arc<AppState>,
) {
    for path in &event.paths {
        if !is_egorec_file(path) {
            continue;
        }

        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                pending.insert(path.clone(), Instant::now());
            }
            EventKind::Remove(_) => {
                pending.remove(path);
                let name = relative_name(path, watch_dir);
                state.file_index.write().remove(&name);
                let _ = app_handle.emit("library:file-removed", &name);
                log::info!("Recording removed: {}", name);
            }
            _ => {}
        }
    }
}

fn process_settled(
    pending: &mut HashMap<PathBuf, Instant>,
    watch_dir: &PathBuf,
    app_handle: &AppHandle,
    state: &Arc<AppState>,
) {
    let now = Instant::now();
    let settled: Vec<PathBuf> = pending
        .iter()
        .filter(|(_, last_event)| now.duration_since(**last_event) >= SETTLE_DURATION)
        .map(|(path, _)| path.clone())
        .collect();

    for path in settled {
        pending.remove(&path);

        if !path.exists() {
            // File was created then quickly deleted — skip
            continue;
        }

        match parse_egorec_metadata(&path) {
            Ok(metadata) => {
                let name = relative_name(&path, watch_dir);
                let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let conversion_status = if metadata.rgb_codec == 2 {
                    ConversionStatus::Streamable
                } else {
                    ConversionStatus::Idle
                };

                let item = EgorecListItem {
                    name: name.clone(),
                    dataset: extract_dataset(&name),
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
                };

                state.file_index.write().insert(
                    name.clone(),
                    FileEntry {
                        name: name.clone(),
                        path: path.clone(),
                        size_bytes,
                        metadata,
                        conversion_status,
                    },
                );

                let _ = app_handle.emit("library:file-added", &item);
                log::info!("New recording settled: {}", name);
            }
            Err(e) => {
                log::debug!("Skipping {:?}: {}", path, e);
            }
        }
    }
}

fn relative_name(path: &std::path::Path, watch_dir: &std::path::Path) -> String {
    path.strip_prefix(watch_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}
