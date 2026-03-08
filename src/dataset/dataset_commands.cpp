#include "dataset/dataset_commands.h"
#include "dataset/dataset_manifest.h"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <string>
#include <vector>

namespace fs = std::filesystem;

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

// ---- dataset init ----------------------------------------------------------

int cmd_dataset_init(int argc, char* argv[]) {
    // Parse: dataset init -o DIR --name NAME [--description DESC] [--tags t1,t2] [--force]
    std::string dir;
    std::string name;
    std::string description;
    std::string tags_str;
    bool force = false;

    for (int i = 0; i < argc; ++i) {
        std::string arg = argv[i];
        if ((arg == "-o" || arg == "--output") && i + 1 < argc) {
            dir = argv[++i];
        } else if (arg == "--name" && i + 1 < argc) {
            name = argv[++i];
        } else if (arg == "--description" && i + 1 < argc) {
            description = argv[++i];
        } else if (arg == "--tags" && i + 1 < argc) {
            tags_str = argv[++i];
        } else if (arg == "--force") {
            force = true;
        }
    }

    if (dir.empty() || name.empty()) {
        fprintf(stderr, "Usage: ego-recorder dataset init -o DIR --name NAME "
                "[--description DESC] [--tags t1,t2] [--force]\n");
        return 1;
    }

    // Create directory if needed
    std::error_code ec;
    fs::create_directories(dir, ec);
    if (ec) {
        fprintf(stderr, "Error: cannot create directory '%s': %s\n",
                dir.c_str(), ec.message().c_str());
        return 1;
    }

    if (has_manifest(dir) && !force) {
        fprintf(stderr, "Error: dataset.json already exists in '%s'. Use --force to overwrite.\n",
                dir.c_str());
        return 1;
    }

    // Parse comma-separated tags
    std::vector<std::string> tags;
    if (!tags_str.empty()) {
        size_t pos = 0;
        while (pos < tags_str.size()) {
            size_t comma = tags_str.find(',', pos);
            if (comma == std::string::npos) comma = tags_str.size();
            std::string tag = tags_str.substr(pos, comma - pos);
            // Trim whitespace
            while (!tag.empty() && tag.front() == ' ') tag.erase(tag.begin());
            while (!tag.empty() && tag.back() == ' ')  tag.pop_back();
            if (!tag.empty()) tags.push_back(tag);
            pos = comma + 1;
        }
    }

    DatasetManifest manifest;
    manifest.version     = 1;
    manifest.name        = name;
    manifest.description = description;
    manifest.tags        = tags;
    manifest.created     = iso8601_now();

    if (!save_manifest(dir, manifest)) {
        fprintf(stderr, "Error: failed to write dataset.json\n");
        return 1;
    }

    printf("Dataset created: %s/dataset.json\n", dir.c_str());
    printf("  Name: %s\n", name.c_str());
    if (!description.empty()) {
        printf("  Description: %s\n", description.c_str());
    }
    if (!tags.empty()) {
        printf("  Tags: ");
        for (size_t i = 0; i < tags.size(); ++i) {
            if (i > 0) printf(", ");
            printf("%s", tags[i].c_str());
        }
        printf("\n");
    }

    return 0;
}

// ---- dataset info ----------------------------------------------------------

int cmd_dataset_info(int argc, char* argv[]) {
    if (argc < 1) {
        fprintf(stderr, "Usage: ego-recorder dataset info DIR\n");
        return 1;
    }

    std::string dir = argv[0];

    DatasetManifest manifest;
    if (!load_manifest(dir, manifest)) {
        fprintf(stderr, "Error: no valid dataset.json in '%s'\n", dir.c_str());
        return 1;
    }

    printf("Dataset: %s\n", manifest.name.c_str());
    if (!manifest.description.empty()) {
        printf("  Description: %s\n", manifest.description.c_str());
    }
    if (!manifest.tags.empty()) {
        printf("  Tags: ");
        for (size_t i = 0; i < manifest.tags.size(); ++i) {
            if (i > 0) printf(", ");
            printf("%s", manifest.tags[i].c_str());
        }
        printf("\n");
    }
    printf("  Created: %s\n", manifest.created.c_str());
    printf("  Episodes: %zu\n", manifest.episodes.size());

    if (!manifest.episodes.empty()) {
        printf("\n");
        double total_duration = 0;
        uint64_t total_frames = 0;
        for (size_t i = 0; i < manifest.episodes.size(); ++i) {
            const auto& ep = manifest.episodes[i];
            printf("  [%zu] %s\n", i, ep.filename.c_str());
            printf("      Session: %s\n", ep.session_name.c_str());
            printf("      Recorded: %s\n", ep.recorded_at.c_str());
            printf("      Duration: %.2f s\n", ep.duration_s);
            printf("      Frames: %lu\n", static_cast<unsigned long>(ep.frames));
            total_duration += ep.duration_s;
            total_frames += ep.frames;
        }
        printf("\n  Total: %.2f s, %lu frames\n",
               total_duration, static_cast<unsigned long>(total_frames));
    }

    return 0;
}

// ---- dataset add -----------------------------------------------------------

int cmd_dataset_add(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: ego-recorder dataset add DIR file.egorec [...]\n");
        return 1;
    }

    std::string dir = argv[0];

    if (!has_manifest(dir)) {
        fprintf(stderr, "Error: no dataset.json in '%s'. Run 'dataset init' first.\n", dir.c_str());
        return 1;
    }

    int errors = 0;
    for (int i = 1; i < argc; ++i) {
        std::string path = argv[i];
        if (!fs::exists(path)) {
            fprintf(stderr, "Error: file not found: %s\n", path.c_str());
            errors++;
            continue;
        }
        if (!register_episode(dir, path)) {
            fprintf(stderr, "Error: failed to register %s\n", path.c_str());
            errors++;
        } else {
            printf("Registered: %s\n", path.c_str());
        }
    }

    return errors > 0 ? 1 : 0;
}

// ---- dataset remove --------------------------------------------------------

int cmd_dataset_remove(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: ego-recorder dataset remove DIR filename.egorec\n");
        return 1;
    }

    std::string dir = argv[0];
    std::string filename = argv[1];

    DatasetManifest manifest;
    if (!load_manifest(dir, manifest)) {
        fprintf(stderr, "Error: no valid dataset.json in '%s'\n", dir.c_str());
        return 1;
    }

    auto it = manifest.episodes.begin();
    bool found = false;
    while (it != manifest.episodes.end()) {
        if (it->filename == filename) {
            it = manifest.episodes.erase(it);
            found = true;
        } else {
            ++it;
        }
    }

    if (!found) {
        fprintf(stderr, "Error: episode '%s' not found in dataset\n", filename.c_str());
        return 1;
    }

    if (!save_manifest(dir, manifest)) {
        fprintf(stderr, "Error: failed to save dataset.json\n");
        return 1;
    }

    printf("Removed: %s\n", filename.c_str());
    return 0;
}
