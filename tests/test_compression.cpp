#include <gtest/gtest.h>
#include "compression/jpeg_compressor.h"
#include "compression/zstd_compressor.h"
#include "compression/zdepth_compressor.h"
#include "compression/h264_encoder.h"

#include <vector>
#include <cstring>
#include <random>
#include <numeric>

// ---- JpegCompressor ---------------------------------------------------------

class JpegCompressorTest : public ::testing::Test {
protected:
    static constexpr int W = 640;
    static constexpr int H = 480;
    std::vector<uint8_t> rgb;

    void SetUp() override {
        rgb.resize(W * H * 3);
        // Gradient pattern
        for (int y = 0; y < H; y++) {
            for (int x = 0; x < W; x++) {
                int idx = (y * W + x) * 3;
                rgb[idx + 0] = static_cast<uint8_t>(x % 256);
                rgb[idx + 1] = static_cast<uint8_t>(y % 256);
                rgb[idx + 2] = 128;
            }
        }
    }
};

TEST_F(JpegCompressorTest, CompressProducesOutput) {
    JpegCompressor comp(W, H, 90);
    auto [data, size] = comp.compress(rgb.data(), W, H);
    EXPECT_NE(data, nullptr);
    EXPECT_GT(size, 0u);
    // JPEG should be smaller than raw
    EXPECT_LT(size, rgb.size());
}

TEST_F(JpegCompressorTest, CompressedDataStartsWithJpegMagic) {
    JpegCompressor comp(W, H, 90);
    auto [data, size] = comp.compress(rgb.data(), W, H);
    ASSERT_GE(size, 2u);
    EXPECT_EQ(data[0], 0xFF);
    EXPECT_EQ(data[1], 0xD8);  // JPEG SOI marker
}

TEST_F(JpegCompressorTest, HigherQualityProducesLargerOutput) {
    JpegCompressor low(W, H, 50);
    JpegCompressor high(W, H, 95);

    auto [data_low, size_low] = low.compress(rgb.data(), W, H);
    auto [data_high, size_high] = high.compress(rgb.data(), W, H);

    EXPECT_GT(size_high, size_low);
}

TEST_F(JpegCompressorTest, RepeatedCallsReuseBuffer) {
    JpegCompressor comp(W, H, 90);

    auto [data1, size1] = comp.compress(rgb.data(), W, H);
    size_t first_size = size1;

    // Second call should reuse internal buffer
    auto [data2, size2] = comp.compress(rgb.data(), W, H);
    EXPECT_EQ(size2, first_size);
}

// ---- ZstdCompressor ---------------------------------------------------------

class ZstdCompressorTest : public ::testing::Test {
protected:
    static constexpr size_t DEPTH_SIZE = 640 * 480 * 2;
    std::vector<uint8_t> depth_bytes;

    void SetUp() override {
        depth_bytes.resize(DEPTH_SIZE);
        // Simulate Z16 depth with realistic-ish values
        auto* ptr = reinterpret_cast<uint16_t*>(depth_bytes.data());
        for (size_t i = 0; i < DEPTH_SIZE / 2; i++) {
            ptr[i] = static_cast<uint16_t>(1000 + (i % 500));
        }
    }
};

TEST_F(ZstdCompressorTest, CompressProducesOutput) {
    ZstdCompressor comp(DEPTH_SIZE, 1);
    auto [data, size] = comp.compress(depth_bytes.data(), depth_bytes.size());
    EXPECT_NE(data, nullptr);
    EXPECT_GT(size, 0u);
    EXPECT_LT(size, depth_bytes.size());  // Should compress well
}

TEST_F(ZstdCompressorTest, HigherLevelCompressesBetter) {
    ZstdCompressor fast(DEPTH_SIZE, 1);
    ZstdCompressor better(DEPTH_SIZE, 9);

    auto [data1, size1] = fast.compress(depth_bytes.data(), depth_bytes.size());
    auto [data2, size2] = better.compress(depth_bytes.data(), depth_bytes.size());

    EXPECT_LE(size2, size1);  // Higher level should be same or smaller
}

TEST_F(ZstdCompressorTest, DecompressRoundTrip) {
    ZstdCompressor comp(DEPTH_SIZE, 1);
    auto [cdata, csize] = comp.compress(depth_bytes.data(), depth_bytes.size());

    // Decompress with zstd directly
    std::vector<uint8_t> decompressed(DEPTH_SIZE);
    size_t result = ZSTD_decompress(decompressed.data(), decompressed.size(), cdata, csize);
    ASSERT_FALSE(ZSTD_isError(result));
    EXPECT_EQ(result, DEPTH_SIZE);
    EXPECT_EQ(decompressed, depth_bytes);
}

