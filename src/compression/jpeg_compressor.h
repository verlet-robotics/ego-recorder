#pragma once

#include <cstdint>
#include <stdexcept>
#include <turbojpeg.h>
#include <utility>

/// RAII wrapper around libjpeg-turbo TurboJPEG compress handle.
///
/// Pre-allocates the output buffer at construction time and uses
/// TJFLAG_NOREALLOC to guarantee zero per-frame heap allocations.
/// Non-copyable, non-movable -- owns a C handle.
class JpegCompressor {
public:
    /// @param width    Frame width in pixels (e.g. 640)
    /// @param height   Frame height in pixels (e.g. 480)
    /// @param quality  JPEG quality 1-100 (default 90)
    JpegCompressor(int width, int height, int quality = 90);
    ~JpegCompressor();

    // Non-copyable, non-movable
    JpegCompressor(const JpegCompressor&) = delete;
    JpegCompressor& operator=(const JpegCompressor&) = delete;
    JpegCompressor(JpegCompressor&&) = delete;
    JpegCompressor& operator=(JpegCompressor&&) = delete;

    /// Compress an RGB24 frame.
    ///
    /// @param rgb    Pointer to RGB24 pixel buffer (width * height * 3 bytes)
    /// @param width  Width of this frame (must match constructor width)
    /// @param height Height of this frame (must match constructor height)
    /// @returns {pointer to compressed data, compressed size in bytes}
    ///          The pointer remains valid until the next call to compress().
    std::pair<const uint8_t*, size_t> compress(const uint8_t* rgb, int width, int height);

private:
    tjhandle      handle_{nullptr};
    unsigned char* buf_{nullptr};
    unsigned long  buf_size_{0};
    int            quality_;
    int            width_;
    int            height_;
};
