/// LeRobot v3 dataset builder: orchestrates video, parquet, and metadata writers.
use crate::metadata::{self, EpisodeEntry};
use crate::parquet::EpisodeParquetWriter;
use crate::video::{Mp4Writer, VideoError};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DatasetError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("video error: {0}")]
    Video(#[from] VideoError),
    #[error("parquet error: {0}")]
    Parquet(#[from] crate::parquet::ParquetError),
    #[error("no episode in progress")]
    NoEpisode,
}

pub struct LeRobotDatasetBuilder {
    output_dir: PathBuf,
    fps: u32,
    depth_width: u32,
    depth_height: u32,
    episodes: Vec<EpisodeEntry>,
    total_frames: usize,

    // Current episode state
    current_video: Option<Mp4Writer>,
    current_parquet: Option<EpisodeParquetWriter>,
    current_episode_index: usize,
    current_frame_in_episode: usize,
}

impl LeRobotDatasetBuilder {
    pub fn new(output_dir: &Path, fps: u32, depth_width: u32, depth_height: u32) -> Self {
        Self {
            output_dir: output_dir.to_path_buf(),
            fps,
            depth_width,
            depth_height,
            episodes: Vec::new(),
            total_frames: 0,
            current_video: None,
            current_parquet: None,
            current_episode_index: 0,
            current_frame_in_episode: 0,
        }
    }

    /// Start a new episode.
    pub fn start_episode(&mut self) -> Result<(), DatasetError> {
        let episode_idx = self.episodes.len();
        self.current_episode_index = episode_idx;
        self.current_frame_in_episode = 0;

        // Create video file
        let video_dir = self.output_dir.join("videos/chunk-000");
        std::fs::create_dir_all(&video_dir)?;
        let video_path = video_dir.join(format!(
            "observation.images.rgb_episode_{:06}.mp4",
            episode_idx
        ));
        let video = Mp4Writer::new(&video_path, 640, 480, self.fps)?;
        self.current_video = Some(video);

        // Create parquet writer
        self.current_parquet = Some(EpisodeParquetWriter::new(
            self.depth_width,
            self.depth_height,
        ));

        Ok(())
    }

    /// Add a frame to the current episode.
    pub fn add_frame(
        &mut self,
        rgb: &[u8],
        depth: &[u16],
        timestamp: f32,
    ) -> Result<(), DatasetError> {
        let video = self.current_video.as_mut().ok_or(DatasetError::NoEpisode)?;
        video.add_frame(rgb)?;

        let parquet = self
            .current_parquet
            .as_mut()
            .ok_or(DatasetError::NoEpisode)?;
        parquet.add_frame(
            self.total_frames as i64,
            self.current_episode_index as i64,
            timestamp,
            depth,
            0, // task_index = 0 (ego_recording)
        );

        self.current_frame_in_episode += 1;
        self.total_frames += 1;

        Ok(())
    }

    /// Finish the current episode.
    pub fn save_episode(&mut self) -> Result<(), DatasetError> {
        // Finish video
        if let Some(video) = self.current_video.take() {
            video.finish()?;
        }

        // Write parquet file
        if let Some(parquet) = self.current_parquet.take() {
            let parquet_dir = self.output_dir.join("data/chunk-000");
            std::fs::create_dir_all(&parquet_dir)?;
            let parquet_path = parquet_dir.join(format!(
                "episode_{:06}.parquet",
                self.current_episode_index
            ));
            parquet.write(&parquet_path)?;

            self.episodes.push(EpisodeEntry {
                episode_index: self.current_episode_index,
                tasks: vec!["ego_recording".to_string()],
                length: self.current_frame_in_episode,
            });
        }

        Ok(())
    }

    /// Finalize the dataset: write all metadata files.
    pub fn finalize(self) -> Result<(), DatasetError> {
        let meta_dir = self.output_dir.join("meta");
        std::fs::create_dir_all(&meta_dir)?;

        metadata::write_info_json(
            &meta_dir.join("info.json"),
            self.episodes.len(),
            self.total_frames,
            self.fps,
        )?;

        metadata::write_episodes_jsonl(&meta_dir.join("episodes.jsonl"), &self.episodes)?;

        metadata::write_tasks_jsonl(&meta_dir.join("tasks.jsonl"))?;

        Ok(())
    }
}
