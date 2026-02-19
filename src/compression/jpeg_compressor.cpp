#include "jpeg_compressor.h"

#include <stdexcept>
#include <string>

JpegCompressor::JpegCompressor(int width, int height, int quality)
    : quality_(quality), width_(width), height_(height)
{
    handle_ = tjInitCompress();
    if (!handle_) {
        throw std::runtime_error(
            std::string("tjInitCompress failed: ") + tjGetErrorStr());
    }

    // Pre-allocate output buffer large enough for worst-case compressed size.
    // Using TJSAMP_420 (4:2:0 chroma subsampling) for ~30% smaller output vs 4:4:4.
    buf_size_ = tjBufSize(width_, height_, TJSAMP_420);
    buf_ = static_cast<unsigned char*>(tjAlloc(static_cast<int>(buf_size_)));
    if (!buf_) {
        tjDestroy(handle_);
        handle_ = nullptr;
        throw std::runtime_error("tjAlloc failed: out of memory");
    }
}

JpegCompressor::~JpegCompressor() {
    if (buf_) {
        tjFree(buf_);
        buf_ = nullptr;
    }
    if (handle_) {
        tjDestroy(handle_);
        handle_ = nullptr;
    }
}

std::pair<const uint8_t*, size_t>
JpegCompressor::compress(const uint8_t* rgb, int width, int height)
{
    unsigned long compressed_size = buf_size_;
    unsigned char* out_buf = buf_;

    // TJFLAG_NOREALLOC: never reallocate buf_ -- we own it and it's pre-sized.
    // TJFLAG_FASTDCT: use fast but slightly less accurate DCT.
    // TJSAMP_420: 4:2:0 chroma subsampling -- adequate for ML training data.
    int ret = tjCompress2(
        handle_,
        rgb,
        width,
        0,           // pitch = 0 means width * pixel_size (tightly packed)
        height,
        TJPF_RGB,
        &out_buf,
        &compressed_size,
        TJSAMP_420,
        quality_,
        TJFLAG_FASTDCT | TJFLAG_NOREALLOC
    );

    if (ret != 0) {
        throw std::runtime_error(
            std::string("tjCompress2 failed: ") + tjGetErrorStr2(handle_));
    }

    return {buf_, static_cast<size_t>(compressed_size)};
}
