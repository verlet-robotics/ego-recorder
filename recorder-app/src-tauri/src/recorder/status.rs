use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub state: RecorderState,
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub capture_fps: f64,
    pub write_fps: f64,
    pub file_size_mb: f64,
    pub elapsed_seconds: f64,
    pub episode_count: u32,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecorderState {
    Idle,
    Countdown,
    Recording,
    Stopping,
    Error,
}

impl Default for RecorderStatus {
    fn default() -> Self {
        Self {
            state: RecorderState::Idle,
            frames_written: 0,
            frames_dropped: 0,
            capture_fps: 0.0,
            write_fps: 0.0,
            file_size_mb: 0.0,
            elapsed_seconds: 0.0,
            episode_count: 0,
            current_file: None,
        }
    }
}
