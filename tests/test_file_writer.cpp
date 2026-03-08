#include <gtest/gtest.h>
#include "storage/file_writer.h"
#include "storage/binary_format.h"

#include <cstring>
#include <fstream>
#include <filesystem>
#include <vector>

namespace fs = std::filesystem;

class FileWriterTest : public ::testing::Test {
protected:
    std::string test_file;

    void SetUp() override {
        test_file = (fs::temp_directory_path() / "test_recording.egorec").string();
    }

    void TearDown() override {
        fs::remove(test_file);
    }

    FileHeader make_test_header() {
        FileHeader h{};
        std::memcpy(h.magic, FILE_MAGIC, 8);
        h.header_size = sizeof(FileHeader);
        h.flags = 0;
        h.depth_width = 640;
        h.depth_height = 480;
        h.color_width = 640;
        h.color_height = 480;
        h.depth_scale = 0.001f;
        h.rgb_codec = 2;    // H264
        h.depth_codec = 2;  // Zdepth
        h.rgb_quality = 23;
        std::strncpy(h.session_name, "test_session", sizeof(h.session_name));
        return h;
    }
};

TEST_F(FileWriterTest, CreateAndFinalizeEmpty) {
    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());
        writer.finalize();
        EXPECT_TRUE(writer.is_finalized());
    }

    // Verify file exists and has header + footer
    std::ifstream f(test_file, std::ios::binary | std::ios::ate);
    ASSERT_TRUE(f.is_open());
    auto file_size = f.tellg();
    EXPECT_GE(file_size, static_cast<std::streampos>(sizeof(FileHeader) + sizeof(FileFooter)));

    // Read footer
    f.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
    FileFooter footer{};
    f.read(reinterpret_cast<char*>(&footer), sizeof(footer));
    EXPECT_EQ(footer.index_magic, INDEX_MAGIC);
    EXPECT_EQ(footer.footer_magic, FOOTER_MAGIC);
    EXPECT_EQ(footer.total_frames, 0u);
    EXPECT_EQ(footer.index_entry_count, 0u);
}

TEST_F(FileWriterTest, WriteFramesAndVerifyIndex) {
    const int NUM_FRAMES = 10;
    std::vector<uint8_t> fake_rgb(100, 0xAB);
    std::vector<uint8_t> fake_depth(50, 0xCD);

    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());

        for (int i = 0; i < NUM_FRAMES; i++) {
            writer.write_frame(
                fake_rgb.data(), fake_rgb.size(),
                fake_depth.data(), fake_depth.size(),
                1000 * (i + 1),  // timestamp_us
                i,               // frame_number
                {});             // no IMU
        }
        writer.finalize();
    }

    // Read back footer
    std::ifstream f(test_file, std::ios::binary);
    ASSERT_TRUE(f.is_open());

    f.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
    FileFooter footer{};
    f.read(reinterpret_cast<char*>(&footer), sizeof(footer));

    EXPECT_EQ(footer.total_frames, static_cast<uint64_t>(NUM_FRAMES));
    EXPECT_EQ(footer.index_entry_count, static_cast<uint32_t>(NUM_FRAMES));
    EXPECT_EQ(footer.total_duration_us, 1000u * (NUM_FRAMES - 1));

    // Read index entries
    f.seekg(footer.index_offset, std::ios::beg);
    for (int i = 0; i < NUM_FRAMES; i++) {
        IndexEntry entry{};
        f.read(reinterpret_cast<char*>(&entry), sizeof(entry));
        EXPECT_EQ(entry.frame_number, static_cast<uint64_t>(i));
        EXPECT_EQ(entry.timestamp_us, 1000u * (i + 1));
        EXPECT_GT(entry.file_offset, 0u);
    }
}

TEST_F(FileWriterTest, WriteFrameWithIMU) {
    std::vector<uint8_t> fake_rgb(100, 0x01);
    std::vector<uint8_t> fake_depth(50, 0x02);
    std::vector<IMUSampleWire> imu(3);
    for (int i = 0; i < 3; i++) {
        imu[i].timestamp_us = 500 + i;
        imu[i].accel_x = static_cast<float>(i);
    }

    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());
        writer.write_frame(
            fake_rgb.data(), fake_rgb.size(),
            fake_depth.data(), fake_depth.size(),
            1000, 0, imu);
        writer.finalize();
    }

    // Verify frame block has correct IMU count and block size
    std::ifstream f(test_file, std::ios::binary);
    f.seekg(sizeof(FileHeader), std::ios::beg);

    FrameBlockHeader fbh{};
    f.read(reinterpret_cast<char*>(&fbh), sizeof(fbh));
    EXPECT_EQ(fbh.magic, FRAME_MAGIC);
    EXPECT_EQ(fbh.imu_sample_count, 3u);
    EXPECT_EQ(fbh.rgb_compressed_size, 100u);
    EXPECT_EQ(fbh.depth_compressed_size, 50u);

    uint32_t expected_block = sizeof(FrameBlockHeader) + 100 + 50 + 3 * sizeof(IMUSampleWire);
    EXPECT_EQ(fbh.block_size, expected_block);
}

