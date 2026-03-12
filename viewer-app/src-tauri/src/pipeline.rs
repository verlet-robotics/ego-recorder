use egorec::{
    AnalysisResult, EgorecScanner, EpisodeFeatures, ScanConfig, ScanSummary, StationProfile,
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::Path;

fn stable_hash(input: &str, length: usize) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)[..length].to_string()
}

fn episode_id(source_key: &str) -> String {
    format!("ep_{}", stable_hash(source_key, 16))
}

fn interval_id(source_key: &str, start_s: f64, end_s: f64) -> String {
    let start_ms = (start_s * 1000.0).round() as i64;
    let end_ms = (end_s * 1000.0).round() as i64;
    let input = format!("{}::{}::{}", source_key, start_ms, end_ms);
    format!("int_{}", stable_hash(&input, 12))
}

fn determine_episode_status(
    validate_ok: bool,
    has_footer: bool,
    duration_s: f64,
    analysis: Option<&AnalysisResult>,
    reject_shorter_than_s: f64,
) -> &'static str {
    if !validate_ok || !has_footer {
        return "invalid";
    }
    if duration_s < reject_shorter_than_s {
        return "reject";
    }
    match analysis {
        None => "invalid",
        Some(result) => match result.verdict {
            egorec::Verdict::Keep => "keep",
            egorec::Verdict::PruneConfident => "reject",
            _ => "review",
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageManifestRow {
    pub key: String,
    pub local_path: String,
    #[serde(default)]
    pub source_prefix: Option<String>,
    #[serde(default)]
    pub camera_serial: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub recorded_at: Option<String>,
    #[serde(default)]
    pub duration_s: Option<f64>,
    #[serde(default)]
    pub frame_count: Option<u64>,
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub rgb_codec: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeRow {
    pub episode_id: String,
    pub source_key: String,
    #[serde(default)]
    pub source_prefix: Option<String>,
    pub local_path: String,
    #[serde(default)]
    pub camera_serial: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub recorded_at: Option<String>,
    pub duration_s: f64,
    pub frame_count: u64,
    pub fps: f64,
    pub size_bytes: u64,
    pub validate_ok: bool,
    pub validation_status: String,
    #[serde(default)]
    pub analysis_error: Option<String>,
    #[serde(default)]
    pub analyze_verdict: Option<String>,
    #[serde(default)]
    pub activity_score: Option<f32>,
    #[serde(default)]
    pub reasons_keep: Vec<String>,
    #[serde(default)]
    pub reasons_prune: Vec<String>,
    pub used_profile: bool,
    #[serde(default)]
    pub profile_path: Option<String>,
    pub episode_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalRow {
    pub interval_id: String,
    pub source_key: String,
    #[serde(default)]
    pub source_prefix: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub camera_serial: Option<String>,
    pub local_path: String,
    #[serde(default)]
    pub episode_status: Option<String>,
    pub proposal_source: String,
    pub start_s: f64,
    pub end_s: f64,
    pub duration_s: f64,
    pub active_fraction: f64,
    pub effective_start_s: f64,
    pub effective_end_s: f64,
    pub effective_duration_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(rename = "sourcePrefix", default)]
    pub source_prefix: Option<String>,
    #[serde(rename = "publishPrefix", default)]
    pub publish_prefix: Option<String>,
    #[serde(rename = "qcBinary", default)]
    pub qc_binary: Option<String>,
    #[serde(rename = "pythonBinary", default)]
    pub python_binary: Option<String>,
    #[serde(rename = "configPath", default)]
    pub config_path: Option<String>,
    #[serde(rename = "stageTimestamps", default)]
    pub stage_timestamps: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineResult {
    pub stage: String,
    pub success: bool,
    pub message: String,
    pub counts: HashMap<String, usize>,
}

fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|e| format!("JSONL parse: {}", e)))
        .collect()
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
    }
    let mut out = String::new();
    for row in rows {
        let line = serde_json::to_string(row).map_err(|e| format!("serialize: {}", e))?;
        out.push_str(&line);
        out.push('\n');
    }
    std::fs::write(path, &out).map_err(|e| format!("write {}: {}", path.display(), e))
}

