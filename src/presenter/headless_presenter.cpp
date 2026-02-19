// headless_presenter.cpp -- HeadlessPresenter implementation.
//
// Implements IPresenter for unattended systemd service operation.
// Key features:
//   - sd_notify lifecycle: READY after camera open, WATCHDOG at half-interval,
//     STATUS with live stats, STOPPING before exit
//   - D-Bus inhibitor lock via sd-bus blocking handle-lid-switch:sleep
//   - Disk space monitoring stopping recording below config_.disk_min_mb
//   - Machine-readable JSON status file at /run/ego-recorder/status
//   - Graceful degradation when HAVE_SYSTEMD is not defined

#include "presenter/headless_presenter.h"

#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <thread>
#include <unistd.h>

#ifdef HAVE_SYSTEMD
#include <systemd/sd-bus.h>
#include <systemd/sd-daemon.h>
#endif

// ---------------------------------------------------------------------------
// Constructor / Destructor
// ---------------------------------------------------------------------------

HeadlessPresenter::HeadlessPresenter(const Config& config,
                                     std::function<void()> on_request_shutdown)
    : config_(config)
    , on_request_shutdown_(std::move(on_request_shutdown))
    , status_file_path_("/run/ego-recorder/status")
{
}

HeadlessPresenter::~HeadlessPresenter()
{
    // Ensure cleanup even if shutdown() was not called explicitly.
    if (!shutdown_.load()) {
        shutdown_.store(true);
        if (watchdog_thread_.joinable()) {
            watchdog_thread_.join();
        }
        release_inhibitor_lock();
    }
}

// ---------------------------------------------------------------------------
// IPresenter::start()
// ---------------------------------------------------------------------------

bool HeadlessPresenter::start()
{
    // 1. Take D-Bus inhibitor lock (non-fatal if it fails).
    if (!take_inhibitor_lock()) {
        fprintf(stderr, "[headless] Warning: D-Bus inhibitor lock failed -- "
                        "lid-close will rely on logind.conf drop-in fallback.\n");
    }

    // 2. Start watchdog thread (no-op if systemd watchdog is not configured).
#ifdef HAVE_SYSTEMD
    uint64_t interval_us = 0;
    int enabled = sd_watchdog_enabled(0, &interval_us);
    if (enabled > 0 && interval_us > 0) {
        uint64_t ping_interval_us = interval_us / 2;  // Per spec: ping at half-interval

        watchdog_thread_ = std::thread([this, ping_interval_us]() {
            while (!shutdown_.load()) {
                sd_notify(0, "WATCHDOG=1");

                uint64_t frames = cached_frames_.load();
                double fps = static_cast<double>(cached_fps_x10_.load()) / 10.0;
                uint64_t free_mb = disk_free_mb();

                if (free_mb == UINT64_MAX) {
                    sd_notifyf(0, "STATUS=Frames: %llu | FPS: %.1f | Disk: unknown",
                               (unsigned long long)frames, fps);
                } else {
                    sd_notifyf(0, "STATUS=Frames: %llu | FPS: %.1f | Free: %llu MB",
                               (unsigned long long)frames, fps,
                               (unsigned long long)free_mb);
                }

                std::this_thread::sleep_for(
                    std::chrono::microseconds(ping_interval_us));
            }
        });
    } else {
        fprintf(stderr, "[headless] Note: systemd watchdog not configured "
                        "(WatchdogSec not set or not running under systemd).\n");
    }

    // 3. Send READY=1 -- camera is confirmed open and recording started.
    //    (start() is only called by main.cpp after camera init succeeds.)
    sd_notify(0, "READY=1");
#else
    fprintf(stderr, "[headless] Note: built without systemd support "
                    "(HAVE_SYSTEMD not defined) -- watchdog and READY=1 disabled.\n");
#endif

    // 4. Write initial status file.
    write_status_file("recording");

    fprintf(stderr, "[headless] Headless mode started. Status: %s\n",
            status_file_path_.c_str());
    return true;
}

// ---------------------------------------------------------------------------
// IPresenter::tick()
// ---------------------------------------------------------------------------

