use crate::recent::RecentWorkspace;
use crate::state::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurationWorkspaceInfo {
    pub root: Option<String>,
    pub active_workspace: Option<String>,
    pub active_name: Option<String>,
    pub has_workspace: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub name: String,
    pub path: String,
    pub source_prefix: Option<String>,
    pub episode_count: usize,
    pub completed_stages: Vec<String>,
    pub has_intervals: bool,
    pub has_labels: bool,
    pub has_buckets: bool,
}

fn is_workspace_dir(dir: &Path) -> bool {
    dir.join("curation/v1").is_dir()
        || dir.join("staging/v1").is_dir()
        || dir.join("inventory/v1").is_dir()
}

fn scan_workspace(dir: &Path) -> WorkspaceSummary {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let curation_dir = dir.join("curation/v1");

    let mut source_prefix = None;
    let mut completed_stages = Vec::new();
    if let Ok(contents) = std::fs::read_to_string(curation_dir.join("workspace.json")) {
        if let Ok(ws) = serde_json::from_str::<serde_json::Value>(&contents) {
            source_prefix = ws
                .get("sourcePrefix")
                .and_then(|v| v.as_str())
                .map(String::from);
            if let Some(ts) = ws.get("stageTimestamps").and_then(|v| v.as_object()) {
                completed_stages = ts.keys().cloned().collect();
            }
        }
    }

    let episode_count = count_jsonl_lines(&curation_dir.join("episodes.jsonl"));

    let has_intervals = curation_dir.join("intervals.jsonl").exists();
    let has_labels = curation_dir.join("labels.jsonl").exists();
    let has_buckets = curation_dir.join("bucket_map.json").exists();

    WorkspaceSummary {
        name,
        path: dir.to_string_lossy().to_string(),
        source_prefix,
        episode_count,
        completed_stages,
        has_intervals,
        has_labels,
        has_buckets,
    }
}

fn count_jsonl_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

fn is_multi_workspace_root(dir: &Path) -> bool {
    if is_workspace_dir(dir) {
        return false;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && is_workspace_dir(&p) {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn get_curation_workspace(
    state: State<'_, Arc<AppState>>,
) -> Result<CurationWorkspaceInfo, String> {
    let root = state.curation_root.read().clone();
    let ws = state.curation_workspace.read().clone();
    let active_name = ws
        .as_ref()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

    Ok(CurationWorkspaceInfo {
        root: root.as_ref().map(|p| p.to_string_lossy().to_string()),
        active_workspace: ws.as_ref().map(|p| p.to_string_lossy().to_string()),
        active_name,
        has_workspace: ws.is_some(),
    })
}

/// Open a curation root. If `dir` is itself a workspace, use it directly.
/// If `dir` contains workspace subdirectories, set it as root (multi-workspace mode).
#[tauri::command]
pub async fn set_curation_root(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    dir: String,
) -> Result<CurationWorkspaceInfo, String> {
    let path = PathBuf::from(&dir);
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", dir));
    }

    if is_workspace_dir(&path) {
        let parent = path.parent().map(|p| p.to_path_buf());
        *state.curation_root.write() = parent;
        *state.curation_workspace.write() = Some(path.clone());
    } else if is_multi_workspace_root(&path) {
        *state.curation_root.write() = Some(path.clone());
        *state.curation_workspace.write() = None;
    } else {
        return Err("Directory is neither a workspace nor contains workspaces".into());
    }

    if let Ok(data_dir) = app.path().app_data_dir() {
        let dir_str = dir.clone();
        let _ = crate::recent::touch(&data_dir, &dir_str);
    }

    get_curation_workspace(state).await
}

/// List all workspace subdirectories within the curation root.
#[tauri::command]
pub async fn list_workspaces(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<WorkspaceSummary>, String> {
    let root = state.curation_root.read().clone();
    let active = state.curation_workspace.read().clone();

    // If no root but we have an active workspace, just return that one
    if root.is_none() {
        if let Some(ref ws) = active {
            if is_workspace_dir(ws) {
                return Ok(vec![scan_workspace(ws)]);
            }
        }
        return Ok(vec![]);
    }

    let root_path = root.unwrap();

    let summaries = tokio::task::spawn_blocking(move || {
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&root_path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && is_workspace_dir(&p) {
                    results.push(scan_workspace(&p));
                }
            }
        }
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    })
    .await
    .map_err(|e| format!("Scan failed: {}", e))?;

    Ok(summaries)
}

/// Switch the active workspace to one of the children.
#[tauri::command]
pub async fn set_active_workspace(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<CurationWorkspaceInfo, String> {
    let root = state
        .curation_root
        .read()
        .clone()
        .ok_or("No curation root set")?;

    let ws_path = root.join(&name);
    if !ws_path.is_dir() || !is_workspace_dir(&ws_path) {
        return Err(format!("Not a valid workspace: {}", name));
    }

    *state.curation_workspace.write() = Some(ws_path.clone());

    if let Ok(data_dir) = app.path().app_data_dir() {
        let _ = crate::recent::touch(&data_dir, &ws_path.to_string_lossy());
    }

    get_curation_workspace(state).await
}

#[tauri::command]
pub async fn set_curation_workspace(
    state: State<'_, Arc<AppState>>,
    workspace: String,
) -> Result<(), String> {
    let path = PathBuf::from(&workspace);
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", workspace));
    }
    *state.curation_workspace.write() = Some(path);
    Ok(())
}

/// Run a curation pipeline stage.
/// `qc` and `intervals` run natively in Rust using the egorec crate.
/// Other stages (`stage`, `label`, `cluster`, etc.) fall back to Python subprocess.
#[tauri::command]
pub async fn run_curation_job(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    stage: String,
    source_prefix: Option<String>,
    publish_prefix: Option<String>,
) -> Result<String, String> {
    let workspace = state
        .curation_workspace
        .read()
        .clone()
        .ok_or("No curation workspace set")?;

    match stage.as_str() {
        "qc" => {
            let ws = workspace.clone();
            let app_handle = app.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::pipeline::run_qc_stage(&ws, &|progress| {
                    let _ = app_handle.emit("pipeline:progress", &progress);
                })
            })
            .await
            .map_err(|e| format!("QC task panicked: {}", e))??;

            Ok(result.message)
        }
        "intervals" => {
            let ws = workspace.clone();
            let app_handle = app.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::pipeline::run_intervals_stage(&ws, &|progress| {
                    let _ = app_handle.emit("pipeline:progress", &progress);
                })
            })
            .await
            .map_err(|e| format!("Intervals task panicked: {}", e))??;

            Ok(result.message)
        }
        _ => {
            run_python_stage(&state, &workspace, &stage, source_prefix, publish_prefix).await
        }
    }
}

