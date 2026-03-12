use egorec::{AnalysisResult, FileHeader};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
    pub recordings_dir: RwLock<Option<PathBuf>>,
    pub file_index: RwLock<HashMap<String, FileEntry>>,
    pub analysis_cache: RwLock<Option<Vec<AnalysisResult>>>,
    pub analysis_running: RwLock<bool>,
    pub video_server_port: RwLock<Option<u16>>,
    /// Root directory containing multiple workspace subdirectories.
    pub curation_root: RwLock<Option<PathBuf>>,
    /// Currently active workspace (a child of curation_root, or standalone).
    pub curation_workspace: RwLock<Option<PathBuf>>,
    pub qc_binary: RwLock<String>,
    pub python_binary: RwLock<String>,
    /// In-memory MP4 cache: keyed by absolute file path, stores the complete MP4 bytes.
    pub mp4_cache: RwLock<HashMap<String, Arc<Vec<u8>>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            recordings_dir: RwLock::new(None),
            file_index: RwLock::new(HashMap::new()),
            analysis_cache: RwLock::new(None),
            analysis_running: RwLock::new(false),
            video_server_port: RwLock::new(None),
            curation_root: RwLock::new(None),
            curation_workspace: RwLock::new(None),
            qc_binary: RwLock::new("ego-qc".into()),
            python_binary: RwLock::new("python3".into()),
            mp4_cache: RwLock::new(HashMap::new()),
        }
    }
}
