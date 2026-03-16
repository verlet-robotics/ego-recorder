use crate::config::UploadConfig;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, SharedCredentialsProvider};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_smithy_types::byte_stream::ByteStream;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use tauri::Emitter;

/// Minimum file size for multipart upload (10 MB).
const MULTIPART_THRESHOLD: u64 = 10 * 1024 * 1024;

/// Progress event emitted to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgressEvent {
    pub filename: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub phase: String,
}

/// Build an S3 client from our UploadConfig.
pub fn build_s3_client(config: &UploadConfig) -> Result<aws_sdk_s3::Client, String> {
    let endpoint = config
        .endpoint
        .as_deref()
        .ok_or("Upload endpoint not configured")?;
    let access_key = config
        .access_key
        .as_deref()
        .ok_or("Access key not configured")?;
    let secret_key = config
        .secret_key
        .as_deref()
        .ok_or("Secret key not configured")?;
    let region_str = config.region.as_deref().unwrap_or("auto");

    let credentials = Credentials::new(access_key, secret_key, None, None, "ego-recorder");

    let s3_config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region_str.to_string()))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .build();

    Ok(aws_sdk_s3::Client::from_conf(s3_config))
}

/// Construct S3 object key from prefix and relative path.
pub fn make_object_key(prefix: Option<&str>, rel_path: &str) -> String {
    match prefix {
        Some(p) if !p.is_empty() => {
            let p = p.trim_end_matches('/');
            format!("{}/{}", p, rel_path)
        }
        _ => rel_path.to_string(),
    }
}

/// Compute SHA-256 of a file using 1MB reads in a blocking context.
/// Emits hashing progress via the provided callback.
pub async fn compute_sha256<F>(path: &Path, total_size: u64, mut on_progress: F) -> Result<String, String>
where
    F: FnMut(u64) + Send + 'static,
{
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file =
            std::fs::File::open(&path).map_err(|e| format!("Failed to open file for hashing: {}", e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024]; // 1MB chunks
        let mut bytes_read: u64 = 0;

        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            bytes_read += n as u64;
            on_progress(bytes_read);
        }

        let _ = total_size; // used by caller for progress fraction
        let hash = hasher.finalize();
        Ok(hex::encode(hash))
    })
    .await
    .map_err(|e| format!("Hash task panicked: {}", e))?
}

/// Upload a file to S3. Uses single PutObject for small files, multipart for large.
pub async fn upload_file(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    path: &Path,
    chunk_mb: u32,
    app_handle: &tauri::AppHandle,
    filename: &str,
) -> Result<(), String> {
    let file_size = std::fs::metadata(path)
        .map_err(|e| format!("Failed to get file metadata: {}", e))?
        .len();

    if file_size < MULTIPART_THRESHOLD {
        upload_single(client, bucket, key, path).await
    } else {
        upload_multipart(client, bucket, key, path, file_size, chunk_mb, app_handle, filename).await
    }
}

