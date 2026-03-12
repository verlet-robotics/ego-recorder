use crate::state::{AppState, ConversionStatus, FileEntry};
use egorec::{EgorecScanner, EgorecWriter, FileHeader, ScanConfig};
use serde::Serialize;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneResponse {
    pub status: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpliceResponse {
    pub status: String,
    pub name: String,
    pub segments: Vec<String>,
    pub new_files: Vec<SpliceNewFile>,
    pub original_removed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpliceNewFile {
    pub name: String,
    pub dataset: Option<String>,
    pub session_name: String,
    pub rgb_codec: u8,
    pub color_width: u32,
    pub color_height: u32,
    pub fps: f64,
    pub total_frames: u64,
    pub duration_s: f64,
    pub size_bytes: u64,
    pub conversion_status: ConversionStatus,
    pub has_imu: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResponse {
    pub status: String,
    pub file: SpliceNewFile,
}

#[tauri::command]
pub async fn prune_file(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<PruneResponse, String> {
    let (file_path, _) = {
        let index = state.file_index.read();
        let entry = index.get(&name).ok_or_else(|| format!("File not found: {}", name))?;
        (entry.path.clone(), entry.name.clone())
    };

    let state_clone = Arc::clone(&state);
    let name_clone = name.clone();

    tokio::task::spawn_blocking(move || {
        let parent = file_path.parent().ok_or("No parent directory")?;
        let pruned_dir = parent.join(".pruned");
        std::fs::create_dir_all(&pruned_dir)
            .map_err(|e| format!("Failed to create .pruned/: {}", e))?;

        let file_name = file_path
            .file_name()
            .ok_or("No filename")?;
        let dest = pruned_dir.join(file_name);

        std::fs::rename(&file_path, &dest)
            .map_err(|e| format!("Failed to move file: {}", e))?;

        state_clone.file_index.write().remove(&name_clone);

        if let Some(ref mut cache) = *state_clone.analysis_cache.write() {
            cache.retain(|r| r.filename != name_clone);
        }

        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Prune task failed: {}", e))??;

    Ok(PruneResponse {
        status: "pruned".into(),
        name,
    })
}

#[tauri::command]
pub async fn splice_file(
    state: State<'_, Arc<AppState>>,
    name: String,
    min_gap: Option<f64>,
    min_duration: Option<f64>,
    replace_original: Option<bool>,
) -> Result<SpliceResponse, String> {
    let (file_path, recordings_dir) = {
        let index = state.file_index.read();
        let entry = index.get(&name).ok_or_else(|| format!("File not found: {}", name))?;
        let rdir = state.recordings_dir.read().clone().unwrap_or_default();
        (entry.path.clone(), rdir)
    };

    let replace = replace_original.unwrap_or(false);
    let state_clone = Arc::clone(&state);
    let name_clone = name.clone();

    let (segments, new_files) = tokio::task::spawn_blocking(move || {
        let config = ScanConfig::default();

        let summary = EgorecScanner::scan(&file_path, &config)
            .map_err(|e| format!("Scan failed: {}", e))?;

        let fps = if summary.duration_us > 0 && summary.total_frames > 0 {
            summary.total_frames as f64 / (summary.duration_us as f64 / 1_000_000.0)
        } else {
            30.0
        };

        let min_gap_frames = (min_gap.unwrap_or(1.0) * fps) as usize;
        let min_duration_frames = (min_duration.unwrap_or(2.0) * fps) as usize;
        let pad_frames = (0.5 * fps) as usize;

        let proposals = summary.compute_segments(
            min_gap_frames,
            min_duration_frames,
            pad_frames,
        );

        if proposals.is_empty() {
            return Ok::<_, String>((vec![], vec![]));
        }

        let header = {
            let mut f = BufReader::new(
                File::open(&file_path).map_err(|e| format!("Open: {}", e))?,
            );
            FileHeader::read_from(&mut f).map_err(|e| format!("Read header: {}", e))?
        };

        let parent = file_path.parent().ok_or("No parent dir")?;
        let stem = file_path
            .file_stem()
            .ok_or("No file stem")?
            .to_string_lossy();

        let mut segment_names = Vec::new();
        let mut new_file_entries = Vec::new();

        for (i, seg) in proposals.iter().enumerate() {
            let seg_name = format!("{}_seg{:03}.egorec", stem, i);
            let seg_path = parent.join(&seg_name);

            let mut writer = EgorecWriter::create(&seg_path, &header)
                .map_err(|e| format!("Failed to create segment file: {}", e))?;

            let frame_infos: Vec<_> = summary.frame_infos[seg.start_frame..=seg.end_frame].to_vec();

            let mut source_file = File::open(&file_path)
                .map_err(|e| format!("Failed to open source: {}", e))?;

            writer
                .copy_span(&mut source_file, &frame_infos)
                .map_err(|e| format!("Failed to copy span: {}", e))?;

            writer.finalize().map_err(|e| format!("Failed to finalize: {}", e))?;

            let seg_size = std::fs::metadata(&seg_path)
                .map(|m| m.len())
                .unwrap_or(0);

            let (seg_metadata, _) = crate::commands::files::parse_egorec_metadata(&seg_path)
                .map_err(|e| format!("Metadata parse failed: {}", e))?;

            let conv_status = if seg_metadata.rgb_codec == 2 {
                ConversionStatus::Streamable
            } else {
                ConversionStatus::Idle
            };

            let rel_name = if let Ok(rel) = seg_path.strip_prefix(&recordings_dir) {
                rel.to_string_lossy().to_string()
            } else {
                seg_name.clone()
            };

            let dataset = Path::new(&rel_name)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .filter(|s| s != ".");

            new_file_entries.push((
                rel_name.clone(),
                seg_path.clone(),
                seg_size,
                seg_metadata.clone(),
                conv_status,
                SpliceNewFile {
                    name: rel_name,
                    dataset,
                    session_name: seg_metadata.session_name.clone(),
                    rgb_codec: seg_metadata.rgb_codec,
                    color_width: seg_metadata.color_width,
                    color_height: seg_metadata.color_height,
                    fps: seg_metadata.fps,
                    total_frames: seg_metadata.total_frames,
                    duration_s: seg_metadata.duration_s,
                    size_bytes: seg_size,
                    conversion_status: conv_status,
                    has_imu: seg_metadata.has_imu,
                },
            ));

            segment_names.push(seg_name);
        }

        {
            let mut index = state_clone.file_index.write();
            for (rel_name, seg_path, seg_size, metadata, conv_status, _) in &new_file_entries {
                index.insert(
                    rel_name.clone(),
                    FileEntry {
                        name: rel_name.clone(),
                        path: seg_path.clone(),
                        size_bytes: *seg_size,
                        metadata: metadata.clone(),
                        conversion_status: *conv_status,
                    },
                );
            }

            if replace {
                index.remove(&name_clone);
                if let Err(e) = std::fs::remove_file(&file_path) {
                    log::warn!("Failed to remove original after splice: {}", e);
                }

                if let Some(ref mut cache) = *state_clone.analysis_cache.write() {
                    cache.retain(|r| r.filename != name_clone);
                }
            }
        }

        let new_files: Vec<SpliceNewFile> = new_file_entries
            .into_iter()
            .map(|(_, _, _, _, _, nf)| nf)
            .collect();

        Ok((segment_names, new_files))
    })
    .await
    .map_err(|e| format!("Splice task failed: {}", e))??;

    Ok(SpliceResponse {
        status: "spliced".into(),
        name,
        segments,
        new_files,
        original_removed: replace_original.unwrap_or(false),
    })
}

#[tauri::command]
pub async fn restore_file(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<RestoreResponse, String> {
    let recordings_dir = state
        .recordings_dir
        .read()
        .clone()
        .ok_or_else(|| "No recordings directory set".to_string())?;

    let rel_dir = Path::new(&name).parent().unwrap_or(Path::new(""));
    let file_name = Path::new(&name)
        .file_name()
        .ok_or("No filename")?
        .to_string_lossy()
        .to_string();

    let source_dir = if rel_dir == Path::new("") || rel_dir == Path::new(".") {
        recordings_dir.clone()
    } else {
        recordings_dir.join(rel_dir)
    };

    let pruned_path = source_dir.join(".pruned").join(&file_name);
    let dest_path = source_dir.join(&file_name);

    if !pruned_path.exists() {
        return Err(format!("Pruned file not found: {}", pruned_path.display()));
    }

    std::fs::rename(&pruned_path, &dest_path)
        .map_err(|e| format!("Failed to restore: {}", e))?;

    let (metadata, file_size) = crate::commands::files::parse_egorec_metadata(&dest_path)
        .map_err(|e| format!("Metadata parse failed: {}", e))?;

    let conv_status = if metadata.rgb_codec == 2 {
        ConversionStatus::Streamable
    } else {
        ConversionStatus::Idle
    };

    let dataset = Path::new(&name)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|s| s != "." && !s.is_empty());

    let new_file = SpliceNewFile {
        name: name.clone(),
        dataset: dataset.clone(),
        session_name: metadata.session_name.clone(),
        rgb_codec: metadata.rgb_codec,
        color_width: metadata.color_width,
        color_height: metadata.color_height,
        fps: metadata.fps,
        total_frames: metadata.total_frames,
        duration_s: metadata.duration_s,
        size_bytes: file_size,
        conversion_status: conv_status,
        has_imu: metadata.has_imu,
    };

    state.file_index.write().insert(
        name.clone(),
        FileEntry {
            name: name.clone(),
            path: dest_path,
            size_bytes: file_size,
            metadata,
            conversion_status: conv_status,
        },
    );

    Ok(RestoreResponse {
        status: "restored".into(),
        file: new_file,
    })
}

#[tauri::command]
pub async fn list_pruned(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    let recordings_dir = state
        .recordings_dir
        .read()
        .clone()
        .ok_or_else(|| "No recordings directory set".to_string())?;

    let dir = recordings_dir.clone();
    tokio::task::spawn_blocking(move || {
        let mut pruned = Vec::new();
        collect_pruned(&dir, &dir, &mut pruned);
        pruned.sort();
        pruned
    })
    .await
    .map_err(|e| format!("List pruned failed: {}", e))
}

fn collect_pruned(root: &Path, current: &Path, out: &mut Vec<String>) {
    let pruned_dir = current.join(".pruned");
    if let Ok(entries) = std::fs::read_dir(&pruned_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("egorec") {
                let prefix = current.strip_prefix(root).unwrap_or(Path::new(""));
                let name = if prefix == Path::new("") {
                    entry.file_name().to_string_lossy().to_string()
                } else {
                    format!(
                        "{}/{}",
                        prefix.to_string_lossy(),
                        entry.file_name().to_string_lossy()
                    )
                };
                out.push(name);
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
                collect_pruned(root, &path, out);
            }
        }
    }
}
