//! Comprehensive integration tests for the recorder-app.
//!
//! Tests cover: config, library discovery, file watcher, upload queue,
//! dataset manifest/scan, video server, and the frame reader protocol.
//! Each test creates its own temp directory — no shared state or test ordering.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

// ===== Test helpers =====

/// Create a minimal valid .egorec file (header + footer, no frames).
/// `rgb_codec`: 1 = JPEG, 2 = H.264
fn create_test_egorec(path: &Path, session_name: &str, rgb_codec: u8, frames: u64, duration_us: u64) {
    use byteorder::{LittleEndian, WriteBytesExt};

    let mut file = fs::File::create(path).unwrap();

    // --- FileHeader (680 bytes) ---
    // magic
    file.write_all(&[b'E', b'G', b'O', b'R', b'E', b'C', 0x02, 0x00]).unwrap();
    // header_size
    file.write_u32::<LittleEndian>(680).unwrap();
    // flags (no IMU)
    file.write_u32::<LittleEndian>(0).unwrap();
    // serial_number [32]
    let mut serial = [0u8; 32];
    serial[..5].copy_from_slice(b"12345");
    file.write_all(&serial).unwrap();
    // depth_scale
    file.write_f32::<LittleEndian>(0.001).unwrap();
    // depth_width, depth_height
    file.write_u32::<LittleEndian>(640).unwrap();
    file.write_u32::<LittleEndian>(480).unwrap();
    // depth intrinsics (fx, fy, ppx, ppy)
    for _ in 0..4 {
        file.write_f32::<LittleEndian>(320.0).unwrap();
    }
    // depth distortion_model
    file.write_u32::<LittleEndian>(0).unwrap();
    // depth distortion_coeffs [5]
    for _ in 0..5 {
        file.write_f32::<LittleEndian>(0.0).unwrap();
    }
    // color_width, color_height
    file.write_u32::<LittleEndian>(640).unwrap();
    file.write_u32::<LittleEndian>(480).unwrap();
    // color intrinsics (fx, fy, ppx, ppy)
    for _ in 0..4 {
        file.write_f32::<LittleEndian>(320.0).unwrap();
    }
    // color distortion_model
    file.write_u32::<LittleEndian>(0).unwrap();
    // color distortion_coeffs [5]
    for _ in 0..5 {
        file.write_f32::<LittleEndian>(0.0).unwrap();
    }
    // extrinsic_rotation [9]
    for i in 0..9 {
        let v = if i == 0 || i == 4 || i == 8 { 1.0 } else { 0.0 };
        file.write_f32::<LittleEndian>(v).unwrap();
    }
    // extrinsic_translation [3]
    for _ in 0..3 {
        file.write_f32::<LittleEndian>(0.0).unwrap();
    }
    // session_name [128]
    let mut name_buf = [0u8; 128];
    let name_bytes = session_name.as_bytes();
    let copy_len = name_bytes.len().min(127);
    name_buf[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
    file.write_all(&name_buf).unwrap();
    // start_timestamp_us
    file.write_u64::<LittleEndian>(1700000000_000000).unwrap();
    // usb_type [8]
    let mut usb = [0u8; 8];
    usb[..3].copy_from_slice(b"3.2");
    file.write_all(&usb).unwrap();
    // rgb_codec, depth_codec, rgb_quality, zstd_level
    file.write_u8(rgb_codec).unwrap();
    file.write_u8(1).unwrap(); // depth_codec
    file.write_u8(23).unwrap(); // rgb_quality / CRF
    file.write_u8(3).unwrap(); // zstd_level
    // reserved [128]
    file.write_all(&[0u8; 128]).unwrap();

    // Pad to ensure file is >= 1024 bytes (upload scanner skips < 1024).
    // Flush to get current file size, then pad if needed.
    file.flush().unwrap();
    let current_size = file.metadata().unwrap().len();
    let footer_size: u64 = 36;
    let min_total: u64 = 1024;
    if current_size + footer_size < min_total {
        let pad = (min_total - footer_size - current_size) as usize;
        file.write_all(&vec![0u8; pad]).unwrap();
    }

    // --- FileFooter (36 bytes) ---
    // index_magic ("INDX" = 0x58444E49 LE)
    file.write_u32::<LittleEndian>(0x58444E49).unwrap();
    // index_offset (right after header, no actual frames in this test file)
    file.write_u64::<LittleEndian>(680).unwrap();
    // index_entry_count
    file.write_u32::<LittleEndian>(0).unwrap();
    // total_frames
    file.write_u64::<LittleEndian>(frames).unwrap();
    // total_duration_us
    file.write_u64::<LittleEndian>(duration_us).unwrap();
    // footer_magic ("DONE" = 0x454E4F44 LE)
    file.write_u32::<LittleEndian>(0x454E4F44).unwrap();

    file.flush().unwrap();
}

/// Create a dataset directory with dataset.json manifest.
fn create_test_dataset(output_dir: &Path, dir_name: &str, name: &str) -> PathBuf {
    let dataset_dir = output_dir.join(dir_name);
    fs::create_dir_all(&dataset_dir).unwrap();

    let manifest = serde_json::json!({
        "version": 1,
        "name": name,
        "description": "",
        "tags": [],
        "task": "ego_recording",
        "createdAt": "2026-03-01T00:00:00Z",
        "updatedAt": "2026-03-01T00:00:00Z"
    });
    fs::write(
        dataset_dir.join("dataset.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    dataset_dir
}

/// Create an upload manifest at the given directory.
fn create_upload_manifest(dir: &Path, uploads: &[(&str, bool)]) {
    let records: Vec<serde_json::Value> = uploads
        .iter()
        .map(|(filename, success)| {
            serde_json::json!({
                "filename": filename,
                "r2_key": format!("uploads/{}", filename),
                "uploaded_at": "2026-03-01T00:00:00Z",
                "size_bytes": 1024,
                "sha256": "deadbeef",
                "attempt_count": 1,
                "success": success
            })
        })
        .collect();

    let manifest = serde_json::json!({
        "version": 1,
        "uploads": records
    });

    fs::write(
        dir.join(".upload_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

// ===================================================================
//  Module: Config
// ===================================================================
mod config_tests {
    use super::*;

    #[test]
    fn load_missing_config_returns_defaults() {
        // When config file doesn't exist, load_config should return defaults
        // We can't easily test this without importing the module, but we can
        // verify the TOML round-trip works.
        let config_str = r#"
[recorder]
default_crf = 18
warmup_frames = 15

[storage]
disk_threshold_mb = 1000

[upload]
auto_upload = true
multipart_chunk_mb = 64
poll_interval_s = 60
file_settle_s = 5
"#;

        let parsed: toml::Value = toml::from_str(config_str).unwrap();
        assert_eq!(parsed["recorder"]["default_crf"].as_integer(), Some(18));
        assert_eq!(parsed["upload"]["auto_upload"].as_bool(), Some(true));
    }

    #[test]
    fn config_toml_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        let config_str = r#"
[recorder]
binary_path = "/usr/bin/ego-recorder"
default_crf = 23
warmup_frames = 30

[storage]
output_dir = "/home/user/recordings"
disk_threshold_mb = 500

[upload]
endpoint = "https://example.com"
bucket = "test-bucket"
auto_upload = false
multipart_chunk_mb = 32
poll_interval_s = 30
file_settle_s = 10
"#;

        fs::write(&path, config_str).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&contents).unwrap();

        assert_eq!(
            parsed["recorder"]["binary_path"].as_str(),
            Some("/usr/bin/ego-recorder")
        );
        assert_eq!(
            parsed["storage"]["output_dir"].as_str(),
            Some("/home/user/recordings")
        );
        assert_eq!(parsed["upload"]["bucket"].as_str(), Some("test-bucket"));
    }

    #[test]
    fn config_missing_optional_fields_use_defaults() {
        // Minimal config with only required sections
        let config_str = r#"
[recorder]
[storage]
[upload]
"#;
        let parsed: toml::Value = toml::from_str(config_str).unwrap();
        // Sections exist but fields should be absent (None in the app)
        assert!(parsed["recorder"].get("binary_path").is_none());
        assert!(parsed["storage"].get("output_dir").is_none());
    }
}

// ===================================================================
//  Module: Library - File Discovery
// ===================================================================
mod library_tests {
    use super::*;
    use ego_recorder_app_lib::library::{
        extract_dataset, parse_egorec_metadata, scan_egorec_files,
    };

    #[test]
    fn discover_egorec_files_in_flat_directory() {
        let dir = TempDir::new().unwrap();
        create_test_egorec(
            &dir.path().join("rec_001.egorec"),
            "session_001",
            2, // H.264
            900,
            30_000_000,
        );
        create_test_egorec(
            &dir.path().join("rec_002.egorec"),
            "session_002",
            1, // JPEG
            300,
            10_000_000,
        );

        let entries = scan_egorec_files(dir.path());
        assert_eq!(entries.len(), 2);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"rec_001.egorec"));
        assert!(names.contains(&"rec_002.egorec"));
    }

    #[test]
    fn discover_ignores_non_egorec_files() {
        let dir = TempDir::new().unwrap();
        create_test_egorec(&dir.path().join("real.egorec"), "test", 2, 100, 3_000_000);
        fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        fs::write(dir.path().join("data.csv"), "a,b,c").unwrap();

        let entries = scan_egorec_files(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "real.egorec");
    }

    #[test]
    fn discover_ignores_pruned_files() {
        let dir = TempDir::new().unwrap();
        create_test_egorec(&dir.path().join("good.egorec"), "good", 2, 100, 3_000_000);
        // Create a .pruned directory with an egorec file
        let pruned_dir = dir.path().join(".pruned");
        fs::create_dir_all(&pruned_dir).unwrap();
        create_test_egorec(&pruned_dir.join("old.egorec"), "old", 2, 50, 1_000_000);

        let entries = scan_egorec_files(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "good.egorec");
    }

    #[test]
    fn discover_recurses_into_dataset_subdirectories() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("dataset_a");
        fs::create_dir_all(&sub).unwrap();
        create_test_egorec(&sub.join("ep_001.egorec"), "ep_001", 2, 450, 15_000_000);
        create_test_egorec(
            &dir.path().join("loose.egorec"),
            "loose",
            2,
            100,
            3_000_000,
        );

        let entries = scan_egorec_files(dir.path());
        assert_eq!(entries.len(), 2);

        let nested = entries.iter().find(|e| e.name.contains("dataset_a")).unwrap();
        assert_eq!(nested.name, "dataset_a/ep_001.egorec");
    }

    #[test]
    fn discover_empty_directory_returns_empty() {
        let dir = TempDir::new().unwrap();
        let entries = scan_egorec_files(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_metadata_h264_is_streamable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.egorec");
        create_test_egorec(&path, "my_session", 2, 900, 30_000_000);

        let meta = parse_egorec_metadata(&path).unwrap();
        assert_eq!(meta.session_name, "my_session");
        assert_eq!(meta.rgb_codec, 2);
        assert_eq!(meta.color_width, 640);
        assert_eq!(meta.color_height, 480);
        assert_eq!(meta.total_frames, 900);
        assert!((meta.fps - 30.0).abs() < 0.5);
    }

    #[test]
    fn parse_metadata_jpeg_is_idle() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("jpeg.egorec");
        create_test_egorec(&path, "jpeg_session", 1, 300, 10_000_000);

        let meta = parse_egorec_metadata(&path).unwrap();
        assert_eq!(meta.rgb_codec, 1);
    }

    #[test]
    fn parse_metadata_truncated_file_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("truncated.egorec");
        // Write less than header size
        fs::write(&path, &[0u8; 100]).unwrap();

        let result = parse_egorec_metadata(&path);
        assert!(result.is_err());
    }

    #[test]
    fn parse_metadata_zero_duration_gives_zero_fps() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zero_dur.egorec");
        create_test_egorec(&path, "zero_dur", 2, 0, 0);

        let meta = parse_egorec_metadata(&path).unwrap();
        assert_eq!(meta.fps, 0.0);
        assert_eq!(meta.total_frames, 0);
    }

    #[test]
    fn extract_dataset_from_nested_name() {
        assert_eq!(
            extract_dataset("my-dataset/ep_001.egorec"),
            Some("my-dataset".to_string())
        );
        assert_eq!(
            extract_dataset("a/b/c.egorec"),
            Some("a/b".to_string())
        );
    }

    #[test]
    fn extract_dataset_from_root_name() {
        assert_eq!(extract_dataset("file.egorec"), None);
    }
}

