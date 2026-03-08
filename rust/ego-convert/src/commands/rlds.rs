use crate::progress::ExportProgress;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub fn run(
    files: &[String],
    output: Option<&str>,
    name: Option<&str>,
    quiet: bool,
    dataset_name: Option<&str>,
    dataset_description: Option<&str>,
    _dataset_tags: Option<&str>,
) -> Result<()> {
    // Validate files exist
    for f in files {
        if !Path::new(f).exists() {
            bail!("file not found: {}", f);
        }
    }

    // Determine output directory
    let output_dir = match output {
        Some(o) => PathBuf::from(o),
        None => {
            let first = Path::new(&files[0]);
            let stem = first.file_stem().unwrap_or_default().to_string_lossy();
            first.parent().unwrap_or(Path::new(".")).join(format!("{}_rlds", stem))
        }
    };

    // Determine dataset name
    let effective_name = dataset_name.or(name);
    let ds_name = match effective_name {
        Some(n) => n.to_string(),
        None => {
            let reader = egorec::EgorecReader::open(&files[0])?;
            let sn = reader.header().session_name_str().to_string();
            if sn.is_empty() {
                "ego_recording".to_string()
            } else {
                sn
            }
        }
    };

    let description = dataset_description.unwrap_or("").to_string();

    if !quiet {
        eprintln!("Exporting {} file(s) to RLDS format", files.len());
        eprintln!("Output: {}", output_dir.display());
    }

    let config = tfrecord::rlds::RldsExportConfig {
        output_dir: output_dir.clone(),
        dataset_name: ds_name.clone(),
        dataset_description: description,
    };

    let mut progress: Option<ExportProgress> = None;
    let mut last_filename = String::new();

    let _episode_count = tfrecord::rlds::export_rlds(&config, files, |_frame, total, filename| {
        if quiet {
            return;
        }
        if filename != last_filename {
            if let Some(p) = progress.take() {
                p.finish();
            }
            progress = Some(ExportProgress::new(total, filename));
            last_filename = filename.to_string();
        }
        if let Some(ref mut p) = progress {
            // Approximate frame bytes: 640*480*3 (RGB) + 640*480*2 (depth)
            p.update(640 * 480 * 3 + 640 * 480 * 2);
        }
    })?;

    if let Some(p) = progress.take() {
        p.finish();
    }

    if !quiet {
        eprintln!("\nExport complete: {}", output_dir.display());
        eprintln!(
            "Load with: tfds.load('{}', data_dir='{}')",
            ds_name,
            output_dir.display()
        );
    }

    Ok(())
}
