#pragma once

#include <cstdint>
#include <memory>
#include <utility>
#include <vector>

/// RAII wrapper around catid/Zdepth DepthCompressor.
///
/// Compresses Z16 depth frames using block-prediction + ZSTD for
/// significantly better compression than raw ZSTD on depth pixels.
/// Lossless round-trip (bit-exact) for D435 Z16 data.
/// Non-copyable, non-movable -- owns internal compressor state.
class ZdepthCompressor {
public:
    /// @param width   Depth frame width in pixels (e.g. 640)
    /// @param height  Depth frame height in pixels (e.g. 480)
    ZdepthCompressor(int width, int height);
    ~ZdepthCompressor();

    // Non-copyable, non-movable
    ZdepthCompressor(const ZdepthCompressor&) = delete;
    ZdepthCompressor& operator=(const ZdepthCompressor&) = delete;
    ZdepthCompressor(ZdepthCompressor&&) = delete;
    ZdepthCompressor& operator=(ZdepthCompressor&&) = delete;

    /// Compress a Z16 depth frame. Returns {pointer, size}.
    /// Pointer valid until next compress() call.
    /// @param depth     Pointer to Z16 depth data (width * height uint16 values)
    /// @param width     Frame width (must match constructor)
    /// @param height    Frame height (must match constructor)
    /// @param keyframe  If true, forces an I-frame (must be true for first frame
    ///                  and recommended every ~30 frames)
    std::pair<const uint8_t*, size_t> compress(const uint16_t* depth,
                                                int width, int height,
                                                bool keyframe);

    /// Decompress a Zdepth-compressed buffer back to Z16.
    /// @param data  Pointer to compressed data
    /// @param size  Size of compressed data in bytes
    /// @returns Vector of uint16_t (width * height elements)
    std::vector<uint16_t> decompress(const uint8_t* data, size_t size);

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
    int width_;
    int height_;
};
