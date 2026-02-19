#pragma once

#include <cstdint>
#include <stdexcept>
#include <vector>
#include <zstd.h>
#include <utility>

/// RAII wrapper around a reusable ZSTD compression context.
///
/// Pre-allocates the output buffer to ZSTD_compressBound(max_input_size)
/// at construction. ZSTD_compressCCtx reuses the context across calls,
/// eliminating per-frame allocation overhead.
/// Non-copyable, non-movable -- owns a C context pointer.
class ZstdCompressor {
public:
    /// @param max_input_size  Maximum byte size of any single input buffer
    ///                        (used to pre-allocate the output buffer)
    /// @param level           Compression level 1-22 (default 1 = fastest)
    ZstdCompressor(size_t max_input_size, int level = 1);
    ~ZstdCompressor();

    // Non-copyable, non-movable
    ZstdCompressor(const ZstdCompressor&) = delete;
    ZstdCompressor& operator=(const ZstdCompressor&) = delete;
    ZstdCompressor(ZstdCompressor&&) = delete;
    ZstdCompressor& operator=(ZstdCompressor&&) = delete;

    /// Compress arbitrary binary data.
    ///
    /// @param src      Pointer to input data
    /// @param src_size Number of bytes to compress
    /// @returns {pointer to compressed data, compressed size in bytes}
    ///          The pointer remains valid until the next call to compress().
    std::pair<const uint8_t*, size_t> compress(const void* src, size_t src_size);

private:
    ZSTD_CCtx*           ctx_{nullptr};
    std::vector<uint8_t> buf_;
    int                  level_;
};
