use crate::h264_annex_b;
use crate::mp4_mux::{Mp4Sample, Mp4TrackConfig};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, Query, State as AxumState};
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use std::net::TcpListener;
use std::sync::Arc;

pub async fn spawn_video_server(state: Arc<AppState>) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind video server: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local addr: {}", e))?
        .port();

    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Failed to set nonblocking: {}", e))?;

    let tokio_listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| format!("Failed to convert listener: {}", e))?;

    let app = Router::new()
        .route("/stream/{name}", get(handle_stream))
        .route("/curation-stream", get(handle_curation_stream))
        .with_state(state);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(tokio_listener, app).await {
            log::error!("Video server error: {}", e);
        }
    });

    log::info!("Video stream server running on http://localhost:{}", port);
    Ok(port)
}

async fn handle_stream(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let decoded_name = urlencoding::decode(&name)
        .unwrap_or_else(|_| name.clone().into())
        .to_string();

    let file_path = {
        let index = state.file_index.read();
        match index.get(&decoded_name) {
            Some(entry) => {
                if entry.metadata.rgb_codec != 2 {
                    return error_response(StatusCode::BAD_REQUEST, "Not H.264 — not streamable");
                }
                entry.path.to_string_lossy().to_string()
            }
            None => {
                return error_response(StatusCode::NOT_FOUND, "File not found");
            }
        }
    };

    serve_cached_mp4(&state, &file_path, &headers).await
}

#[derive(serde::Deserialize)]
struct CurationStreamParams {
    path: String,
}

async fn handle_curation_stream(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<CurationStreamParams>,
) -> impl IntoResponse {
    let file_path = params.path;

    if !std::path::Path::new(&file_path).exists() {
        return error_response(StatusCode::NOT_FOUND, &format!("File not found: {}", file_path));
    }

    serve_cached_mp4(&state, &file_path, &headers).await
}

fn error_response(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Body::from(msg.to_string()))
        .unwrap()
}

// ── Cached MP4 + Range serving ───────────────────────────────────────────────

async fn serve_cached_mp4(
    state: &AppState,
    file_path: &str,
    headers: &HeaderMap,
) -> Response<Body> {
    let mp4 = {
        let cache = state.mp4_cache.read();
        cache.get(file_path).cloned()
    };

    let mp4 = match mp4 {
        Some(cached) => cached,
        None => {
            let path_owned = file_path.to_string();
            let result = tokio::task::spawn_blocking(move || egorec_to_mp4(&path_owned)).await;

            let mp4_bytes = match result {
                Ok(Ok(bytes)) => Arc::new(bytes),
                Ok(Err(e)) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("MP4 build failed: {}", e),
                    );
                }
                Err(e) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("Task join error: {}", e),
                    );
                }
            };

            // Evict old entries if cache is too large (>512 MB)
            {
                let mut cache = state.mp4_cache.write();
                let total: usize = cache.values().map(|v| v.len()).sum();
                if total > 512 * 1024 * 1024 {
                    cache.clear();
                }
                cache.insert(file_path.to_string(), Arc::clone(&mp4_bytes));
            }

            mp4_bytes
        }
    };

    serve_range_response(&mp4, headers)
}

fn serve_range_response(data: &[u8], headers: &HeaderMap) -> Response<Body> {
    let total = data.len() as u64;

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range_header(s, total));

    match range {
        Some((start, end)) => {
            let content_length = end - start + 1;
            let body = data[start as usize..=end as usize].to_vec();
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, "video/mp4")
                .header(header::CONTENT_LENGTH, content_length)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, total),
                )
                .body(Body::from(body))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .header(header::CONTENT_LENGTH, total)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(data.to_vec()))
            .unwrap(),
    }
}

/// Parse "bytes=START-END" or "bytes=START-" from the Range header.
fn parse_range_header(header_value: &str, total: u64) -> Option<(u64, u64)> {
    let range_spec = header_value.strip_prefix("bytes=")?;
    let mut parts = range_spec.splitn(2, '-');
    let start_str = parts.next()?;
    let end_str = parts.next()?;

    let start: u64 = start_str.parse().ok()?;
    let end: u64 = if end_str.is_empty() {
        total.saturating_sub(1)
    } else {
        end_str.parse().ok()?
    };

    if start > end || start >= total {
        return None;
    }
    let end = end.min(total - 1);
    Some((start, end))
}