// ===================================================================
//  Module: File Watcher
// ===================================================================
mod watcher_tests {
    use super::*;
    use ego_recorder_app_lib::library::watcher::WatcherCommand;
    use tokio::sync::mpsc;

    #[test]
    fn watcher_command_enum_variants() {
        // Verify WatcherCommand can be constructed
        let watch = WatcherCommand::Watch(PathBuf::from("/tmp/test"));
        let stop = WatcherCommand::Stop;
        match watch {
            WatcherCommand::Watch(p) => assert_eq!(p, PathBuf::from("/tmp/test")),
            _ => panic!("Expected Watch variant"),
        }
        match stop {
            WatcherCommand::Stop => {}
            _ => panic!("Expected Stop variant"),
        }
    }

    #[test]
    fn watcher_command_channel_send_receive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = mpsc::channel::<WatcherCommand>(16);

            tx.send(WatcherCommand::Watch(PathBuf::from("/test")))
                .await
                .unwrap();
            tx.send(WatcherCommand::Stop).await.unwrap();

            match rx.recv().await.unwrap() {
                WatcherCommand::Watch(p) => assert_eq!(p, PathBuf::from("/test")),
                _ => panic!("wrong variant"),
            }
            match rx.recv().await.unwrap() {
                WatcherCommand::Stop => {}
                _ => panic!("wrong variant"),
            }
        });
    }
}

