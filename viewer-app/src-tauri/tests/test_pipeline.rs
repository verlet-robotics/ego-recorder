use egorec_viewer_lib::commands::curation::build_ego_curate_args;
use std::path::Path;

fn workspace_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../curation-workspace")
        .join(name)
}

fn has_workspace(name: &str) -> bool {
    let ws = workspace_path(name);
    ws.join("staging/v1/stage_manifest.jsonl").exists()
}

// ── Native QC stage ─────────────────────────────────────────────────────────

#[test]
fn qc_produces_valid_episodes_jsonl() {
    if !has_workspace("ctj") {
        eprintln!("SKIP: ctj workspace not present");
        return;
    }

    let ws = workspace_path("ctj");
    let result = egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|_| {})
        .expect("QC stage should succeed");

    assert!(result.success);
    assert!(result.counts.values().sum::<usize>() > 0, "should produce episodes");

    // Verify the output file is valid JSONL
    let episodes_path = ws.join("curation/v1/episodes.jsonl");
    assert!(episodes_path.exists(), "episodes.jsonl should be created");

    let contents = std::fs::read_to_string(&episodes_path).unwrap();
    for (i, line) in contents.lines().enumerate() {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("Line {} is not valid JSON: {}", i + 1, e));

        // Every row must have required fields
        assert!(parsed.get("episode_id").is_some(), "line {} missing episode_id", i + 1);
        assert!(parsed.get("source_key").is_some(), "line {} missing source_key", i + 1);
        assert!(parsed.get("episode_status").is_some(), "line {} missing episode_status", i + 1);
        assert!(parsed.get("validate_ok").is_some(), "line {} missing validate_ok", i + 1);
        assert!(parsed.get("duration_s").is_some(), "line {} missing duration_s", i + 1);
        assert!(parsed.get("local_path").is_some(), "line {} missing local_path", i + 1);

        // episode_id should have the ep_ prefix
        let eid = parsed["episode_id"].as_str().unwrap();
        assert!(eid.starts_with("ep_"), "episode_id should start with ep_: {}", eid);

        // episode_status must be a known value
        let status = parsed["episode_status"].as_str().unwrap();
        assert!(
            ["keep", "review", "reject", "invalid"].contains(&status),
            "unknown episode_status: {}",
            status
        );
    }
}

#[test]
fn qc_episode_ids_are_deterministic() {
    if !has_workspace("ctj") {
        eprintln!("SKIP: ctj workspace not present");
        return;
    }

    let ws = workspace_path("ctj");

    // Run twice
    let r1 = egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|_| {}).unwrap();
    let ids1 = read_episode_ids(&ws);

    let r2 = egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|_| {}).unwrap();
    let ids2 = read_episode_ids(&ws);

    assert_eq!(r1.counts, r2.counts, "counts should be identical across runs");
    assert_eq!(ids1, ids2, "episode IDs should be identical across runs");
}

fn read_episode_ids(ws: &Path) -> Vec<String> {
    let contents = std::fs::read_to_string(ws.join("curation/v1/episodes.jsonl")).unwrap();
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["episode_id"].as_str().unwrap().to_string()
        })
        .collect()
}

// ── Native intervals stage ──────────────────────────────────────────────────

#[test]
fn intervals_produces_valid_jsonl() {
    if !has_workspace("ctj") {
        eprintln!("SKIP: ctj workspace not present");
        return;
    }

    let ws = workspace_path("ctj");

    // Ensure QC has run first
    egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|_| {}).unwrap();

    let result = egorec_viewer_lib::pipeline::run_intervals_stage(&ws, &|_| {})
        .expect("Intervals stage should succeed");

    assert!(result.success);

    let intervals_path = ws.join("curation/v1/intervals.jsonl");
    assert!(intervals_path.exists(), "intervals.jsonl should be created");

    let contents = std::fs::read_to_string(&intervals_path).unwrap();
    for (i, line) in contents.lines().enumerate() {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("Line {} is not valid JSON: {}", i + 1, e));

        assert!(parsed.get("interval_id").is_some(), "line {} missing interval_id", i + 1);
        assert!(parsed.get("source_key").is_some(), "line {} missing source_key", i + 1);
        assert!(parsed.get("start_s").is_some(), "line {} missing start_s", i + 1);
        assert!(parsed.get("end_s").is_some(), "line {} missing end_s", i + 1);
        assert!(parsed.get("duration_s").is_some(), "line {} missing duration_s", i + 1);
        assert!(parsed.get("active_fraction").is_some(), "line {} missing active_fraction", i + 1);

        let iid = parsed["interval_id"].as_str().unwrap();
        assert!(iid.starts_with("int_"), "interval_id should start with int_: {}", iid);

        let start = parsed["start_s"].as_f64().unwrap();
        let end = parsed["end_s"].as_f64().unwrap();
        assert!(end > start, "end_s ({}) must be > start_s ({})", end, start);

        let dur = parsed["duration_s"].as_f64().unwrap();
        assert!((dur - (end - start)).abs() < 0.01, "duration_s mismatch");
    }
}

#[test]
fn intervals_requires_episodes() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::create_dir_all(ws.join("curation/v1")).unwrap();
    // No episodes.jsonl

    let result = egorec_viewer_lib::pipeline::run_intervals_stage(ws, &|_| {});
    assert!(result.is_err(), "should fail without episodes");
}

