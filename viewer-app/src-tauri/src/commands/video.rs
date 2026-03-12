use crate::state::AppState;
use std::sync::Arc;
use tauri::State;

/// Get the video server port for constructing stream URLs on the frontend.
#[tauri::command]
pub async fn get_video_server_port(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<u16>, String> {
    Ok(*state.video_server_port.read())
}

/// Get the stream URL for a named file. Frontend uses this in <video src="...">.
#[tauri::command]
pub async fn get_stream_url(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<Option<String>, String> {
    let port = *state.video_server_port.read();
    let has_file = state.file_index.read().contains_key(&name);

    if !has_file {
        return Err(format!("File not found: {}", name));
    }

    match port {
        Some(p) => Ok(Some(format!(
            "http://localhost:{}/stream/{}",
            p,
            urlencoding::encode(&name)
        ))),
        None => Ok(None),
    }
}

/// Get a stream URL for a curation episode by resolving its local_path from
/// episodes.jsonl. The path can be absolute or relative to the workspace.
#[tauri::command]
pub async fn get_curation_stream_url(
    state: State<'_, Arc<AppState>>,
    source_key: String,
) -> Result<Option<String>, String> {
    let port = match *state.video_server_port.read() {
        Some(p) => p,
        None => return Ok(None),
    };

    let workspace = state
        .curation_workspace
        .read()
        .clone()
        .ok_or("No curation workspace set")?;

    let episodes_path = workspace.join("curation/v1/episodes.jsonl");
    if !episodes_path.exists() {
        return Err("No episodes.jsonl found".into());
    }

    let contents = std::fs::read_to_string(&episodes_path)
        .map_err(|e| format!("read episodes.jsonl: {}", e))?;

    let local_path = contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .find_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            if v.get("source_key")?.as_str()? == source_key {
                Some(v.get("local_path")?.as_str()?.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("Episode not found for source_key: {}", source_key))?;

    let resolved = if std::path::Path::new(&local_path).is_absolute() {
        local_path
    } else {
        workspace
            .join(&local_path)
            .to_string_lossy()
            .to_string()
    };

    if !std::path::Path::new(&resolved).exists() {
        return Err(format!("Episode file not found: {}", resolved));
    }

    Ok(Some(format!(
        "http://localhost:{}/curation-stream?path={}",
        port,
        urlencoding::encode(&resolved)
    )))
}