/// Single PutObject upload for small files.
async fn upload_single(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    path: &Path,
) -> Result<(), String> {
    let body = ByteStream::from_path(path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_type("application/octet-stream")
        .body(body)
        .send()
        .await
        .map_err(|e| classify_s3_error(&e.to_string()))?;

    Ok(())
}

/// Multipart upload for large files with progress events.
async fn upload_multipart(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    path: &Path,
    file_size: u64,
    chunk_mb: u32,
    app_handle: &tauri::AppHandle,
    filename: &str,
) -> Result<(), String> {
    let chunk_size = (chunk_mb as u64) * 1024 * 1024;

    // Create multipart upload
    let create_resp = client
        .create_multipart_upload()
        .bucket(bucket)
        .key(key)
        .content_type("application/octet-stream")
        .send()
        .await
        .map_err(|e| classify_s3_error(&e.to_string()))?;

    let upload_id = create_resp
        .upload_id()
        .ok_or("No upload_id returned")?
        .to_string();

    let mut parts: Vec<CompletedPart> = Vec::new();
    let mut offset: u64 = 0;
    let mut part_number: i32 = 1;
    let upload_start = std::time::Instant::now();

    while offset < file_size {
        let length = std::cmp::min(chunk_size, file_size - offset);

        let body = ByteStream::read_from()
            .path(path)
            .offset(offset)
            .length(aws_smithy_types::byte_stream::Length::Exact(length))
            .build()
            .await
            .map_err(|e| {
                let msg = format!("Failed to read file chunk: {}", e);
                // Fire-and-forget abort
                let client = client.clone();
                let bucket = bucket.to_string();
                let key = key.to_string();
                let uid = upload_id.clone();
                tokio::spawn(async move {
                    let _ = client
                        .abort_multipart_upload()
                        .bucket(&bucket)
                        .key(&key)
                        .upload_id(&uid)
                        .send()
                        .await;
                });
                msg
            })?;

        let resp = match client
            .upload_part()
            .bucket(bucket)
            .key(key)
            .upload_id(&upload_id)
            .part_number(part_number)
            .body(body)
            .content_length(length as i64)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Abort on failure
                let _ = client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                return Err(classify_s3_error(&e.to_string()));
            }
        };

        let etag = resp.e_tag().unwrap_or_default().to_string();
        parts.push(
            CompletedPart::builder()
                .part_number(part_number)
                .e_tag(&etag)
                .build(),
        );

        offset += length;
        part_number += 1;

        // Emit progress
        let elapsed = upload_start.elapsed().as_secs_f64();
        let speed_bps = if elapsed > 0.0 {
            (offset as f64 / elapsed) as u64
        } else {
            0
        };

        let progress = UploadProgressEvent {
            filename: filename.to_string(),
            bytes_transferred: offset,
            total_bytes: file_size,
            speed_bps,
            phase: "uploading".to_string(),
        };
        let _ = app_handle.emit("upload:progress", &progress);
    }

    // Complete multipart
    let completed = CompletedMultipartUpload::builder()
        .set_parts(Some(parts))
        .build();

    client
        .complete_multipart_upload()
        .bucket(bucket)
        .key(key)
        .upload_id(&upload_id)
        .multipart_upload(completed)
        .send()
        .await
        .map_err(|e| {
            let client = client.clone();
            let bucket = bucket.to_string();
            let key = key.to_string();
            let uid = upload_id.clone();
            tokio::spawn(async move {
                let _ = client
                    .abort_multipart_upload()
                    .bucket(&bucket)
                    .key(&key)
                    .upload_id(&uid)
                    .send()
                    .await;
            });
            classify_s3_error(&e.to_string())
        })?;

    Ok(())
}

/// Test connectivity by checking if the bucket exists.
pub async fn test_connection(config: &UploadConfig) -> Result<String, String> {
    let client = build_s3_client(config)?;
    let bucket = config
        .bucket
        .as_deref()
        .ok_or("Bucket not configured")?;

    client
        .head_bucket()
        .bucket(bucket)
        .send()
        .await
        .map_err(|e| classify_s3_error(&e.to_string()))?;

    Ok(format!("Connected to bucket '{}'", bucket))
}

/// Classify S3 errors into user-friendly messages.
fn classify_s3_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("403") || lower.contains("access denied") || lower.contains("invalid access key") || lower.contains("signature") {
        format!("Authentication failed — check your access key and secret key. ({})", raw)
    } else if lower.contains("nosuchbucket") || lower.contains("not found") || lower.contains("404") {
        format!("Bucket not found — check the bucket name. ({})", raw)
    } else if lower.contains("connection refused") || lower.contains("dns") || lower.contains("dispatch failure") || lower.contains("connect") {
        format!("Connection failed — check the endpoint URL and network. ({})", raw)
    } else if lower.contains("timeout") {
        format!("Connection timed out — check network connectivity. ({})", raw)
    } else {
        format!("Upload error: {}", raw)
    }
}