bool HeadlessPresenter::tick()
{
    if (shutdown_.load()) {
        return false;
    }

    ++tick_counter_;

    // Check disk space and update status file every 30 ticks (~1s at 30fps).
    if (tick_counter_ % 30 == 0) {
        uint64_t free_mb = disk_free_mb();

        // Disk full: stop recording.
        if (free_mb != UINT64_MAX && free_mb < config_.disk_min_mb) {
            fprintf(stderr,
                    "[headless] Disk space below threshold (%llu MB free < %llu MB required). "
                    "Stopping recording.\n",
                    (unsigned long long)free_mb,
                    (unsigned long long)config_.disk_min_mb);

#ifdef HAVE_SYSTEMD
            sd_notifyf(0, "STATUS=Stopping -- disk space below %llu MB threshold",
                       (unsigned long long)config_.disk_min_mb);
#endif
            // Signal main.cpp to stop recording cleanly.
            on_request_shutdown_();
            shutdown_.store(true);
            return false;
        }

        write_status_file("recording");
    }

    // Headless mode sleeps briefly to avoid busy-wait (unlike GUI which is vsync-limited).
    std::this_thread::sleep_for(std::chrono::milliseconds(100));

    return !shutdown_.load();
}

// ---------------------------------------------------------------------------
// IPresenter::shutdown()
// ---------------------------------------------------------------------------

void HeadlessPresenter::shutdown()
{
    shutdown_.store(true);

    // Join watchdog thread before sending STOPPING=1.
    if (watchdog_thread_.joinable()) {
        watchdog_thread_.join();
    }

#ifdef HAVE_SYSTEMD
    sd_notify(0, "STOPPING=1");
#endif

    // Release inhibitor lock (allows lid-close and sleep to resume).
    release_inhibitor_lock();

    // Final status file update.
    write_status_file("stopped");

    fprintf(stderr, "[headless] Headless mode stopped.\n");
}

// ---------------------------------------------------------------------------
// IPresenter::on_camera_disconnect()
// ---------------------------------------------------------------------------

void HeadlessPresenter::on_camera_disconnect()
{
    fprintf(stderr, "[headless] Camera disconnected, waiting for reconnect...\n");
    write_status_file("disconnected");

#ifdef HAVE_SYSTEMD
    sd_notify(0, "STATUS=Camera disconnected - waiting for reconnect");
#endif
}

// ---------------------------------------------------------------------------
// IPresenter::on_camera_reconnect()
// ---------------------------------------------------------------------------

void HeadlessPresenter::on_camera_reconnect()
{
    fprintf(stderr, "[headless] Camera reconnected.\n");
    write_status_file("recording");

#ifdef HAVE_SYSTEMD
    sd_notify(0, "STATUS=Camera reconnected - recording resumed");
#endif
}

// ---------------------------------------------------------------------------
// IPresenter::update_stats()
// ---------------------------------------------------------------------------

void HeadlessPresenter::update_stats(const Stats& stats)
{
    cached_frames_.store(stats.written());
    // Store FPS * 10 as integer for lock-free atomic storage.
    auto fps_x10 = static_cast<uint64_t>(stats.write_fps() * 10.0);
    cached_fps_x10_.store(fps_x10);
}

// ---------------------------------------------------------------------------
// Private: take_inhibitor_lock()
// ---------------------------------------------------------------------------

bool HeadlessPresenter::take_inhibitor_lock()
{
#ifdef HAVE_SYSTEMD
    int r = sd_bus_open_system(&bus_);
    if (r < 0) {
        fprintf(stderr, "[headless] sd_bus_open_system failed: %s\n", strerror(-r));
        return false;
    }

    sd_bus_message* reply = nullptr;
    sd_bus_error error = SD_BUS_ERROR_NULL;

    r = sd_bus_call_method(
        bus_,
        "org.freedesktop.login1",           // service
        "/org/freedesktop/login1",          // object path
        "org.freedesktop.login1.Manager",   // interface
        "Inhibit",                          // method
        &error,                             // error out
        &reply,                             // reply out
        "ssss",                             // signature: 4 strings in
        "handle-lid-switch:sleep",          // what to block
        "ego-recorder",                     // who (app name)
        "Recording in progress",            // why
        "block"                             // mode
    );

    if (r < 0) {
        fprintf(stderr, "[headless] D-Bus Inhibit call failed: %s\n",
                error.message ? error.message : strerror(-r));
        sd_bus_error_free(&error);
        return false;
    }

    // Extract the returned file descriptor.
    int raw_fd = -1;
    r = sd_bus_message_read(reply, "h", &raw_fd);
    if (r < 0 || raw_fd < 0) {
        fprintf(stderr, "[headless] Failed to read inhibitor fd from reply.\n");
        sd_bus_message_unref(reply);
        sd_bus_error_free(&error);
        return false;
    }

    // dup() the fd BEFORE unref-ing the reply message (critical -- research pitfall 5).
    // The fd inside the reply is owned by the message object; unref closes it.
    inhibitor_fd_ = dup(raw_fd);
    sd_bus_message_unref(reply);
    sd_bus_error_free(&error);

    if (inhibitor_fd_ < 0) {
        fprintf(stderr, "[headless] dup() of inhibitor fd failed: %s\n", strerror(errno));
        return false;
    }

    fprintf(stderr, "[headless] D-Bus inhibitor lock acquired (fd=%d). "
                    "Blocking handle-lid-switch:sleep.\n", inhibitor_fd_);
    return true;
#else
    return false;
#endif
}

