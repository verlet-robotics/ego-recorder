#include <gtest/gtest.h>
#include "dataset/dataset_manifest.h"
#include "storage/file_writer.h"
#include "storage/binary_format.h"

#include <cstring>
#include <filesystem>
#include <fstream>

namespace fs = std::filesystem;

class DatasetManifestTest : public ::testing::Test {
protected:
    std::string test_dir;

    void SetUp() override {
        test_dir = (fs::temp_directory_path() / "test_dataset_manifest").string();
        fs::create_directories(test_dir);
    }

    void TearDown() override {
        fs::remove_all(test_dir);
    }

    FileHeader make_test_header(const char* session = "test_session") {
        FileHeader h{};
        std::memcpy(h.magic, FILE_MAGIC, 8);
        h.header_size = sizeof(FileHeader);
        h.flags = 0;
        h.depth_width = 640;
        h.depth_height = 480;
        h.color_width = 640;
        h.color_height = 480;
        h.depth_scale = 0.001f;
        h.rgb_codec = 2;
        h.depth_codec = 2;
        h.rgb_quality = 23;
        h.start_timestamp_us = 1709884800000000ULL;  // 2024-03-08 fixed timestamp
        std::strncpy(h.session_name, session, sizeof(h.session_name) - 1);
        return h;
    }

    /// Create a minimal .egorec file with header and footer (no frames).
    std::string create_test_egorec(const std::string& name) {
        std::string path = (fs::path(test_dir) / (name + ".egorec")).string();
        FileWriter writer(path);
        writer.write_header(make_test_header(name.c_str()));
        writer.finalize();
        return path;
    }
};

TEST_F(DatasetManifestTest, CreateAndLoad) {
    DatasetManifest m;
    m.name        = "test_dataset";
    m.description = "A test dataset";
    m.tags        = {"tag1", "tag2"};
    m.created     = "2026-03-08T10:00:00Z";

    ASSERT_TRUE(save_manifest(test_dir, m));
    EXPECT_TRUE(has_manifest(test_dir));

    DatasetManifest loaded;
    ASSERT_TRUE(load_manifest(test_dir, loaded));

    EXPECT_EQ(loaded.version, 1);
    EXPECT_EQ(loaded.name, "test_dataset");
    EXPECT_EQ(loaded.description, "A test dataset");
    ASSERT_EQ(loaded.tags.size(), 2u);
    EXPECT_EQ(loaded.tags[0], "tag1");
    EXPECT_EQ(loaded.tags[1], "tag2");
    EXPECT_EQ(loaded.created, "2026-03-08T10:00:00Z");
    EXPECT_TRUE(loaded.episodes.empty());
}

TEST_F(DatasetManifestTest, RegisterEpisode) {
    // Create manifest
    DatasetManifest m;
    m.name    = "test_ds";
    m.created = "2026-03-08T10:00:00Z";
    ASSERT_TRUE(save_manifest(test_dir, m));

    // Create .egorec file
    std::string egorec_path = create_test_egorec("episode1");

    // Register it
    ASSERT_TRUE(register_episode(test_dir, egorec_path));

    // Verify
    DatasetManifest loaded;
    ASSERT_TRUE(load_manifest(test_dir, loaded));
    ASSERT_EQ(loaded.episodes.size(), 1u);
    EXPECT_EQ(loaded.episodes[0].filename, "episode1.egorec");
    EXPECT_EQ(loaded.episodes[0].session_name, "episode1");
    // Empty recording has 0 frames -- just verify the field was populated from footer
    EXPECT_EQ(loaded.episodes[0].frames, 0u);
}

TEST_F(DatasetManifestTest, RegisterEpisodeIdempotent) {
    DatasetManifest m;
    m.name    = "test_ds";
    m.created = "2026-03-08T10:00:00Z";
    ASSERT_TRUE(save_manifest(test_dir, m));

    std::string egorec_path = create_test_egorec("episode1");

    // Register twice
    ASSERT_TRUE(register_episode(test_dir, egorec_path));
    ASSERT_TRUE(register_episode(test_dir, egorec_path));

    // Should still be one entry
    DatasetManifest loaded;
    ASSERT_TRUE(load_manifest(test_dir, loaded));
    EXPECT_EQ(loaded.episodes.size(), 1u);
}

TEST_F(DatasetManifestTest, NoManifestIsNoop) {
    // No dataset.json exists
    EXPECT_FALSE(has_manifest(test_dir));

    // register_episode should return true (no-op)
    std::string egorec_path = create_test_egorec("episode1");
    EXPECT_TRUE(register_episode(test_dir, egorec_path));
}

TEST_F(DatasetManifestTest, MalformedJsonWarns) {
    // Write garbage to dataset.json
    std::string path = (fs::path(test_dir) / "dataset.json").string();
    std::ofstream f(path);
    f << "not valid json {{{";
    f.close();

    EXPECT_TRUE(has_manifest(test_dir));

    DatasetManifest loaded;
    EXPECT_FALSE(load_manifest(test_dir, loaded));
}

TEST_F(DatasetManifestTest, SaveLoadWithEpisodes) {
    DatasetManifest m;
    m.name    = "roundtrip_test";
    m.created = "2026-03-08T12:00:00Z";

    EpisodeEntry ep;
    ep.filename     = "rec1.egorec";
    ep.session_name = "rec1";
    ep.recorded_at  = "2026-03-08T12:05:00Z";
    ep.duration_s   = 30.5;
    ep.frames       = 915;
    m.episodes.push_back(ep);

    ASSERT_TRUE(save_manifest(test_dir, m));

    DatasetManifest loaded;
    ASSERT_TRUE(load_manifest(test_dir, loaded));
    ASSERT_EQ(loaded.episodes.size(), 1u);
    EXPECT_EQ(loaded.episodes[0].filename, "rec1.egorec");
    EXPECT_EQ(loaded.episodes[0].session_name, "rec1");
    EXPECT_EQ(loaded.episodes[0].recorded_at, "2026-03-08T12:05:00Z");
    EXPECT_DOUBLE_EQ(loaded.episodes[0].duration_s, 30.5);
    EXPECT_EQ(loaded.episodes[0].frames, 915u);
}
