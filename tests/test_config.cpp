#include <gtest/gtest.h>
#include "config/config.h"

#include <fstream>
#include <filesystem>

namespace fs = std::filesystem;

class ConfigTest : public ::testing::Test {
protected:
    std::string config_path;

    void SetUp() override {
        config_path = (fs::temp_directory_path() / "test_config.toml").string();
    }

    void TearDown() override {
        fs::remove(config_path);
    }

    void write_config(const std::string& content) {
        std::ofstream f(config_path);
        f << content;
    }
};

TEST_F(ConfigTest, DefaultsWhenNoFile) {
    Config cfg = load_config("");
    EXPECT_EQ(cfg.output_dir, ".");
    EXPECT_EQ(cfg.session_name, "capture");
    EXPECT_EQ(cfg.jpeg_quality, 90);
    EXPECT_EQ(cfg.zstd_level, 1);
    EXPECT_EQ(cfg.h264_crf, 23);
    EXPECT_EQ(cfg.queue_size, 4);
    EXPECT_EQ(cfg.warmup_frames, 30);
    EXPECT_EQ(cfg.disk_min_mb, 500u);
    EXPECT_FALSE(cfg.headless);
}

TEST_F(ConfigTest, LoadsAllFields) {
    write_config(R"(
[output]
dir = "/tmp/recordings"
session_name = "my_session"

[compression]
jpeg_quality = 80
zstd_level = 5
h264_crf = 28

[recording]
queue_size = 8
warmup_frames = 60
disk_min_mb = 2000

[service]
headless = true
)");

    Config cfg = load_config(config_path);
    EXPECT_EQ(cfg.output_dir, "/tmp/recordings");
    EXPECT_EQ(cfg.session_name, "my_session");
    EXPECT_EQ(cfg.jpeg_quality, 80);
    EXPECT_EQ(cfg.zstd_level, 5);
    EXPECT_EQ(cfg.h264_crf, 28);
    EXPECT_EQ(cfg.queue_size, 8);
    EXPECT_EQ(cfg.warmup_frames, 60);
    EXPECT_EQ(cfg.disk_min_mb, 2000u);
    EXPECT_TRUE(cfg.headless);
}

TEST_F(ConfigTest, PartialConfigUsesDefaults) {
    write_config(R"(
[compression]
h264_crf = 30
)");

    Config cfg = load_config(config_path);
    EXPECT_EQ(cfg.h264_crf, 30);
    // Everything else should be default
    EXPECT_EQ(cfg.output_dir, ".");
    EXPECT_EQ(cfg.jpeg_quality, 90);
    EXPECT_EQ(cfg.queue_size, 4);
    EXPECT_FALSE(cfg.headless);
}

TEST_F(ConfigTest, MalformedConfigReturnsDefaults) {
    write_config("this is not valid toml {{{");

    Config cfg = load_config(config_path);
    EXPECT_EQ(cfg.output_dir, ".");
    EXPECT_EQ(cfg.h264_crf, 23);
}

TEST_F(ConfigTest, MissingFileReturnsDefaults) {
    Config cfg = load_config("/nonexistent/path/config.toml");
    EXPECT_EQ(cfg.output_dir, ".");
    EXPECT_EQ(cfg.h264_crf, 23);
}

TEST_F(ConfigTest, StoresConfigPath) {
    write_config("[output]\ndir = \"/tmp\"");
    Config cfg = load_config(config_path);
    EXPECT_EQ(cfg.config_path, config_path);
}
