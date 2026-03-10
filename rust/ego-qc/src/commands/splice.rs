use anyhow::Result;
use egorec::{EgorecScanner, EgorecWriter, ScanConfig, StationProfile};
use std::fs::File;
use std::path::Path;

use super::prune::{load_manifest, save_manifest, write_audit_entry, AuditEntry};
use super::validate::collect_egorec_files;

fn load_profile(path: &str) -> Result<StationProfile> {
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn run(
    paths: &[String],
    min_gap: Option<u64>,
    min_duration: Option<u64>,
    replace_original: bool,
    profile_path: Option<&str>,
) -> Result<()> {
    let files = collect_egorec_files(paths)?;
    if files.is_empty() {
        anyhow::bail!("no .egorec files found");
    }

    let config = ScanConfig::default();
    let min_gap_frames = min_gap.unwrap_or(300) as usize; // 10s default
    let min_dur_frames = min_duration.unwrap_or(60) as usize; // 2s default
    let pad_frames = 30usize; // 1s

    let profile = match profile_path {
        Some(p) => Some(load_profile(p)?),
        None => None,
    };

    let mut total_spliced = 0;
    let mut total_segments = 0;

    for file in &files {
        let path = Path::new(file);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let summary = match EgorecScanner::scan_with_profile(path, &config, profile.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("SKIP  {}: {}", name, e);
                continue;
            }
        };

        // Only splice if there are idle regions worth removing
        let segments = summary.compute_segments(min_gap_frames, min_dur_frames, pad_frames);

        if segments.is_empty() {
            println!("SKIP  {}  (no active segments found)", name);
            continue;
        }

        // If segments cover the entire file, skip
        if segments.len() == 1
            && segments[0].start_frame == 0
            && segments[0].end_frame >= summary.frame_infos.len()
        {
            println!("SKIP  {}  (entire file is active)", name);
            continue;
        }

        let dir = path.parent().unwrap_or(Path::new("."));

        // Read the source header for the writer
        let mut source_file = File::open(path)?;
        let header = {
            use std::io::{BufReader, Seek, SeekFrom};
            let mut reader = BufReader::new(&source_file);
            let h = egorec::FileHeader::read_from(&mut reader)?;
            source_file.seek(SeekFrom::Start(0))?;
            h
        };

        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let mut segment_files = Vec::new();

        for (seg_idx, segment) in segments.iter().enumerate() {
            let seg_name = format!("{}_seg{:03}.egorec", stem, seg_idx);
            let seg_path = dir.join(&seg_name);

            let frame_infos = &summary.frame_infos[segment.start_frame..segment.end_frame];

            let mut writer = EgorecWriter::create(&seg_path, &header)?;
            writer.copy_span(&mut source_file, frame_infos)?;
            writer.finalize()?;

            println!(
                "  SPLICE  {} -> {}  ({} frames, {}-{})",
                name,
                seg_name,
                segment.total_frames,
                segment.start_frame,
                segment.end_frame,
            );
            segment_files.push(seg_name);
        }

        // Write audit entry
        let audit_dir = dir.join(".pruned");
        std::fs::create_dir_all(&audit_dir)?;
        write_audit_entry(
            &audit_dir,
            &AuditEntry::Splice {
                timestamp: chrono::Utc::now().to_rfc3339(),
                source: name.clone(),
                segments: segment_files.clone(),
                active_frames: segments.iter().map(|s| s.active_frames as u64).sum(),
                total_frames: summary.total_frames,
            },
        )?;

        // Move original if requested
        if replace_original {
            let dest = audit_dir.join(&name);
            std::fs::rename(path, &dest)?;
            println!("  MOVED   {} -> .pruned/{}", name, name);

            // Update manifest
            if let Ok(mut manifest) = load_manifest(dir) {
                // Remove original episode
                manifest.episodes.retain(|e| e.filename != name);
                // Add segments
                for seg_name in &segment_files {
                    let seg_path = dir.join(seg_name);
                    if let Ok(summary) = EgorecScanner::scan(&seg_path, &config) {
                        manifest.episodes.push(super::prune::EpisodeEntry {
                            filename: seg_name.clone(),
                            session_name: header.session_name_str().to_string(),
                            recorded_at: String::new(),
                            duration_s: summary.duration_us as f64 / 1e6,
                            frames: summary.total_frames,
                        });
                    }
                }
                save_manifest(dir, &manifest)?;
            }
        }

        total_spliced += 1;
        total_segments += segments.len();
    }

    println!(
        "\n{} files spliced into {} segments",
        total_spliced, total_segments
    );

    Ok(())
}
