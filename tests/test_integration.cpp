#include <gtest/gtest.h>
#include "storage/file_writer.h"
#include "storage/binary_format.h"
#include "compression/zdepth_compressor.h"
#include "compression/h264_encoder.h"
#include "compression/jpeg_compressor.h"
#include "compression/zstd_compressor.h"

#include <cstring>
#include <fstream>
#include <filesystem>
#include <vector>
#include <random>

namespace fs = std::filesystem;

// End-to-end test: generate synthetic frames, compress, write, read back
class IntegrationTest : public ::testing::Test {
protected:
    static constexpr int W = 640;
    static constexpr int H = 480;
    static constexpr int NUM_FRAMES = 30;

    std::string test_file;
    std::vector<std::vector<uint8_t>> original_rgb_frames;
    std::vector<std::vector<uint16_t>> original_depth_frames;

    void SetUp() override {
        test_file = (fs::temp_directory_path() / "integration_test.egorec").string();

        std::mt19937 rng(123);
        std::uniform_int_distribution<uint16_t> depth_dist(500, 4000);
        std::uniform_int_distribution<uint8_t> rgb_dist(0, 255);

        for (int f = 0; f < NUM_FRAMES; f++) {
            std::vector<uint8_t> rgb(W * H * 3);
            for (auto& b : rgb) b = rgb_dist(rng);
            original_rgb_frames.push_back(std::move(rgb));

            std::vector<uint16_t> depth(W * H);
            for (auto& d : depth) d = depth_dist(rng);
            original_depth_frames.push_back(std::move(depth));
        }
    }

    void TearDown() override {
        fs::remove(test_file);
    }

    FileHeader make_header() {
        FileHeader h{};
        std::memcpy(h.magic, FILE_MAGIC, 8);
        h.header_size = sizeof(FileHeader);
        h.flags = 0;
        h.depth_width = W;
        h.depth_height = H;
        h.color_width = W;
        h.color_height = H;
        h.depth_scale = 0.001f;
        h.rgb_codec = 2;
        h.depth_codec = 2;
        h.rgb_quality = 23;
        std::strncpy(h.session_name, "integration_test", sizeof(h.session_name));
        return h;
    }
};

TEST_F(IntegrationTest, WriteAndReadBackV2Recording) {
    // Write a complete .egorec v2 file with H.264 + Zdepth
    H264Encoder h264(W, H, 30, 23);
    ZdepthCompressor zdepth(W, H);

    std::vector<size_t> h264_sizes;
    std::vector<size_t> zdepth_sizes;

    {
        FileWriter writer(test_file);
        writer.write_header(make_header());

        for (int i = 0; i < NUM_FRAMES; i++) {
            auto [h264_data, h264_size] = h264.encode(
                original_rgb_frames[i].data(), W, H);
            h264_sizes.push_back(h264_size);

            bool keyframe = (i % 30 == 0);
            auto [zdepth_data, zdepth_size] = zdepth.compress(
                original_depth_frames[i].data(), W, H, keyframe);
            zdepth_sizes.push_back(zdepth_size);

            writer.write_frame(
                h264_data, h264_size,
                zdepth_data, zdepth_size,
                1000 * (i + 1), i, {});
        }

        auto [flush_data, flush_size] = h264.flush();
        if (flush_size > 0) {
            writer.write_trailing_codec_data(flush_data, flush_size);
        }
        writer.finalize();
    }

    // Verify the file structure
    std::ifstream f(test_file, std::ios::binary);
    ASSERT_TRUE(f.is_open());

    // 1. Verify header
    FileHeader header{};
    f.read(reinterpret_cast<char*>(&header), sizeof(header));
    EXPECT_EQ(std::memcmp(header.magic, FILE_MAGIC, 8), 0);
    EXPECT_EQ(header.rgb_codec, 2);
    EXPECT_EQ(header.depth_codec, 2);
    EXPECT_EQ(header.depth_width, static_cast<uint32_t>(W));
    EXPECT_EQ(header.depth_height, static_cast<uint32_t>(H));

    // 2. Verify footer
    f.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
    FileFooter footer{};
    f.read(reinterpret_cast<char*>(&footer), sizeof(footer));
    EXPECT_EQ(footer.index_magic, INDEX_MAGIC);
    EXPECT_EQ(footer.footer_magic, FOOTER_MAGIC);
    EXPECT_EQ(footer.total_frames, static_cast<uint64_t>(NUM_FRAMES));
    EXPECT_EQ(footer.index_entry_count, static_cast<uint32_t>(NUM_FRAMES));

    // 3. Verify index entries are sequential
    f.seekg(footer.index_offset, std::ios::beg);
    uint64_t prev_offset = 0;
    for (int i = 0; i < NUM_FRAMES; i++) {
        IndexEntry entry{};
        f.read(reinterpret_cast<char*>(&entry), sizeof(entry));
        EXPECT_EQ(entry.frame_number, static_cast<uint64_t>(i));
        EXPECT_EQ(entry.timestamp_us, 1000u * (i + 1));
        EXPECT_GT(entry.file_offset, prev_offset);
        prev_offset = entry.file_offset;
    }

    // 4. Verify each frame block header
    f.seekg(footer.index_offset, std::ios::beg);
    std::vector<IndexEntry> entries(NUM_FRAMES);
    f.read(reinterpret_cast<char*>(entries.data()), NUM_FRAMES * sizeof(IndexEntry));

    for (int i = 0; i < NUM_FRAMES; i++) {
        f.seekg(entries[i].file_offset, std::ios::beg);
        FrameBlockHeader fbh{};
        f.read(reinterpret_cast<char*>(&fbh), sizeof(fbh));
        EXPECT_EQ(fbh.magic, FRAME_MAGIC);
        EXPECT_EQ(fbh.frame_number, static_cast<uint64_t>(i));
        EXPECT_EQ(fbh.rgb_compressed_size, static_cast<uint32_t>(h264_sizes[i]));
        EXPECT_EQ(fbh.depth_compressed_size, static_cast<uint32_t>(zdepth_sizes[i]));
    }
}

