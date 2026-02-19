// Temporary verification main -- replaced by real main.cpp in Plan 04.
// Tests: JpegCompressor, ZstdCompressor, BoundedQueue<int>.

#include "compression/jpeg_compressor.h"
#include "compression/zstd_compressor.h"
#include "threading/bounded_queue.h"

#include <cstdint>
#include <cstdio>
#include <thread>
#include <vector>

int main() {
    // ---- JPEG compressor ------------------------------------------------
    {
        constexpr int W = 640, H = 480;
        constexpr size_t RGB_BYTES = static_cast<size_t>(W) * H * 3;

        JpegCompressor jpeg(W, H, 90);

        // Synthetic gradient RGB
        std::vector<uint8_t> rgb(RGB_BYTES);
        for (size_t i = 0; i < RGB_BYTES; ++i) {
            rgb[i] = static_cast<uint8_t>(i % 256);
        }

        auto [data, size] = jpeg.compress(rgb.data(), W, H);
        double ratio = static_cast<double>(RGB_BYTES) / static_cast<double>(size);
        std::printf("JPEG: input=%zu bytes, compressed=%zu bytes, ratio=%.2fx\n",
                    RGB_BYTES, size, ratio);
    }

    // ---- ZSTD compressor ------------------------------------------------
    {
        constexpr int W = 640, H = 480;
        constexpr size_t Z16_BYTES = static_cast<size_t>(W) * H * 2;

        ZstdCompressor zstd(Z16_BYTES, 1);

        // Synthetic depth data (16-bit values with low entropy pattern)
        std::vector<uint8_t> depth(Z16_BYTES);
        for (size_t i = 0; i < Z16_BYTES; i += 2) {
            uint16_t val = static_cast<uint16_t>(1000 + (i / 2) % 500);
            depth[i]     = static_cast<uint8_t>(val & 0xFF);
            depth[i + 1] = static_cast<uint8_t>(val >> 8);
        }

        auto [data, size] = zstd.compress(depth.data(), Z16_BYTES);
        double ratio = static_cast<double>(Z16_BYTES) / static_cast<double>(size);
        std::printf("ZSTD: input=%zu bytes, compressed=%zu bytes, ratio=%.2fx\n",
                    Z16_BYTES, size, ratio);
    }

    // ---- BoundedQueue drop-oldest ---------------------------------------
    {
        BoundedQueue<int> q(4);

        // Push 6 items into a queue that holds 4 -- oldest 2 should be dropped
        for (int i = 0; i < 6; ++i) {
            q.push(i);
        }

        // Pop all 4 remaining items
        int count = 0;
        q.close();
        while (auto item = q.pop()) {
            ++count;
        }

        std::printf("BoundedQueue: pushed=6, capacity=4, dropped=%zu, popped=%d\n",
                    q.dropped(), count);
    }

    return 0;
}
