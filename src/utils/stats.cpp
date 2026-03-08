// Recording statistics tracker -- implementation.

#include "utils/stats.h"

#include <cstdio>
#include <algorithm>

static uint64_t now_us() {
    auto now = std::chrono::steady_clock::now();
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(
            now.time_since_epoch()).count());
}

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

void Stats::recording_started() {
    // Reset per-episode counters so each recording shows its own stats
    frames_captured_.store(0, std::memory_order_relaxed);
    frames_written_.store(0, std::memory_order_relaxed);
    frames_dropped_.store(0, std::memory_order_relaxed);
    bytes_written_.store(0, std::memory_order_relaxed);
    rec_accumulated_us_.store(0, std::memory_order_relaxed);
    rec_start_us_.store(now_us(), std::memory_order_relaxed);
    recording_.store(true, std::memory_order_release);
}

void Stats::recording_stopped() {
    recording_.store(false, std::memory_order_release);
    uint64_t start = rec_start_us_.load(std::memory_order_relaxed);
    uint64_t elapsed = now_us() - start;
    rec_accumulated_us_.fetch_add(elapsed, std::memory_order_relaxed);
}

// ---- Accessors --------------------------------------------------------------

uint64_t Stats::captured()    const { return frames_captured_.load(std::memory_order_relaxed); }
uint64_t Stats::written()     const { return frames_written_.load(std::memory_order_relaxed); }
uint64_t Stats::dropped()     const { return frames_dropped_.load(std::memory_order_relaxed); }
uint64_t Stats::total_bytes() const { return bytes_written_.load(std::memory_order_relaxed); }
bool     Stats::is_recording() const { return recording_.load(std::memory_order_acquire); }

double Stats::elapsed_seconds() const {
    auto now = std::chrono::steady_clock::now();
    auto dur = std::chrono::duration_cast<std::chrono::microseconds>(now - start_time_);
    return static_cast<double>(dur.count()) / 1e6;
}

double Stats::recording_elapsed_seconds() const {
    uint64_t acc = rec_accumulated_us_.load(std::memory_order_relaxed);
    if (recording_.load(std::memory_order_acquire)) {
        uint64_t start = rec_start_us_.load(std::memory_order_relaxed);
        acc += now_us() - start;
    }
    return static_cast<double>(acc) / 1e6;
}

double Stats::capture_fps() const {
    double elapsed = elapsed_seconds();
    if (elapsed < 1e-6) return 0.0;
    return static_cast<double>(captured()) / elapsed;
}

double Stats::write_fps() const {
    double elapsed = recording_elapsed_seconds();
    if (elapsed < 1e-6) return 0.0;
    return static_cast<double>(written()) / elapsed;
}

std::string Stats::summary() const {
    uint64_t wrt     = written();
    uint64_t drop    = dropped();
    uint64_t bytes   = total_bytes();
    double   cap_fps = capture_fps();

    char buf[256];

    if (is_recording()) {
        double rec_s   = recording_elapsed_seconds();
        double wrt_fps = write_fps();
        double mb      = static_cast<double>(bytes) / (1024.0 * 1024.0);
        int    rec_min = static_cast<int>(rec_s) / 60;
        int    rec_sec = static_cast<int>(rec_s) % 60;

        std::snprintf(buf, sizeof(buf),
            "REC %02d:%02d | Frames: %llu written, %llu dropped | FPS: %.1f cap / %.1f write | Size: %.1f MB",
            rec_min, rec_sec,
            (unsigned long long)wrt,
            (unsigned long long)drop,
            cap_fps,
            wrt_fps,
            mb);
    } else if (wrt > 0) {
        // Idle but has previous recording data
        double mb      = static_cast<double>(bytes) / (1024.0 * 1024.0);
        double rec_s   = recording_elapsed_seconds();
        int    rec_min = static_cast<int>(rec_s) / 60;
        int    rec_sec = static_cast<int>(rec_s) % 60;

        std::snprintf(buf, sizeof(buf),
            "Idle | Last rec: %llu frames in %02d:%02d, %.1f MB | Camera FPS: %.1f",
            (unsigned long long)wrt,
            rec_min, rec_sec,
            mb,
            cap_fps);
    } else {
        std::snprintf(buf, sizeof(buf),
            "Idle | Camera FPS: %.1f",
            cap_fps);
    }

    return std::string(buf);
}
