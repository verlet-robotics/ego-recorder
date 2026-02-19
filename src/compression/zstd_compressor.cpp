#include "zstd_compressor.h"

#include <stdexcept>
#include <string>

ZstdCompressor::ZstdCompressor(size_t max_input_size, int level)
    : level_(level)
{
    ctx_ = ZSTD_createCCtx();
    if (!ctx_) {
        throw std::runtime_error("ZSTD_createCCtx failed: out of memory");
    }

    // Pre-allocate the output buffer to the maximum possible compressed size.
    // ZSTD_compressBound gives a safe upper bound for any input of this size.
    buf_.resize(ZSTD_compressBound(max_input_size));
}

ZstdCompressor::~ZstdCompressor() {
    if (ctx_) {
        ZSTD_freeCCtx(ctx_);
        ctx_ = nullptr;
    }
}

std::pair<const uint8_t*, size_t>
ZstdCompressor::compress(const void* src, size_t src_size)
{
    // ZSTD_compressCCtx reuses the context -- no per-call allocation.
    size_t result = ZSTD_compressCCtx(
        ctx_,
        buf_.data(), buf_.size(),
        src, src_size,
        level_
    );

    if (ZSTD_isError(result)) {
        throw std::runtime_error(
            std::string("ZSTD_compressCCtx failed: ") + ZSTD_getErrorName(result));
    }

    return {buf_.data(), result};
}
