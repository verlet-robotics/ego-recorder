use crate::recorder::status::{RecorderState, RecorderStatus};
use crate::state::AppState;
use std::process::Stdio;
use std::sync::Arc;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Build CLI args for the ego-recorder binary.
pub fn build_args(
    output_dir: &str,
    session_name: &str,
    crf: u8,
    warmup: u32,
) -> Vec<String> {
    vec![
        "-o".into(),
        output_dir.into(),
        "-s".into(),
        session_name.into(),
        "--crf".into(),
        crf.to_string(),
        "--warmup".into(),
        warmup.to_string(),
    ]
}

/// Spawn the C++ ego-recorder process and return its PID.
/// Spawns a background task that reads stderr and emits events.
pub async fn spawn_recorder(
    binary_path: &str,
    args: Vec<String>,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
) -> Result<u32, String> {
    let mut child = Command::new(binary_path)
        .args(&args)
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn ego-recorder: {}", e))?;

    let pid = child.id().ok_or("Failed to get PID")?;

    // Update state
    {
        let mut status = state.recorder_status.write();
        status.state = RecorderState::Recording;
        status.frames_written = 0;
        status.frames_dropped = 0;
        status.file_size_mb = 0.0;
        status.elapsed_seconds = 0.0;
    }
    *state.recorder_pid.write() = Some(pid);

    let stderr = child.stderr.take().ok_or("No stderr")?;
    let reader = BufReader::new(stderr);

    // Spawn stderr reader task
    let state_clone = Arc::clone(&state);
    let app_clone = app_handle.clone();
    tokio::spawn(async move {
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim_start_matches('\r').trim();
            if line.is_empty() {
                continue;
            }

            if let Some(status) = parse_stats_line(line) {
                *state_clone.recorder_status.write() = status;
                let _ = app_clone.emit(
                    "recorder:stats",
                    state_clone.recorder_status.read().clone(),
                );
            }

            if line.contains("Recording complete") {
                let mut s = state_clone.recorder_status.write();
                s.state = RecorderState::Idle;
                let _ = app_clone.emit("recorder:stats", s.clone());
            }
        }

        // Process exited
        let exit_status = child.wait().await;
        let mut s = state_clone.recorder_status.write();
        s.state = RecorderState::Idle;
        *state_clone.recorder_pid.write() = None;

        match exit_status {
            Ok(status) if status.success() => {
                let _ = app_clone.emit("recorder:stopped", "clean");
            }
            Ok(status) => {
                let code = status.code().unwrap_or(-1);
                s.state = RecorderState::Error;
                let _ = app_clone.emit("recorder:stopped", format!("exit code {}", code));
            }
            Err(e) => {
                s.state = RecorderState::Error;
                let _ = app_clone.emit("recorder:stopped", format!("error: {}", e));
            }
        }
    });

    Ok(pid)
}

/// Send SIGINT to the recorder process for graceful shutdown.
pub fn stop_recorder(pid: u32) -> Result<(), String> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGINT)
        .map_err(|e| format!("Failed to send SIGINT: {}", e))
}

/// Parse a stats line from the C++ ego-recorder stderr.
/// Recognizes format: "REC MM:SS | Frames: NNN written, NNN dropped | FPS: N.N cap / N.N write | Size: N.N MB"
pub fn parse_stats_line(line: &str) -> Option<RecorderStatus> {
    // Recording line
    if line.starts_with("REC ") {
        let mut status = RecorderStatus::default();
        status.state = RecorderState::Recording;

        // Parse elapsed time "REC MM:SS"
        if let Some(time_part) = line.strip_prefix("REC ") {
            if let Some(pipe_idx) = time_part.find(" | ") {
                let time_str = &time_part[..pipe_idx];
                if let Some((min_s, sec_s)) = time_str.split_once(':') {
                    let min: f64 = min_s.trim().parse().unwrap_or(0.0);
                    let sec: f64 = sec_s.trim().parse().unwrap_or(0.0);
                    status.elapsed_seconds = min * 60.0 + sec;
                }
            }
        }

        // Parse frames
        if let Some(idx) = line.find("Frames: ") {
            let rest = &line[idx + 8..];
            if let Some(w_idx) = rest.find(" written") {
                status.frames_written = rest[..w_idx].trim().parse().unwrap_or(0);
            }
            if let Some(d_start) = rest.find(", ") {
                let after_comma = &rest[d_start + 2..];
                if let Some(d_idx) = after_comma.find(" dropped") {
                    status.frames_dropped = after_comma[..d_idx].trim().parse().unwrap_or(0);
                }
            }
        }

        // Parse FPS
        if let Some(idx) = line.find("FPS: ") {
            let rest = &line[idx + 5..];
            if let Some(cap_end) = rest.find(" cap") {
                status.capture_fps = rest[..cap_end].trim().parse().unwrap_or(0.0);
            }
            if let Some(slash_idx) = rest.find("/ ") {
                let after_slash = &rest[slash_idx + 2..];
                if let Some(w_end) = after_slash.find(" write") {
                    status.write_fps = after_slash[..w_end].trim().parse().unwrap_or(0.0);
                }
            }
        }

        // Parse size
        if let Some(idx) = line.find("Size: ") {
            let rest = &line[idx + 6..];
            if let Some(mb_end) = rest.find(" MB") {
                status.file_size_mb = rest[..mb_end].trim().parse().unwrap_or(0.0);
            }
        }

        return Some(status);
    }

    // Idle line
    if line.starts_with("Idle") {
        let mut status = RecorderStatus::default();
        status.state = RecorderState::Idle;

        if let Some(idx) = line.find("Camera FPS: ") {
            let rest = &line[idx + 12..];
            status.capture_fps = rest.trim().parse().unwrap_or(0.0);
        }

        return Some(status);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recording_line() {
        let line = "REC 01:30 | Frames: 2700 written, 3 dropped | FPS: 30.0 cap / 29.8 write | Size: 145.2 MB";
        let status = parse_stats_line(line).unwrap();
        assert_eq!(status.state, RecorderState::Recording);
        assert!((status.elapsed_seconds - 90.0).abs() < 0.1);
        assert_eq!(status.frames_written, 2700);
        assert_eq!(status.frames_dropped, 3);
        assert!((status.capture_fps - 30.0).abs() < 0.1);
        assert!((status.write_fps - 29.8).abs() < 0.1);
        assert!((status.file_size_mb - 145.2).abs() < 0.1);
    }

    #[test]
    fn parse_idle_line() {
        let line = "Idle | Camera FPS: 30.0";
        let status = parse_stats_line(line).unwrap();
        assert_eq!(status.state, RecorderState::Idle);
        assert!((status.capture_fps - 30.0).abs() < 0.1);
    }

    #[test]
    fn parse_garbage_returns_none() {
        assert!(parse_stats_line("random text").is_none());
        assert!(parse_stats_line("").is_none());
    }
}