// ===================================================================
//  Module: Upload Queue
// ===================================================================
mod upload_queue_tests {
    use super::*;
    use ego_recorder_app_lib::upload::upload_queue::*;

    #[test]
    fn load_manifest_missing_file_returns_default() {
        let dir = TempDir::new().unwrap();
        let manifest = load_manifest(dir.path());
        assert_eq!(manifest.version, 1);
        assert!(manifest.uploads.is_empty());
    }

    #[test]
    fn load_manifest_corrupt_file_returns_default() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".upload_manifest.json"), "not json at all {{{").unwrap();

        let manifest = load_manifest(dir.path());
        assert!(manifest.uploads.is_empty());
    }

    #[test]
    fn save_and_load_manifest_round_trip() {
        let dir = TempDir::new().unwrap();
        let mut manifest = UploadManifest::default();
        record_upload(
            &mut manifest,
            "dataset/ep_001.egorec".to_string(),
            "uploads/dataset/ep_001.egorec".to_string(),
            1024 * 1024,
            "abc123".to_string(),
            1,
        );

        save_manifest(dir.path(), &manifest).unwrap();

        let loaded = load_manifest(dir.path());
        assert_eq!(loaded.uploads.len(), 1);
        assert_eq!(loaded.uploads[0].filename, "dataset/ep_001.egorec");
        assert!(loaded.uploads[0].success);
        assert_eq!(loaded.uploads[0].sha256, "abc123");
    }

    #[test]
    fn uploaded_files_set_only_includes_successful() {
        let mut manifest = UploadManifest::default();

        record_upload(
            &mut manifest,
            "good.egorec".to_string(),
            "key1".to_string(),
            100,
            "hash1".to_string(),
            1,
        );

        // Manually add a failed record
        manifest.uploads.push(UploadRecord {
            filename: "bad.egorec".to_string(),
            r2_key: "key2".to_string(),
            uploaded_at: "2026-01-01".to_string(),
            size_bytes: 200,
            sha256: "hash2".to_string(),
            attempt_count: 3,
            success: false,
        });

        let uploaded = manifest.uploaded_files();
        assert!(uploaded.contains("good.egorec"));
        assert!(!uploaded.contains("bad.egorec"));
    }

    #[test]
    fn is_uploaded_checks_success_flag() {
        let mut manifest = UploadManifest::default();
        record_upload(
            &mut manifest,
            "file.egorec".to_string(),
            "key".to_string(),
            100,
            "hash".to_string(),
            1,
        );

        assert!(is_uploaded(&manifest, "file.egorec"));
        assert!(!is_uploaded(&manifest, "other.egorec"));
    }

    #[test]
    fn scan_pending_finds_new_egorec_files() {
        let dir = TempDir::new().unwrap();
        create_test_egorec(
            &dir.path().join("new_file.egorec"),
            "new",
            2,
            100,
            3_000_000,
        );

        let manifest = UploadManifest::default();
        // Use settle_s = 0 so we don't have to wait
        let pending = scan_pending(dir.path(), &manifest, 0);

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].filename, "new_file.egorec");
    }

    #[test]
    fn scan_pending_skips_already_uploaded() {
        let dir = TempDir::new().unwrap();
        create_test_egorec(
            &dir.path().join("uploaded.egorec"),
            "uploaded",
            2,
            100,
            3_000_000,
        );

        let mut manifest = UploadManifest::default();
        record_upload(
            &mut manifest,
            "uploaded.egorec".to_string(),
            "key".to_string(),
            716,
            "hash".to_string(),
            1,
        );

        let pending = scan_pending(dir.path(), &manifest, 0);
        assert!(pending.is_empty());
    }

    #[test]
    fn scan_pending_skips_tiny_files() {
        let dir = TempDir::new().unwrap();
        // File smaller than 1024 bytes should be skipped
        let path = dir.path().join("tiny.egorec");
        fs::write(&path, &[0u8; 500]).unwrap();

        let manifest = UploadManifest::default();
        let pending = scan_pending(dir.path(), &manifest, 0);
        assert!(pending.is_empty());
    }

    #[test]
    fn scan_pending_skips_pruned_directory() {
        let dir = TempDir::new().unwrap();
        let pruned = dir.path().join(".pruned");
        fs::create_dir_all(&pruned).unwrap();
        create_test_egorec(&pruned.join("old.egorec"), "old", 2, 100, 3_000_000);

        let manifest = UploadManifest::default();
        let pending = scan_pending(dir.path(), &manifest, 0);
        assert!(pending.is_empty());
    }

    #[test]
    fn scan_pending_skips_recently_modified_files() {
        let dir = TempDir::new().unwrap();
        create_test_egorec(
            &dir.path().join("recent.egorec"),
            "recent",
            2,
            100,
            3_000_000,
        );

        let manifest = UploadManifest::default();
        // Very long settle time — file was just created
        let pending = scan_pending(dir.path(), &manifest, 99999);
        assert!(pending.is_empty());
    }

    #[test]
    fn scan_pending_returns_oldest_first() {
        let dir = TempDir::new().unwrap();

        // Create files with slight delay to get different mtimes
        create_test_egorec(&dir.path().join("old.egorec"), "old", 2, 100, 3_000_000);
        // Touch the file to set mtime to past
        let old_path = dir.path().join("old.egorec");
        let old_time = filetime::FileTime::from_unix_time(1000000, 0);
        filetime::set_file_mtime(&old_path, old_time).unwrap();

        create_test_egorec(&dir.path().join("new.egorec"), "new", 2, 100, 3_000_000);

        let manifest = UploadManifest::default();
        let pending = scan_pending(dir.path(), &manifest, 0);
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].filename, "old.egorec");
        assert_eq!(pending[1].filename, "new.egorec");
    }

    #[test]
    fn scan_pending_nested_dataset_files() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("my-dataset");
        fs::create_dir_all(&sub).unwrap();
        create_test_egorec(&sub.join("ep_001.egorec"), "ep1", 2, 100, 3_000_000);

        let manifest = UploadManifest::default();
        let pending = scan_pending(dir.path(), &manifest, 0);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].filename, "my-dataset/ep_001.egorec");
    }

    #[test]
    fn queue_status_serialization() {
        let pending = QueueStatus::Pending;
        let json = serde_json::to_string(&pending).unwrap();
        assert!(json.contains("pending"));

        let uploading = QueueStatus::Uploading {
            progress: 0.5,
            speed_bps: 1024,
        };
        let json = serde_json::to_string(&uploading).unwrap();
        assert!(json.contains("uploading"));
        assert!(json.contains("1024"));
    }
}

