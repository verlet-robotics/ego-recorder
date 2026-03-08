/// RLDS episode/step construction + TFDS metadata generation.
/// Matches the schema in python/export_rlds.py.
use crate::proto::{self, Feature};
use crate::record::write_record;
use egorec::{DecodedFrame, FileHeader};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RldsError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("image encoding error: {0}")]
    Image(#[from] image::ImageError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Encodes an RGB24 frame as JPEG bytes.
fn encode_jpeg(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, RldsError> {
    let mut buf = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut buf, 95);
    encoder.write_image(rgb, width, height, ColorType::Rgb8.into())?;
    Ok(buf)
}

/// Encodes a depth frame as 16-bit grayscale PNG.
fn encode_depth_png(depth: &[u16], width: u32, height: u32) -> Result<Vec<u8>, RldsError> {
    let mut buf = Vec::new();
    let encoder = PngEncoder::new(&mut buf);
    let depth_bytes: Vec<u8> = depth.iter().flat_map(|v| v.to_le_bytes()).collect();
    encoder.write_image(&depth_bytes, width, height, ColorType::L16.into())?;
    Ok(buf)
}

/// Build a tf.train.Example for one RLDS step.
fn build_step_example(
    frame: &DecodedFrame,
    header: &FileHeader,
    step_idx: usize,
    total_frames: u64,
) -> Result<Vec<u8>, RldsError> {
    let mut features = HashMap::new();

    // observation/image: JPEG-encoded RGB
    let jpeg = encode_jpeg(&frame.rgb, header.color_width, header.color_height)?;
    features.insert(
        "steps/observation/image".to_string(),
        Feature::Bytes(vec![jpeg]),
    );

    // observation/depth: PNG-encoded 16-bit depth
    let depth_png = encode_depth_png(&frame.depth, header.depth_width, header.depth_height)?;
    features.insert(
        "steps/observation/depth".to_string(),
        Feature::Bytes(vec![depth_png]),
    );

    // observation/depth_intrinsics: [fx, fy, ppx, ppy]
    features.insert(
        "steps/observation/depth_intrinsics".to_string(),
        Feature::Float(vec![
            header.depth_fx,
            header.depth_fy,
            header.depth_ppx,
            header.depth_ppy,
        ]),
    );

    // observation/color_intrinsics: [fx, fy, ppx, ppy]
    features.insert(
        "steps/observation/color_intrinsics".to_string(),
        Feature::Float(vec![
            header.color_fx,
            header.color_fy,
            header.color_ppx,
            header.color_ppy,
        ]),
    );

    // observation/extrinsic_R: 3x3 rotation flattened
    features.insert(
        "steps/observation/extrinsic_R".to_string(),
        Feature::Float(header.extrinsic_rotation.to_vec()),
    );

    // observation/extrinsic_t: translation vector
    features.insert(
        "steps/observation/extrinsic_t".to_string(),
        Feature::Float(header.extrinsic_translation.to_vec()),
    );

    // timestamp
    features.insert(
        "steps/timestamp".to_string(),
        Feature::Float(vec![frame.timestamp_relative_s as f32]),
    );

    // is_first, is_last, is_terminal
    let bv = |b: bool| Feature::Int64(vec![if b { 1 } else { 0 }]);
    features.insert("steps/is_first".to_string(), bv(step_idx == 0));
    features.insert(
        "steps/is_last".to_string(),
        bv(step_idx as u64 == total_frames - 1),
    );
    features.insert(
        "steps/is_terminal".to_string(),
        bv(step_idx as u64 == total_frames - 1),
    );

    Ok(proto::encode_example(&features))
}

/// Configuration for RLDS export.
pub struct RldsExportConfig {
    pub output_dir: PathBuf,
    pub dataset_name: String,
    pub dataset_description: String,
}

/// Export multiple .egorec files as RLDS episodes to TFRecord files.
///
/// Each .egorec file becomes one episode. Returns the number of episodes written.
pub fn export_rlds<F>(
    config: &RldsExportConfig,
    egorec_paths: &[String],
    mut progress_callback: F,
) -> Result<usize, RldsError>
where
    F: FnMut(u64, u64, &str), // (current_frame, total_frames, filename)
{
    let train_dir = config.output_dir.join(&config.dataset_name).join("1.0.0");
    fs::create_dir_all(&train_dir)?;

    let mut episode_count = 0;

    for path_str in egorec_paths {
        let reader = egorec::EgorecReader::open(path_str)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let header = reader.header().clone();
        let total = reader.frame_count();
        let filename = Path::new(path_str)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let frames = reader
            .frames()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        // Write TFRecord file for this episode
        let record_path = train_dir.join(format!(
            "ego_recording-train.tfrecord-{:05}-of-{:05}",
            episode_count,
            egorec_paths.len()
        ));
        let mut writer = BufWriter::new(fs::File::create(&record_path)?);

        for (step_idx, frame_result) in frames.enumerate() {
            let frame = frame_result
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            progress_callback(step_idx as u64, total, &filename);

            let encoded = build_step_example(&frame, &header, step_idx, total)?;
            write_record(&mut writer, &encoded)?;
        }

        writer.flush()?;
        episode_count += 1;
    }

    // Write TFDS metadata
    write_tfds_metadata(config, episode_count)?;

    Ok(episode_count)
}

#[derive(Serialize)]
struct DatasetInfo {
    name: String,
    description: String,
    version: String,
    splits: HashMap<String, SplitInfo>,
}

#[derive(Serialize)]
struct SplitInfo {
    name: String,
    num_examples: usize,
}

fn write_tfds_metadata(config: &RldsExportConfig, num_episodes: usize) -> Result<(), RldsError> {
    let metadata_dir = config.output_dir.join(&config.dataset_name).join("1.0.0");
    fs::create_dir_all(&metadata_dir)?;

    let info = DatasetInfo {
        name: config.dataset_name.clone(),
        description: config.dataset_description.clone(),
        version: "1.0.0".to_string(),
        splits: {
            let mut m = HashMap::new();
            m.insert(
                "train".to_string(),
                SplitInfo {
                    name: "train".to_string(),
                    num_examples: num_episodes,
                },
            );
            m
        },
    };

    let json = serde_json::to_string_pretty(&info)?;
    fs::write(metadata_dir.join("dataset_info.json"), json)?;

    // features.json describing the RLDS schema
    let features_json = serde_json::json!({
        "pythonClassName": "tensorflow_datasets.core.features.features_dict.FeaturesDict",
        "featuresDict": {
            "features": {
                "steps": {
                    "pythonClassName": "tensorflow_datasets.core.features.dataset_feature.Dataset",
                    "sequence": {
                        "feature": {
                            "pythonClassName": "tensorflow_datasets.core.features.features_dict.FeaturesDict",
                            "featuresDict": {
                                "features": {
                                    "observation": {
                                        "pythonClassName": "tensorflow_datasets.core.features.features_dict.FeaturesDict",
                                        "featuresDict": {
                                            "features": {
                                                "image": {
                                                    "pythonClassName": "tensorflow_datasets.core.features.image_feature.Image",
                                                    "image": { "shape": { "dimensions": [480, 640, 3] }, "dtype": "uint8", "encodingFormat": "jpeg" }
                                                },
                                                "depth": {
                                                    "pythonClassName": "tensorflow_datasets.core.features.image_feature.Image",
                                                    "image": { "shape": { "dimensions": [480, 640, 1] }, "dtype": "uint16", "encodingFormat": "png" }
                                                },
                                                "depth_intrinsics": {
                                                    "pythonClassName": "tensorflow_datasets.core.features.tensor_feature.Tensor",
                                                    "tensor": { "shape": { "dimensions": [4] }, "dtype": "float32" }
                                                },
                                                "color_intrinsics": {
                                                    "pythonClassName": "tensorflow_datasets.core.features.tensor_feature.Tensor",
                                                    "tensor": { "shape": { "dimensions": [4] }, "dtype": "float32" }
                                                },
                                                "extrinsic_R": {
                                                    "pythonClassName": "tensorflow_datasets.core.features.tensor_feature.Tensor",
                                                    "tensor": { "shape": { "dimensions": [3, 3] }, "dtype": "float32" }
                                                },
                                                "extrinsic_t": {
                                                    "pythonClassName": "tensorflow_datasets.core.features.tensor_feature.Tensor",
                                                    "tensor": { "shape": { "dimensions": [3] }, "dtype": "float32" }
                                                }
                                            }
                                        }
                                    },
                                    "timestamp": {
                                        "pythonClassName": "tensorflow_datasets.core.features.scalar.Scalar",
                                        "tensor": { "shape": {}, "dtype": "float64" }
                                    },
                                    "is_first": {
                                        "pythonClassName": "tensorflow_datasets.core.features.scalar.Scalar",
                                        "tensor": { "shape": {}, "dtype": "bool" }
                                    },
                                    "is_last": {
                                        "pythonClassName": "tensorflow_datasets.core.features.scalar.Scalar",
                                        "tensor": { "shape": {}, "dtype": "bool" }
                                    },
                                    "is_terminal": {
                                        "pythonClassName": "tensorflow_datasets.core.features.scalar.Scalar",
                                        "tensor": { "shape": {}, "dtype": "bool" }
                                    }
                                }
                            }
                        }
                    }
                },
                "episode_metadata": {
                    "pythonClassName": "tensorflow_datasets.core.features.features_dict.FeaturesDict",
                    "featuresDict": {
                        "features": {
                            "file_path": { "pythonClassName": "tensorflow_datasets.core.features.text_feature.Text", "text": {} },
                            "session_name": { "pythonClassName": "tensorflow_datasets.core.features.text_feature.Text", "text": {} },
                            "duration_s": { "pythonClassName": "tensorflow_datasets.core.features.scalar.Scalar", "tensor": { "shape": {}, "dtype": "float64" } },
                            "dataset_name": { "pythonClassName": "tensorflow_datasets.core.features.text_feature.Text", "text": {} },
                            "dataset_description": { "pythonClassName": "tensorflow_datasets.core.features.text_feature.Text", "text": {} }
                        }
                    }
                }
            }
        }
    });

    let features_str = serde_json::to_string_pretty(&features_json)?;
    fs::write(metadata_dir.join("features.json"), features_str)?;

    Ok(())
}
