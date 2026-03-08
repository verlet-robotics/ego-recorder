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

    /// Call when recording starts to begin tracking recording-specific stats.
    void recording_started();

    /// Call when recording stops to freeze recording elapsed time.
    void recording_stopped();

    // ---- Accessors (const, thread-safe) ----

    uint64_t captured()    const;
    uint64_t written()     const;
    uint64_t dropped()     const;
    uint64_t total_bytes() const;
    bool     is_recording() const;

    /// Seconds elapsed since construction (total uptime).
    double elapsed_seconds() const;

    /// Seconds elapsed during recording only.
    double recording_elapsed_seconds() const;

    /// Capture frames per second (captured / uptime elapsed).
    double capture_fps() const;

    /// Write frames per second (written / recording elapsed).
    double write_fps() const;

    /// Formatted one-line summary string.
    /// When recording: "Recording: 1800 frames, 0 dropped | FPS: 30.0 | Size: 45.2 MB | Rec: 01:00"
    /// When idle:      "Idle | Camera FPS: 30.0 | Uptime: 120.0s"
    std::string summary() const;

private:
    std::atomic<uint64_t> frames_captured_{0};
    std::atomic<uint64_t> frames_written_{0};
    std::atomic<uint64_t> frames_dropped_{0};
    std::atomic<uint64_t> bytes_written_{0};
    std::atomic<bool>     recording_{false};

    std::chrono::steady_clock::time_point start_time_;
    std::atomic<uint64_t> rec_start_us_{0};
    std::atomic<uint64_t> rec_accumulated_us_{0};
};
