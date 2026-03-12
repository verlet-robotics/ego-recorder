use crate::state::AppState;
use egorec::{
    AnalysisResult, EgorecScanner, EpisodeFeatures, ScanConfig, ScanSummary, StationProfile,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisProgress {
    pub current: usize,
    pub total: usize,
    pub file: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResponse {
    pub status: String,
    pub results: Option<Vec<AnalysisResult>>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn run_analysis(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<AnalysisResponse, String> {
    {
        let running = state.analysis_running.read();
        if *running {
            return Ok(AnalysisResponse {
                status: "running".into(),
                results: None,
                error: None,
            });
        }
    }

    *state.analysis_running.write() = true;
    *state.analysis_cache.write() = None;

    let paths: Vec<(String, String)> = {
        let index = state.file_index.read();
        index
            .values()
            .map(|f| (f.name.clone(), f.path.to_string_lossy().to_string()))
            .collect()
    };

    if paths.is_empty() {
        *state.analysis_running.write() = false;
        return Ok(AnalysisResponse {
            status: "done".into(),
            results: Some(vec![]),
            error: None,
        });
    }

    let state_clone = Arc::clone(&state);

    let result = tokio::task::spawn_blocking(move || {
        let config = ScanConfig::default();
        let total = paths.len();

        let mut summaries: Vec<(String, String, ScanSummary)> = Vec::with_capacity(total);

        for (i, (name, path)) in paths.iter().enumerate() {
            let _ = app.emit(
                "analysis:progress",
                AnalysisProgress {
                    current: i + 1,
                    total,
                    file: name.clone(),
                },
            );

            let p = std::path::Path::new(path);
            match EgorecScanner::scan(p, &config) {
                Ok(summary) => {
                    summaries.push((name.clone(), path.clone(), summary));
                }
                Err(e) => {
                    log::warn!("Failed to scan {}: {}", name, e);
                }
            }
        }

        let profile = StationProfile::merge(
            &summaries.iter().map(|(_, _, s)| s.clone()).collect::<Vec<_>>(),
            config.idle_percentile,
        );

        let mut results: Vec<AnalysisResult> = Vec::with_capacity(summaries.len());
        for (name, path, summary) in &summaries {
            let p = std::path::Path::new(path);
            match EgorecScanner::scan_with_profile(p, &config, Some(&profile)) {
                Ok(profiled_summary) => {
                    let features = EpisodeFeatures::from_summary(&profiled_summary, &config);
                    let result = AnalysisResult::compute(name, &profiled_summary, &features);
                    results.push(result);
                }
                Err(e) => {
                    log::warn!("Failed profiled scan of {}: {}", name, e);
                    let features = EpisodeFeatures::from_summary(summary, &config);
                    let result = AnalysisResult::compute(name, summary, &features);
                    results.push(result);
                }
            }
        }

        results
    })
    .await
    .map_err(|e| format!("Analysis task panicked: {}", e))?;

    *state_clone.analysis_cache.write() = Some(result.clone());
    *state_clone.analysis_running.write() = false;

    Ok(AnalysisResponse {
        status: "done".into(),
        results: Some(result),
        error: None,
    })
}

#[tauri::command]
pub async fn get_analysis(state: State<'_, Arc<AppState>>) -> Result<AnalysisResponse, String> {
    let running = *state.analysis_running.read();
    if running {
        return Ok(AnalysisResponse {
            status: "running".into(),
            results: None,
            error: None,
        });
    }

    let cache = state.analysis_cache.read();
    if let Some(ref results) = *cache {
        return Ok(AnalysisResponse {
            status: "done".into(),
            results: Some(results.clone()),
            error: None,
        });
    }

    Ok(AnalysisResponse {
        status: "idle".into(),
        results: None,
        error: None,
    })
}
