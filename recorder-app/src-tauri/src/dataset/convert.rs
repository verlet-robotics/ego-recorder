use crossbeam_channel::Sender;
use egorec::EgorecReader;
use lerobot_writer::LeRobotDatasetBuilder;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionProgress {
    pub dataset_name: String,
    pub current_file: String,
    pub file_index: usize,
    pub total_files: usize,
    pub frames_done: u64,
    pub total_frames: u64,
    pub phase: String,
    pub error: Option<String>,
}

/// Convert all .egorec files in a dataset directory to LeRobot v3 format.
/// Writes output to `{dataset_dir}/_lerobot/`.
/// Sends progress updates via the channel.
pub fn convert_dataset_to_lerobot(
    dataset_dir: &Path,
    task: &str,
    dataset_name: &str,
    progress_tx: Sender<ConversionProgress>,
) -> Result<(), String> {
    let _ = task; // reserved for future use in metadata

    // Collect .egorec files
    let mut egorec_files: Vec<_> = walkdir::WalkDir::new(dataset_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "egorec")
                && !e.path().to_string_lossy().contains(".pruned")
                && !e.path().to_string_lossy().contains("_lerobot")
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    egorec_files.sort();

    if egorec_files.is_empty() {
        return Err("No .egorec files found in dataset".to_string());
    }

    let total_files = egorec_files.len();

    // Count total frames across all files for progress
    let total_frames: u64 = egorec_files
        .iter()
        .filter_map(|p| {
            let reader = EgorecReader::open(p.to_str()?).ok()?;
            Some(reader.frame_count())
        })
        .sum();

    // Read first file to get fps and depth dimensions
    let first_reader = EgorecReader::open(
        egorec_files[0]
            .to_str()
            .ok_or("Invalid path encoding")?,
    )
    .map_err(|e| format!("Failed to open first file: {}", e))?;

    let header = first_reader.header();
    let fps = {
        let fc = first_reader.frame_count();
        let dur = first_reader.duration_s();
        if dur > 0.0 && fc > 0 {
            (fc as f64 / dur).round() as u32
        } else {
            30 // fallback
        }
    };
    let depth_w = header.depth_width;
    let depth_h = header.depth_height;
    drop(first_reader);

    // Clean output directory
    let lerobot_dir = dataset_dir.join("_lerobot");
    if lerobot_dir.exists() {
        std::fs::remove_dir_all(&lerobot_dir)
            .map_err(|e| format!("Failed to clean _lerobot dir: {}", e))?;
    }

    let mut builder = LeRobotDatasetBuilder::new(&lerobot_dir, fps, depth_w, depth_h);
    let mut global_frames_done: u64 = 0;

    for (file_index, egorec_path) in egorec_files.iter().enumerate() {
        let file_name = egorec_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let _ = progress_tx.send(ConversionProgress {
            dataset_name: dataset_name.to_string(),
            current_file: file_name.clone(),
            file_index,
            total_files,
            frames_done: global_frames_done,
            total_frames,
            phase: "converting".to_string(),
            error: None,
        });

        let reader = EgorecReader::open(
            egorec_path
                .to_str()
                .ok_or("Invalid path encoding")?,
        )
        .map_err(|e| format!("Failed to open {}: {}", file_name, e))?;

        let frames = reader
            .frames()
            .map_err(|e| format!("Failed to create frame iterator for {}: {}", file_name, e))?;

        builder
            .start_episode()
            .map_err(|e| format!("Failed to start episode: {}", e))?;

        for frame_result in frames {
            let frame = frame_result
                .map_err(|e| format!("Frame decode error in {}: {}", file_name, e))?;

            builder
                .add_frame(&frame.rgb, &frame.depth, frame.timestamp_relative_s as f32)
                .map_err(|e| format!("Failed to add frame: {}", e))?;

            global_frames_done += 1;

            // Send progress every 30 frames to avoid flooding
            if global_frames_done % 30 == 0 {
                let _ = progress_tx.send(ConversionProgress {
                    dataset_name: dataset_name.to_string(),
                    current_file: file_name.clone(),
                    file_index,
                    total_files,
                    frames_done: global_frames_done,
                    total_frames,
                    phase: "converting".to_string(),
                    error: None,
                });
            }
        }

        builder
            .save_episode()
            .map_err(|e| format!("Failed to save episode: {}", e))?;
    }

    let _ = progress_tx.send(ConversionProgress {
        dataset_name: dataset_name.to_string(),
        current_file: String::new(),
        file_index: total_files,
        total_files,
        frames_done: global_frames_done,
        total_frames,
        phase: "finalizing".to_string(),
        error: None,
    });

    builder
        .finalize()
        .map_err(|e| format!("Failed to finalize dataset: {}", e))?;

    let _ = progress_tx.send(ConversionProgress {
        dataset_name: dataset_name.to_string(),
        current_file: String::new(),
        file_index: total_files,
        total_files,
        frames_done: global_frames_done,
        total_frames,
        phase: "completed".to_string(),
        error: None,
    });

    Ok(())
}
