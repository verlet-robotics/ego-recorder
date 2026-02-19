#pragma once

// Recording statistics tracker.
//
// All counters are std::atomic<uint64_t> for lock-free thread-safe access.
// The capture thread calls frame_captured()/frames_dropped(),
// the write thread calls frame_written()/bytes_written().
// The main thread reads the stats and calls summary() for display.

#include <atomic>
#include <chrono>
#include <cstdint>
#include <string>

class Stats {
public:
    /// Starts the elapsed timer at construction time.
    Stats();

    // Non-copyable (atomics are not copyable).
    Stats(const Stats&) = delete;
    Stats& operator=(const Stats&) = delete;

    // ---- Mutators (called from capture/write threads) ----

    /// Increment captured frame count by 1.
    void frame_captured();

    /// Increment written frame count by 1.
    void frame_written();

    /// Add \p count to the dropped frame counter.
    void frames_dropped(size_t count);

    /// Add \p bytes to the total bytes written counter.
    void bytes_written(size_t bytes);

    // ---- Accessors (const, thread-safe) ----

    uint64_t captured()    const;
    uint64_t written()     const;
    uint64_t dropped()     const;
    uint64_t total_bytes() const;

    /// Seconds elapsed since construction.
    double elapsed_seconds() const;

    /// Capture frames per second (captured / elapsed).
    double capture_fps() const;

    /// Write frames per second (written / elapsed).
    double write_fps() const;

    /// Formatted one-line summary string, e.g.:
    ///   "Frames: 1800 captured, 1800 written, 0 dropped | FPS: 30.0 | Written: 45.2 MB | Elapsed: 60.0s"
    std::string summary() const;

private:
    std::atomic<uint64_t> frames_captured_{0};
    std::atomic<uint64_t> frames_written_{0};
    std::atomic<uint64_t> frames_dropped_{0};
    std::atomic<uint64_t> bytes_written_{0};

    std::chrono::steady_clock::time_point start_time_;
};
