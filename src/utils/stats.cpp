// Recording statistics tracker -- implementation.

#include "utils/stats.h"

#include <cstdio>
#include <algorithm>

Stats::Stats()
    : start_time_(std::chrono::steady_clock::now())
{}

// ---- Mutators ---------------------------------------------------------------

void Stats::frame_captured() {
    frames_captured_.fetch_add(1, std::memory_order_relaxed);
}

void Stats::frame_written() {
    frames_written_.fetch_add(1, std::memory_order_relaxed);
}

void Stats::frames_dropped(size_t count) {
    frames_dropped_.fetch_add(static_cast<uint64_t>(count), std::memory_order_relaxed);
}

void Stats::bytes_written(size_t bytes) {
    bytes_written_.fetch_add(static_cast<uint64_t>(bytes), std::memory_order_relaxed);
}

// ---- Accessors --------------------------------------------------------------

uint64_t Stats::captured()    const { return frames_captured_.load(std::memory_order_relaxed); }
uint64_t Stats::written()     const { return frames_written_.load(std::memory_order_relaxed); }
uint64_t Stats::dropped()     const { return frames_dropped_.load(std::memory_order_relaxed); }
uint64_t Stats::total_bytes() const { return bytes_written_.load(std::memory_order_relaxed); }

double Stats::elapsed_seconds() const {
    auto now = std::chrono::steady_clock::now();
    auto dur = std::chrono::duration_cast<std::chrono::microseconds>(now - start_time_);
    return static_cast<double>(dur.count()) / 1e6;
}

double Stats::capture_fps() const {
    double elapsed = elapsed_seconds();
    if (elapsed < 1e-6) return 0.0;
    return static_cast<double>(captured()) / elapsed;
}

double Stats::write_fps() const {
    double elapsed = elapsed_seconds();
    if (elapsed < 1e-6) return 0.0;
    return static_cast<double>(written()) / elapsed;
}

std::string Stats::summary() const {
    uint64_t cap     = captured();
    uint64_t wrt     = written();
    uint64_t drop    = dropped();
    uint64_t bytes   = total_bytes();
    double   fps     = capture_fps();
    double   elapsed = elapsed_seconds();

    // Convert bytes to MB for readability.
    double mb = static_cast<double>(bytes) / (1024.0 * 1024.0);

    char buf[256];
    std::snprintf(buf, sizeof(buf),
        "Frames: %llu captured, %llu written, %llu dropped | FPS: %.1f | Written: %.1f MB | Elapsed: %.1fs",
        (unsigned long long)cap,
        (unsigned long long)wrt,
        (unsigned long long)drop,
        fps,
        mb,
        elapsed);
    return std::string(buf);
}
