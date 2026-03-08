/// Parquet writer for LeRobot v3 dataset format.
/// Each episode gets one parquet file with frame_index, episode_index,
/// timestamp, depth data, and task_index.
use arrow::array::{ArrayRef, Float32Builder, ListBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParquetError {
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Accumulates frames for one episode and writes a parquet file.
pub struct EpisodeParquetWriter {
    frame_indices: Vec<i64>,
    episode_indices: Vec<i64>,
    timestamps: Vec<f32>,
    depth_frames: Vec<Vec<f32>>,
    task_indices: Vec<i64>,
    _depth_width: u32,
    _depth_height: u32,
}

impl EpisodeParquetWriter {
    pub fn new(depth_width: u32, depth_height: u32) -> Self {
        Self {
            frame_indices: Vec::new(),
            episode_indices: Vec::new(),
            timestamps: Vec::new(),
            depth_frames: Vec::new(),
            task_indices: Vec::new(),
            _depth_width: depth_width,
            _depth_height: depth_height,
        }
    }

    /// Add a frame's data.
    pub fn add_frame(
        &mut self,
        frame_index: i64,
        episode_index: i64,
        timestamp: f32,
        depth: &[u16],
        task_index: i64,
    ) {
        self.frame_indices.push(frame_index);
        self.episode_indices.push(episode_index);
        self.timestamps.push(timestamp);
        self.depth_frames
            .push(depth.iter().map(|&v| v as f32).collect());
        self.task_indices.push(task_index);
    }

    /// Write accumulated data to a parquet file.
    pub fn write(&self, path: &Path) -> Result<(), ParquetError> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("frame_index", DataType::Int64, false),
            Field::new("episode_index", DataType::Int64, false),
            Field::new("timestamp", DataType::Float32, false),
            Field::new(
                "observation.depth_mm",
                DataType::List(Arc::new(Field::new("item", DataType::Float32, true))),
                false,
            ),
            Field::new("task_index", DataType::Int64, false),
        ]));

        let frame_index_array: ArrayRef =
            Arc::new(arrow::array::Int64Array::from(self.frame_indices.clone()));
        let episode_index_array: ArrayRef =
            Arc::new(arrow::array::Int64Array::from(self.episode_indices.clone()));
        let timestamp_array: ArrayRef =
            Arc::new(arrow::array::Float32Array::from(self.timestamps.clone()));

        // Build depth list array
        let mut depth_builder = ListBuilder::new(Float32Builder::new());
        for depth_frame in &self.depth_frames {
            let values = depth_builder.values();
            for &v in depth_frame {
                values.append_value(v);
            }
            depth_builder.append(true);
        }
        let depth_array: ArrayRef = Arc::new(depth_builder.finish());

        let task_index_array: ArrayRef =
            Arc::new(arrow::array::Int64Array::from(self.task_indices.clone()));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                frame_index_array,
                episode_index_array,
                timestamp_array,
                depth_array,
                task_index_array,
            ],
        )?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        let mut writer = ArrowWriter::try_new(file, schema, None)?;
        writer.write(&batch)?;
        writer.close()?;

        Ok(())
    }

    pub fn frame_count(&self) -> usize {
        self.frame_indices.len()
    }

    pub fn clear(&mut self) {
        self.frame_indices.clear();
        self.episode_indices.clear();
        self.timestamps.clear();
        self.depth_frames.clear();
        self.task_indices.clear();
    }
}
