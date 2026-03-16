use crate::preview::{frame_reader, CameraInfo, PreviewState};
use crate::recorder::status::RecorderState;
use crate::recorder::subprocess::parse_stats_line;
use crate::state::AppState;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

/// Spawn the `ego-recorder preview` subprocess and start streaming frames.
/// Returns camera info once the subprocess is ready.
#[tauri::command]
pub async fn start_preview(
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<CameraInfo, String> {
    // Serialize lifecycle operations (Fix 6)
    let _lifecycle_guard = state.preview_lock.lock().await;

    // Check not already running
    {
        let ps = *state.preview_state.read();
        if ps != PreviewState::Off && ps != PreviewState::Error {
            return Err("Preview already running".into());
        }
    }

    // Bump generation to invalidate stale background tasks (Fix 7)
    let generation = state
        .preview_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;

    // Clean up any old task handles
    {
        let mut tasks = state.preview_tasks.lock().await;
        tasks.clear();
    }

    *state.preview_state.write() = PreviewState::Starting;

    let binary_path = {
        let config = state.config.read();
        config
            .recorder
            .binary_path
            .clone()
            .ok_or_else(|| {
                *state.preview_state.write() = PreviewState::Error;
                "Recorder binary not configured. Please set it in Settings.".to_string()
            })?
    };

    let warmup = state.config.read().recorder.warmup_frames;
    let preset = state.config.read().recorder.h264_preset.clone();

    let mut child = Command::new(&binary_path)
        .arg("preview")
        .arg("--warmup")
        .arg(warmup.to_string())
        .arg("--preset")
        .arg(&preset)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            *state.preview_state.write() = PreviewState::Error;
            format!("Failed to spawn ego-recorder preview: {}", e)
        })?;

    let pid = child.id().ok_or_else(|| {
        *state.preview_state.write() = PreviewState::Error;
        "Failed to get preview PID".to_string()
    })?;

    *state.preview_pid.write() = Some(pid);

    // Take ownership of stdin, stdout, stderr
    let stdin = child.stdin.take().ok_or("No stdin")?;
    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    // Store stdin for sending commands later
    *state.preview_stdin.lock().await = Some(stdin);

    // Create oneshot for camera info
    let (info_tx, info_rx) = oneshot::channel();

    // Spawn stdout frame reader task (Fix 9: store handle)
    let rgb_tx = state.rgb_frame_tx.clone();
    let depth_tx = state.depth_frame_tx.clone();
    let state_for_reader = Arc::clone(&state);
    let app_for_reader = app_handle.clone();
    let reader_gen = generation;
    let reader_handle = tokio::spawn(async move {
        frame_reader::read_frames(stdout, info_tx, rgb_tx, depth_tx).await;
        // Frame reader exited = subprocess stdout closed
        // Only update state if generation matches (Fix 7)
        if state_for_reader
            .preview_generation
            .load(Ordering::SeqCst)
            == reader_gen
        {
            *state_for_reader.preview_state.write() = PreviewState::Off;
            *state_for_reader.preview_pid.write() = None;
            *state_for_reader.preview_stdin.lock().await = None;
            let _ = app_for_reader.emit("preview:disconnected", ());
        }
    });

    // Spawn stderr reader task (Fix 9: store handle)
    let state_for_stderr = Arc::clone(&state);
    let app_for_stderr = app_handle.clone();
    let stderr_gen = generation;
    let stderr_handle = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // Skip if generation changed (Fix 7)
            if state_for_stderr
                .preview_generation
                .load(Ordering::SeqCst)
                != stderr_gen
            {
                break;
            }

            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            // Check for disconnect/reconnect sentinels
            if line == "DISCONNECTED" {
                *state_for_stderr.preview_state.write() = PreviewState::Error;
                let _ = app_for_stderr.emit("preview:state-changed", "error");
                continue;
            }
            if line == "RECONNECTED" {
                *state_for_stderr.preview_state.write() = PreviewState::Previewing;
                let _ = app_for_stderr.emit("preview:state-changed", "previewing");
                continue;
            }

            // Parse stats lines
            if let Some(status) = parse_stats_line(&line) {
                *state_for_stderr.recorder_status.write() = status.clone();
                let _ = app_for_stderr.emit("recorder:stats", status);
            }

            if line.contains("Recording complete") {
                let ps = *state_for_stderr.preview_state.read();
                // Only transition if we were recording or stopping — avoid
                // overwriting states set by start_recording/stop_recording.
                if ps == PreviewState::Recording || ps == PreviewState::Stopping {
                    *state_for_stderr.preview_state.write() = PreviewState::Previewing;
                    let _ = app_for_stderr.emit("preview:state-changed", "previewing");
                }
                let mut s = state_for_stderr.recorder_status.write();
                s.state = RecorderState::Idle;
                let _ = app_for_stderr.emit("recorder:stats", s.clone());
                // Notify frontend so it can reset recordingInFlight
                let _ = app_for_stderr.emit("recorder:stopped", "clean");
            }
        }

        // Stderr closed = process exited
        // Only update state if generation matches (Fix 7)
        if state_for_stderr
            .preview_generation
            .load(Ordering::SeqCst)
            != stderr_gen
        {
            return;
        }

        let exit_status = child.wait().await;
        let mut s = state_for_stderr.recorder_status.write();
        s.state = RecorderState::Idle;
        *state_for_stderr.preview_pid.write() = None;
        *state_for_stderr.preview_state.write() = PreviewState::Off;

        match exit_status {
            Ok(status) if status.success() => {
                let _ = app_for_stderr.emit("recorder:stopped", "clean");
            }
            Ok(status) => {
                let code = status.code().unwrap_or(-1);
                s.state = RecorderState::Error;
                let _ = app_for_stderr.emit("recorder:stopped", format!("exit code {}", code));
            }
            Err(e) => {
                s.state = RecorderState::Error;
                let _ = app_for_stderr.emit("recorder:stopped", format!("error: {}", e));
            }
        }
    });

    // Store task handles (Fix 9)
    {
        let mut tasks = state.preview_tasks.lock().await;
        tasks.push(reader_handle);
        tasks.push(stderr_handle);
    }

    // Wait for camera info with timeout (Fix 5)
    let camera_info = match tokio::time::timeout(Duration::from_secs(15), info_rx).await {
        Ok(Ok(info)) => info,
        Ok(Err(_)) => {
            // Channel dropped -- subprocess died
            // Kill subprocess to be safe
            kill_preview_process(pid);
            *state.preview_state.write() = PreviewState::Error;
            return Err("Preview subprocess failed to send camera info".to_string());
        }
        Err(_) => {
            // Timeout -- subprocess hung (e.g. no camera plugged in)
            kill_preview_process(pid);
            *state.preview_state.write() = PreviewState::Error;
            *state.preview_pid.write() = None;
            *state.preview_stdin.lock().await = None;
            return Err(
                "Camera not detected within 15 seconds. Check USB connection.".to_string(),
            );
        }
    };

    if camera_info.serial.is_empty() {
        *state.preview_state.write() = PreviewState::Error;
        return Err("Preview subprocess failed to initialize camera".into());
    }

    *state.camera_info.write() = Some(camera_info.clone());
    *state.preview_state.write() = PreviewState::Previewing;
    let _ = app_handle.emit("preview:camera-info", &camera_info);
    let _ = app_handle.emit("preview:state-changed", "previewing");

    Ok(camera_info)
}

