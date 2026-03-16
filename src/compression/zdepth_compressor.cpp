#include "compression/zdepth_compressor.h"

#include <cassert>
#include <stdexcept>
#include <zdepth.hpp>

/// Pimpl to isolate Zdepth includes from the header.
struct ZdepthCompressor::Impl {
    zdepth::DepthCompressor compressor;
    zdepth::DepthCompressor decompressor;
    std::vector<uint8_t> compressed_buf;
};

ZdepthCompressor::ZdepthCompressor(int width, int height)
    : impl_(std::make_unique<Impl>())
    , width_(width)
    , height_(height)
{
    // Zdepth requires dimensions to be multiples of kBlockSize (8).
    if (width % zdepth::kBlockSize != 0 || height % zdepth::kBlockSize != 0) {
        throw std::runtime_error(
            "ZdepthCompressor: width (" + std::to_string(width) +
            ") and height (" + std::to_string(height) +
            ") must be multiples of " + std::to_string(zdepth::kBlockSize));
    }

    // Use kNotQuantized8191mm for D435: covers 0-8191mm range without
    // quantization loss. D435 max range is ~10m but typical indoor use
    // is well within 8m. Values >= 8192mm are clipped to 0 (invalid).
    impl_->compressor.set_encode_mode(zdepth::EncodeMode::kNotQuantized8191mm);

    // Keyframe every 30 frames (~1s at 30fps) when using auto-gop Compress().
    impl_->compressor.set_gop(30);
}

ZdepthCompressor::~ZdepthCompressor() = default;

std::pair<const uint8_t*, size_t> ZdepthCompressor::compress(
    const uint16_t* depth, int width, int height, bool keyframe)
{
    if (width != width_ || height != height_) {
        throw std::runtime_error(
            "ZdepthCompressor::compress: dimension mismatch (expected " +
            std::to_string(width_) + "x" + std::to_string(height_) +
            ", got " + std::to_string(width) + "x" + std::to_string(height) + ")");
    }
    if (depth == nullptr) {
        throw std::runtime_error("ZdepthCompressor::compress: null depth pointer");
    }

    impl_->compressed_buf.clear();

    zdepth::DepthResult result = impl_->compressor.Compress(
        width, height, depth, impl_->compressed_buf, keyframe);

    if (result != zdepth::DepthResult::Success) {
        throw std::runtime_error(
            std::string("Zdepth compress failed: ") +
            zdepth::DepthResultString(result));
    }

    return {impl_->compressed_buf.data(), impl_->compressed_buf.size()};
}

std::vector<uint16_t> ZdepthCompressor::decompress(
    const uint8_t* data, size_t size)
{
    if (data == nullptr) {
        throw std::runtime_error("ZdepthCompressor::decompress: null data pointer");
    }
    if (size == 0) {
        throw std::runtime_error("ZdepthCompressor::decompress: empty compressed data");
    }

    // Zdepth Decompress takes a const reference to std::vector<uint8_t>
    std::vector<uint8_t> compressed_vec(data, data + size);

    int out_width = 0;
    int out_height = 0;
    std::vector<uint16_t> depth_out;

    zdepth::DepthResult result = impl_->decompressor.Decompress(
        compressed_vec, out_width, out_height, depth_out);

    if (result != zdepth::DepthResult::Success) {
        throw std::runtime_error(
            std::string("Zdepth decompress failed: ") +
            zdepth::DepthResultString(result));
    }

    return depth_out;
}