fn update_workspace_timestamp(workspace: &Path, stage: &str) -> Result<(), String> {
    let config_path = workspace.join("curation/v1/workspace.json");
    let mut config: WorkspaceConfig = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("read workspace.json: {}", e))?;
        serde_json::from_str(&contents).unwrap_or(WorkspaceConfig {
            version: Some("v1".into()),
            workspace: Some(workspace.to_string_lossy().to_string()),
            source_prefix: None,
            publish_prefix: None,
            qc_binary: None,
            python_binary: None,
            config_path: None,
            stage_timestamps: HashMap::new(),
        })
    } else {
        WorkspaceConfig {
            version: Some("v1".into()),
            workspace: Some(workspace.to_string_lossy().to_string()),
            source_prefix: None,
            publish_prefix: None,
            qc_binary: None,
            python_binary: None,
            config_path: None,
            stage_timestamps: HashMap::new(),
        }
    };

    config
        .stage_timestamps
        .insert(stage.into(), chrono::Utc::now().to_rfc3339());

    std::fs::create_dir_all(config_path.parent().unwrap())
        .map_err(|e| format!("mkdir: {}", e))?;
    let serialized =
        serde_json::to_string_pretty(&config).map_err(|e| format!("serialize: {}", e))?;
    std::fs::write(&config_path, serialized).map_err(|e| format!("write: {}", e))
}

