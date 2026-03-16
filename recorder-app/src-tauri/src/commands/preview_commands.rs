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

/// Maximum number of automatic restart attempts when the subprocess crashes
/// during preview (not recording).
const MAX_AUTO_RETRIES: u32 = 3;
/// Delay between auto-retry attempts.
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// If no RGB frame arrives within this duration, the watchdog kills the subprocess.
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(10);

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
        if ps != PreviewState::Off && ps != PreviewState::Error && ps != PreviewState::Retrying {
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

    // Clear stderr buffer from previous attempts
    state.preview_stderr.write().clear();

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
            // Set Error instead of Off so the stderr task's auto-retry logic
            // or the frontend can decide what to do.
            let current = *state_for_reader.preview_state.read();
            if current != PreviewState::Off && current != PreviewState::Error && current != PreviewState::Retrying {
                *state_for_reader.preview_state.write() = PreviewState::Error;
                let _ = app_for_reader.emit("preview:state-changed", "error");
            }
            *state_for_reader.preview_pid.write() = None;
            *state_for_reader.preview_stdin.lock().await = None;
            // Note: NOT emitting preview:disconnected here. The state-changed
            // event already conveys the Error state. The old preview:disconnected
            // handler in record-page sets state to "off", which would overwrite
            // the Error state and prevent the retry UI from showing.
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

            // Check for USB 2.0 warning sentinel
            if line.starts_with("USB_WARNING:") {
                let msg = line.trim_start_matches("USB_WARNING:").trim().to_string();
                log::warn!("Preview: {}", msg);
                let _ = app_for_stderr.emit("preview:usb-warning", msg);
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

            // Capture recording file path from subprocess
            if line.contains("Recording to:") {
                if let Some(path) = line.split("Recording to:").nth(1) {
                    let path = path.trim().to_string();
                    log::info!("Recording path: {}", path);
                    *state_for_stderr.last_recording_path.write() = Some(path);
                }
            }

            // Parse stats lines
            if let Some(status) = parse_stats_line(&line) {
                *state_for_stderr.recorder_status.write() = status.clone();
                let _ = app_for_stderr.emit("recorder:stats", status);
                continue;
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
                continue;
            }

            // Log and buffer unrecognized stderr for error diagnostics
            log::info!("ego-recorder: {}", line);
            {
                let mut buf = state_for_stderr.preview_stderr.write();
                buf.push(line);
                if buf.len() > 50 {
                    buf.remove(0);
                }
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

        // Determine exit info while holding locks briefly, then drop before await
        enum ExitAction {
            Clean,
            CrashRetry(i32),
            CrashError(i32),
            Error(String),
        }

        let action = {
            state_for_stderr.recorder_status.write().state = RecorderState::Idle;
            *state_for_stderr.preview_pid.write() = None;

            match exit_status {
                Ok(status) if status.success() => ExitAction::Clean,
                Ok(status) => {
                    let code = status.code().unwrap_or(-1);
                    state_for_stderr.recorder_status.write().state = RecorderState::Error;

                    let was_recording = {
                        let ps = *state_for_stderr.preview_state.read();
                        ps == PreviewState::Recording || ps == PreviewState::Stopping
                    };

                    if !was_recording {
                        ExitAction::CrashRetry(code)
                    } else {
                        ExitAction::CrashError(code)
                    }
                }
                Err(e) => {
                    state_for_stderr.recorder_status.write().state = RecorderState::Error;
                    ExitAction::Error(e.to_string())
                }
            }
        };

        match action {
            ExitAction::Clean => {
                *state_for_stderr.preview_state.write() = PreviewState::Off;
                let _ = app_for_stderr.emit("recorder:stopped", "clean");
            }
            ExitAction::CrashRetry(code) => {
                // Auto-retry on non-zero exit when NOT recording.
                auto_retry_preview(
                    &state_for_stderr,
                    &app_for_stderr,
                    stderr_gen,
                    code,
                )
                .await;
            }
            ExitAction::CrashError(code) => {
                *state_for_stderr.preview_state.write() = PreviewState::Error;
                let _ = app_for_stderr.emit("preview:state-changed", "error");
                let _ = app_for_stderr
                    .emit("recorder:stopped", format!("exit code {}", code));
            }
            ExitAction::Error(msg) => {
                *state_for_stderr.preview_state.write() = PreviewState::Error;
                let _ = app_for_stderr.emit("preview:state-changed", "error");
                let _ = app_for_stderr.emit("recorder:stopped", format!("error: {}", msg));
            }
        }
    });

    // Spawn watchdog task: kills the subprocess if no RGB frame arrives for
    // WATCHDOG_TIMEOUT. This is defense-in-depth alongside the frame reader
    // timeout — catches cases where the subprocess is alive but stuck.
    let state_for_watchdog = Arc::clone(&state);
    let app_for_watchdog = app_handle.clone();
    let watchdog_gen = generation;
    let mut watchdog_rx = state.rgb_frame_tx.subscribe();
    let watchdog_handle = tokio::spawn(async move {
        // Wait for the first frame before starting the timeout loop.
        // During startup (camera warmup, camera info handshake) no frames are
        // expected, so we must not fire the watchdog prematurely.
        loop {
            if state_for_watchdog
                .preview_generation
                .load(Ordering::SeqCst)
                != watchdog_gen
            {
                return;
            }
            match watchdog_rx.changed().await {
                Ok(()) => break,  // First frame arrived, start watchdog loop
                Err(_) => return, // Channel closed before first frame
            }
        }

        // Now that frames are flowing, enforce the timeout
        loop {
            if state_for_watchdog
                .preview_generation
                .load(Ordering::SeqCst)
                != watchdog_gen
            {
                break;
            }

            match tokio::time::timeout(WATCHDOG_TIMEOUT, watchdog_rx.changed()).await {
                Ok(Ok(())) => {
                    // Frame arrived, keep watching
                    continue;
                }
                Ok(Err(_)) => {
                    // Channel closed (frame reader exited), stop watching
                    break;
                }
                Err(_) => {
                    // Timeout — no frame for WATCHDOG_TIMEOUT
                    if state_for_watchdog
                        .preview_generation
                        .load(Ordering::SeqCst)
                        != watchdog_gen
                    {
                        break;
                    }

                    log::error!(
                        "Preview watchdog: no frame for {}s, killing subprocess",
                        WATCHDOG_TIMEOUT.as_secs()
                    );

                    if let Some(pid) = *state_for_watchdog.preview_pid.read() {
                        kill_preview_process(pid);
                    }

                    *state_for_watchdog.preview_state.write() = PreviewState::Error;
                    let _ = app_for_watchdog.emit("preview:state-changed", "error");
                    break;
                }
            }
        }
    });

    // Store task handles (Fix 9)
    {
        let mut tasks = state.preview_tasks.lock().await;
        tasks.push(reader_handle);
        tasks.push(stderr_handle);
        tasks.push(watchdog_handle);
    }

    // Wait for camera info with timeout (Fix 5)
    let camera_info = match tokio::time::timeout(Duration::from_secs(15), info_rx).await {
        Ok(Ok(info)) => info,
        Ok(Err(_)) => {
            // Channel dropped -- subprocess died
            kill_preview_process(pid);
            *state.preview_state.write() = PreviewState::Error;
            // Wait briefly for stderr reader to collect output
            tokio::time::sleep(Duration::from_millis(500)).await;
            let stderr_lines = state.preview_stderr.read().join("\n");
            let detail = if stderr_lines.is_empty() {
                format!("ego-recorder crashed immediately. Binary: {}", binary_path)
            } else {
                format!("ego-recorder failed:\n{}", stderr_lines)
            };
            return Err(detail);
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
        // Wait briefly for stderr reader to collect output
        tokio::time::sleep(Duration::from_millis(500)).await;
        let stderr_lines = state.preview_stderr.read().join("\n");
        let detail = if stderr_lines.is_empty() {
            format!("ego-recorder exited without camera info. Binary: {}", binary_path)
        } else {
            format!("ego-recorder failed to initialize:\n{}", stderr_lines)
        };
        return Err(detail);
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

    // Bump generation FIRST to invalidate background tasks and cancel any
    // in-progress auto-retry, even if preview_pid is None (between retry attempts).
    state.preview_generation.fetch_add(1, Ordering::SeqCst);

    // Send SIGINT if a subprocess is running
    if let Some(pid) = *state.preview_pid.read() {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGINT);
    }

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

/// Check if a RealSense camera is currently connected (live sysfs scan).
#[tauri::command]
pub fn check_camera() -> bool {
    !crate::camera_watcher::detect_realsense_cameras().is_empty()
}

/// Kill a preview subprocess by PID (SIGKILL).
fn kill_preview_process(pid: u32) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
}

/// Attempt to restart the preview subprocess up to MAX_AUTO_RETRIES times.
/// Called from the stderr reader task when the subprocess exits with a non-zero
/// code and we were NOT recording.
async fn auto_retry_preview(
    state: &Arc<AppState>,
    app: &tauri::AppHandle,
    generation: u64,
    exit_code: i32,
) {
    for attempt in 1..=MAX_AUTO_RETRIES {
        // Bail if generation changed (user started a new lifecycle)
        if state.preview_generation.load(Ordering::SeqCst) != generation {
            return;
        }

        log::warn!(
            "Preview subprocess exited with code {}. Auto-retry {}/{}",
            exit_code,
            attempt,
            MAX_AUTO_RETRIES
        );

        *state.preview_state.write() = PreviewState::Retrying;
        let _ = app.emit("preview:state-changed", "retrying");

        tokio::time::sleep(RETRY_DELAY).await;

        // Check generation again after sleep
        if state.preview_generation.load(Ordering::SeqCst) != generation {
            return;
        }

        let binary_path = {
            let config = state.config.read();
            match config.recorder.binary_path.clone() {
                Some(p) => p,
                None => {
                    *state.preview_state.write() = PreviewState::Error;
                    let _ = app.emit("preview:state-changed", "error");
                    return;
                }
            }
        };

        let warmup = state.config.read().recorder.warmup_frames;
        let preset = state.config.read().recorder.h264_preset.clone();

        let child_result = Command::new(&binary_path)
            .arg("preview")
            .arg("--warmup")
            .arg(warmup.to_string())
            .arg("--preset")
            .arg(&preset)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match child_result {
            Ok(c) => c,
            Err(e) => {
                log::error!("Auto-retry {}: failed to spawn: {}", attempt, e);
                continue;
            }
        };

        let pid = match child.id() {
            Some(p) => p,
            None => {
                log::error!("Auto-retry {}: no PID", attempt);
                continue;
            }
        };

        *state.preview_pid.write() = Some(pid);

        let stdin = match child.stdin.take() {
            Some(s) => s,
            None => continue,
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => continue,
        };
        let stderr = match child.stderr.take() {
            Some(s) => s,
            None => continue,
        };

        *state.preview_stdin.lock().await = Some(stdin);

        let (info_tx, info_rx) = oneshot::channel();
        let rgb_tx = state.rgb_frame_tx.clone();
        let depth_tx = state.depth_frame_tx.clone();

        // Spawn new frame reader
        let state_r = Arc::clone(state);
        let app_r = app.clone();
        let gen = generation;
        let reader_handle = tokio::spawn(async move {
            frame_reader::read_frames(stdout, info_tx, rgb_tx, depth_tx).await;
            if state_r.preview_generation.load(Ordering::SeqCst) == gen {
                let current = *state_r.preview_state.read();
                if current != PreviewState::Off && current != PreviewState::Error && current != PreviewState::Retrying {
                    *state_r.preview_state.write() = PreviewState::Error;
                    let _ = app_r.emit("preview:state-changed", "error");
                }
                *state_r.preview_pid.write() = None;
                *state_r.preview_stdin.lock().await = None;
            }
        });

        // Spawn new stderr reader (non-recursive: won't auto-retry again)
        let state_s = Arc::clone(state);
        let app_s = app.clone();
        let gen_s = generation;
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if state_s.preview_generation.load(Ordering::SeqCst) != gen_s {
                    break;
                }
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if line.starts_with("USB_WARNING:") {
                    let msg = line.trim_start_matches("USB_WARNING:").trim().to_string();
                    let _ = app_s.emit("preview:usb-warning", msg);
                    continue;
                }
                if line == "DISCONNECTED" {
                    *state_s.preview_state.write() = PreviewState::Error;
                    let _ = app_s.emit("preview:state-changed", "error");
                    continue;
                }
                if line == "RECONNECTED" {
                    *state_s.preview_state.write() = PreviewState::Previewing;
                    let _ = app_s.emit("preview:state-changed", "previewing");
                    continue;
                }
                if line.contains("Recording to:") {
                    if let Some(path) = line.split("Recording to:").nth(1) {
                        *state_s.last_recording_path.write() = Some(path.trim().to_string());
                    }
                }
                if let Some(status) = parse_stats_line(&line) {
                    *state_s.recorder_status.write() = status.clone();
                    let _ = app_s.emit("recorder:stats", status);
                    continue;
                }
                if line.contains("Recording complete") {
                    let ps = *state_s.preview_state.read();
                    if ps == PreviewState::Recording || ps == PreviewState::Stopping {
                        *state_s.preview_state.write() = PreviewState::Previewing;
                        let _ = app_s.emit("preview:state-changed", "previewing");
                    }
                    let mut s = state_s.recorder_status.write();
                    s.state = RecorderState::Idle;
                    let _ = app_s.emit("recorder:stats", s.clone());
                    let _ = app_s.emit("recorder:stopped", "clean");
                    continue;
                }
                log::info!("ego-recorder: {}", line);
            }
            // Process exited — just update state, no further retry
            if state_s.preview_generation.load(Ordering::SeqCst) == gen_s {
                let exit_status = child.wait().await;
                *state_s.preview_pid.write() = None;
                match exit_status {
                    Ok(s) if s.success() => {
                        *state_s.preview_state.write() = PreviewState::Off;
                    }
                    _ => {
                        // Don't overwrite Retrying — auto_retry_preview may be
                        // between attempts and will set the right state itself.
                        let current = *state_s.preview_state.read();
                        if current != PreviewState::Retrying {
                            *state_s.preview_state.write() = PreviewState::Error;
                            let _ = app_s.emit("preview:state-changed", "error");
                        }
                    }
                }
            }
        });

        // Spawn new watchdog (wait for first frame before starting timeout)
        let state_w = Arc::clone(state);
        let app_w = app.clone();
        let gen_w = generation;
        let mut watchdog_rx = state.rgb_frame_tx.subscribe();
        let watchdog_handle = tokio::spawn(async move {
            // Wait for first frame (no timeout during startup)
            loop {
                if state_w.preview_generation.load(Ordering::SeqCst) != gen_w {
                    return;
                }
                match watchdog_rx.changed().await {
                    Ok(()) => break,
                    Err(_) => return,
                }
            }
            // Enforce timeout after first frame
            loop {
                if state_w.preview_generation.load(Ordering::SeqCst) != gen_w {
                    break;
                }
                match tokio::time::timeout(WATCHDOG_TIMEOUT, watchdog_rx.changed()).await {
                    Ok(Ok(())) => continue,
                    Ok(Err(_)) => break,
                    Err(_) => {
                        if state_w.preview_generation.load(Ordering::SeqCst) != gen_w {
                            break;
                        }
                        log::error!("Preview watchdog (retry): no frame, killing subprocess");
                        if let Some(pid) = *state_w.preview_pid.read() {
                            kill_preview_process(pid);
                        }
                        *state_w.preview_state.write() = PreviewState::Error;
                        let _ = app_w.emit("preview:state-changed", "error");
                        break;
                    }
                }
            }
        });

        {
            let mut tasks = state.preview_tasks.lock().await;
            tasks.push(reader_handle);
            tasks.push(stderr_handle);
            tasks.push(watchdog_handle);
        }

        // Wait for camera info
        match tokio::time::timeout(Duration::from_secs(15), info_rx).await {
            Ok(Ok(info)) if !info.serial.is_empty() => {
                *state.camera_info.write() = Some(info.clone());
                *state.preview_state.write() = PreviewState::Previewing;
                let _ = app.emit("preview:camera-info", &info);
                let _ = app.emit("preview:state-changed", "previewing");
                log::info!("Auto-retry {}: preview restarted successfully", attempt);
                return;
            }
            _ => {
                // Kill this failed attempt, try again
                kill_preview_process(pid);
                *state.preview_pid.write() = None;
                *state.preview_stdin.lock().await = None;
                continue;
            }
        }
    }

    // All retries exhausted
    log::error!(
        "Preview auto-retry: all {} attempts failed (last exit code {})",
        MAX_AUTO_RETRIES,
        exit_code
    );
    *state.preview_state.write() = PreviewState::Error;
    let _ = app.emit("preview:state-changed", "error");
}

/// Discard the last recorded episode by deleting the .egorec file.
#[tauri::command]
pub async fn discard_last_recording(
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    // Don't allow discard while recording
    {
        let ps = *state.preview_state.read();
        if ps == PreviewState::Recording {
            return Err("Cannot discard while recording".to_string());
        }
    }

    let path = state
        .last_recording_path
        .write()
        .take()
        .ok_or("No recording to discard")?;

    let file_path = std::path::PathBuf::from(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {}", path));
    }

    tokio::fs::remove_file(&file_path)
        .await
        .map_err(|e| format!("Failed to delete {}: {}", path, e))?;

    log::info!("Discarded recording: {}", path);
    Ok(path)
}