// ── .egorec → MP4 conversion ─────────────────────────────────────────────────

fn egorec_to_mp4(file_path: &str) -> Result<Vec<u8>, String> {
    use egorec::format::*;
    use std::io::{BufReader, Read, Seek, SeekFrom};

    let mut file = BufReader::new(
        std::fs::File::open(file_path).map_err(|e| format!("open: {}", e))?,
    );

    let header = FileHeader::read_from(&mut file)
        .map_err(|e| format!("read header: {}", e))?;

    if header.rgb_codec != 2 {
        return Err("Not H.264 — not streamable".into());
    }

    let file_size = file
        .get_ref()
        .metadata()
        .map_err(|e| format!("metadata: {}", e))?
        .len();

    if file_size < (FILE_HEADER_SIZE as u64 + FileFooter::SIZE as u64) {
        return Err("File too small".into());
    }

    file.seek(SeekFrom::End(-(FileFooter::SIZE as i64)))
        .map_err(|e| format!("seek footer: {}", e))?;
    let footer = FileFooter::read_from(&mut file)
        .map_err(|e| format!("read footer: {}", e))?;

    let total_frames = footer.total_frames;
    let index_offset = footer.index_offset;

    if total_frames == 0 {
        return Err("No frames in file".into());
    }

    // Read the first index entry to get the starting frame offset
    file.seek(SeekFrom::Start(index_offset))
        .map_err(|e| format!("seek index: {}", e))?;
    let first_entry = IndexEntry::read_from(&mut file)
        .map_err(|e| format!("read index: {}", e))?;

    let mut offset = first_entry.file_offset;
    let mut samples = Vec::with_capacity(total_frames as usize);
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;

    for _ in 0..total_frames {
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("seek frame: {}", e))?;

        let block = FrameBlockHeader::read_from(&mut file)
            .map_err(|e| format!("read block header: {}", e))?;

        if block.rgb_compressed_size > 0 {
            let mut h264_data = vec![0u8; block.rgb_compressed_size as usize];
            file.read_exact(&mut h264_data)
                .map_err(|e| format!("read rgb: {}", e))?;

            let nals = h264_annex_b::parse_annex_b(&h264_data);

            if sps.is_none() {
                if let Some((s, p)) = h264_annex_b::extract_sps_pps(&nals) {
                    sps = Some(s);
                    pps = Some(p);
                }
            }

            let is_key = h264_annex_b::is_keyframe(&nals);
            let avcc_data = h264_annex_b::nals_to_avcc(&nals);
            samples.push(Mp4Sample {
                data: avcc_data,
                is_keyframe: is_key,
            });
        } else {
            samples.push(Mp4Sample {
                data: Vec::new(),
                is_keyframe: false,
            });
        }

        offset += block.block_size as u64;
    }

    // Handle trailing H.264 data between last frame and index
    if offset < index_offset {
        let trailing_size = (index_offset - offset) as usize;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("seek trailing: {}", e))?;
        let mut trailing = vec![0u8; trailing_size];
        file.read_exact(&mut trailing)
            .map_err(|e| format!("read trailing: {}", e))?;

        let nals = h264_annex_b::parse_annex_b(&trailing);
        if !nals.is_empty() {
            let avcc_data = h264_annex_b::nals_to_avcc(&nals);
            let is_key = h264_annex_b::is_keyframe(&nals);
            samples.push(Mp4Sample {
                data: avcc_data,
                is_keyframe: is_key,
            });
        }
    }

    let sps = sps.ok_or("No SPS found in H.264 stream")?;
    let pps = pps.ok_or("No PPS found in H.264 stream")?;

    let duration_s = footer.total_duration_us as f64 / 1_000_000.0;
    let fps = if duration_s > 0.0 && total_frames > 0 {
        total_frames as f64 / duration_s
    } else {
        30.0
    };

    // Use timescale = fps * 1000 (rounded), sample_delta = 1000
    // This gives sub-millisecond precision while keeping integer math clean.
    let timescale = (fps * 1000.0).round() as u32;
    let sample_delta = 1000u32;

    let config = Mp4TrackConfig {
        width: header.color_width,
        height: header.color_height,
        timescale,
        sample_delta,
        sps,
        pps,
    };

    Ok(crate::mp4_mux::build_mp4(&config, &samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_keyframe_h264() -> Vec<u8> {
        let mut data = Vec::new();
        // SPS (NAL type 7)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x67, 0x42, 0xC0, 0x1E, 0xD9, 0x00, 0xA0, 0x47, 0xFE, 0x88]);
        // PPS (NAL type 8)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]);
        // IDR slice (NAL type 5)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x65, 0x88, 0x80, 0x40, 0x00, 0xFF, 0xAA, 0x55]);
        data
    }

    fn make_pframe_h264() -> Vec<u8> {
        let mut data = Vec::new();
        // Non-IDR slice (NAL type 1)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x41, 0x9A, 0x24, 0x6C, 0x41, 0xFF]);
        data
    }

    fn create_test_egorec(path: &std::path::Path, num_frames: u32, fps: u32) {
        use egorec::format::*;
        use egorec::writer::EgorecWriter;

        let header = FileHeader {
            magic: FILE_MAGIC,
            header_size: FILE_HEADER_SIZE as u32,
            flags: 0,
            serial_number: [0u8; 32],
            depth_scale: 0.001,
            depth_width: 0,
            depth_height: 0,
            depth_fx: 0.0,
            depth_fy: 0.0,
            depth_ppx: 0.0,
            depth_ppy: 0.0,
            depth_distortion_model: 0,
            depth_distortion_coeffs: [0.0; 5],
            color_width: 640,
            color_height: 480,
            color_fx: 600.0,
            color_fy: 600.0,
            color_ppx: 320.0,
            color_ppy: 240.0,
            color_distortion_model: 0,
            color_distortion_coeffs: [0.0; 5],
            extrinsic_rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            extrinsic_translation: [0.0; 3],
            session_name: [0u8; 128],
            start_timestamp_us: 0,
            usb_type: [0u8; 8],
            rgb_codec: 2,
            depth_codec: 0,
            rgb_quality: 23,
            zstd_level: 3,
            reserved: [0u8; 128],
        };

        let mut writer = EgorecWriter::create(path, &header).unwrap();
        let frame_duration_us = 1_000_000 / fps as u64;

        for i in 0..num_frames {
            let is_keyframe = i % fps == 0;
            let h264_data = if is_keyframe {
                make_keyframe_h264()
            } else {
                make_pframe_h264()
            };
            writer
                .write_frame(
                    i as u64 * frame_duration_us,
                    i as u64,
                    is_keyframe,
                    &h264_data,
                    &[], // no depth
                )
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn egorec_to_mp4_produces_valid_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("test.egorec");
        create_test_egorec(&egorec_path, 90, 30); // 3 seconds at 30fps

        let mp4 = egorec_to_mp4(egorec_path.to_str().unwrap()).unwrap();

        // Verify it starts with ftyp
        assert_eq!(&mp4[4..8], b"ftyp");

        // Walk top-level boxes
        let mut offset = 0;
        let mut found_ftyp = false;
        let mut found_moov = false;
        let mut found_mdat = false;
        while offset < mp4.len() {
            let size = u32::from_be_bytes([
                mp4[offset], mp4[offset + 1], mp4[offset + 2], mp4[offset + 3],
            ]) as usize;
            let box_type = &mp4[offset + 4..offset + 8];
            match box_type {
                b"ftyp" => found_ftyp = true,
                b"moov" => found_moov = true,
                b"mdat" => found_mdat = true,
                _ => panic!("Unexpected box: {:?}", std::str::from_utf8(box_type)),
            }
            assert!(size >= 8);
            offset += size;
        }
        assert_eq!(offset, mp4.len(), "Box sizes don't sum to file size");
        assert!(found_ftyp, "Missing ftyp");
        assert!(found_moov, "Missing moov");
        assert!(found_mdat, "Missing mdat");
    }

    #[test]
    fn egorec_to_mp4_keyframes_match() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("test_kf.egorec");
        create_test_egorec(&egorec_path, 60, 30); // 2 seconds, keyframes at 0 and 30

        let mp4 = egorec_to_mp4(egorec_path.to_str().unwrap()).unwrap();

        // Find stss box inside moov to verify keyframe count
        let stss_pos = find_box(&mp4, b"stss").expect("stss box not found");
        let entry_count_offset = stss_pos + 8 + 4; // box header(8) + version/flags(4)
        let count = u32::from_be_bytes([
            mp4[entry_count_offset],
            mp4[entry_count_offset + 1],
            mp4[entry_count_offset + 2],
            mp4[entry_count_offset + 3],
        ]);
        assert_eq!(count, 2, "Expected 2 keyframes (at frame 0 and 30)");

        // First keyframe should be sample 1 (1-indexed)
        let first_kf = u32::from_be_bytes([
            mp4[entry_count_offset + 4],
            mp4[entry_count_offset + 5],
            mp4[entry_count_offset + 6],
            mp4[entry_count_offset + 7],
        ]);
        assert_eq!(first_kf, 1);

        // Second keyframe should be sample 31 (1-indexed)
        let second_kf = u32::from_be_bytes([
            mp4[entry_count_offset + 8],
            mp4[entry_count_offset + 9],
            mp4[entry_count_offset + 10],
            mp4[entry_count_offset + 11],
        ]);
        assert_eq!(second_kf, 31);
    }

    #[test]
    fn range_header_parsing() {
        assert_eq!(parse_range_header("bytes=0-499", 1000), Some((0, 499)));
        assert_eq!(parse_range_header("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range_header("bytes=0-0", 1000), Some((0, 0)));
        assert_eq!(parse_range_header("bytes=999-999", 1000), Some((999, 999)));
        assert_eq!(parse_range_header("bytes=1000-", 1000), None); // past end
        assert_eq!(parse_range_header("bytes=500-200", 1000), None); // start > end
        assert_eq!(parse_range_header("notbytes=0-10", 1000), None);
    }

    #[test]
    fn egorec_to_mp4_ffprobe_validates() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("test_ffprobe.egorec");
        create_test_egorec(&egorec_path, 90, 30);

        let mp4 = egorec_to_mp4(egorec_path.to_str().unwrap()).unwrap();
        let mp4_path = dir.path().join("test_output.mp4");
        std::fs::write(&mp4_path, &mp4).unwrap();

        let output = std::process::Command::new("ffprobe")
            .args([
                "-v", "error",
                "-show_entries", "stream=codec_name,width,height,nb_frames",
                "-of", "json",
                mp4_path.to_str().unwrap(),
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let json: serde_json::Value =
                    serde_json::from_slice(&out.stdout).unwrap();
                let stream = &json["streams"][0];
                assert_eq!(stream["codec_name"], "h264");
                // Width/height come from SPS, not from avc1 box. Our synthetic
                // SPS encodes a different resolution than the avc1 container
                // claims, but the important thing is ffprobe parses it as valid H.264.
                assert!(stream["width"].as_u64().unwrap() > 0);
                assert!(stream["height"].as_u64().unwrap() > 0);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                panic!("ffprobe returned error: {}", stderr);
            }
            Err(_) => {
                eprintln!("ffprobe not available, skipping validation");
            }
        }
    }

    /// Recursively find a box by type in MP4 data.
    fn find_box(data: &[u8], target: &[u8; 4]) -> Option<usize> {
        find_box_recursive(data, target, 0, data.len())
    }

    fn find_box_recursive(data: &[u8], target: &[u8; 4], start: usize, end: usize) -> Option<usize> {
        let mut offset = start;
        while offset + 8 <= end {
            let size = u32::from_be_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
            ]) as usize;
            if size < 8 || offset + size > end {
                break;
            }
            let box_type = &data[offset + 4..offset + 8];
            if box_type == target {
                return Some(offset);
            }
            // Container boxes — recurse into their contents
            let containers: &[&[u8]] = &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"dinf"];
            if containers.iter().any(|c| box_type == *c) {
                if let Some(found) = find_box_recursive(data, target, offset + 8, offset + size) {
                    return Some(found);
                }
            }
            offset += size;
        }
        None
    }
}