async fn run_python_stage(
    state: &State<'_, Arc<AppState>>,
    workspace: &Path,
    stage: &str,
    source_prefix: Option<String>,
    publish_prefix: Option<String>,
) -> Result<String, String> {
    // Try to read pythonBinary from workspace.json first
    let ws_config_path = workspace.join("curation/v1/workspace.json");
    let ws_python = if ws_config_path.exists() {
        std::fs::read_to_string(&ws_config_path)
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| v.get("pythonBinary")?.as_str().map(String::from))
    } else {
        None
    };

    let default_python = state.python_binary.read().clone();
    let python = ws_python.unwrap_or(default_python);

    // Resolve relative python paths against the workspace directory
    let python_resolved = if Path::new(&python).is_relative() {
        workspace
            .join(&python)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(python)
    } else {
        python
    };

    let qc = state.qc_binary.read().clone();

    // Find the ego_curate module directory
    let ego_curate_dir = find_ego_curate_dir(workspace);

    let mut cmd = tokio::process::Command::new(&python_resolved);

    if let Some(ref curate_dir) = ego_curate_dir {
        cmd.current_dir(curate_dir);
    }

    let args = build_ego_curate_args(
        workspace,
        stage,
        &qc,
        source_prefix.as_deref(),
        publish_prefix.as_deref(),
    );
    for arg in &args {
        cmd.arg(arg);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn Python ({}): {}", python_resolved, e))?;

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("Curation job failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Curation job failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.to_string())
}

/// Build the argument list for a `python -m ego_curate` invocation.
/// This is extracted as a pure function so it can be unit-tested.
pub fn build_ego_curate_args(
    workspace: &Path,
    stage: &str,
    qc_binary: &str,
    source_prefix: Option<&str>,
    publish_prefix: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-m".into(),
        "ego_curate".into(),
        "--workspace".into(),
        workspace.to_string_lossy().to_string(),
        stage.into(),
    ];

    let stages_with_ego_qc = ["qc", "intervals", "proxies", "segments"];
    if stages_with_ego_qc.contains(&stage) {
        args.push("--ego-qc".into());
        args.push(qc_binary.into());
    }

    if let Some(prefix) = source_prefix {
        args.push("--source-prefix".into());
        args.push(prefix.into());
    }
    if let Some(prefix) = publish_prefix {
        args.push("--publish-prefix".into());
        args.push(prefix.into());
    }

    args
}