// ---- ZdepthCompressor -------------------------------------------------------

class ZdepthCompressorTest : public ::testing::Test {
protected:
    static constexpr int W = 640;
    static constexpr int H = 480;
    std::vector<uint16_t> depth;

    void SetUp() override {
        depth.resize(W * H);
        // Simulate realistic depth: mostly 1-3 meters, some zeros (invalid)
        std::mt19937 rng(42);
        std::uniform_int_distribution<uint16_t> dist(800, 3000);
        for (auto& d : depth) {
            d = dist(rng);
        }
        // Add some invalid pixels
        depth[0] = 0;
        depth[100] = 0;
        depth[W * H - 1] = 0;
    }
};

TEST_F(ZdepthCompressorTest, CompressProducesOutput) {
    ZdepthCompressor comp(W, H);
    auto [data, size] = comp.compress(depth.data(), W, H, true);
    EXPECT_NE(data, nullptr);
    EXPECT_GT(size, 0u);
    EXPECT_LT(size, depth.size() * sizeof(uint16_t));
}

TEST_F(ZdepthCompressorTest, LosslessRoundTrip) {
    ZdepthCompressor comp(W, H);
    auto [cdata, csize] = comp.compress(depth.data(), W, H, true);

    auto decompressed = comp.decompress(cdata, csize);
    ASSERT_EQ(decompressed.size(), depth.size());

    for (size_t i = 0; i < depth.size(); i++) {
        EXPECT_EQ(decompressed[i], depth[i])
            << "Mismatch at pixel " << i
            << ": expected " << depth[i] << " got " << decompressed[i];
    }
}

TEST_F(ZdepthCompressorTest, KeyframeAndPFrameRoundTrip) {
    ZdepthCompressor comp(W, H);

    // Compress keyframe and immediately decompress to sync decompressor state
    auto [kdata, ksize] = comp.compress(depth.data(), W, H, true);
    // Copy compressed data since the pointer is invalidated by next compress()
    std::vector<uint8_t> kbuf(kdata, kdata + ksize);
    auto dec1 = comp.decompress(kbuf.data(), kbuf.size());
    EXPECT_EQ(dec1, depth);

    // Now compress a P-frame with slight changes
    std::vector<uint16_t> depth2 = depth;
    for (int i = 0; i < 100; i++) {
        depth2[i] = static_cast<uint16_t>(
            std::min(8191, static_cast<int>(depth2[i]) + 5));
    }
    auto [pdata, psize] = comp.compress(depth2.data(), W, H, false);
    std::vector<uint8_t> pbuf(pdata, pdata + psize);
    auto dec2 = comp.decompress(pbuf.data(), pbuf.size());
    EXPECT_EQ(dec2, depth2);

    // P-frame may be smaller than keyframe for similar data
    EXPECT_GT(ksize, 0u);
    EXPECT_GT(psize, 0u);
}

TEST_F(ZdepthCompressorTest, HandlesZeroDepthPixels) {
    // All zeros (invalid depth)
    std::vector<uint16_t> zeros(W * H, 0);
    ZdepthCompressor comp(W, H);
    auto [cdata, csize] = comp.compress(zeros.data(), W, H, true);
    auto decompressed = comp.decompress(cdata, csize);
    EXPECT_EQ(decompressed, zeros);
}

TEST_F(ZdepthCompressorTest, MultipleFrameSequence) {
    ZdepthCompressor comp(W, H);

    // Compress 30 frames (one full GOP) and decompress each
    for (int i = 0; i < 30; i++) {
        bool keyframe = (i == 0);
        // Slightly vary depth each frame
        std::vector<uint16_t> frame = depth;
        for (size_t j = 0; j < frame.size(); j++) {
            frame[j] = static_cast<uint16_t>(
                std::min(8191, static_cast<int>(frame[j]) + i));
        }

        auto [cdata, csize] = comp.compress(frame.data(), W, H, keyframe);
        auto dec = comp.decompress(cdata, csize);
        EXPECT_EQ(dec, frame) << "Mismatch at frame " << i;
    }
}

// ---- H264Encoder ------------------------------------------------------------

class H264EncoderTest : public ::testing::Test {
protected:
    static constexpr int W = 640;
    static constexpr int H = 480;
    std::vector<uint8_t> rgb;

