use crate::config::AppConfig;
use crate::dataset::convert::ConversionProgress;
use crate::library::WatcherCommand;
use crate::preview::{CameraInfo, PreviewState};
use crate::recorder::status::RecorderStatus;
use crate::upload::upload_queue::UploadQueueEntry;
use egorec::FileHeader;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub metadata: EgorecMetadataDto,
    pub conversion_status: ConversionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversionStatus {
    Idle,
    Streamable,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraIntrinsicsDto {
    pub width: u32,
    pub height: u32,
    pub fx: f32,
    pub fy: f32,
    pub ppx: f32,
    pub ppy: f32,
    pub distortion_model: u32,
    pub distortion_coeffs: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthIntrinsicsDto {
    pub width: u32,
    pub height: u32,
    pub fx: f32,
    pub fy: f32,
    pub ppx: f32,
    pub ppy: f32,
    pub distortion_model: u32,
    pub distortion_coeffs: Vec<f32>,
    pub scale: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtrinsicsDto {
    pub rotation: Vec<f32>,
    pub translation: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntrinsicsDto {
    pub color: CameraIntrinsicsDto,
    pub depth: DepthIntrinsicsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgorecMetadataDto {
    pub session_name: String,
    pub serial_number: String,
    pub usb_type: String,
    pub color_width: u32,
    pub color_height: u32,
    pub depth_width: u32,
    pub depth_height: u32,
    pub depth_scale: f32,
    pub fps: f64,
    pub total_frames: u64,
    pub duration_s: f64,
    pub start_timestamp_us: u64,
    pub has_imu: bool,
    pub rgb_codec: u8,
    pub depth_codec: u8,
    pub rgb_quality: u8,
    pub zstd_level: u8,
    pub intrinsics: IntrinsicsDto,
    pub extrinsics: ExtrinsicsDto,
}

impl EgorecMetadataDto {
    pub fn from_header(header: &FileHeader, total_frames: u64, duration_us: u64) -> Self {
        let duration_s = duration_us as f64 / 1_000_000.0;
        let fps = if duration_s > 0.0 && total_frames > 0 {
            (total_frames as f64 / duration_s * 100.0).round() / 100.0
        } else {
            0.0
        };

        Self {
            session_name: header.session_name_str().to_string(),
            serial_number: header.serial_number_str().to_string(),
            usb_type: header.usb_type_str().to_string(),
            color_width: header.color_width,
            color_height: header.color_height,
            depth_width: header.depth_width,
            depth_height: header.depth_height,
            depth_scale: header.depth_scale,
            fps,
            total_frames,
            duration_s,
            start_timestamp_us: header.start_timestamp_us,
            has_imu: header.has_imu(),
            rgb_codec: header.rgb_codec,
            depth_codec: header.depth_codec,
            rgb_quality: header.rgb_quality,
            zstd_level: header.zstd_level,
            intrinsics: IntrinsicsDto {
                color: CameraIntrinsicsDto {
                    width: header.color_width,
                    height: header.color_height,
                    fx: header.color_fx,
                    fy: header.color_fy,
                    ppx: header.color_ppx,
                    ppy: header.color_ppy,
                    distortion_model: header.color_distortion_model,
                    distortion_coeffs: header.color_distortion_coeffs.to_vec(),
                },
                depth: DepthIntrinsicsDto {
                    width: header.depth_width,
                    height: header.depth_height,
                    fx: header.depth_fx,
                    fy: header.depth_fy,
                    ppx: header.depth_ppx,
                    ppy: header.depth_ppy,
                    distortion_model: header.depth_distortion_model,
                    distortion_coeffs: header.depth_distortion_coeffs.to_vec(),
                    scale: header.depth_scale,
                },
            },
            extrinsics: ExtrinsicsDto {
                rotation: header.extrinsic_rotation.to_vec(),
                translation: header.extrinsic_translation.to_vec(),
            },
        }
    }
}

pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub recorder_status: RwLock<RecorderStatus>,
    pub recorder_pid: RwLock<Option<u32>>,
    pub file_index: RwLock<HashMap<String, FileEntry>>,
    pub video_server_port: RwLock<Option<u16>>,
    pub mp4_cache: RwLock<HashMap<String, Arc<Vec<u8>>>>,
    pub inhibitor_fd: RwLock<Option<i32>>,
    pub first_run: RwLock<bool>,
    pub upload_queue: RwLock<Vec<UploadQueueEntry>>,
    pub upload_enabled: AtomicBool,

    // Dataset conversion state
    pub conversion_progress: RwLock<Option<ConversionProgress>>,
    pub conversion_running: AtomicBool,

    // Preview subprocess state
    pub preview_state: RwLock<PreviewState>,
    pub preview_pid: RwLock<Option<u32>>,
    pub preview_stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    pub camera_info: RwLock<Option<CameraInfo>>,
    pub rgb_frame_tx: watch::Sender<Option<Arc<Vec<u8>>>>,
    pub depth_frame_tx: watch::Sender<Option<Arc<Vec<u8>>>>,
    // Receivers kept alive so watch channels don't close
    _rgb_frame_rx: watch::Receiver<Option<Arc<Vec<u8>>>>,
    _depth_frame_rx: watch::Receiver<Option<Arc<Vec<u8>>>>,

    // Preview lifecycle serialization and stale task protection
    pub preview_lock: tokio::sync::Mutex<()>,
    pub preview_generation: AtomicU64,
    pub preview_tasks: tokio::sync::Mutex<Vec<JoinHandle<()>>>,

    // File watcher state
    pub watcher_cmd_tx: tokio::sync::Mutex<Option<mpsc::Sender<WatcherCommand>>>,
    pub watched_dir: RwLock<Option<String>>,

    // Last recording path (for discard feature)
    pub last_recording_path: RwLock<Option<String>>,

    // Camera hotplug detection
    pub camera_connected: AtomicBool,

    // Recent subprocess stderr lines (for error diagnostics)
    pub preview_stderr: RwLock<Vec<String>>,
}

impl Drop for AppState {
    fn drop(&mut self) {
        // Kill lingering preview subprocess on app exit to prevent zombies
        if let Some(pid) = *self.preview_pid.read() {
            log::info!("AppState drop: killing preview subprocess pid={}", pid);
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }

        // Release D-Bus inhibitor fd
        if let Some(fd) = self.inhibitor_fd.read().as_ref() {
            let _ = nix::unistd::close(*fd);
        }
    }
}

impl AppState {
    pub fn new(config: AppConfig, first_run: bool) -> Self {
        let auto_upload = config.upload.auto_upload;
        let (rgb_tx, rgb_rx) = watch::channel(None);
        let (depth_tx, depth_rx) = watch::channel(None);
        Self {
            config: RwLock::new(config),
            recorder_status: RwLock::new(RecorderStatus::default()),
            recorder_pid: RwLock::new(None),
            file_index: RwLock::new(HashMap::new()),
            video_server_port: RwLock::new(None),
            mp4_cache: RwLock::new(HashMap::new()),
            inhibitor_fd: RwLock::new(None),
            first_run: RwLock::new(first_run),
            upload_queue: RwLock::new(Vec::new()),
            upload_enabled: AtomicBool::new(auto_upload),

            conversion_progress: RwLock::new(None),
            conversion_running: AtomicBool::new(false),

            preview_state: RwLock::new(PreviewState::default()),
            preview_pid: RwLock::new(None),
            preview_stdin: tokio::sync::Mutex::new(None),
            camera_info: RwLock::new(None),
            rgb_frame_tx: rgb_tx,
            depth_frame_tx: depth_tx,
            _rgb_frame_rx: rgb_rx,
            _depth_frame_rx: depth_rx,

            preview_lock: tokio::sync::Mutex::new(()),
            preview_generation: AtomicU64::new(0),
            preview_tasks: tokio::sync::Mutex::new(Vec::new()),

            watcher_cmd_tx: tokio::sync::Mutex::new(None),
            watched_dir: RwLock::new(None),

            last_recording_path: RwLock::new(None),

            camera_connected: AtomicBool::new(false),

            preview_stderr: RwLock::new(Vec::new()),
        }
    }
}