/// Walk up from the workspace to find the ego-recorder/python directory
/// containing ego_curate.py.
fn find_ego_curate_dir(workspace: &Path) -> Option<PathBuf> {
    let mut dir = workspace.to_path_buf();
    for _ in 0..5 {
        let candidate = dir.join("python/ego_curate.py");
        if candidate.exists() {
            return Some(dir.join("python"));
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn resolve_curation_file(workspace: &Path, data_type: &str) -> Result<PathBuf, String> {
    let v1 = workspace.join("curation/v1");
    match data_type {
        "episodes" => Ok(v1.join("episodes.jsonl")),
        "intervals" => Ok(v1.join("intervals.jsonl")),
        "labels" => Ok(v1.join("labels.jsonl")),
        "buckets" => Ok(v1.join("bucket_map.json")),
        "review_queue" => Ok(v1.join("review_queue.jsonl")),
        "overrides" => Ok(v1.join("review_overrides.json")),
        "segments" => Ok(v1.join("segments.jsonl")),
        "workspace" => Ok(v1.join("workspace.json")),
        other => Err(format!("Unknown curation data type: {}", other)),
    }
}

/// Read curation data files from the active workspace.
/// JSONL files are parsed line-by-line and returned as a JSON array.
#[tauri::command]
pub async fn read_curation_data(
    state: State<'_, Arc<AppState>>,
    data_type: String,
) -> Result<serde_json::Value, String> {
    let workspace = state
        .curation_workspace
        .read()
        .clone()
        .ok_or("No curation workspace set")?;

    let file_path = resolve_curation_file(&workspace, &data_type)?;

    if !file_path.exists() {
        return Ok(serde_json::Value::Null);
    }

    let contents = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;

    let is_jsonl = file_path
        .extension()
        .and_then(|e| e.to_str())
        == Some("jsonl");

    if is_jsonl {
        let items: Vec<serde_json::Value> = contents
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|e| format!("JSONL parse error: {}", e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(serde_json::Value::Array(items))
    } else {
        serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse {}: {}", file_path.display(), e))
    }
}

/// Write a review override to the overrides file.
#[tauri::command]
pub async fn write_curation_override(
    state: State<'_, Arc<AppState>>,
    override_type: String,
    id: String,
    data: serde_json::Value,
) -> Result<(), String> {
    let workspace = state
        .curation_workspace
        .read()
        .clone()
        .ok_or("No curation workspace set")?;

    let overrides_path = workspace.join("curation/v1/review_overrides.json");

    let mut overrides: serde_json::Value = if overrides_path.exists() {
        let contents = tokio::fs::read_to_string(&overrides_path)
            .await
            .map_err(|e| format!("Failed to read overrides: {}", e))?;
        serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse overrides: {}", e))?
    } else {
        std::fs::create_dir_all(overrides_path.parent().unwrap())
            .map_err(|e| format!("Failed to create curation dir: {}", e))?;
        serde_json::json!({
            "version": "1",
            "updated_at": null,
            "episodes": {},
            "intervals": {},
            "labels": {},
            "buckets": {"renames": {}, "interval_assignments": {}}
        })
    };

    overrides[&override_type][&id] = data;
    overrides["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());

    let serialized = serde_json::to_string_pretty(&overrides)
        .map_err(|e| format!("Failed to serialize overrides: {}", e))?;

    tokio::fs::write(&overrides_path, serialized)
        .await
        .map_err(|e| format!("Failed to write overrides: {}", e))?;

    Ok(())
}

// ── Recent workspaces ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_recent_workspaces(app: AppHandle) -> Result<Vec<RecentWorkspace>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("No app data dir: {}", e))?;
    Ok(crate::recent::list(&data_dir))
}

#[tauri::command]
pub async fn remove_recent_workspace(app: AppHandle, path: String) -> Result<(), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("No app data dir: {}", e))?;
    crate::recent::remove(&data_dir, &path)
}

#[tauri::command]
pub async fn update_recent_workspace_alias(
    app: AppHandle,
    path: String,
    alias: Option<String>,
) -> Result<(), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("No app data dir: {}", e))?;
    crate::recent::set_alias(&data_dir, &path, alias)
}
