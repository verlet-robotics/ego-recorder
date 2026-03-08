use crate::progress::ExportProgress;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

fn export_single_dataset(
    egorec_paths: &[String],
    output_dir: &Path,
    quiet: bool,
) -> Result<()> {
    let mut builder = lerobot_writer::LeRobotDatasetBuilder::new(output_dir, 30, 640, 480);

    for path_str in egorec_paths {
        let reader = egorec::EgorecReader::open(path_str)?;
        let total = reader.frame_count();
        let filename = Path::new(path_str)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let frames = reader.frames()?;
        builder.start_episode()?;

        let mut progress = if !quiet {
            Some(ExportProgress::new(total, &filename))
        } else {
            None
        };

        for frame_result in frames {
            let frame = frame_result?;

            builder.add_frame(
                &frame.rgb,
                &frame.depth,
                frame.timestamp_relative_s as f32,
            )?;

            if let Some(ref mut p) = progress {
                let frame_bytes = frame.rgb.len() as u64 + frame.depth.len() as u64 * 2;
                p.update(frame_bytes);
            }
        }

        if let Some(p) = progress.take() {
            p.finish();
        }

        builder.save_episode()?;
    }

    builder.finalize()?;

    if !quiet {
        eprintln!("\nDataset created: {}", output_dir.display());
        eprintln!("Episodes: {}", egorec_paths.len());
    }

    Ok(())
}

pub fn run(
    files: &[String],
    output: Option<&str>,
    name: Option<&str>,
    quiet: bool,
    separate: bool,
    dataset_name: Option<&str>,
    _dataset_description: Option<&str>,
    _dataset_tags: Option<&str>,
) -> Result<()> {
    // Validate files exist
    for f in files {
        if !Path::new(f).exists() {
            bail!("file not found: {}", f);
        }
    }

    let effective_name = dataset_name.or(name);

    if separate {
        for f in files {
            let reader = egorec::EgorecReader::open(f)?;
            let sn = reader.header().session_name_str().to_string();
            let ds_name = effective_name
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    if sn.is_empty() {
                        Path::new(f)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    } else {
                        sn
                    }
                });

            let out_dir = match output {
                Some(o) => PathBuf::from(o).join(&ds_name),
                None => {
                    let p = Path::new(f);
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                    p.parent()
                        .unwrap_or(Path::new("."))
                        .join(format!("{}_lerobot", stem))
                }
            };

            if !quiet {
                eprintln!("Exporting {} -> {}", f, out_dir.display());
            }

            export_single_dataset(&[f.clone()], &out_dir, quiet)?;
        }
    } else {
        let first_reader = egorec::EgorecReader::open(&files[0])?;
        let sn = first_reader.header().session_name_str().to_string();
        let ds_name = effective_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if sn.is_empty() {
                    "ego_recording".to_string()
                } else {
                    sn
                }
            });

        let out_dir = match output {
            Some(o) => PathBuf::from(o),
            None => {
                let first = Path::new(&files[0]);
                first
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(format!("{}_lerobot", ds_name))
            }
        };

        if !quiet {
            eprintln!("Exporting {} file(s) to LeRobot v3 format", files.len());
            eprintln!("Output: {}", out_dir.display());
        }

        export_single_dataset(files, &out_dir, quiet)?;
    }

    if !quiet {
        eprintln!("\nDone.");
    }

    Ok(())
}