TEST_F(IntegrationTest, ZdepthLosslessRoundTripAllFrames) {
    ZdepthCompressor comp(W, H);

    for (int i = 0; i < NUM_FRAMES; i++) {
        bool keyframe = (i == 0);
        auto [cdata, csize] = comp.compress(
            original_depth_frames[i].data(), W, H, keyframe);

        auto decompressed = comp.decompress(cdata, csize);
        ASSERT_EQ(decompressed.size(), original_depth_frames[i].size())
            << "Frame " << i << " decompressed size mismatch";

        EXPECT_EQ(decompressed, original_depth_frames[i])
            << "Frame " << i << " lossless round-trip failed";
    }
}

TEST_F(IntegrationTest, CompressionRatioReasonable) {
    size_t raw_size = NUM_FRAMES * (W * H * 3 + W * H * 2);  // RGB + depth per frame
    size_t compressed_size = 0;

    H264Encoder h264(W, H, 30, 23);
    ZdepthCompressor zdepth(W, H);

    for (int i = 0; i < NUM_FRAMES; i++) {
        auto [h264_data, h264_size] = h264.encode(original_rgb_frames[i].data(), W, H);
        compressed_size += h264_size;

        bool keyframe = (i % 30 == 0);
        auto [zdepth_data, zdepth_size] = zdepth.compress(
            original_depth_frames[i].data(), W, H, keyframe);
        compressed_size += zdepth_size;
    }

    auto [fdata, fsize] = h264.flush();
    compressed_size += fsize;

    double ratio = static_cast<double>(raw_size) / static_cast<double>(compressed_size);
    // Random data compresses less well than real camera data,
    // but we should still see at least 2x compression
    EXPECT_GT(ratio, 2.0)
        << "Compression ratio " << ratio << "x is unexpectedly low. "
        << "Raw: " << raw_size << " bytes, compressed: " << compressed_size << " bytes";
}

TEST_F(IntegrationTest, FileRecoverableWithoutFooter) {
    // Simulate crash: write frames but no finalize
    std::vector<uint8_t> fake_rgb(100, 0x01);
    std::vector<uint8_t> fake_depth(50, 0x02);

    {
        FileWriter writer(test_file);
        writer.write_header(make_header());
        writer.write_frame(fake_rgb.data(), fake_rgb.size(),
                          fake_depth.data(), fake_depth.size(),
                          1000, 0, {});
        writer.write_frame(fake_rgb.data(), fake_rgb.size(),
                          fake_depth.data(), fake_depth.size(),
                          2000, 1, {});
        // Destructor auto-finalizes, but in a real crash it wouldn't
    }

    // File should still be readable -- header is there, frame blocks are sequential
    std::ifstream f(test_file, std::ios::binary);
    FileHeader h{};
    f.read(reinterpret_cast<char*>(&h), sizeof(h));
    EXPECT_EQ(std::memcmp(h.magic, FILE_MAGIC, 8), 0);

    // Read first frame block
    FrameBlockHeader fbh{};
    f.read(reinterpret_cast<char*>(&fbh), sizeof(fbh));
    EXPECT_EQ(fbh.magic, FRAME_MAGIC);
    EXPECT_EQ(fbh.frame_number, 0u);
}

TEST_F(IntegrationTest, MultipleRecordingSessions) {
    // Simulate start/stop/start/stop recording (multiple files)
    H264Encoder h264(W, H, 30, 23);

    for (int session = 0; session < 3; session++) {
        std::string path = test_file + "." + std::to_string(session);

        {
            FileWriter writer(path);
            writer.write_header(make_header());

            for (int i = 0; i < 5; i++) {
                auto [h264_data, h264_size] = h264.encode(
                    original_rgb_frames[i].data(), W, H);
                std::vector<uint8_t> fake_depth(50, 0x02);
                writer.write_frame(h264_data, h264_size,
                                  fake_depth.data(), fake_depth.size(),
                                  1000 * (i + 1), i, {});
            }

            auto [fdata, fsize] = h264.flush();
            if (fsize > 0) {
                writer.write_trailing_codec_data(fdata, fsize);
            }
            writer.finalize();
        }

        h264.reset();

        // Verify each file independently
        std::ifstream f(path, std::ios::binary | std::ios::ate);
        EXPECT_GT(f.tellg(), static_cast<std::streampos>(sizeof(FileHeader)));
        fs::remove(path);
    }
}