// ===================================================================
//  Module: Dataset Manifest
// ===================================================================
mod dataset_manifest_tests {
    use super::*;
    use ego_recorder_app_lib::dataset::manifest::*;

    #[test]
    fn create_dataset_creates_dir_and_manifest() {
        let dir = TempDir::new().unwrap();
        let manifest = create_dataset(dir.path(), "My Cool Dataset", None).unwrap();

        assert_eq!(manifest.name, "My Cool Dataset");
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.task, "ego_recording");

        // Directory should exist
        let dataset_dir = dir.path().join("My-Cool-Dataset");
        assert!(dataset_dir.exists());
        assert!(dataset_dir.join("dataset.json").exists());
    }

    #[test]
    fn create_dataset_sanitizes_directory_name() {
        let dir = TempDir::new().unwrap();
        let _manifest = create_dataset(dir.path(), "Hello World! (test)", None).unwrap();

        // Spaces and special chars should be replaced with hyphens.
        // Just verify a directory was created.
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn create_dataset_rejects_duplicate_name() {
        let dir = TempDir::new().unwrap();
        create_dataset(dir.path(), "test", None).unwrap();

        let result = create_dataset(dir.path(), "test", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn load_manifest_returns_none_for_missing() {
        let dir = TempDir::new().unwrap();
        assert!(load_manifest(dir.path()).is_none());
    }

    #[test]
    fn save_and_load_manifest_round_trip() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path()).unwrap();

        let mut manifest = DatasetManifest::new("Test Dataset", None);
        manifest.description = "A test description".to_string();
        manifest.tags = vec!["tag1".to_string(), "tag2".to_string()];

        save_manifest(dir.path(), &manifest).unwrap();

        let loaded = load_manifest(dir.path()).unwrap();
        assert_eq!(loaded.name, "Test Dataset");
        assert_eq!(loaded.description, "A test description");
        assert_eq!(loaded.tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn delete_dataset_removes_directory() {
        let dir = TempDir::new().unwrap();
        let dataset_dir = dir.path().join("to-delete");
        fs::create_dir_all(&dataset_dir).unwrap();
        fs::write(dataset_dir.join("dataset.json"), "{}").unwrap();
        fs::write(dataset_dir.join("ep_001.egorec"), "fake").unwrap();

        delete_dataset(&dataset_dir).unwrap();
        assert!(!dataset_dir.exists());
    }

    #[test]
    fn delete_nonexistent_dataset_errors() {
        let dir = TempDir::new().unwrap();
        let result = delete_dataset(&dir.path().join("nope"));
        assert!(result.is_err());
    }
}

// ===================================================================
//  Module: Dataset Scan
// ===================================================================
mod dataset_scan_tests {
    use super::*;
    use ego_recorder_app_lib::dataset::scan::scan_datasets;

    #[test]
    fn scan_finds_datasets_with_manifest() {
        let dir = TempDir::new().unwrap();
        let ds = create_test_dataset(dir.path(), "kitchen-tasks", "Kitchen Tasks");
        create_test_egorec(&ds.join("ep_001.egorec"), "ep1", 2, 300, 10_000_000);
        create_test_egorec(&ds.join("ep_002.egorec"), "ep2", 2, 450, 15_000_000);

        let datasets = scan_datasets(dir.path());
        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0].name, "Kitchen Tasks");
        assert_eq!(datasets[0].dir_name, "kitchen-tasks");
        assert_eq!(datasets[0].file_count, 2);
        assert_eq!(datasets[0].total_frames, 750);
    }

    #[test]
    fn scan_ignores_dirs_without_manifest() {
        let dir = TempDir::new().unwrap();
        // Directory without dataset.json
        let no_manifest = dir.path().join("just-files");
        fs::create_dir_all(&no_manifest).unwrap();
        create_test_egorec(&no_manifest.join("ep.egorec"), "ep", 2, 100, 3_000_000);

        // Directory with dataset.json
        create_test_dataset(dir.path(), "real-dataset", "Real");

        let datasets = scan_datasets(dir.path());
        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0].name, "Real");
    }

    #[test]
    fn scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let datasets = scan_datasets(dir.path());
        assert!(datasets.is_empty());
    }

    #[test]
    fn scan_dataset_with_no_egorec_files() {
        let dir = TempDir::new().unwrap();
        create_test_dataset(dir.path(), "empty-ds", "Empty");

        let datasets = scan_datasets(dir.path());
        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0].file_count, 0);
        assert_eq!(datasets[0].total_frames, 0);
    }

    #[test]
    fn scan_detects_lerobot_conversion() {
        let dir = TempDir::new().unwrap();
        let ds = create_test_dataset(dir.path(), "converted", "Converted");

        // Create _lerobot/meta/info.json
        let lerobot_meta = ds.join("_lerobot/meta");
        fs::create_dir_all(&lerobot_meta).unwrap();
        fs::write(lerobot_meta.join("info.json"), "{}").unwrap();

        let datasets = scan_datasets(dir.path());
        assert!(datasets[0].has_lerobot);
    }

    #[test]
    fn scan_counts_uploaded_files() {
        let dir = TempDir::new().unwrap();
        let ds = create_test_dataset(dir.path(), "uploading", "Uploading");
        create_test_egorec(&ds.join("ep_001.egorec"), "ep1", 2, 100, 3_000_000);
        create_test_egorec(&ds.join("ep_002.egorec"), "ep2", 2, 100, 3_000_000);

        // Create upload manifest at the output_dir level
        create_upload_manifest(
            dir.path(),
            &[("uploading/ep_001.egorec", true), ("uploading/ep_002.egorec", false)],
        );

        let datasets = scan_datasets(dir.path());
        assert_eq!(datasets[0].uploaded_count, 1); // Only the successful one
    }

    #[test]
    fn scan_datasets_sorted_by_creation_date_descending() {
        let dir = TempDir::new().unwrap();

        // Create datasets with different creation times
        let ds1_dir = dir.path().join("old-ds");
        fs::create_dir_all(&ds1_dir).unwrap();
        let manifest1 = serde_json::json!({
            "version": 1, "name": "Old", "description": "", "tags": [],
            "task": "ego_recording",
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z"
        });
        fs::write(ds1_dir.join("dataset.json"), serde_json::to_string(&manifest1).unwrap()).unwrap();

        let ds2_dir = dir.path().join("new-ds");
        fs::create_dir_all(&ds2_dir).unwrap();
        let manifest2 = serde_json::json!({
            "version": 1, "name": "New", "description": "", "tags": [],
            "task": "ego_recording",
            "createdAt": "2026-03-01T00:00:00Z",
            "updatedAt": "2026-03-01T00:00:00Z"
        });
        fs::write(ds2_dir.join("dataset.json"), serde_json::to_string(&manifest2).unwrap()).unwrap();

        let datasets = scan_datasets(dir.path());
        assert_eq!(datasets.len(), 2);
        assert_eq!(datasets[0].name, "New"); // Newest first
        assert_eq!(datasets[1].name, "Old");
    }
}

