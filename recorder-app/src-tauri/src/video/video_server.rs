use crate::video::h264_annex_b;
use crate::video::mp4_mux::{Mp4Sample, Mp4TrackConfig};
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
        .route("/file-stream", get(handle_file_stream))
        .route("/preview/{stream_type}", get(handle_preview_stream))
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
                    return error_response(StatusCode::BAD_REQUEST, "Not H.264 -- not streamable");
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
struct FileStreamParams {
    path: String,
}

async fn handle_file_stream(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<FileStreamParams>,
) -> impl IntoResponse {
    let file_path = params.path;
    if !std::path::Path::new(&file_path).exists() {
        return error_response(
            StatusCode::NOT_FOUND,
            &format!("File not found: {}", file_path),
        );
    }
    serve_cached_mp4(&state, &file_path, &headers).await
}

/// MJPEG multipart stream for live camera preview.
/// stream_type is "rgb" or "depth".
async fn handle_preview_stream(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(stream_type): Path<String>,
) -> impl IntoResponse {
    let rx = match stream_type.as_str() {
        "rgb" => state.rgb_frame_tx.subscribe(),
        "depth" => state.depth_frame_tx.subscribe(),
        _ => {
            return error_response(StatusCode::BAD_REQUEST, "stream_type must be 'rgb' or 'depth'");
        }
    };

    // MJPEG multipart boundary
    let boundary = "frame";

    let stream = async_stream::stream! {
        let mut rx = rx;
        loop {
            // Wait for the next frame change
            if rx.changed().await.is_err() {
                break; // Channel closed
            }

            let frame = rx.borrow_and_update().clone();
            if let Some(jpeg_data) = frame {
                // MJPEG multipart format
                let header = format!(
                    "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    jpeg_data.len()
                );
                yield Ok::<_, std::io::Error>(bytes::Bytes::from(header));
                yield Ok(bytes::Bytes::from(jpeg_data.as_ref().clone()));
                yield Ok(bytes::Bytes::from("\r\n"));
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/x-mixed-replace; boundary={}", boundary),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

fn error_response(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Body::from(msg.to_string()))
        .unwrap()
}

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

fn parse_range_header(header_value: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let range_spec = header_value.strip_prefix("bytes=")?;
    let mut parts = range_spec.splitn(2, '-');
    let start_str = parts.next()?;
    let end_str = parts.next()?;

    let start: u64 = start_str.parse().ok()?;
    let end: u64 = if end_str.is_empty() {
        total - 1
    } else {
        end_str.parse().ok()?
    };

    if start > end || start >= total {
        return None;
    }
    let end = end.min(total - 1);
    Some((start, end))
}

// -- .egorec to MP4 conversion --

fn egorec_to_mp4(file_path: &str) -> Result<Vec<u8>, String> {
    use egorec::format::*;
    use std::io::{BufReader, Read, Seek, SeekFrom};

    let mut file = BufReader::new(
        std::fs::File::open(file_path).map_err(|e| format!("open: {}", e))?,
    );

    let header =
        FileHeader::read_from(&mut file).map_err(|e| format!("read header: {}", e))?;

    if header.rgb_codec != 2 {
        return Err("Not H.264 -- not streamable".into());
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
    let footer =
        FileFooter::read_from(&mut file).map_err(|e| format!("read footer: {}", e))?;

    let total_frames = footer.total_frames;
    let index_offset = footer.index_offset;

    if total_frames == 0 {
        return Err("No frames in file".into());
    }

    file.seek(SeekFrom::Start(index_offset))
        .map_err(|e| format!("seek index: {}", e))?;
    let first_entry =
        IndexEntry::read_from(&mut file).map_err(|e| format!("read index: {}", e))?;

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

    Ok(crate::video::mp4_mux::build_mp4(&config, &samples))
}
