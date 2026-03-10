use anyhow::Result;
use egorec::{
    AnalysisResult, EgorecScanner, EpisodeFeatures, ScanConfig, StationProfile, Verdict,
};
use rayon::prelude::*;
use std::path::Path;

use super::validate::collect_egorec_files;

fn load_profile(path: &str) -> Result<StationProfile> {
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn run(
    paths: &[String],
    verbose: bool,
    report_path: Option<&str>,
    activity_k: Option<f32>,
    profile_path: Option<&str>,
) -> Result<()> {
    let files = collect_egorec_files(paths)?;
    if files.is_empty() {
        anyhow::bail!("no .egorec files found");
    }

    let mut config = ScanConfig::default();
    if let Some(k) = activity_k {
        config.activity_k = k;
    }

    let profile = match profile_path {
        Some(p) => Some(load_profile(p)?),
        None => None,
    };

    let results: Vec<Result<AnalysisResult, String>> = files
        .par_iter()
        .map(|file| {
            let path = Path::new(file);
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            match EgorecScanner::scan_with_profile(path, &config, profile.as_ref()) {
                Ok(summary) => {
                    let features = EpisodeFeatures::from_summary(&summary, &config);
                    Ok(AnalysisResult::compute(&name, &summary, &features))
                }
                Err(e) => Err(format!("{}: {}", name, e)),
            }
        })
        .collect();

    // Print results
    let mut all_results = Vec::new();
    for result in &results {
        match result {
            Ok(r) => {
                let verdict_color = match r.verdict {
                    Verdict::Keep => "\x1b[32m",          // green
                    Verdict::PruneConfident => "\x1b[31m", // red
                    Verdict::PruneSuggested => "\x1b[33m", // yellow
                    Verdict::Review => "\x1b[36m",         // cyan
                };
                println!(
                    "{}{:17}\x1b[0m  {}  (score: {:.2}, {} frames, {:.1}s)",
                    verdict_color,
                    r.verdict.to_string(),
                    r.filename,
                    r.activity_score,
                    r.total_frames,
                    r.duration_s,
                );

                if verbose {
                    if !r.reasons_keep.is_empty() {
                        for reason in &r.reasons_keep {
                            println!("    \x1b[32m+ {}\x1b[0m", reason);
                        }
                    }
                    if !r.reasons_prune.is_empty() {
                        for reason in &r.reasons_prune {
                            println!("    \x1b[31m- {}\x1b[0m", reason);
                        }
                    }
                    let baseline_source = if r.used_profile { "profile" } else { "per-episode" };
                    println!(
                        "    rgb: baseline median={:.0} MAD={:.0} threshold={:.0} [{}]",
                        r.idle_baseline.median,
                        r.idle_baseline.mad,
                        r.idle_baseline.threshold,
                        baseline_source,
                    );
                    println!(
                        "    depth: baseline median={:.0} MAD={:.0} onset={:.0} offset={:.0} [{}]",
                        r.depth_idle_baseline.median,
                        r.depth_idle_baseline.mad,
                        r.depth_idle_baseline.median
                            + config.depth_activity_k_onset
                                * r.depth_idle_baseline.mad
                                * config.mad_consistency,
                        r.depth_idle_baseline.median
                            + config.depth_activity_k_offset
                                * r.depth_idle_baseline.mad
                                * config.mad_consistency,
                        baseline_source,
                    );
                    println!(
                        "    depth: active={:.1}% cv={:.3} (episode) max_window_cv={:.3}",
                        r.features.depth_active_frame_fraction * 100.0,
                        r.features.depth_cv,
                        r.features.window_depth_cv_max,
                    );
                    println!(
                        "    fusion: ego_motion={:.0}% fused_active={:.1}%",
                        r.features.ego_motion_window_fraction * 100.0,
                        r.features.active_frame_fraction * 100.0,
                    );
                    println!(
                        "    features: bursts={} p95/p50={:.2} final_third={:.1}%",
                        r.features.burst_count,
                        r.features.p95_p50_ratio,
                        r.features.final_third_activity * 100.0,
                    );

                    // Warn if baseline may be contaminated
                    if r.features.active_frame_fraction > 0.75 && !r.used_profile {
                        println!(
                            "    \x1b[33m! >75% active without profile — baseline may be contaminated, consider --profile\x1b[0m"
                        );
                    }
                }

                all_results.push(r.clone());
            }
            Err(e) => {
                println!("\x1b[31mERROR\x1b[0m  {}", e);
            }
        }
    }

    // Summary
    let keep = all_results.iter().filter(|r| r.verdict == Verdict::Keep).count();
    let prune_confident = all_results
        .iter()
        .filter(|r| r.verdict == Verdict::PruneConfident)
        .count();
    let prune_suggested = all_results
        .iter()
        .filter(|r| r.verdict == Verdict::PruneSuggested)
        .count();
    let review = all_results
        .iter()
        .filter(|r| r.verdict == Verdict::Review)
        .count();

    println!(
        "\nSummary: {} keep, {} prune_confident, {} prune_suggested, {} review",
        keep, prune_confident, prune_suggested, review
    );

    // Write report if requested
    if let Some(report) = report_path {
        let json = serde_json::to_string_pretty(&all_results)?;
        std::fs::write(report, json)?;
        println!("Report written to {}", report);
    }

    Ok(())
}