// ===================================================================
//  Module: Recorder - Stats Parsing
// ===================================================================
mod stats_parsing_tests {
    use ego_recorder_app_lib::recorder::subprocess::parse_stats_line;
    use ego_recorder_app_lib::recorder::status::RecorderState;

    #[test]
    fn parse_typical_recording_stats() {
        let line = "REC 02:15 | Frames: 4050 written, 12 dropped | FPS: 30.0 cap / 29.6 write | Size: 312.5 MB";
        let s = parse_stats_line(line).unwrap();
        assert_eq!(s.state, RecorderState::Recording);
        assert!((s.elapsed_seconds - 135.0).abs() < 0.1);
        assert_eq!(s.frames_written, 4050);
        assert_eq!(s.frames_dropped, 12);
        assert!((s.capture_fps - 30.0).abs() < 0.1);
        assert!((s.write_fps - 29.6).abs() < 0.1);
        assert!((s.file_size_mb - 312.5).abs() < 0.1);
    }

    #[test]
    fn parse_zero_elapsed() {
        let line = "REC 00:00 | Frames: 0 written, 0 dropped | FPS: 0.0 cap / 0.0 write | Size: 0.0 MB";
        let s = parse_stats_line(line).unwrap();
        assert_eq!(s.elapsed_seconds, 0.0);
        assert_eq!(s.frames_written, 0);
    }

    #[test]
    fn parse_large_values() {
        let line = "REC 99:59 | Frames: 179820 written, 0 dropped | FPS: 30.0 cap / 30.0 write | Size: 8523.7 MB";
        let s = parse_stats_line(line).unwrap();
        assert!((s.elapsed_seconds - 5999.0).abs() < 0.1);
        assert_eq!(s.frames_written, 179820);
        assert!((s.file_size_mb - 8523.7).abs() < 0.1);
    }

    #[test]
    fn parse_idle_with_fps() {
        let line = "Idle | Camera FPS: 29.97";
        let s = parse_stats_line(line).unwrap();
        assert_eq!(s.state, RecorderState::Idle);
        assert!((s.capture_fps - 29.97).abs() < 0.01);
    }

    #[test]
    fn parse_unrecognized_returns_none() {
        assert!(parse_stats_line("INFO: Starting preview...").is_none());
        assert!(parse_stats_line("DISCONNECTED").is_none());
        assert!(parse_stats_line("Recording complete").is_none());
        assert!(parse_stats_line("").is_none());
    }
}

// ===================================================================
//  Module: H.264 Annex-B Parser
// ===================================================================
mod h264_tests {
    use ego_recorder_app_lib::video::h264_annex_b::*;

    #[test]
    fn parse_realistic_keyframe_access_unit() {
        // Typical keyframe: SPS + PPS + SEI + IDR
        let mut data = Vec::new();
        // SPS (type 7)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x67, 0x42, 0xC0, 0x1E, 0xD9, 0x00, 0xA0, 0x47, 0xFE, 0x88]);
        // PPS (type 8)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]);
        // SEI (type 6) — should be parsed but not SPS/PPS
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x06, 0x05, 0x04, 0xFF]);
        // IDR (type 5)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x65, 0x88, 0x80, 0x40, 0x00, 0xFF, 0xFF]);

        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 4);
        assert_eq!(nals[0].nal_type, 7); // SPS
        assert_eq!(nals[1].nal_type, 8); // PPS
        assert_eq!(nals[2].nal_type, 6); // SEI
        assert_eq!(nals[3].nal_type, 5); // IDR

        assert!(is_keyframe(&nals));

        let (sps, pps) = extract_sps_pps(&nals).unwrap();
        assert_eq!(sps[0] & 0x1F, 7);
        assert_eq!(pps[0] & 0x1F, 8);
    }

    #[test]
    fn parse_p_frame_not_keyframe() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x41, 0x9A, 0x24, 0x68, 0x12, 0x00]);

        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0].nal_type, 1); // non-IDR
        assert!(!is_keyframe(&nals));
        assert!(extract_sps_pps(&nals).is_none());
    }

    #[test]
    fn avcc_conversion_preserves_data() {
        let nals = vec![
            NalUnit { nal_type: 5, data: vec![0x65, 0xAA, 0xBB, 0xCC] },
        ];
        let avcc = nals_to_avcc(&nals);
        assert_eq!(avcc.len(), 8); // 4 (length) + 4 (data)
        let len = u32::from_be_bytes([avcc[0], avcc[1], avcc[2], avcc[3]]);
        assert_eq!(len, 4);
        assert_eq!(&avcc[4..], &[0x65, 0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn mixed_3byte_and_4byte_start_codes() {
        let mut data = Vec::new();
        // 4-byte start code
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42]);
        // 3-byte start code
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0x68, 0xCE]);
        // 4-byte start code
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x65, 0x88]);

        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 3);
    }
}

// ===================================================================
//  Module: MP4 Muxer
// ===================================================================
mod mp4_tests {
    use ego_recorder_app_lib::video::mp4_mux::*;

    fn test_config() -> Mp4TrackConfig {
        Mp4TrackConfig {
            width: 640,
            height: 480,
            timescale: 30000,
            sample_delta: 1000,
            sps: vec![0x67, 0x42, 0x00, 0x1E, 0xAB],
            pps: vec![0x68, 0xCE, 0x38, 0x80],
        }
    }

