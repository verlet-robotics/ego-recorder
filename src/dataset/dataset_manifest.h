#pragma once

// Dataset manifest -- sidecar JSON metadata for grouping .egorec recordings.
//
// A dataset is a named collection of .egorec episodes stored in a single
// directory. The manifest lives at {dataset_dir}/dataset.json and tracks
// episode metadata (filename, session name, duration, frame count).
//
// Fully backwards compatible: recording to a directory without a manifest
// works exactly as before. register_episode() is a no-op when no manifest exists.

#include <cstdint>
#include <string>
#include <vector>

/// One episode (recording) within the dataset.
struct EpisodeEntry {
    std::string filename;       ///< Relative path from dataset dir
    std::string session_name;
    std::string recorded_at;    ///< ISO 8601
    double      duration_s{0};
    uint64_t    frames{0};
};

/// Top-level dataset metadata + episode list.
struct DatasetManifest {
    int                      version{1};
    std::string              name;
    std::string              description;
    std::vector<std::string> tags;
    std::string              created;  ///< ISO 8601
    std::vector<EpisodeEntry> episodes;
};

/// Load dataset.json from \p dir. Returns false if missing or malformed.
bool load_manifest(const std::string& dir, DatasetManifest& out);

/// Save dataset.json to \p dir. Uses atomic write (tmp + rename).
bool save_manifest(const std::string& dir, const DatasetManifest& manifest);

/// Check if \p dir contains a dataset.json.
bool has_manifest(const std::string& dir);

/// Read .egorec header+footer, append episode to manifest.
/// No-op if \p dataset_dir has no dataset.json (returns true).
/// Idempotent: skips if filename already in episodes list.
bool register_episode(const std::string& dataset_dir, const std::string& egorec_path);
