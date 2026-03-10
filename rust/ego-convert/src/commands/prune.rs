use anyhow::Result;
use egorec::{AnalysisResult, EgorecScanner, EpisodeFeatures, ScanConfig, StationProfile, Verdict};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

use super::validate::collect_egorec_files;

#[derive(Debug, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created: String,
    pub episodes: Vec<EpisodeEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodeEntry {
    pub filename: String,
    pub session_name: String,
    pub recorded_at: String,
    pub duration_s: f64,
    pub frames: u64,
}

#[derive(Debug, Serialize)]
#[serde(tag = "op")]
pub enum AuditEntry {
    #[serde(rename = "prune")]
    Prune {
        timestamp: String,
        source: String,
        destination: String,
        activity_score: f32,
    },
    #[serde(rename = "splice")]
    Splice {
        timestamp: String,
        source: String,
        segments: Vec<String>,
        active_frames: u64,
        total_frames: u64,
    },
    #[serde(rename = "restore")]
    Restore {
        timestamp: String,
        source: String,
        destination: String,
    },
}

pub fn load_manifest(dir: &Path) -> Result<DatasetManifest> {
    let path = dir.join("dataset.json");
    let data = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn save_manifest(dir: &Path, manifest: &DatasetManifest) -> Result<()> {
    let path = dir.join("dataset.json");
    let tmp = dir.join("dataset.json.tmp");
    let data = serde_json::to_string_pretty(manifest)?;
    fs::write(&tmp, &data)?;
    // fsync temp file
    let f = fs::File::open(&tmp)?;
    f.sync_all()?;
    drop(f);
    // Atomic rename
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn write_audit_entry(pruned_dir: &Path, entry: &AuditEntry) -> Result<()> {
    let audit_path = pruned_dir.join("audit.jsonl");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&audit_path)?;
    let line = serde_json::to_string(entry)?;
    writeln!(f, "{}", line)?;
    f.sync_all()?;
    Ok(())
}

fn load_profile(path: &str) -> Result<StationProfile> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn run(paths: &[String], apply: bool, threshold: Option<f32>, profile_path: Option<&str>) -> Result<()> {
    let files = collect_egorec_files(paths)?;
    if files.is_empty() {
        anyhow::bail!("no .egorec files found");
    }

    let config = ScanConfig::default();

    let profile = match profile_path {
        Some(p) => Some(load_profile(p)?),
        None => None,
    };

    let results: Vec<(String, Result<AnalysisResult, String>)> = files
        .par_iter()
        .map(|file| {
            let path = Path::new(file);
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let result = match EgorecScanner::scan_with_profile(path, &config, profile.as_ref()) {
                Ok(summary) => {
                    let features = EpisodeFeatures::from_summary(&summary, &config);
                    Ok(AnalysisResult::compute(&name, &summary, &features))
                }
                Err(e) => Err(format!("{}", e)),
            };
            (file.clone(), result)
        })
        .collect();

    let mut to_prune = Vec::new();
    let mut to_keep = Vec::new();

    for (file, result) in &results {
        match result {
            Ok(r) => {
                let should_prune = match r.verdict {
                    Verdict::PruneConfident => true,
                    Verdict::PruneSuggested => {
                        if let Some(t) = threshold {
                            r.activity_score < t
                        } else {
                            true
                        }
                    }
                    _ => false,
                };

                if should_prune {
                    println!(
                        "  PRUNE  {}  ({}, score: {:.2})",
                        r.filename, r.verdict, r.activity_score
                    );
                    to_prune.push((file.clone(), r.clone()));
                } else {
                    println!(
                        "  KEEP   {}  ({}, score: {:.2})",
                        r.filename, r.verdict, r.activity_score
                    );
                    to_keep.push((file.clone(), r.clone()));
                }
            }
            Err(e) => {
                eprintln!("  ERROR  {}: {}", file, e);
            }
        }
    }

    println!(
        "\n{} to prune, {} to keep",
        to_prune.len(),
        to_keep.len()
    );

    if !apply {
        if !to_prune.is_empty() {
            println!("\nDry run — pass --apply to execute");
        }
        return Ok(());
    }

    if to_prune.is_empty() {
        println!("Nothing to prune");
        return Ok(());
    }

    // Execute pruning
    let first_path = Path::new(&to_prune[0].0);
    let dir = first_path.parent().unwrap_or(Path::new("."));
    let pruned_dir = dir.join(".pruned");
    fs::create_dir_all(&pruned_dir)?;

    for (file, result) in &to_prune {
        let path = Path::new(file);
        let name = &result.filename;
        let dest = pruned_dir.join(name);
        fs::rename(path, &dest)?;

        write_audit_entry(
            &pruned_dir,
            &AuditEntry::Prune {
                timestamp: chrono::Utc::now().to_rfc3339(),
                source: name.clone(),
                destination: format!(".pruned/{}", name),
                activity_score: result.activity_score,
            },
        )?;
        println!("  MOVED  {} -> .pruned/{}", name, name);
    }

    // Update manifest
    if let Ok(mut manifest) = load_manifest(dir) {
        let pruned_names: Vec<&str> = to_prune.iter().map(|(_, r)| r.filename.as_str()).collect();
        manifest
            .episodes
            .retain(|e| !pruned_names.contains(&e.filename.as_str()));
        save_manifest(dir, &manifest)?;
        println!("Manifest updated ({} episodes removed)", pruned_names.len());
    }

    Ok(())
}
