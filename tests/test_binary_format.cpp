#include <gtest/gtest.h>
#include "storage/binary_format.h"

#include <cstring>

TEST(BinaryFormat, StructSizesMatchWireFormat) {
    EXPECT_EQ(sizeof(IMUSampleWire), 32u);
    EXPECT_EQ(sizeof(IndexEntry), 24u);
    EXPECT_EQ(sizeof(FrameBlockHeader), 36u);
    EXPECT_EQ(sizeof(FileFooter), 36u);
}

TEST(BinaryFormat, FileMagicIsEgorecV2) {
    EXPECT_EQ(FILE_MAGIC[0], 'E');
    EXPECT_EQ(FILE_MAGIC[1], 'G');
    EXPECT_EQ(FILE_MAGIC[2], 'O');
    EXPECT_EQ(FILE_MAGIC[3], 'R');
    EXPECT_EQ(FILE_MAGIC[4], 'E');
    EXPECT_EQ(FILE_MAGIC[5], 'C');
    EXPECT_EQ(FILE_MAGIC[6], 0x02);  // version 2
    EXPECT_EQ(FILE_MAGIC[7], 0x00);
}

TEST(BinaryFormat, MagicConstants) {
    EXPECT_EQ(FRAME_MAGIC,  0x46524D45u);
    EXPECT_EQ(INDEX_MAGIC,  0x58444E49u);
    EXPECT_EQ(FOOTER_MAGIC, 0x454E4F44u);
}

TEST(BinaryFormat, FileHeaderFieldOffsetsArePacked) {
    // Verify no padding sneaks in -- header must be tightly packed
    FileHeader h{};
    auto base = reinterpret_cast<uintptr_t>(&h);
    // magic at offset 0
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&h.magic) - base, 0u);
    // header_size right after magic
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&h.header_size) - base, 8u);
    // flags after header_size
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&h.flags) - base, 12u);
}

TEST(BinaryFormat, FrameBlockHeaderPacking) {
    FrameBlockHeader fb{};
    auto base = reinterpret_cast<uintptr_t>(&fb);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.magic) - base, 0u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.block_size) - base, 4u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.timestamp_us) - base, 8u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.frame_number) - base, 16u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.rgb_compressed_size) - base, 24u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.depth_compressed_size) - base, 28u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.imu_sample_count) - base, 32u);
    EXPECT_EQ(reinterpret_cast<uintptr_t>(&fb.flags) - base, 34u);
}

TEST(BinaryFormat, CodecIdValues) {
    FileHeader h{};
    // RGB: 0=raw, 1=JPEG, 2=H264
    h.rgb_codec = 2;
    EXPECT_EQ(h.rgb_codec, 2);
    // Depth: 0=raw, 1=ZSTD, 2=Zdepth
    h.depth_codec = 2;
    EXPECT_EQ(h.depth_codec, 2);
}