TEST_F(FileWriterTest, WriteTrailingCodecData) {
    std::vector<uint8_t> fake_rgb(100, 0x01);
    std::vector<uint8_t> fake_depth(50, 0x02);
    std::vector<uint8_t> trailing(42, 0xFF);

    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());
        writer.write_frame(
            fake_rgb.data(), fake_rgb.size(),
            fake_depth.data(), fake_depth.size(),
            1000, 0, {});
        writer.write_trailing_codec_data(trailing.data(), trailing.size());
        writer.finalize();
    }

    // Trailing data should be between the last frame block and the index
    std::ifstream f(test_file, std::ios::binary | std::ios::ate);
    auto file_size = static_cast<uint64_t>(f.tellg());

    f.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
    FileFooter footer{};
    f.read(reinterpret_cast<char*>(&footer), sizeof(footer));

    // Read index to find end of last frame
    f.seekg(footer.index_offset, std::ios::beg);
    IndexEntry entry{};
    f.read(reinterpret_cast<char*>(&entry), sizeof(entry));

    // Read the frame block header to compute its end
    f.seekg(entry.file_offset, std::ios::beg);
    FrameBlockHeader fbh{};
    f.read(reinterpret_cast<char*>(&fbh), sizeof(fbh));
    uint64_t frame_end = entry.file_offset + fbh.block_size;

    // Trailing data sits between frame_end and index_offset
    uint64_t trailing_size = footer.index_offset - frame_end;
    EXPECT_EQ(trailing_size, 42u);

    // Verify trailing data content
    f.seekg(frame_end, std::ios::beg);
    std::vector<uint8_t> read_trailing(42);
    f.read(reinterpret_cast<char*>(read_trailing.data()), 42);
    EXPECT_EQ(read_trailing, trailing);
}

TEST_F(FileWriterTest, HeaderMagicWrittenCorrectly) {
    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());
        writer.finalize();
    }

    std::ifstream f(test_file, std::ios::binary);
    uint8_t magic[8];
    f.read(reinterpret_cast<char*>(magic), 8);
    EXPECT_EQ(std::memcmp(magic, FILE_MAGIC, 8), 0);
}

TEST_F(FileWriterTest, FinalizeIsIdempotent) {
    FileWriter writer(test_file);
    writer.write_header(make_test_header());
    writer.finalize();
    EXPECT_TRUE(writer.is_finalized());
    // Second finalize should not crash or corrupt
    writer.finalize();
    EXPECT_TRUE(writer.is_finalized());
}

TEST_F(FileWriterTest, DestructorAutoFinalizes) {
    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());
        // No explicit finalize -- destructor handles it
    }

    // File should still be valid
    std::ifstream f(test_file, std::ios::binary | std::ios::ate);
    EXPECT_GT(f.tellg(), static_cast<std::streampos>(0));
}

TEST_F(FileWriterTest, FrameSeekViaIndex) {
    std::vector<uint8_t> rgb1(100, 0x11);
    std::vector<uint8_t> rgb2(200, 0x22);
    std::vector<uint8_t> depth(50, 0xDD);

    {
        FileWriter writer(test_file);
        writer.write_header(make_test_header());
        writer.write_frame(rgb1.data(), rgb1.size(), depth.data(), depth.size(), 1000, 0, {});
        writer.write_frame(rgb2.data(), rgb2.size(), depth.data(), depth.size(), 2000, 1, {});
        writer.finalize();
    }

    // Use the index to seek directly to frame 1
    std::ifstream f(test_file, std::ios::binary);
    f.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
    FileFooter footer{};
    f.read(reinterpret_cast<char*>(&footer), sizeof(footer));

    // Read second index entry
    f.seekg(footer.index_offset + sizeof(IndexEntry), std::ios::beg);
    IndexEntry entry{};
    f.read(reinterpret_cast<char*>(&entry), sizeof(entry));
    EXPECT_EQ(entry.frame_number, 1u);
    EXPECT_EQ(entry.timestamp_us, 2000u);

    // Seek to that frame and verify
    f.seekg(entry.file_offset, std::ios::beg);
    FrameBlockHeader fbh{};
    f.read(reinterpret_cast<char*>(&fbh), sizeof(fbh));
    EXPECT_EQ(fbh.frame_number, 1u);
    EXPECT_EQ(fbh.rgb_compressed_size, 200u);
}
