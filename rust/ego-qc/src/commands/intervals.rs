use anyhow::Result;
use egorec::{EgorecScanner, ScanConfig, StationProfile};
use serde::Serialize;
use std::path::Path;

use super::validate::collect_egorec_files;

fn load_profile(path: &str) -> Result<StationProfile> {
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

#[derive(Serialize)]
struct IntervalProposal {
    start_frame: usize,
    end_frame: usize,
    start_s: f64,
    end_s: f64,
    duration_s: f64,
    active_frames: usize,
    total_frames: usize,
    active_fraction: f32,
}

#[derive(Serialize)]
struct IntervalReport {
    filename: String,
    total_frames: u64,
    duration_s: f64,
    used_profile: bool,
    min_gap_frames: usize,
    min_duration_frames: usize,
    pad_frames: usize,
    proposals: Vec<IntervalProposal>,
}

fn frame_to_seconds(
    summary: &egorec::ScanSummary,
    frame_idx: usize,
    base_ts_us: u64,
    fallback_end_s: f64,
) -> f64 {
    if frame_idx >= summary.frame_infos.len() {
        return fallback_end_s;
    }
    let ts_us = summary.frame_infos[frame_idx].timestamp_us;
    (ts_us.saturating_sub(base_ts_us)) as f64 / 1e6
}

pub fn run(
    paths: &[String],
    output: Option<&str>,
    min_gap: Option<u64>,
    min_duration: Option<u64>,
    pad: Option<u64>,
    profile_path: Option<&str>,
) -> Result<()> {
    let files = collect_egorec_files(paths)?;
    if files.is_empty() {
        anyhow::bail!("no .egorec files found");
    }

    let config = ScanConfig::default();
    let min_gap_frames = min_gap.unwrap_or(300) as usize;
    let min_duration_frames = min_duration.unwrap_or(60) as usize;
    let pad_frames = pad.unwrap_or(30) as usize;

    let profile = match profile_path {
        Some(p) => Some(load_profile(p)?),
        None => None,
    };

    let mut reports = Vec::new();

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

        let segments = summary.compute_segments(min_gap_frames, min_duration_frames, pad_frames);
        let base_ts_us = summary.start_timestamp_us;
        let duration_s = summary.duration_us as f64 / 1e6;

        let proposals = segments
            .into_iter()
            .map(|segment| {
                let start_s =
                    frame_to_seconds(&summary, segment.start_frame, base_ts_us, duration_s);
                let end_s = frame_to_seconds(&summary, segment.end_frame, base_ts_us, duration_s);
                let duration_s = (end_s - start_s).max(0.0);
                let active_fraction = if segment.total_frames == 0 {
                    0.0
                } else {
                    segment.active_frames as f32 / segment.total_frames as f32
                };

                IntervalProposal {
                    start_frame: segment.start_frame,
                    end_frame: segment.end_frame,
                    start_s,
                    end_s,
                    duration_s,
                    active_frames: segment.active_frames,
                    total_frames: segment.total_frames,
                    active_fraction,
                }
            })
            .collect();

        reports.push(IntervalReport {
            filename: file.clone(),
            total_frames: summary.total_frames,
            duration_s,
            used_profile: summary.used_profile,
            min_gap_frames,
            min_duration_frames,
            pad_frames,
            proposals,
        });
    }

    let json = serde_json::to_string_pretty(&reports)?;
    if let Some(out) = output {
        std::fs::write(out, json)?;
        println!("Interval report written to {}", out);
    } else {
        println!("{}", json);
    }

    Ok(())
}