// ---------------------------------------------------------------------------
// Private: release_inhibitor_lock()
// ---------------------------------------------------------------------------

void HeadlessPresenter::release_inhibitor_lock()
{
    if (inhibitor_fd_ >= 0) {
        close(inhibitor_fd_);
        inhibitor_fd_ = -1;
        fprintf(stderr, "[headless] D-Bus inhibitor lock released.\n");
    }

#ifdef HAVE_SYSTEMD
    if (bus_) {
        sd_bus_unref(bus_);
        bus_ = nullptr;
    }
#endif
}

// ---------------------------------------------------------------------------
// Private: write_status_file()
// ---------------------------------------------------------------------------

void HeadlessPresenter::write_status_file(const std::string& state)
{
    // Get current timestamp in ISO 8601 format.
    std::time_t now = std::time(nullptr);
    char ts_buf[32];
    std::strftime(ts_buf, sizeof(ts_buf), "%Y-%m-%dT%H:%M:%SZ", std::gmtime(&now));

    uint64_t frames = cached_frames_.load();
    double fps = static_cast<double>(cached_fps_x10_.load()) / 10.0;
    uint64_t free_mb = disk_free_mb();
    pid_t pid = getpid();

    // Build the JSON string.
    char json_buf[512];
    if (free_mb == UINT64_MAX) {
        snprintf(json_buf, sizeof(json_buf),
                 "{\"pid\":%d,\"state\":\"%s\",\"frames\":%llu,"
                 "\"fps\":%.1f,\"disk_free_mb\":null,\"timestamp\":\"%s\"}\n",
                 (int)pid, state.c_str(),
                 (unsigned long long)frames, fps, ts_buf);
    } else {
        snprintf(json_buf, sizeof(json_buf),
                 "{\"pid\":%d,\"state\":\"%s\",\"frames\":%llu,"
                 "\"fps\":%.1f,\"disk_free_mb\":%llu,\"timestamp\":\"%s\"}\n",
                 (int)pid, state.c_str(),
                 (unsigned long long)frames, fps,
                 (unsigned long long)free_mb, ts_buf);
    }

    // Ensure parent directory exists.
    std::error_code ec;
    auto dir = std::filesystem::path(status_file_path_).parent_path();
    std::filesystem::create_directories(dir, ec);
    // Ignore ec -- if we can't create the dir, the write will fail harmlessly.

    // Write atomically: write to temp file, then rename.
    std::string tmp_path = status_file_path_ + ".tmp";
    FILE* f = fopen(tmp_path.c_str(), "w");
    if (f) {
        fputs(json_buf, f);
        fclose(f);
        // Atomic rename -- best effort, ignore error.
        std::filesystem::rename(tmp_path, status_file_path_, ec);
    }
    // If write fails (e.g., /run/ego-recorder doesn't exist), silently ignore.
    // Status file is a convenience feature, not critical to operation.
}

// ---------------------------------------------------------------------------
// Private: disk_free_mb()
// ---------------------------------------------------------------------------

uint64_t HeadlessPresenter::disk_free_mb()
{
    std::error_code ec;
    auto si = std::filesystem::space(config_.output_dir, ec);
    if (ec) {
        // Can't check disk space -- don't stop recording.
        return UINT64_MAX;
    }
    return si.available / (1024ULL * 1024ULL);
}