    void SetUp() override {
        rgb.resize(W * H * 3);
        for (int y = 0; y < H; y++) {
            for (int x = 0; x < W; x++) {
                int idx = (y * W + x) * 3;
                rgb[idx + 0] = static_cast<uint8_t>(x % 256);
                rgb[idx + 1] = static_cast<uint8_t>(y % 256);
                rgb[idx + 2] = 128;
            }
        }
    }
};

TEST_F(H264EncoderTest, EncodeProducesOutput) {
    H264Encoder enc(W, H, 30, 23);

    size_t total_bytes = 0;
    // Encode enough frames that the encoder must produce output
    for (int i = 0; i < 60; i++) {
        auto [data, size] = enc.encode(rgb.data(), W, H);
        total_bytes += size;
    }
    auto [fdata, fsize] = enc.flush();
    total_bytes += fsize;

    EXPECT_GT(total_bytes, 0u);
}

TEST_F(H264EncoderTest, FlushDrainsBufferedFrames) {
    H264Encoder enc(W, H, 30, 23);

    for (int i = 0; i < 5; i++) {
        enc.encode(rgb.data(), W, H);
    }

    auto [fdata, fsize] = enc.flush();
    // Flush should produce at least some data
    EXPECT_GT(fsize, 0u);
}

TEST_F(H264EncoderTest, ResetAllowsNewSession) {
    H264Encoder enc(W, H, 30, 23);

    // First session
    for (int i = 0; i < 10; i++) {
        enc.encode(rgb.data(), W, H);
    }
    enc.flush();
    enc.reset();

    // Second session should work
    size_t total = 0;
    for (int i = 0; i < 30; i++) {
        auto [data, size] = enc.encode(rgb.data(), W, H);
        total += size;
    }
    auto [fdata, fsize] = enc.flush();
    total += fsize;
    EXPECT_GT(total, 0u);
}

TEST_F(H264EncoderTest, OutputContainsNALUnits) {
    H264Encoder enc(W, H, 30, 23);

    // Encode enough to get output
    std::vector<uint8_t> all_data;
    for (int i = 0; i < 60; i++) {
        auto [data, size] = enc.encode(rgb.data(), W, H);
        all_data.insert(all_data.end(), data, data + size);
    }
    auto [fdata, fsize] = enc.flush();
    all_data.insert(all_data.end(), fdata, fdata + fsize);

    // H.264 NAL start code: 0x00 0x00 0x00 0x01 or 0x00 0x00 0x01
    bool found_nal = false;
    for (size_t i = 0; i + 3 < all_data.size(); i++) {
        if (all_data[i] == 0 && all_data[i+1] == 0 &&
            (all_data[i+2] == 1 || (all_data[i+2] == 0 && all_data[i+3] == 1))) {
            found_nal = true;
            break;
        }
    }
    EXPECT_TRUE(found_nal) << "No NAL start codes found in H.264 output";
}

TEST_F(H264EncoderTest, HigherCRFProducesSmallerOutput) {
    size_t size_crf18 = 0;
    {
        H264Encoder enc(W, H, 30, 18);
        for (int i = 0; i < 30; i++) {
            auto [d, s] = enc.encode(rgb.data(), W, H);
            size_crf18 += s;
        }
        auto [fd, fs] = enc.flush();
        size_crf18 += fs;
    }

    size_t size_crf35 = 0;
    {
        H264Encoder enc(W, H, 30, 35);
        for (int i = 0; i < 30; i++) {
            auto [d, s] = enc.encode(rgb.data(), W, H);
            size_crf35 += s;
        }
        auto [fd, fs] = enc.flush();
        size_crf35 += fs;
    }

    EXPECT_LT(size_crf35, size_crf18);
}

TEST_F(H264EncoderTest, RejectsOddDimensions) {
    EXPECT_THROW(H264Encoder(641, 480, 30, 23), std::runtime_error);
    EXPECT_THROW(H264Encoder(640, 481, 30, 23), std::runtime_error);
}

TEST_F(H264EncoderTest, EncodeBufferPointerValidUntilNextCall) {
    H264Encoder enc(W, H, 30, 23);

    // First encode
    auto [data1, size1] = enc.encode(rgb.data(), W, H);
    if (size1 > 0) {
        uint8_t first_byte = data1[0];  // Must be readable

        // Second encode invalidates the first pointer, but data1 was valid before this
        auto [data2, size2] = enc.encode(rgb.data(), W, H);
        (void)data2;
        (void)size2;
        (void)first_byte;
    }
}
