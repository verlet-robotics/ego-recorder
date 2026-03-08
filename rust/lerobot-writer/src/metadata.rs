/// LeRobot v3 metadata files: info.json, episodes.jsonl, tasks.jsonl, stats.json.
use serde::Serialize;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct InfoJson {
    pub codebase_version: String,
    pub robot_type: String,
    pub fps: u32,
    pub features: serde_json::Value,
    pub total_episodes: usize,
    pub total_frames: usize,
    pub total_tasks: usize,
    pub total_chunks: usize,
    pub chunks_size: usize,
    pub data_path: String,
    pub video_path: String,
}

#[derive(Debug, Serialize)]
pub struct EpisodeEntry {
    pub episode_index: usize,
    pub tasks: Vec<String>,
    pub length: usize,
}

#[derive(Debug, Serialize)]
pub struct TaskEntry {
    pub task_index: usize,
    pub task: String,
}

/// Write info.json for a LeRobot v3 dataset.
pub fn write_info_json(
    path: &Path,
    total_episodes: usize,
    total_frames: usize,
    fps: u32,
) -> io::Result<()> {
    let features = serde_json::json!({
        "observation.images.rgb": {
            "dtype": "video",
            "shape": [480, 640, 3],
            "names": ["height", "width", "channel"],
            "video_info": {
                "video.fps": fps,
                "video.codec": "av1",
                "video.pix_fmt": "yuv420p",
                "video.is_depth_map": false,
                "has_audio": false
            }
        },
        "observation.depth_mm": {
            "dtype": "float32",
            "shape": [480, 640],
            "names": ["height", "width"]
        }
    });

    let info = InfoJson {
        codebase_version: "v3.0".to_string(),
        robot_type: "realsense_d435".to_string(),
        fps,
        features,
        total_episodes,
        total_frames,
        total_tasks: 1,
        total_chunks: 1,
        chunks_size: 1000,
        data_path: "data/chunk-{chunk_index:03d}/episode_{episode_index:06d}.parquet".to_string(),
        video_path:
            "videos/chunk-{chunk_index:03d}/{video_key}_episode_{episode_index:06d}.mp4"
                .to_string(),
    };

    let json = serde_json::to_string_pretty(&info)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json)
}

/// Write episodes.jsonl (one JSON line per episode).
pub fn write_episodes_jsonl(path: &Path, episodes: &[EpisodeEntry]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(fs::File::create(path)?);
    for ep in episodes {
        let line = serde_json::to_string(ep)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(writer, "{}", line)?;
    }
    writer.flush()
}

/// Write tasks.jsonl.
pub fn write_tasks_jsonl(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let task = TaskEntry {
        task_index: 0,
        task: "ego_recording".to_string(),
    };
    let line =
        serde_json::to_string(&task).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(path, format!("{}\n", line))
}
