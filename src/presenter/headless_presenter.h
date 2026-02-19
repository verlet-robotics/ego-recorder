#pragma once

// HeadlessPresenter -- IPresenter implementation for systemd service operation.
//
// Designed for unattended headless recording on a closed laptop without a display.
// Implements the full IPresenter lifecycle while managing:
//   - systemd sd_notify watchdog (READY=1, WATCHDOG=1, STATUS=..., STOPPING=1)
//   - D-Bus inhibitor lock blocking handle-lid-switch:sleep
//   - Periodic disk space monitoring with configurable threshold
//   - Machine-readable JSON status file at /run/ego-recorder/status
//   - Camera disconnect/reconnect status reporting
//
// All systemd/sd-bus calls are gated behind HAVE_SYSTEMD so this class
// compiles and runs on non-systemd systems (without watchdog/inhibitor).

#include "presenter/ipresenter.h"
#include "config/config.h"

#include <atomic>
#include <cstdint>
#include <functional>
#include <string>
#include <thread>

#ifdef HAVE_SYSTEMD
#include <systemd/sd-bus.h>
#endif

class HeadlessPresenter : public IPresenter {
public:
    /// Construct a HeadlessPresenter.
    ///
    /// \param config           Application configuration (output_dir, disk_min_mb, etc.)
    /// \param on_request_shutdown  Callback invoked when disk space falls below threshold.
    ///                             Signals main.cpp to stop recording and exit cleanly.
    HeadlessPresenter(const Config& config,
                      std::function<void()> on_request_shutdown);

    ~HeadlessPresenter() override;

    // IPresenter interface

    /// Called once after camera and writer are initialized.
    /// Takes D-Bus inhibitor lock, starts watchdog thread, sends READY=1,
    /// writes initial status file.
    bool start() override;

    /// Called each iteration of the main loop.
    /// Checks disk space every ~30 ticks, updates status file, sleeps 100ms.
    /// Returns false when presenter wants to quit (disk full or shutdown requested).
    bool tick() override;

    /// Called on shutdown.  Joins watchdog thread, sends STOPPING=1,
    /// releases inhibitor lock, writes final status file.
    void shutdown() override;

    /// Logs camera disconnect to stderr and updates status file.
    void on_camera_disconnect() override;

    /// Logs camera reconnect to stderr and updates status file.
    void on_camera_reconnect() override;

    /// Caches latest stats for inclusion in STATUS= notifications and status file.
    void update_stats(const Stats& stats) override;

private:
    // ---- Private helpers ----

    /// Acquire D-Bus inhibitor lock blocking handle-lid-switch:sleep.
    /// Non-fatal if it fails (logind.conf drop-in is the fallback).
    bool take_inhibitor_lock();

    /// Release inhibitor lock fd and unref sd_bus.
    void release_inhibitor_lock();

    /// Write JSON status file to status_file_path_.
    /// Format: {"pid": N, "state": "...", "frames": N, "fps": N.N, "disk_free_mb": N, "timestamp": "ISO8601"}
    void write_status_file(const std::string& state);

    /// Query free disk space on config_.output_dir.
    /// Returns UINT64_MAX on error (treated as sufficient -- don't stop).
    uint64_t disk_free_mb();

    // ---- Configuration ----
    const Config& config_;
    std::function<void()> on_request_shutdown_;

    // ---- Watchdog thread ----
    std::thread watchdog_thread_;
    std::atomic<bool> shutdown_{false};

    // ---- D-Bus inhibitor lock ----
    int inhibitor_fd_{-1};

#ifdef HAVE_SYSTEMD
    sd_bus* bus_{nullptr};
#else
    void* bus_{nullptr};  // placeholder; never used without HAVE_SYSTEMD
#endif

    // ---- Status file ----
    std::string status_file_path_;

    // ---- Cached stats (updated by update_stats, read by watchdog thread) ----
    std::atomic<uint64_t> cached_frames_{0};
    std::atomic<uint64_t> cached_fps_x10_{0};  // FPS * 10 for lock-free storage

    // ---- Loop state ----
    uint64_t tick_counter_{0};
};