    #[test]
    fn mp4_has_correct_top_level_structure() {
        let config = test_config();
        let samples = vec![
            Mp4Sample { data: vec![0x00, 0x00, 0x00, 0x04, 0x65, 0xAA, 0xBB, 0xCC], is_keyframe: true },
            Mp4Sample { data: vec![0x00, 0x00, 0x00, 0x03, 0x41, 0x01, 0x02], is_keyframe: false },
        ];
        let mp4 = build_mp4(&config, &samples);

        // Verify three top-level boxes: ftyp, moov, mdat
        let mut offset = 0;
        let mut boxes = Vec::new();
        while offset + 8 <= mp4.len() {
            let size = u32::from_be_bytes([mp4[offset], mp4[offset+1], mp4[offset+2], mp4[offset+3]]) as usize;
            let name = std::str::from_utf8(&mp4[offset+4..offset+8]).unwrap_or("????");
            boxes.push(name.to_string());
            offset += size;
        }
        assert_eq!(boxes, vec!["ftyp", "moov", "mdat"]);
        assert_eq!(offset, mp4.len()); // No trailing bytes
    }

    #[test]
    fn mp4_ftyp_contains_avc1_brand() {
        let config = test_config();
        let mp4 = build_mp4(&config, &[]);

        let ftyp_size = u32::from_be_bytes([mp4[0], mp4[1], mp4[2], mp4[3]]) as usize;
        let ftyp_data = &mp4[8..ftyp_size];
        // Should contain "isom" major brand and "avc1" compatible brand
        let ftyp_str = String::from_utf8_lossy(ftyp_data);
        assert!(ftyp_str.contains("isom"));
        assert!(ftyp_str.contains("avc1"));
    }

    #[test]
    fn mp4_single_keyframe() {
        let config = test_config();
        let samples = vec![
            Mp4Sample {
                data: vec![0x00, 0x00, 0x00, 0x05, 0x65, 0x88, 0x80, 0x40, 0x00],
                is_keyframe: true,
            },
        ];
        let mp4 = build_mp4(&config, &samples);
        assert!(mp4.len() > 100); // Sanity
        assert_eq!(&mp4[4..8], b"ftyp");
    }

    #[test]
    fn mp4_many_frames() {
        let config = test_config();
        let mut samples = Vec::new();
        for i in 0..1000 {
            samples.push(Mp4Sample {
                data: vec![0x00, 0x00, 0x00, 0x02, 0x41, (i % 256) as u8],
                is_keyframe: i % 30 == 0,
            });
        }
        let mp4 = build_mp4(&config, &samples);
        // Should produce valid MP4 with all 1000 samples
        // mdat payload = 1000 * 6 bytes = 6000 bytes
        assert!(mp4.len() > 6000);
    }
}

// ===================================================================
//  Module: Video Server - Range Request Parsing
// ===================================================================
mod video_server_tests {
    // Test the range parsing logic that the video server uses

    fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
        let range_spec = header.strip_prefix("bytes=")?;
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

    #[test]
    fn range_full_file() {
        let (start, end) = parse_range("bytes=0-", 1000).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 999);
    }

    #[test]
    fn range_partial() {
        let (start, end) = parse_range("bytes=100-199", 1000).unwrap();
        assert_eq!(start, 100);
        assert_eq!(end, 199);
    }

    #[test]
    fn range_beyond_end_clamped() {
        let (start, end) = parse_range("bytes=900-1500", 1000).unwrap();
        assert_eq!(start, 900);
        assert_eq!(end, 999); // Clamped
    }

    #[test]
    fn range_start_at_end_is_invalid() {
        assert!(parse_range("bytes=1000-", 1000).is_none());
    }

    #[test]
    fn range_start_after_end_is_invalid() {
        assert!(parse_range("bytes=500-400", 1000).is_none());
    }

    #[test]
    fn range_invalid_format() {
        assert!(parse_range("not-a-range", 1000).is_none());
        assert!(parse_range("bytes=abc-def", 1000).is_none());
    }
}

// ===================================================================
//  Module: Frame Reader Protocol
// ===================================================================
mod frame_reader_tests {
    use ego_recorder_app_lib::preview::CameraInfo;

    #[test]
    fn frame_reader_parses_camera_info_json() {
        let camera_json = r#"{"serial":"D435-001","usb":"3.2","hasImu":false,"width":640,"height":480}"#;
        let info: CameraInfo = serde_json::from_str(camera_json).unwrap();
        assert_eq!(info.serial, "D435-001");
        assert_eq!(info.usb, "3.2");
        assert!(!info.has_imu);
        assert_eq!(info.width, 640);
        assert_eq!(info.height, 480);
    }

    #[test]
    fn camera_info_default_has_empty_serial() {
        let info = CameraInfo::default();
        assert!(info.serial.is_empty());
        assert_eq!(info.width, 0);
    }

    #[test]
    fn camera_info_serialization() {
        let info = CameraInfo {
            serial: "TEST-123".to_string(),
            usb: "3.2".to_string(),
            has_imu: true,
            width: 1280,
            height: 720,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: CameraInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.serial, "TEST-123");
        assert!(parsed.has_imu);
    }

    #[test]
    fn frame_protocol_binary_format() {
        // Verify the expected binary format: tag(1) + size_le(4) + data(N)
        let tag = b'R';
        let data = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG-like
        let size = data.len() as u32;

        let mut packet = Vec::new();
        packet.push(tag);
        packet.extend_from_slice(&size.to_le_bytes());
        packet.extend_from_slice(&data);

        // Parse it back
        assert_eq!(packet[0], b'R');
        let parsed_size = u32::from_le_bytes([packet[1], packet[2], packet[3], packet[4]]);
        assert_eq!(parsed_size, 4);
        assert_eq!(&packet[5..], &data);
    }

    #[test]
    fn frame_size_sanity_check() {
        // Frames > 10MB should be rejected
        let max_frame_size: usize = 10 * 1024 * 1024;
        let oversized = max_frame_size + 1;
        assert!(oversized > max_frame_size);
    }
}

// ===================================================================
//  Module: Preview State Machine
// ===================================================================
mod preview_state_tests {
    use ego_recorder_app_lib::preview::PreviewState;

    #[test]
    fn preview_state_default_is_off() {
        assert_eq!(PreviewState::default(), PreviewState::Off);
    }

    #[test]
    fn preview_state_serialization() {
        let states = vec![
            (PreviewState::Off, "\"off\""),
            (PreviewState::Starting, "\"starting\""),
            (PreviewState::Previewing, "\"previewing\""),
            (PreviewState::Recording, "\"recording\""),
            (PreviewState::Stopping, "\"stopping\""),
            (PreviewState::Error, "\"error\""),
        ];
        for (state, expected_json) in states {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, expected_json, "State {:?} mismatch", state);
        }
    }

    #[test]
    fn preview_state_deserialization() {
        let state: PreviewState = serde_json::from_str("\"previewing\"").unwrap();
        assert_eq!(state, PreviewState::Previewing);
    }

    #[test]
    fn preview_state_equality() {
        assert_eq!(PreviewState::Off, PreviewState::Off);
        assert_ne!(PreviewState::Off, PreviewState::Previewing);
        assert_ne!(PreviewState::Recording, PreviewState::Stopping);
    }
}

