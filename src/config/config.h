#pragma once

// Config -- application configuration loaded from a TOML file.
//
// All settings have sensible defaults so the recorder works out-of-the-box
// with no config file.  Pass a path to load_config() to override defaults.

#include <cstdint>
#include <string>

/// Application configuration.
struct Config {
    // [output]
    std::string output_dir   = ".";        ///< Directory for .egorec files
    std::string session_name = "capture";  ///< Prefix for output filenames

    // [compression]
    int jpeg_quality = 90;  ///< JPEG quality 1-100 (default 90)
    int zstd_level   = 1;   ///< ZSTD compression level 1-22 (default 1, fastest)
    int h264_crf     = 23;  ///< H.264 CRF quality 0-51 (default 23)
    std::string h264_preset = "ultrafast";  ///< x264 preset (ultrafast/superfast/veryfast/fast)

    // [camera]
    int frame_width  = 848;   ///< Capture resolution width  (must be multiple of 8)
    int frame_height = 480;   ///< Capture resolution height (must be multiple of 8)

    // [recording]
    int      queue_size    = 8;    ///< Bounded capture queue depth (2-16)
    int      warmup_frames = 30;   ///< Camera warmup frames to skip before recording
    uint64_t disk_min_mb   = 500;  ///< Stop recording when free disk drops below this (MB)

    // [service]
    bool headless = false;  ///< Run in headless/systemd mode (no GUI)

    // Internal -- path from which config was loaded (empty = defaults only)
    std::string config_path;
};

/// Load a Config from a TOML file at \p path.
///
/// Missing keys use defaults.  If the file is missing or malformed a warning
/// is printed to stderr and all defaults are returned.
Config load_config(const std::string& path);
