pub mod frame_reader;

use serde::{Deserialize, Serialize};

/// Camera information received from the ego-recorder preview subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub serial: String,
    pub usb: String,
    pub has_imu: bool,
    pub width: u32,
    pub height: u32,
}

impl Default for CameraInfo {
    fn default() -> Self {
        Self {
            serial: String::new(),
            usb: String::new(),
            has_imu: false,
            width: 0,
            height: 0,
        }
    }
}

/// State of the preview subprocess lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewState {
    Off,
    Starting,
    Previewing,
    Recording,
    Stopping,
    Error,
    Retrying,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self::Off
    }
}