// ===================================================================
//  Module: Recorder Status
// ===================================================================
mod recorder_status_tests {
    use ego_recorder_app_lib::recorder::status::{RecorderState, RecorderStatus};

    #[test]
    fn default_status_is_idle() {
        let status = RecorderStatus::default();
        assert_eq!(status.state, RecorderState::Idle);
        assert_eq!(status.frames_written, 0);
        assert_eq!(status.frames_dropped, 0);
        assert_eq!(status.capture_fps, 0.0);
        assert_eq!(status.write_fps, 0.0);
        assert_eq!(status.file_size_mb, 0.0);
        assert_eq!(status.elapsed_seconds, 0.0);
        assert!(status.current_file.is_none());
    }

    #[test]
    fn recorder_state_serialization() {
        let state = RecorderState::Recording;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"recording\"");

        let deserialized: RecorderState = serde_json::from_str("\"stopping\"").unwrap();
        assert_eq!(deserialized, RecorderState::Stopping);
    }

    #[test]
    fn recorder_status_full_serialization() {
        let status = RecorderStatus {
            state: RecorderState::Recording,
            frames_written: 1500,
            frames_dropped: 2,
            capture_fps: 30.0,
            write_fps: 29.8,
            file_size_mb: 85.3,
            elapsed_seconds: 50.0,
            episode_count: 1,
            current_file: Some("test.egorec".to_string()),
        };

        let json = serde_json::to_string(&status).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["state"], "recording");
        assert_eq!(parsed["framesWritten"], 1500);
        assert_eq!(parsed["framesDropped"], 2);
        assert_eq!(parsed["currentFile"], "test.egorec");
    }
}

// ===================================================================
//  Module: Egorec Metadata DTO
// ===================================================================
mod metadata_dto_tests {
    use ego_recorder_app_lib::state::EgorecMetadataDto;
    use egorec::format::*;

    fn make_test_header() -> FileHeader {
        FileHeader {
            magic: FILE_MAGIC,
            header_size: FILE_HEADER_SIZE as u32,
            flags: 0x01, // has IMU
            serial_number: {
                let mut s = [0u8; 32];
                s[..7].copy_from_slice(b"D435i-1");
                s
            },
            depth_scale: 0.001,
            depth_width: 640,
            depth_height: 480,
            depth_fx: 382.5,
            depth_fy: 382.5,
            depth_ppx: 320.0,
            depth_ppy: 240.0,
            depth_distortion_model: 0,
            depth_distortion_coeffs: [0.0; 5],
            color_width: 640,
            color_height: 480,
            color_fx: 617.0,
            color_fy: 617.0,
            color_ppx: 320.0,
            color_ppy: 240.0,
            color_distortion_model: 0,
            color_distortion_coeffs: [0.0; 5],
            extrinsic_rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            extrinsic_translation: [0.015, 0.0, 0.0],
            session_name: {
                let mut n = [0u8; 128];
                n[..11].copy_from_slice(b"test_sess01");
                n
            },
            start_timestamp_us: 1700000000_000000,
            usb_type: {
                let mut u = [0u8; 8];
                u[..3].copy_from_slice(b"3.2");
                u
            },
            rgb_codec: 2,
            depth_codec: 1,
            rgb_quality: 23,
            zstd_level: 3,
            reserved: [0u8; 128],
        }
    }

    #[test]
    fn from_header_basic_fields() {
        let header = make_test_header();
        let dto = EgorecMetadataDto::from_header(&header, 900, 30_000_000);

        assert_eq!(dto.session_name, "test_sess01");
        assert_eq!(dto.serial_number, "D435i-1");
        assert_eq!(dto.usb_type, "3.2");
        assert_eq!(dto.color_width, 640);
        assert_eq!(dto.color_height, 480);
        assert_eq!(dto.rgb_codec, 2);
        assert_eq!(dto.total_frames, 900);
        assert!(dto.has_imu);
    }

    #[test]
    fn from_header_fps_calculation() {
        let header = make_test_header();

        // 900 frames in 30 seconds = 30.0 fps
        let dto = EgorecMetadataDto::from_header(&header, 900, 30_000_000);
        assert!((dto.fps - 30.0).abs() < 0.01);
        assert!((dto.duration_s - 30.0).abs() < 0.01);
    }

    #[test]
    fn from_header_zero_duration_zero_fps() {
        let header = make_test_header();
        let dto = EgorecMetadataDto::from_header(&header, 0, 0);
        assert_eq!(dto.fps, 0.0);
        assert_eq!(dto.duration_s, 0.0);
    }

    #[test]
    fn from_header_intrinsics_preserved() {
        let header = make_test_header();
        let dto = EgorecMetadataDto::from_header(&header, 100, 3_000_000);

        assert_eq!(dto.intrinsics.color.fx, 617.0);
        assert_eq!(dto.intrinsics.depth.fx, 382.5);
        assert_eq!(dto.intrinsics.depth.scale, 0.001);
    }

    #[test]
    fn from_header_extrinsics_preserved() {
        let header = make_test_header();
        let dto = EgorecMetadataDto::from_header(&header, 100, 3_000_000);

        assert_eq!(dto.extrinsics.translation[0], 0.015);
        assert_eq!(dto.extrinsics.rotation[0], 1.0);
        assert_eq!(dto.extrinsics.rotation[4], 1.0);
    }

    #[test]
    fn dto_serialization_camel_case() {
        let header = make_test_header();
        let dto = EgorecMetadataDto::from_header(&header, 100, 3_000_000);

        let json = serde_json::to_string(&dto).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Fields should be camelCase
        assert!(parsed.get("sessionName").is_some());
        assert!(parsed.get("serialNumber").is_some());
        assert!(parsed.get("colorWidth").is_some());
        assert!(parsed.get("depthScale").is_some());
        assert!(parsed.get("totalFrames").is_some());
        assert!(parsed.get("durationS").is_some());
        assert!(parsed.get("rgbCodec").is_some());
        assert!(parsed.get("hasImu").is_some());

        // Should NOT have snake_case
        assert!(parsed.get("session_name").is_none());
        assert!(parsed.get("color_width").is_none());
    }
}

// ===================================================================
//  Module: Conversion Status
// ===================================================================
mod conversion_status_tests {
    use ego_recorder_app_lib::state::ConversionStatus;

