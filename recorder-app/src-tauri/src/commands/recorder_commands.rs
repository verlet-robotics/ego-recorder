use crate::recorder::inhibitor;
use crate::recorder::status::RecorderState;
use crate::state::AppState;
use std::sync::Arc;
use tauri::State;

// NOTE: start_recording and stop_recording have moved to preview_commands.rs.
// They now send JSON commands to the preview subprocess stdin instead of
// spawning/killing a separate process.

#[tauri::command]
pub fn get_recorder_status(state: State<'_, Arc<AppState>>) -> RecorderState {
    state.recorder_status.read().state
}

#[tauri::command]
pub fn get_recorder_stats(
    state: State<'_, Arc<AppState>>,
) -> crate::recorder::status::RecorderStatus {
    state.recorder_status.read().clone()
}

#[tauri::command]
pub async fn toggle_lid_safe(
    enable: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<bool, String> {
    if enable {
        // Release any existing inhibitor before acquiring a new one
        // to prevent fd leaks on rapid toggle calls
        if let Some(old_fd) = state.inhibitor_fd.write().take() {
            inhibitor::release_inhibitor(old_fd);
        }
        let fd = inhibitor::acquire_inhibitor().await?;
        *state.inhibitor_fd.write() = Some(fd);
        Ok(true)
    } else {
        if let Some(fd) = state.inhibitor_fd.write().take() {
            inhibitor::release_inhibitor(fd);
        }
        Ok(false)
    }
}
