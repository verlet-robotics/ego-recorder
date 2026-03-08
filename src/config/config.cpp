// config.cpp -- TOML configuration file loading via toml++.
//
// Uses value_or() for all reads so missing keys silently fall back to
// compiled-in defaults.  A toml::parse_error prints a warning to stderr
// and returns the default Config.

#include "config/config.h"

#include <toml++/toml.hpp>

#include <cstdio>

Config load_config(const std::string& path) {
    Config cfg;
    cfg.config_path = path;

    if (path.empty()) {
        return cfg;  // No path provided -- return defaults.
    }

    toml::table tbl;
    try {
        tbl = toml::parse_file(path);
    } catch (const toml::parse_error& e) {
        fprintf(stderr,
                "Warning: failed to parse config file '%s': %s -- using defaults.\n",
                path.c_str(), e.what());
        return cfg;
    }

    // [output]
    cfg.output_dir   = tbl["output"]["dir"].value_or(cfg.output_dir);
    cfg.session_name = tbl["output"]["session_name"].value_or(cfg.session_name);

    // [compression]
    cfg.jpeg_quality = tbl["compression"]["jpeg_quality"].value_or(cfg.jpeg_quality);
    cfg.zstd_level   = tbl["compression"]["zstd_level"].value_or(cfg.zstd_level);
    cfg.h264_crf     = tbl["compression"]["h264_crf"].value_or(cfg.h264_crf);

    // [camera]
    cfg.frame_width  = tbl["camera"]["width"].value_or(cfg.frame_width);
    cfg.frame_height = tbl["camera"]["height"].value_or(cfg.frame_height);

    // [recording]
    cfg.queue_size    = tbl["recording"]["queue_size"].value_or(cfg.queue_size);
    cfg.warmup_frames = tbl["recording"]["warmup_frames"].value_or(cfg.warmup_frames);
    cfg.disk_min_mb   = tbl["recording"]["disk_min_mb"].value_or(cfg.disk_min_mb);

    // [service]
    cfg.headless = tbl["service"]["headless"].value_or(cfg.headless);

    return cfg;
}