/// Stop the preview subprocess.
#[tauri::command]
pub async fn stop_preview(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Serialize lifecycle operations (Fix 6)
    let _lifecycle_guard = state.preview_lock.lock().await;

    let pid = state
        .preview_pid
        .read()
        .ok_or("No preview process running")?;

    // Bump generation to invalidate background tasks (Fix 7)
    state.preview_generation.fetch_add(1, Ordering::SeqCst);

    // Send SIGINT for graceful shutdown
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGINT)
        .map_err(|e| format!("Failed to send SIGINT to preview: {}", e))?;

    *state.preview_state.write() = PreviewState::Off;

    // Drop stdin to trigger EOF in subprocess
    *state.preview_stdin.lock().await = None;

    // Await background task handles with timeout (Fix 9)
    {
        let mut tasks = state.preview_tasks.lock().await;
        for handle in tasks.drain(..) {
            let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
        }
    }

    Ok(())
}

/// Start recording by sending a JSON command to the preview subprocess stdin.
#[tauri::command]
pub async fn start_recording(
    output_dir: String,
    session_name: String,
    crf: u8,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // Serialize lifecycle operations (Fix 6)
    let _lifecycle_guard = state.preview_lock.lock().await;

    // Verify preview is running
    {
        let ps = *state.preview_state.read();
        if ps != PreviewState::Previewing {
            return Err(format!("Cannot start recording: preview is {:?}", ps));
        }
    }

    // Ensure output directory exists before telling the subprocess to record
    let output_path = std::path::Path::new(&output_dir);
    tokio::fs::create_dir_all(output_path)
        .await
        .map_err(|e| format!("Failed to create output directory '{}': {}", output_dir, e))?;

    let warmup = state.config.read().recorder.warmup_frames;

    let cmd = serde_json::json!({
        "cmd": "record",
        "output_dir": output_dir,
        "session": session_name,
        "crf": crf,
        "warmup": warmup,
    });

    let mut line = cmd.to_string();
    line.push('\n');

    let mut stdin_guard = state.preview_stdin.lock().await;
    let stdin = stdin_guard
        .as_mut()
        .ok_or("Preview stdin not available")?;

    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to write record command: {}", e))?;

    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush stdin: {}", e))?;

    // Update state AFTER confirmed write (Fix 8)
    {
        let mut status = state.recorder_status.write();
        status.state = RecorderState::Recording;
        status.frames_written = 0;
        status.frames_dropped = 0;
        status.file_size_mb = 0.0;
        status.elapsed_seconds = 0.0;
    }
    *state.preview_state.write() = PreviewState::Recording;

    Ok(())
}

