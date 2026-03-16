use crate::config::{self, AppConfig};
use crate::disk;
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn get_config(state: State<'_, Arc<AppState>>) -> AppConfig {
    state.config.read().clone()
}

#[tauri::command]
pub fn save_config(config: AppConfig, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    config::save_config(&config)?;
    *state.config.write() = config;
    Ok(())
}

#[tauri::command]
pub fn is_first_run(state: State<'_, Arc<AppState>>) -> bool {
    *state.first_run.read()
}

#[tauri::command]
pub fn complete_first_run(state: State<'_, Arc<AppState>>) {
    *state.first_run.write() = false;
}

#[tauri::command]
pub fn locate_binary() -> Option<String> {
    config::locate_binary()
}

#[tauri::command]
pub async fn test_camera(binary_path: String) -> Result<String, String> {
    // Run ego-recorder --help to test binary is executable
    let output = tokio::process::Command::new(&binary_path)
        .args(["--help"])
        .output()
        .await
        .map_err(|e| format!("Failed to run binary: {}", e))?;

    if output.status.success() {
        Ok("Binary found and executable".into())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Binary test failed: {}", stderr))
    }
}

#[tauri::command]
pub fn get_disk_info(path: String) -> Result<disk::DiskInfo, String> {
    disk::get_disk_info(&path)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub arch: String,
    pub recommended_preset: String,
}

#[tauri::command]
pub fn get_system_info() -> SystemInfo {
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let arch = std::env::consts::ARCH.to_string();

    // Recommend preset based on CPU capabilities.
    // x264 zerolatency at 1280x720 @ 30fps typical encode times:
    //   ultrafast: ~2-4ms   (any CPU)
    //   superfast: ~4-8ms   (4+ cores)
    //   veryfast:  ~7-15ms  (8+ cores)
    //   fast:      ~12-25ms (16+ cores, tight on budget)
    // Budget is 33ms per frame, but need headroom for Zdepth + I/O.
    let recommended_preset = if arch.contains("arm") || arch.contains("aarch64") {
        // ARM: limited single-core perf, stay safe
        "ultrafast"
    } else if cpu_cores <= 2 {
        "ultrafast"
    } else if cpu_cores <= 4 {
        "superfast"
    } else if cpu_cores <= 8 {
        "veryfast"
    } else if cpu_cores <= 16 {
        // 8-16 cores: veryfast is comfortable, ~2x smaller files than ultrafast
        "veryfast"
    } else {
        // 16+ cores (e.g. i9, Xeon, Ultra 9): fast is viable, ~3x smaller files
        "fast"
    }
    .to_string();

    SystemInfo {
        cpu_model,
        cpu_cores,
        arch,
        recommended_preset,
    }
}