// ── Workspace timestamp updates ─────────────────────────────────────────────

#[test]
fn qc_updates_workspace_json_timestamps() {
    if !has_workspace("ctj") {
        eprintln!("SKIP: ctj workspace not present");
        return;
    }

    let ws = workspace_path("ctj");
    egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|_| {}).unwrap();

    let config_path = ws.join("curation/v1/workspace.json");
    assert!(config_path.exists());

    let contents = std::fs::read_to_string(&config_path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

    let timestamps = config.get("stageTimestamps").expect("should have stageTimestamps");
    assert!(timestamps.get("qc").is_some(), "should have qc timestamp");
}

// ── Progress callback ───────────────────────────────────────────────────────

#[test]
fn qc_emits_progress() {
    if !has_workspace("ctj") {
        eprintln!("SKIP: ctj workspace not present");
        return;
    }

    let ws = workspace_path("ctj");
    let progress = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let p = progress.clone();

    egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|evt| {
        p.lock().unwrap().push((evt.current, evt.total, evt.file.clone()));
    })
    .unwrap();

    let events = progress.lock().unwrap();
    assert!(!events.is_empty(), "should emit progress events");

    // Should include both calibration and analysis phases
    let has_calibrating = events.iter().any(|(_, _, f)| f.contains("Calibrating"));
    let has_analyzing = events.iter().any(|(_, _, f)| f.contains("Analyzing"));
    assert!(has_calibrating, "should have calibration progress");
    assert!(has_analyzing, "should have analysis progress");
}

// ── Python CLI argument construction ────────────────────────────────────────

#[test]
fn cli_args_workspace_before_subcommand() {
    let args = build_ego_curate_args(
        Path::new("/tmp/ws"),
        "label",
        "ego-qc",
        None,
        None,
    );

    // ego_curate --workspace /tmp/ws label
    assert_eq!(args[0], "-m");
    assert_eq!(args[1], "ego_curate");
    assert_eq!(args[2], "--workspace");
    assert_eq!(args[3], "/tmp/ws");
    assert_eq!(args[4], "label");
    assert_eq!(args.len(), 5, "label should NOT get --ego-qc");
}

#[test]
fn cli_args_ego_qc_only_for_applicable_stages() {
    for stage in &["qc", "intervals", "proxies", "segments"] {
        let args = build_ego_curate_args(
            Path::new("/tmp/ws"),
            stage,
            "/usr/bin/ego-qc",
            None,
            None,
        );
        assert!(
            args.contains(&"--ego-qc".to_string()),
            "{} should include --ego-qc",
            stage
        );
        let idx = args.iter().position(|a| a == "--ego-qc").unwrap();
        assert_eq!(args[idx + 1], "/usr/bin/ego-qc");
        // --ego-qc must come AFTER the stage name
        let stage_idx = args.iter().position(|a| a == *stage).unwrap();
        assert!(idx > stage_idx, "--ego-qc must be after stage name for {}", stage);
    }

    for stage in &["label", "cluster", "inventory", "stage", "publish", "materialize"] {
        let args = build_ego_curate_args(
            Path::new("/tmp/ws"),
            stage,
            "ego-qc",
            None,
            None,
        );
        assert!(
            !args.contains(&"--ego-qc".to_string()),
            "{} should NOT include --ego-qc",
            stage
        );
    }
}

#[test]
fn cli_args_source_prefix_after_stage() {
    let args = build_ego_curate_args(
        Path::new("/tmp/ws"),
        "stage",
        "ego-qc",
        Some("my-prefix"),
        None,
    );

    let stage_idx = args.iter().position(|a| a == "stage").unwrap();
    let prefix_idx = args.iter().position(|a| a == "--source-prefix").unwrap();
    assert!(prefix_idx > stage_idx, "--source-prefix must be after stage name");
    assert_eq!(args[prefix_idx + 1], "my-prefix");
}

#[test]
fn cli_args_no_qc_flag_name_collision() {
    // The old code used "--qc" which is wrong; verify we use "--ego-qc"
    let args = build_ego_curate_args(
        Path::new("/tmp/ws"),
        "qc",
        "/path/to/ego-qc",
        None,
        None,
    );
    assert!(
        !args.contains(&"--qc".to_string()),
        "should use --ego-qc, not --qc"
    );
    assert!(args.contains(&"--ego-qc".to_string()));
}

// ── Curation stream URL ─────────────────────────────────────────────────────

#[test]
fn curation_episodes_have_local_path() {
    if !has_workspace("ctj") {
        eprintln!("SKIP: ctj workspace not present");
        return;
    }

    let ws = workspace_path("ctj");

    // Ensure QC has run
    egorec_viewer_lib::pipeline::run_qc_stage(&ws, &|_| {}).unwrap();

    let episodes_path = ws.join("curation/v1/episodes.jsonl");
    let contents = std::fs::read_to_string(&episodes_path).unwrap();

    for (i, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).unwrap();

        let local_path = v
            .get("local_path")
            .and_then(|p| p.as_str())
            .unwrap_or_else(|| panic!("line {} missing local_path", i + 1));

        assert!(
            !local_path.is_empty(),
            "line {} has empty local_path",
            i + 1
        );

        assert!(
            std::path::Path::new(local_path).exists(),
            "line {} local_path does not exist: {}",
            i + 1,
            local_path
        );
    }
}