/// Stop recording by sending a JSON command to the preview subprocess stdin.
#[tauri::command]
pub async fn stop_recording(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Serialize lifecycle operations (Fix 6)
    let _lifecycle_guard = state.preview_lock.lock().await;

    {
        let ps = *state.preview_state.read();
        if ps == PreviewState::Previewing {
            // Recording already stopped by subprocess (e.g., camera disconnect).
            // Reset recorder status and return success so the frontend can recover.
            let mut status = state.recorder_status.write();
            status.state = RecorderState::Idle;
            return Ok(());
        }
        if ps != PreviewState::Recording {
            return Err(format!("Not recording: preview is {:?}", ps));
        }
    }

    let cmd = serde_json::json!({"cmd": "stop"});
    let mut line = cmd.to_string();
    line.push('\n');

    let mut stdin_guard = state.preview_stdin.lock().await;
    let stdin = stdin_guard
        .as_mut()
        .ok_or("Preview stdin not available")?;

    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to write stop command: {}", e))?;

    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush stdin: {}", e))?;

    // Update state AFTER confirmed write (Fix 8)
    {
        let mut status = state.recorder_status.write();
        status.state = RecorderState::Stopping;
    }
    *state.preview_state.write() = PreviewState::Stopping;

    Ok(())
}

/// Get the current preview state.
#[tauri::command]
pub fn get_preview_state(state: State<'_, Arc<AppState>>) -> PreviewState {
    *state.preview_state.read()
}

/// Get cached camera info (available after start_preview succeeds).
#[tauri::command]
pub fn get_camera_info(state: State<'_, Arc<AppState>>) -> Option<CameraInfo> {
    state.camera_info.read().clone()
}

/// Get the MJPEG preview URL for a given stream type ("rgb" or "depth").
#[tauri::command]
pub fn get_preview_url(
    stream_type: String,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let port = state
        .video_server_port
        .read()
        .ok_or("Video server not running")?;

    Ok(format!("http://localhost:{}/preview/{}", port, stream_type))
}

/// Kill a preview subprocess by PID (SIGKILL).
fn kill_preview_process(pid: u32) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
}