/// Native Rust implementation of the `qc` pipeline stage.
///
/// Reads stage_manifest.jsonl, validates and analyzes each episode using
/// the egorec crate directly (no subprocess), and writes episodes.jsonl.
pub fn run_qc_stage(
    workspace: &Path,
    progress_fn: &dyn Fn(PipelineProgress),
) -> Result<PipelineResult, String> {
    let manifest_path = workspace.join("staging/v1/stage_manifest.jsonl");
    let episodes_path = workspace.join("curation/v1/episodes.jsonl");
    let profiles_dir = workspace.join("staging/v1/profiles");

    let manifest: Vec<StageManifestRow> = read_jsonl(&manifest_path)?;
    if manifest.is_empty() {
        return Err(format!(
            "No staging manifest found at {}",
            manifest_path.display()
        ));
    }

    let config = ScanConfig::default();
    let total = manifest.len();

    // Group files by camera serial for profile calibration
    let mut groups: HashMap<String, Vec<&StageManifestRow>> = HashMap::new();
    for row in &manifest {
        let key = row.camera_serial.clone().unwrap_or_else(|| "unknown".into());
        groups.entry(key).or_default().push(row);
    }

    // Phase 1: Scan all files and build per-camera profiles
    progress_fn(PipelineProgress {
        stage: "qc".into(),
        current: 0,
        total,
        file: "Calibrating profiles...".into(),
    });

    let mut scan_cache: HashMap<String, ScanSummary> = HashMap::new();
    for (i, row) in manifest.iter().enumerate() {
        let path = Path::new(&row.local_path);
        if !path.exists() {
            log::warn!("Staged file missing: {}", row.local_path);
            continue;
        }
        progress_fn(PipelineProgress {
            stage: "qc".into(),
            current: i + 1,
            total,
            file: row.key.clone(),
        });
        match EgorecScanner::scan(path, &config) {
            Ok(summary) => {
                scan_cache.insert(row.key.clone(), summary);
            }
            Err(e) => {
                log::warn!("Scan failed for {}: {}", row.key, e);
            }
        }
    }

    // Build and save profiles per camera group
    let mut profile_map: HashMap<String, StationProfile> = HashMap::new();
    std::fs::create_dir_all(&profiles_dir).ok();

    for (camera_key, rows) in &groups {
        let group_summaries: Vec<&ScanSummary> = rows
            .iter()
            .filter_map(|r| scan_cache.get(&r.key))
            .collect();
        if group_summaries.is_empty() {
            continue;
        }
        let cloned: Vec<ScanSummary> = group_summaries.iter().map(|s| (*s).clone()).collect();
        let profile = StationProfile::merge(&cloned, config.idle_percentile);

        let profile_path = profiles_dir.join(format!("{}.json", camera_key));
        if let Ok(json) = serde_json::to_string_pretty(&profile) {
            let _ = std::fs::write(&profile_path, json);
        }

        profile_map.insert(camera_key.clone(), profile);
    }

    // Phase 2: Re-scan with profiles and produce analysis results
    let mut episode_rows: Vec<EpisodeRow> = Vec::with_capacity(total);
    let reject_shorter_than_s = 2.0;

    for (i, row) in manifest.iter().enumerate() {
        progress_fn(PipelineProgress {
            stage: "qc".into(),
            current: i + 1,
            total,
            file: format!("Analyzing {}", row.key),
        });

        let path = Path::new(&row.local_path);
        let camera_key = row.camera_serial.clone().unwrap_or_else(|| "unknown".into());
        let profile = profile_map.get(&camera_key);
        let profile_path = profiles_dir.join(format!("{}.json", camera_key));

        // Validate
        let validation = EgorecScanner::validate(path);
        let (validate_ok, has_footer) = match &validation {
            Ok(v) => (v.valid, v.has_footer),
            Err(_) => (false, false),
        };

        // Analyze with profile
        let (analysis, analysis_error) = if validate_ok {
            match EgorecScanner::scan_with_profile(path, &config, profile) {
                Ok(summary) => {
                    let features = EpisodeFeatures::from_summary(&summary, &config);
                    let result = AnalysisResult::compute(&row.key, &summary, &features);
                    (Some(result), None)
                }
                Err(e) => (None, Some(format!("{}", e))),
            }
        } else {
            (None, validation.as_ref().err().map(|e| format!("{}", e)))
        };

        let duration_s = analysis
            .as_ref()
            .map(|a| a.duration_s)
            .or(row.duration_s)
            .unwrap_or(0.0);
        let frame_count = analysis
            .as_ref()
            .map(|a| a.total_frames)
            .or(row.frame_count)
            .unwrap_or(0);
        let fps = if duration_s > 0.0 && frame_count > 0 {
            ((frame_count as f64 / duration_s) * 10000.0).round() / 10000.0
        } else {
            row.fps.unwrap_or(0.0)
        };

        let episode_status = determine_episode_status(
            validate_ok,
            has_footer,
            duration_s,
            analysis.as_ref(),
            reject_shorter_than_s,
        );

        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        episode_rows.push(EpisodeRow {
            episode_id: episode_id(&row.key),
            source_key: row.key.clone(),
            source_prefix: row.source_prefix.clone(),
            local_path: row.local_path.clone(),
            camera_serial: row.camera_serial.clone(),
            session_name: row.session_name.clone(),
            recorded_at: row.recorded_at.clone(),
            duration_s: (duration_s * 1e6).round() / 1e6,
            frame_count,
            fps,
            size_bytes: file_size,
            validate_ok,
            validation_status: if validate_ok && has_footer {
                "valid".into()
            } else {
                "invalid".into()
            },
            analysis_error,
            analyze_verdict: analysis.as_ref().map(|a| format!("{:?}", a.verdict)),
            activity_score: analysis.as_ref().map(|a| a.activity_score),
            reasons_keep: analysis
                .as_ref()
                .map(|a| a.reasons_keep.clone())
                .unwrap_or_default(),
            reasons_prune: analysis
                .as_ref()
                .map(|a| a.reasons_prune.clone())
                .unwrap_or_default(),
            used_profile: analysis.as_ref().map(|a| a.used_profile).unwrap_or(false),
            profile_path: if profile.is_some() {
                Some(profile_path.to_string_lossy().to_string())
            } else {
                None
            },
            episode_status: episode_status.into(),
        });
    }

    episode_rows.sort_by(|a, b| a.source_key.cmp(&b.source_key));
    write_jsonl(&episodes_path, &episode_rows)?;
    update_workspace_timestamp(workspace, "qc")?;

    let mut counts = HashMap::new();
    for row in &episode_rows {
        *counts.entry(row.episode_status.clone()).or_insert(0) += 1;
    }

    let msg = format!(
        "Wrote {} episodes (keep={}, review={}, reject={}, invalid={})",
        episode_rows.len(),
        counts.get("keep").unwrap_or(&0),
        counts.get("review").unwrap_or(&0),
        counts.get("reject").unwrap_or(&0),
        counts.get("invalid").unwrap_or(&0),
    );

    Ok(PipelineResult {
        stage: "qc".into(),
        success: true,
        message: msg,
        counts,
    })
}

