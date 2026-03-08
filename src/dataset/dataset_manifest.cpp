#include "dataset/dataset_manifest.h"
#include "storage/binary_format.h"

#include <nlohmann/json.hpp>

#include <chrono>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <fstream>

namespace fs = std::filesystem;
using json = nlohmann::json;

// ---- JSON serialisation ----------------------------------------------------

static json episode_to_json(const EpisodeEntry& e) {
    return json{
        {"filename",     e.filename},
        {"session_name", e.session_name},
        {"recorded_at",  e.recorded_at},
        {"duration_s",   e.duration_s},
        {"frames",       e.frames},
    };
}

static EpisodeEntry episode_from_json(const json& j) {
    EpisodeEntry e;
    e.filename     = j.value("filename", "");
    e.session_name = j.value("session_name", "");
    e.recorded_at  = j.value("recorded_at", "");
    e.duration_s   = j.value("duration_s", 0.0);
    e.frames       = j.value("frames", uint64_t{0});
    return e;
}

static json manifest_to_json(const DatasetManifest& m) {
    json episodes = json::array();
    for (const auto& ep : m.episodes) {
        episodes.push_back(episode_to_json(ep));
    }
    return json{
        {"version",     m.version},
        {"name",        m.name},
        {"description", m.description},
        {"tags",        m.tags},
        {"created",     m.created},
        {"episodes",    episodes},
    };
}

static DatasetManifest manifest_from_json(const json& j) {
    DatasetManifest m;
    m.version     = j.value("version", 1);
    m.name        = j.value("name", "");
    m.description = j.value("description", "");
    m.tags        = j.value("tags", std::vector<std::string>{});
    m.created     = j.value("created", "");

    if (j.contains("episodes") && j["episodes"].is_array()) {
        for (const auto& ej : j["episodes"]) {
            m.episodes.push_back(episode_from_json(ej));
        }
    }
    return m;
}

// ---- Helpers ---------------------------------------------------------------

static std::string manifest_path(const std::string& dir) {
    fs::path p = fs::path(dir) / "dataset.json";
    return p.string();
}

/// ISO 8601 timestamp for current UTC time.
static std::string iso8601_now() {
    auto now = std::chrono::system_clock::now();
    std::time_t tt = std::chrono::system_clock::to_time_t(now);
    std::tm utc{};
    gmtime_r(&tt, &utc);
    char buf[32];
    std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%SZ", &utc);
    return buf;
}

/// ISO 8601 timestamp from microseconds since epoch.
static std::string iso8601_from_us(uint64_t us) {
    std::time_t sec = static_cast<std::time_t>(us / 1'000'000ULL);
    std::tm utc{};
    gmtime_r(&sec, &utc);
    char buf[32];
    std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%SZ", &utc);
    return buf;
}

// ---- Public API ------------------------------------------------------------

bool has_manifest(const std::string& dir) {
    return fs::exists(manifest_path(dir));
}

bool load_manifest(const std::string& dir, DatasetManifest& out) {
    std::string path = manifest_path(dir);
    std::ifstream f(path);
    if (!f.is_open()) return false;

    try {
        json j = json::parse(f);
        out = manifest_from_json(j);
        return true;
    } catch (const json::exception& e) {
        fprintf(stderr, "Warning: failed to parse %s: %s\n", path.c_str(), e.what());
        return false;
    }
}

bool save_manifest(const std::string& dir, const DatasetManifest& manifest) {
    std::string path = manifest_path(dir);
    std::string tmp_path = path + ".tmp";

    // Atomic write: write to .tmp then rename
    {
        std::ofstream f(tmp_path);
        if (!f.is_open()) {
            fprintf(stderr, "Error: cannot write %s\n", tmp_path.c_str());
            return false;
        }
        f << manifest_to_json(manifest).dump(2) << '\n';
        f.flush();
        if (!f.good()) {
            fprintf(stderr, "Error: write failed for %s\n", tmp_path.c_str());
            return false;
        }
    }

    std::error_code ec;
    fs::rename(tmp_path, path, ec);
    if (ec) {
        fprintf(stderr, "Error: rename %s -> %s: %s\n",
                tmp_path.c_str(), path.c_str(), ec.message().c_str());
        return false;
    }
    return true;
}

bool register_episode(const std::string& dataset_dir, const std::string& egorec_path) {
    if (!has_manifest(dataset_dir)) {
        return true;  // No manifest = no-op (backwards compatible)
    }

    DatasetManifest manifest;
    if (!load_manifest(dataset_dir, manifest)) {
        fprintf(stderr, "Warning: could not load dataset.json in %s\n", dataset_dir.c_str());
        return false;
    }

    // Compute relative path from dataset_dir to the .egorec file
    fs::path abs_egorec = fs::absolute(egorec_path);
    fs::path abs_dir    = fs::absolute(dataset_dir);
    std::string rel_path = fs::relative(abs_egorec, abs_dir).string();

    // Idempotent: skip if already registered
    for (const auto& ep : manifest.episodes) {
        if (ep.filename == rel_path) {
            return true;
        }
    }

    // Read .egorec header and footer
    std::ifstream in(egorec_path, std::ios::binary);
    if (!in.is_open()) {
        fprintf(stderr, "Warning: cannot open %s for registration\n", egorec_path.c_str());
        return false;
    }

    FileHeader header{};
    in.read(reinterpret_cast<char*>(&header), sizeof(header));
    if (!in.good() || std::memcmp(header.magic, FILE_MAGIC, 6) != 0) {
        fprintf(stderr, "Warning: %s is not a valid .egorec file\n", egorec_path.c_str());
        return false;
    }

    // Read footer
    in.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
    FileFooter footer{};
    in.read(reinterpret_cast<char*>(&footer), sizeof(footer));
    bool has_footer = in.good() && footer.footer_magic == FOOTER_MAGIC;

    EpisodeEntry entry;
    entry.filename     = rel_path;
    entry.session_name = header.session_name;
    entry.recorded_at  = iso8601_from_us(header.start_timestamp_us);
    entry.duration_s   = has_footer ? (footer.total_duration_us / 1e6) : 0.0;
    entry.frames       = has_footer ? footer.total_frames : 0;

    manifest.episodes.push_back(entry);
    return save_manifest(dataset_dir, manifest);
}
