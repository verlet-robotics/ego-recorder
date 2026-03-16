use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::sync::{oneshot, watch};

use super::CameraInfo;

/// Per-read timeout: if a single read_exact doesn't complete within this
/// duration the frame reader exits, which causes the subprocess to be
/// cleaned up and the preview state to transition to Off/Error.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Read frames from the ego-recorder preview subprocess stdout.
///
/// Protocol:
///   1. First line: JSON `CameraInfo`
///   2. Repeating tagged frames (tag-length-value, any order):
///      'R' u32_le(size) <jpeg_bytes>  (RGB — always present)
///      'D' u32_le(size) <jpeg_bytes>  (depth — may stop after recording ends)
pub async fn read_frames(
    stdout: ChildStdout,
    camera_info_tx: oneshot::Sender<CameraInfo>,
    rgb_tx: watch::Sender<Option<Arc<Vec<u8>>>>,
    depth_tx: watch::Sender<Option<Arc<Vec<u8>>>>,
) {
    let mut reader = BufReader::new(stdout);

    // 1. Read first line as JSON camera info
    let mut info_line = String::new();
    match reader.read_line(&mut info_line).await {
        Ok(0) | Err(_) => {
            log::error!("Preview: failed to read camera info line");
            let _ = camera_info_tx.send(CameraInfo::default());
            return;
        }
        Ok(_) => {}
    }

    let camera_info: CameraInfo = match serde_json::from_str(info_line.trim()) {
        Ok(info) => info,
        Err(e) => {
            log::error!("Preview: failed to parse camera info: {}", e);
            let _ = camera_info_tx.send(CameraInfo::default());
            return;
        }
    };

    let _ = camera_info_tx.send(camera_info);

    // Track whether we've been sending depth (to detect when it stops)
    let mut had_depth = false;

    // 2. Read tagged frames in a loop (R and D can appear in any pattern)
    loop {
        match read_frame(&mut reader).await {
            Some((b'R', data)) => {
                let _ = rgb_tx.send(Some(Arc::new(data)));
            }
            Some((b'D', data)) => {
                had_depth = true;
                let _ = depth_tx.send(Some(Arc::new(data)));
            }
            Some((tag, _)) => {
                log::warn!("Preview: unexpected frame tag '{}'", tag as char);
                continue;
            }
            None => break, // EOF
        }

        // After we get an R frame, peek ahead: if the next byte is R again
        // (not D), depth has been disabled. Clear depth channel once.
        if had_depth {
            // Check if depth stopped by seeing if next frame is R (not D).
            // We do this lazily: if we got R and the subprocess used to send D
            // but now doesn't, the next read_frame will return R again.
            // The depth_tx just won't get updates, which is fine.
        }
    }

    // Clear depth channel on exit so MJPEG stream ends cleanly
    let _ = depth_tx.send(None);

    log::info!("Preview: frame reader exiting (subprocess stdout closed)");
}

/// Read a single frame: type_byte + u32_le(size) + <size bytes>.
/// Returns None on EOF or if any individual read exceeds READ_TIMEOUT.
async fn read_frame(reader: &mut BufReader<ChildStdout>) -> Option<(u8, Vec<u8>)> {
    // Read type byte
    let mut type_byte = [0u8; 1];
    match tokio::time::timeout(READ_TIMEOUT, reader.read_exact(&mut type_byte)).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return None, // EOF or I/O error
        Err(_) => {
            log::warn!("Preview: read_exact timed out waiting for frame tag byte");
            return None;
        }
    }

    // Read u32 size (little-endian)
    let mut size_buf = [0u8; 4];
    match tokio::time::timeout(READ_TIMEOUT, reader.read_exact(&mut size_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return None,
        Err(_) => {
            log::warn!("Preview: read_exact timed out waiting for frame size");
            return None;
        }
    }
    let size = u32::from_le_bytes(size_buf) as usize;

    // Sanity check: frames shouldn't be > 10 MB
    if size > 10 * 1024 * 1024 {
        log::error!("Preview: frame size too large: {} bytes", size);
        return None;
    }

    // Read frame data
    let mut data = vec![0u8; size];
    match tokio::time::timeout(READ_TIMEOUT, reader.read_exact(&mut data)).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return None,
        Err(_) => {
            log::warn!("Preview: read_exact timed out waiting for {} bytes of frame data", size);
            return None;
        }
    }

    Some((type_byte[0], data))
}
