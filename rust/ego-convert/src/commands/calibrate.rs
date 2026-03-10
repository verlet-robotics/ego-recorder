use anyhow::Result;
use egorec::{EgorecScanner, EpisodeFeatures, ScanConfig, ScanSummary, StationProfile};
use std::path::Path;

use super::validate::collect_egorec_files;

/// Standard deviation of per-window active_frame_fraction values.
fn window_activity_std(summary: &ScanSummary) -> f64 {
    if summary.windows.len() < 2 {
        return 0.0;
    }

    let fracs: Vec<f64> = summary
        .windows
        .iter()
        .map(|w| w.active_frame_fraction as f64)
        .collect();

    let mean = fracs.iter().sum::<f64>() / fracs.len() as f64;
    let variance =
        fracs.iter().map(|&f| (f - mean).powi(2)).sum::<f64>() / (fracs.len() - 1) as f64;
    variance.sqrt()
}

/// Activity fraction in first half vs second half of the episode (by frame index).
fn half_activity(summary: &ScanSummary) -> (f32, f32) {
    let n = summary.frame_infos.len();
    if n == 0 {
        return (0.0, 0.0);
    }

    let mid = n / 2;

    let compute_frac = |start: usize, end: usize| -> f32 {
        let total_p = summary.frame_infos[start..end]
            .iter()
            .filter(|f| !f.is_expected_keyframe)
            .count();
        if total_p == 0 {
            return 0.0;
        }
        let active_p = summary.frame_infos[start..end]
            .iter()
            .enumerate()
            .filter(|(i, f)| !f.is_expected_keyframe && summary.active_mask[start + i])
            .count();
        active_p as f32 / total_p as f32
    };

    (compute_frac(0, mid), compute_frac(mid, n))
}

/// Episode-level RGB-depth Pearson correlation.
fn rgb_depth_correlation(summary: &ScanSummary) -> f64 {
    let pairs: Vec<(f64, f64)> = summary
        .frame_infos
        .iter()
        .filter(|f| !f.is_expected_keyframe)
        .map(|f| (f.rgb_compressed_size as f64, f.depth_compressed_size as f64))
        .collect();

    let n = pairs.len();
    if n < 2 {
        return 0.0;
    }

    let mean_x = pairs.iter().map(|(x, _)| x).sum::<f64>() / n as f64;
    let mean_y = pairs.iter().map(|(_, y)| y).sum::<f64>() / n as f64;

    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for (x, y) in &pairs {
        let dx = x - mean_x;
        let dy = y - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }

    let denom = (var_x * var_y).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        cov / denom
    }
}

pub fn run(
    paths: &[String],
    output: Option<&str>,
    format: &str,
    save_profile: Option<&str>,
) -> Result<()> {
    let files = collect_egorec_files(paths)?;
    if files.is_empty() {
        anyhow::bail!("no .egorec files found");
    }

    let config = ScanConfig::default();

    #[derive(serde::Serialize)]
    struct Row {
        filename: String,
        total_frames: u64,
        duration_s: f64,
        // RGB idle baseline
        idle_median: f32,
        idle_mad: f32,
        idle_threshold: f32,
        // RGB activity features
        active_frame_fraction: f32,
        burst_count: u32,
        p95_p50_ratio: f32,
        final_third_activity: f32,
        active_window_fraction: f32,
        longest_idle_prefix: usize,
        longest_idle_suffix: usize,
        rgb_p_mean: f64,
        rgb_p_std: f64,
        // Depth baseline
        depth_idle_median: f32,
        depth_idle_mad: f32,
        depth_idle_threshold: f32,
        // Depth activity features
        depth_active_frame_fraction: f32,
        depth_p_mean: f64,
        depth_p_std: f64,
        depth_cv: f32,
        // Per-window depth features
        window_depth_cv_mean: f32,
        window_depth_cv_max: f32,
        // Cross-modal
        rgb_depth_correlation: f64,
        ego_motion_window_fraction: f32,
        // Temporal features
        window_activity_std: f64,
        first_half_activity: f32,
        second_half_activity: f32,
        half_activity_diff: f32,
    }

    let mut rows = Vec::new();
    let mut summaries = Vec::new();

    for file in &files {
        let path = Path::new(file);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match EgorecScanner::scan(path, &config) {
            Ok(summary) => {
                let features = EpisodeFeatures::from_summary(&summary, &config);

                // Episode-level RGB-depth correlation
                let corr = rgb_depth_correlation(&summary);

                // Window activity std
                let w_std = window_activity_std(&summary);

                // First/second half activity
                let (first_half, second_half) = half_activity(&summary);

                rows.push(Row {
                    filename: name,
                    total_frames: summary.total_frames,
                    duration_s: summary.duration_us as f64 / 1e6,
                    idle_median: summary.idle_baseline.median,
                    idle_mad: summary.idle_baseline.mad,
                    idle_threshold: summary.idle_baseline.threshold,
                    active_frame_fraction: features.active_frame_fraction,
                    burst_count: features.burst_count,
                    p95_p50_ratio: features.p95_p50_ratio,
                    final_third_activity: features.final_third_activity,
                    active_window_fraction: features.active_window_fraction,
                    longest_idle_prefix: features.longest_idle_prefix,
                    longest_idle_suffix: features.longest_idle_suffix,
                    rgb_p_mean: summary.rgb_p_stats.mean,
                    rgb_p_std: summary.rgb_p_stats.std_dev(),
                    // Depth baseline
                    depth_idle_median: summary.depth_idle_baseline.median,
                    depth_idle_mad: summary.depth_idle_baseline.mad,
                    depth_idle_threshold: summary.depth_idle_baseline.threshold,
                    // Depth features
                    depth_active_frame_fraction: features.depth_active_frame_fraction,
                    depth_p_mean: summary.depth_p_stats.mean,
                    depth_p_std: summary.depth_p_stats.std_dev(),
                    depth_cv: features.depth_cv,
                    // Per-window depth
                    window_depth_cv_mean: features.window_depth_cv_mean,
                    window_depth_cv_max: features.window_depth_cv_max,
                    // Cross-modal
                    rgb_depth_correlation: corr,
                    ego_motion_window_fraction: features.ego_motion_window_fraction,
                    // Temporal
                    window_activity_std: w_std,
                    first_half_activity: first_half,
                    second_half_activity: second_half,
                    half_activity_diff: second_half - first_half,
                });

                summaries.push(summary);
            }
            Err(e) => {
                eprintln!("SKIP  {}: {}", name, e);
            }
        }
    }

    // Save station profile if requested
    if let Some(profile_path) = save_profile {
        let profile = StationProfile::merge(&summaries, config.idle_percentile);
        let json = serde_json::to_string_pretty(&profile)?;
        std::fs::write(profile_path, &json)?;
        println!(
            "Profile saved: rgb_median={:.0} depth_median={:.0} from {} recordings ({} frames)",
            profile.rgb_median, profile.depth_median, profile.recording_count, profile.frame_count
        );
    }

    let output_str;
    match format {
        "json" => {
            output_str = serde_json::to_string_pretty(&rows)?;
        }
        _ => {
            // CSV
            let mut wtr = csv::Writer::from_writer(Vec::new());
            for row in &rows {
                wtr.serialize(row)?;
            }
            wtr.flush()?;
            output_str = String::from_utf8(wtr.into_inner()?)?;
        }
    }

    if let Some(out) = output {
        std::fs::write(out, &output_str)?;
        println!("Calibration data written to {}", out);
    } else {
        print!("{}", output_str);
    }

    Ok(())
}
