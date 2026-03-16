use crate::state::AppState;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Emitter;

/// Scan sysfs for connected Intel RealSense USB devices.
/// Returns a list of (serial, product_name) tuples.
pub fn detect_realsense_cameras() -> Vec<(String, String)> {
    let usb_devices = match std::fs::read_dir("/sys/bus/usb/devices") {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut cameras = Vec::new();

    for entry in usb_devices.flatten() {
        let path = entry.path();

        // Check idVendor == 8086 (Intel)
        let vendor_path = path.join("idVendor");
        let vendor = match std::fs::read_to_string(&vendor_path) {
            Ok(v) => v.trim().to_lowercase(),
            Err(_) => continue,
        };
        if vendor != "8086" {
            continue;
        }

        // Check product name contains "RealSense"
        let product_path = path.join("product");
        let product = match std::fs::read_to_string(&product_path) {
            Ok(p) => p.trim().to_string(),
            Err(_) => continue,
        };
        if !product.contains("RealSense") {
            continue;
        }

        // Read serial number if available
        let serial_path = path.join("serial");
        let serial = std::fs::read_to_string(&serial_path)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        cameras.push((serial, product));
    }

    cameras
}

/// Spawn a background task that polls for RealSense camera presence every 2 seconds.
/// Emits `camera:connected` events on transitions.
pub fn spawn_camera_watcher(app_handle: tauri::AppHandle, state: Arc<AppState>) {
    // Initial scan
    let initially_connected = !detect_realsense_cameras().is_empty();
    state
        .camera_connected
        .store(initially_connected, Ordering::SeqCst);
    let _ = app_handle.emit("camera:connected", initially_connected);
    log::info!(
        "Camera watcher: initial scan — {}",
        if initially_connected {
            "camera connected"
        } else {
            "no camera"
        }
    );

    tauri::async_runtime::spawn(async move {
        let mut was_connected = initially_connected;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let is_connected = !detect_realsense_cameras().is_empty();

            if is_connected != was_connected {
                log::info!(
                    "Camera availability changed: {}",
                    if is_connected {
                        "connected"
                    } else {
                        "disconnected"
                    }
                );
                state
                    .camera_connected
                    .store(is_connected, Ordering::SeqCst);
                let _ = app_handle.emit("camera:connected", is_connected);
                was_connected = is_connected;
            }
        }
    });
}