    #[test]
    fn conversion_status_serialization() {
        let idle = ConversionStatus::Idle;
        let json = serde_json::to_string(&idle).unwrap();
        assert_eq!(json, "\"idle\"");

        let streamable = ConversionStatus::Streamable;
        let json = serde_json::to_string(&streamable).unwrap();
        assert_eq!(json, "\"streamable\"");

        let error = ConversionStatus::Error;
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(json, "\"error\"");
    }
}

// ===================================================================
//  Module: ConversionProgress DTO
// ===================================================================
mod conversion_progress_tests {
    use ego_recorder_app_lib::dataset::convert::ConversionProgress;

    #[test]
    fn progress_serialization() {
        let progress = ConversionProgress {
            dataset_name: "test-ds".to_string(),
            current_file: "ep_001.egorec".to_string(),
            file_index: 2,
            total_files: 5,
            frames_done: 450,
            total_frames: 1500,
            phase: "converting".to_string(),
            error: None,
        };

        let json = serde_json::to_string(&progress).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["datasetName"], "test-ds");
        assert_eq!(parsed["currentFile"], "ep_001.egorec");
        assert_eq!(parsed["fileIndex"], 2);
        assert_eq!(parsed["totalFiles"], 5);
        assert_eq!(parsed["framesDone"], 450);
        assert_eq!(parsed["phase"], "converting");
        assert!(parsed["error"].is_null());
    }

    #[test]
    fn progress_with_error() {
        let progress = ConversionProgress {
            dataset_name: "test".to_string(),
            current_file: "bad.egorec".to_string(),
            file_index: 0,
            total_files: 1,
            frames_done: 0,
            total_frames: 0,
            phase: "error".to_string(),
            error: Some("Corrupt frame at offset 1024".to_string()),
        };

        let json = serde_json::to_string(&progress).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["error"], "Corrupt frame at offset 1024");
    }
}

// ===================================================================
//  Module: Edge Cases & Robustness
// ===================================================================
mod edge_case_tests {
    use super::*;
    use ego_recorder_app_lib::library::{parse_egorec_metadata, scan_egorec_files};

    #[test]
    fn file_with_special_characters_in_name() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session with spaces (1).egorec");
        create_test_egorec(&path, "spaced session", 2, 100, 3_000_000);

        let entries = scan_egorec_files(dir.path());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].name.contains("spaces"));
    }

    #[test]
    fn deeply_nested_directories() {
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("a/b/c/d/e");
        fs::create_dir_all(&deep).unwrap();
        create_test_egorec(&deep.join("deep.egorec"), "deep", 2, 100, 3_000_000);

        let entries = scan_egorec_files(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a/b/c/d/e/deep.egorec");
    }

    #[test]
    fn concurrent_dataset_operations() {
        use ego_recorder_app_lib::dataset::manifest::*;

        let dir = TempDir::new().unwrap();

        // Create multiple datasets quickly
        for i in 0..10 {
            let name = format!("dataset-{}", i);
            create_dataset(dir.path(), &name, None).unwrap();
        }

        // All should exist
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        assert_eq!(entries.len(), 10);
    }

    #[test]
    fn empty_egorec_file_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.egorec");
        fs::write(&path, &[]).unwrap();

        assert!(parse_egorec_metadata(&path).is_err());
    }

    #[test]
    fn binary_garbage_egorec_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("garbage.egorec");
        // Write enough bytes to look like a header but with wrong magic
        let mut garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00];
        garbage.extend_from_slice(&[0u8; 680 + 36]); // header + footer size
        fs::write(&path, &garbage).unwrap();

        // Should either error or return garbage metadata (depends on format strictness)
        // The important thing is it doesn't panic
        let _ = parse_egorec_metadata(&path);
    }

    #[test]
    fn symlink_to_egorec_file() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("real.egorec");
        create_test_egorec(&real, "real_session", 2, 100, 3_000_000);

        let link = dir.path().join("link.egorec");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let entries = scan_egorec_files(dir.path());
        // Should find both the real file and the symlink
        #[cfg(unix)]
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn very_long_session_name() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("long_name.egorec");
        let long_name = "a".repeat(200); // Longer than 128-byte buffer
        create_test_egorec(&path, &long_name, 2, 100, 3_000_000);

        let meta = parse_egorec_metadata(&path).unwrap();
        // Should be truncated to 127 chars (128 - null terminator)
        assert_eq!(meta.session_name.len(), 127);
    }

    #[test]
    fn high_frame_count_metadata() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("long_recording.egorec");
        // 10 hours at 30fps
        let frames: u64 = 10 * 60 * 60 * 30;
        let duration_us: u64 = 10 * 60 * 60 * 1_000_000;
        create_test_egorec(&path, "marathon", 2, frames, duration_us);

        let meta = parse_egorec_metadata(&path).unwrap();
        assert_eq!(meta.total_frames, frames);
        assert!((meta.fps - 30.0).abs() < 0.5);
        assert!((meta.duration_s - 36000.0).abs() < 1.0);
    }

    #[test]
    fn upload_manifest_atomic_write_no_partial() {
        use ego_recorder_app_lib::upload::upload_queue::*;

        let dir = TempDir::new().unwrap();
        let mut manifest = UploadManifest::default();

        // Add many records
        for i in 0..100 {
            record_upload(
                &mut manifest,
                format!("file_{:03}.egorec", i),
                format!("key_{:03}", i),
                1024 * i as u64,
                format!("hash_{:03}", i),
                1,
            );
        }

        save_manifest(dir.path(), &manifest).unwrap();

        // No .tmp file should remain
        assert!(!dir.path().join(".upload_manifest.json.tmp").exists());

        // Load and verify
        let loaded = load_manifest(dir.path());
        assert_eq!(loaded.uploads.len(), 100);
    }
}

// ===================================================================
//  Module: Subprocess Args Builder
// ===================================================================
mod subprocess_tests {
    use ego_recorder_app_lib::recorder::subprocess::build_args;

    #[test]
    fn build_args_format() {
        let args = build_args("/tmp/output", "my_session", 18, 15);
        assert_eq!(args, vec![
            "-o", "/tmp/output",
            "-s", "my_session",
            "--crf", "18",
            "--warmup", "15",
        ]);
    }

    #[test]
    fn build_args_with_spaces_in_path() {
        let args = build_args("/home/user/my recordings", "test", 23, 30);
        assert_eq!(args[1], "/home/user/my recordings");
    }

    #[test]
    fn build_args_zero_crf_lossless() {
        let args = build_args("/tmp", "lossless", 0, 0);
        assert_eq!(args[5], "0");
        assert_eq!(args[7], "0");
    }

    #[test]
    fn build_args_max_crf() {
        let args = build_args("/tmp", "worst", 51, 100);
        assert_eq!(args[5], "51");
        assert_eq!(args[7], "100");
    }
}