/// Native Rust implementation of the `intervals` pipeline stage.
///
/// Reads episodes.jsonl, computes activity segments using egorec's
/// compute_segments, and writes intervals.jsonl.
pub fn run_intervals_stage(
    workspace: &Path,
    progress_fn: &dyn Fn(PipelineProgress),
) -> Result<PipelineResult, String> {
    let episodes_path = workspace.join("curation/v1/episodes.jsonl");
    let intervals_path = workspace.join("curation/v1/intervals.jsonl");

    let episodes: Vec<EpisodeRow> = read_jsonl(&episodes_path)?;
    if episodes.is_empty() {
        return Err("No episodes found. Run QC first.".into());
    }

    let eligible: Vec<&EpisodeRow> = episodes
        .iter()
        .filter(|ep| ep.episode_status == "keep" || ep.episode_status == "review")
        .collect();

    let config = ScanConfig::default();
    let total = eligible.len();
    let mut interval_rows: Vec<IntervalRow> = Vec::new();

    let min_gap_s = 1.0;
    let min_duration_s = 2.0;
    let pad_s = 0.5;

    for (i, episode) in eligible.iter().enumerate() {
        progress_fn(PipelineProgress {
            stage: "intervals".into(),
            current: i + 1,
            total,
            file: episode.source_key.clone(),
        });

        let path = Path::new(&episode.local_path);
        if !path.exists() {
            log::warn!("Episode file missing: {}", episode.local_path);
            continue;
        }

        // Load profile if available
        let profile: Option<StationProfile> = episode
            .profile_path
            .as_ref()
            .and_then(|pp| std::fs::read_to_string(pp).ok())
            .and_then(|json| serde_json::from_str(&json).ok());

        let summary = match EgorecScanner::scan_with_profile(path, &config, profile.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Scan failed for {}: {}", episode.source_key, e);
                continue;
            }
        };

        let fps = if summary.duration_us > 0 && summary.total_frames > 0 {
            summary.total_frames as f64 / (summary.duration_us as f64 / 1_000_000.0)
        } else {
            30.0
        };

        let min_gap_frames = (min_gap_s * fps) as usize;
        let min_duration_frames = (min_duration_s * fps) as usize;
        let pad_frames = (pad_s * fps) as usize;

        let proposals = summary.compute_segments(min_gap_frames, min_duration_frames, pad_frames);

        if proposals.is_empty() {
            continue;
        }

        for seg in &proposals {
            let start_s = seg.start_frame as f64 / fps;
            let end_s = seg.end_frame as f64 / fps;
            let duration = end_s - start_s;
            let active_frac = if seg.total_frames > 0 {
                seg.active_frames as f64 / seg.total_frames as f64
            } else {
                0.0
            };

            let iid = interval_id(&episode.source_key, start_s, end_s);

            interval_rows.push(IntervalRow {
                interval_id: iid,
                source_key: episode.source_key.clone(),
                source_prefix: episode.source_prefix.clone(),
                session_name: episode.session_name.clone(),
                camera_serial: episode.camera_serial.clone(),
                local_path: episode.local_path.clone(),
                episode_status: Some(episode.episode_status.clone()),
                proposal_source: "activity_v1".into(),
                start_s: (start_s * 1e6).round() / 1e6,
                end_s: (end_s * 1e6).round() / 1e6,
                duration_s: (duration * 1e6).round() / 1e6,
                active_fraction: (active_frac * 1e6).round() / 1e6,
                effective_start_s: (start_s * 1e6).round() / 1e6,
                effective_end_s: (end_s * 1e6).round() / 1e6,
                effective_duration_s: (duration * 1e6).round() / 1e6,
            });
        }
    }

    interval_rows.sort_by(|a, b| {
        a.source_key
            .cmp(&b.source_key)
            .then(a.start_s.partial_cmp(&b.start_s).unwrap())
    });

    write_jsonl(&intervals_path, &interval_rows)?;
    update_workspace_timestamp(workspace, "intervals")?;

    let mut counts = HashMap::new();
    counts.insert("intervals".into(), interval_rows.len());
    counts.insert("episodes_processed".into(), eligible.len());

    let msg = format!(
        "Wrote {} intervals from {} episodes",
        interval_rows.len(),
        eligible.len(),
    );

    Ok(PipelineResult {
        stage: "intervals".into(),
        success: true,
        message: msg,
        counts,
    })
}
