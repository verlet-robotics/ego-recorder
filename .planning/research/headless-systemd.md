# Headless Linux Deployment, systemd Service, and Laptop Lid-Close Operation

**Project:** RealSense Ego Recorder
**Domain:** Headless Linux service for USB camera capture
**Researched:** 2026-02-19
**Overall Confidence:** HIGH

---

## 1. Creating a systemd Service for USB Camera Capture

**Confidence: HIGH** (official systemd documentation, multiple verified sources)

### Recommended Service Unit File

The service should use `Type=notify` so the application can signal readiness to systemd after the RealSense pipeline is initialized (not just after the process forks). This prevents systemd from routing traffic or declaring the service "active" before the camera is actually streaming.

```ini
[Unit]
Description=RealSense Ego Recorder - RGBD+IMU Capture Service
After=local-fs.target
Wants=local-fs.target

[Service]
Type=notify
NotifyAccess=main
ExecStart=/usr/local/bin/realsense-ego-recorder --headless --config /etc/realsense-ego-recorder/config.toml
Restart=on-failure
RestartSec=5
WatchdogSec=30

# Graceful shutdown: give time to flush buffers and close files
TimeoutStopSec=15
KillMode=mixed
KillSignal=SIGTERM

# Run as dedicated user
User=realsense-recorder
Group=plugdev

# Directory management
StateDirectory=realsense-ego-recorder
LogsDirectory=realsense-ego-recorder
ConfigurationDirectory=realsense-ego-recorder
RuntimeDirectory=realsense-ego-recorder

# Environment
EnvironmentFile=-/etc/realsense-ego-recorder/env

# Resource limits
LimitNOFILE=4096
MemoryMax=2G
CPUQuota=80%

# Security hardening (see section 11)
ProtectSystem=strict
ProtectHome=yes
NoNewPrivileges=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/realsense-ego-recorder
# Do NOT use PrivateDevices=yes -- it blocks USB camera access
DevicePolicy=auto
DeviceAllow=/dev/bus/usb

[Install]
WantedBy=multi-user.target
```

### Key Design Decisions

**Type=notify over Type=simple:** `Type=simple` considers the service started the moment the process is spawned. With camera hardware initialization (which can take 1-3 seconds for RealSense), this creates a race. `Type=notify` lets the application call `sd_notify(0, "READY=1")` after the pipeline is confirmed streaming.

**KillMode=mixed:** Sends SIGTERM to the main process first (allowing graceful shutdown), then SIGKILL to remaining child processes after TimeoutStopSec. This is safer than `control-group` (which SIGTERMs everything simultaneously) for an application that needs ordered shutdown (stop pipeline -> flush writes -> close files).

**Restart=on-failure with RestartSec=5:** Restarts on crashes but not on clean exit (exit code 0). The 5-second delay prevents rapid restart loops if the camera is physically disconnected. Combined with WatchdogSec, this provides resilience against both crashes and hangs.

### CMake Integration for libsystemd

```cmake
find_package(PkgConfig REQUIRED)
pkg_check_modules(SYSTEMD IMPORTED_TARGET libsystemd)

if(SYSTEMD_FOUND)
    target_compile_definitions(${PROJECT_NAME} PRIVATE HAVE_SYSTEMD)
    target_link_libraries(${PROJECT_NAME} PRIVATE PkgConfig::SYSTEMD)
endif()
```

This makes systemd integration optional -- the binary works on systems without libsystemd.

### Sources

- [systemd.service official documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html)
- [systemd.exec official documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html)
- [sd_notify manpage](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html)
- [Basic sd_notify + watchdog C++ example](https://gist.github.com/hacst/ee12cd91167aa55b19444fc74c91a8e8)

---

## 2. Preventing Linux from Suspending When the Laptop Lid is Closed

**Confidence: HIGH** (official systemd documentation, verified across distros)

### Two Complementary Approaches

There are two mechanisms that should be used together for maximum reliability:

#### Approach A: logind.conf (System-Wide, Static)

Edit `/etc/systemd/logind.conf`:

```ini
[Login]
HandleLidSwitch=ignore
HandleLidSwitchExternalPower=ignore
HandleLidSwitchDocked=ignore
```

Then apply: `sudo systemctl restart systemd-logind.service`

**What this does:** Tells systemd-logind to take no action when the lid is closed, regardless of power source or docking state. This is the primary mechanism and is persistent across reboots.

**Caveat:** GNOME, KDE, and other desktop environments may override this via their own power management daemons (gnome-settings-daemon, powerdevil). If a desktop environment is running, it may take its own inhibitor locks. On a headless deployment, this is not an issue. On a machine that also has a desktop session, this needs testing.

#### Approach B: Programmatic Inhibitor Lock via D-Bus (Application-Level, Dynamic)

The application itself should take a `handle-lid-switch` inhibitor lock via the logind D-Bus API. This is the belt-and-suspenders approach.

D-Bus method call:
```
org.freedesktop.login1.Manager.Inhibit(
    what: "sleep:handle-lid-switch",
    who: "realsense-ego-recorder",
    why: "Recording RGBD+IMU data",
    mode: "block"
)
```

This returns a file descriptor. The lock is held as long as the fd is open. When the application exits (gracefully or via crash), the fd is closed and the lock is released automatically.

**C++ implementation using sd-bus:**

```cpp
#include <systemd/sd-bus.h>

class SleepInhibitor {
    sd_bus* bus_ = nullptr;
    int fd_ = -1;
public:
    bool acquire() {
        sd_bus_default_system(&bus_);
        sd_bus_message* reply = nullptr;
        int r = sd_bus_call_method(
            bus_,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "Inhibit",
            nullptr, &reply,
            "ssss",
            "sleep:handle-lid-switch",     // what
            "realsense-ego-recorder",       // who
            "Recording RGBD+IMU data",      // why
            "block"                         // mode
        );
        if (r >= 0) {
            sd_bus_message_read(reply, "h", &fd_);
            fd_ = dup(fd_);  // Must dup -- sd_bus_message_unref closes original
            sd_bus_message_unref(reply);
            return true;
        }
        return false;
    }
    ~SleepInhibitor() {
        if (fd_ >= 0) close(fd_);
        if (bus_) sd_bus_unref(bus_);
    }
};
```

#### Recommendation

Use both approaches:
1. **logind.conf** as the deployment configuration (handled by an install script or systemd drop-in)
2. **Programmatic inhibitor lock** as a runtime safety net (released on exit, so the machine can sleep when not recording)

This double-layer approach means: if the application is running, sleep is always inhibited. If the application is stopped, the logind.conf setting still prevents lid-close suspend (which may or may not be desired -- could be made configurable).

### Sources

- [logind.conf official documentation](https://www.freedesktop.org/software/systemd/man/latest/logind.conf.html)
- [systemd Inhibitor Locks specification](https://systemd.io/INHIBITOR_LOCKS/)
- [Baeldung: Disable Suspend on Lid Close](https://www.baeldung.com/linux/disable-suspend-lid-close)
- [It's FOSS: Disable Sleep on Lid Close](https://itsfoss.com/laptop-lid-suspend-ubuntu/)

---

## 3. USB Device Permissions for RealSense Without Root (udev Rules)

**Confidence: HIGH** (official librealsense repository, well-documented)

### Official udev Rules

Intel provides official udev rules in the librealsense repository at `config/99-realsense-libusb.rules`. These rules set `MODE:="0666"` and `GROUP:="plugdev"` for all Intel RealSense USB device IDs (vendor 8086).

### Installation Methods

**Package installation (preferred):**
```bash
sudo apt-get install librealsense2-udev-rules
```

**Manual installation:**
```bash
sudo cp 99-realsense-libusb.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

### Key Rule Structure

```udev
# D400 series (includes D435)
SUBSYSTEMS=="usb", ATTRS{idVendor}=="8086", ATTRS{idProduct}=="0b07", MODE:="0666", GROUP:="plugdev"
SUBSYSTEMS=="usb", ATTRS{idVendor}=="8086", ATTRS{idProduct}=="0b3a", MODE:="0666", GROUP:="plugdev"
# ... additional product IDs for DFU/recovery modes
```

### Service User Setup

The systemd service user must be in the `plugdev` group:

```bash
sudo useradd -r -s /usr/sbin/nologin -G plugdev realsense-recorder
```

The `-r` flag creates a system user (no home directory, no login). The `-G plugdev` adds it to the plugdev group for USB access.

### IMU Access

The D435i (with IMU) requires additional udev rules for accelerometer/gyroscope sensor access. The official rules include entries for `accel_3d` devices on kernel 4.15+. For the D435 (without built-in IMU -- note: the PROJECT.md mentions IMU but the D435 does NOT have an IMU; only the D435i does), these are not needed.

**IMPORTANT NOTE:** The Intel RealSense D435 does NOT have a built-in IMU. Only the D435i variant has an IMU. The project requirements mention IMU capture -- verify which hardware model is actually being used. If it is the D435 (not D435i), IMU capture is not possible without an external IMU.

### Sources

- [99-realsense-libusb.rules on GitHub](https://github.com/IntelRealSense/librealsense/blob/master/config/99-realsense-libusb.rules)
- [librealsense Linux build guide](https://dev.intelrealsense.com/docs/compiling-librealsense-for-linux-ubuntu-guide)
- [librealsense udev rules issue #5126](https://github.com/IntelRealSense/librealsense/issues/5126)

---

## 4. Handling USB Disconnects/Reconnects Gracefully

**Confidence: MEDIUM** (official librealsense docs + community reports of edge cases)

### librealsense2 Error Handling Hierarchy

```
std::exception
  std::runtime_error
    rs2::error
      rs2::unrecoverable_error
        rs2::camera_disconnected_error    <-- Key one for USB disconnects
        rs2::backend_error
        rs2::device_in_recovery_mode_error
      rs2::recoverable_error
        rs2::invalid_value_error
        rs2::wrong_api_call_sequence_error
        rs2::not_implemented_error
```

**State safety guarantee:** "If the API implied a state transition, but the call failed with an exception, state will not change." This means failed operations roll back cleanly.

### Recommended Recovery Pattern

```cpp
class DeviceManager {
    rs2::context ctx_;
    rs2::pipeline pipe_;
    std::atomic<bool> device_connected_{false};
    std::string target_serial_;

public:
    void initialize() {
        // Register for device change notifications
        ctx_.set_devices_changed_callback(
            [this](rs2::event_information& info) {
                // Check for removal
                if (info.was_removed(pipe_.get_active_profile().get_device())) {
                    device_connected_ = false;
                    log("Camera disconnected");
                }
                // Check for arrival
                auto new_devices = info.get_new_devices();
                for (auto&& dev : new_devices) {
                    if (dev.get_info(RS2_CAMERA_INFO_SERIAL_NUMBER) == target_serial_) {
                        device_connected_ = true;
                        log("Camera reconnected");
                    }
                }
            }
        );
    }

    void capture_loop() {
        while (running_) {
            if (!device_connected_) {
                // Wait for reconnection with exponential backoff
                wait_for_device();
                restart_pipeline();
                continue;
            }
            try {
                auto frames = pipe_.wait_for_frames(5000);  // 5s timeout
                process_frames(frames);
            }
            catch (const rs2::camera_disconnected_error& e) {
                device_connected_ = false;
                log("Camera disconnected during capture: " + std::string(e.what()));
                // Pipeline will be restarted on next iteration
            }
            catch (const rs2::error& e) {
                log("RealSense error: " + std::string(e.what()));
                // Attempt recovery
            }
        }
    }

    void restart_pipeline() {
        try {
            pipe_.stop();
        } catch (...) { /* May throw if already stopped */ }

        rs2::config cfg;
        cfg.enable_device(target_serial_);
        // ... configure streams ...
        pipe_.start(cfg);
        device_connected_ = true;
    }
};
```

### Known Issues and Mitigations

1. **Callback inconsistency after hardware_reset():** The `set_devices_changed_callback` may not fire reliably after `device.hardware_reset()`. Workaround: use polling via `ctx.query_devices()` as a fallback detection mechanism.

2. **Cannot reconnect after unplug/replug (issue #11881):** Some users report that the D435 cannot be re-opened after physical disconnect/reconnect without creating a new `rs2::context`. Mitigation: destroy and recreate the entire pipeline and context on reconnection rather than trying to reuse them.

3. **Operations between disconnect and notification fail:** There is an inherent race between physical disconnect and callback delivery. All frame operations must be wrapped in try-catch.

4. **Polling-based detection:** The device change callback relies on periodic polling internally, which means very fast disconnect/reconnect cycles might be missed. For a systemd service, this is acceptable since recording would pause during the gap anyway.

### Systemd Integration for Reconnection

If the camera is disconnected for an extended period and the application cannot recover, it should exit with a non-zero code. systemd's `Restart=on-failure` will restart the service, which provides a clean-slate recovery.

### Sources

- [librealsense error handling documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/error_handling.md)
- [Cannot reconnect D435 after unplug - issue #11881](https://github.com/IntelRealSense/librealsense/issues/11881)
- [set_devices_changed_callback inconsistency - issue #9287](https://github.com/IntelRealSense/librealsense/issues/9287)
- [Camera disconnection handling - issue #931](https://github.com/IntelRealSense/librealsense/issues/931)

---

## 5. Systemd Service Best Practices: Restart, Logging, Resource Limits

**Confidence: HIGH** (official systemd documentation)

### Restart Policies

| Setting | Value | Rationale |
|---------|-------|-----------|
| `Restart=` | `on-failure` | Restarts on crash/signal but not on clean `exit(0)`. Recommended by systemd docs for long-running services. |
| `RestartSec=` | `5` | Prevents rapid restart loops. 5 seconds is enough for USB device to settle after hot-unplug. |
| `StartLimitIntervalSec=` | `300` | Combined with `StartLimitBurst=5`: if the service fails 5 times in 5 minutes, stop trying. |
| `StartLimitBurst=` | `5` | Prevents infinite restart loops if the camera is permanently broken. |

### Watchdog Configuration

```ini
WatchdogSec=30
```

The application must call `sd_notify(0, "WATCHDOG=1")` at least every 15 seconds (half the WatchdogSec interval, per systemd recommendation). This catches hangs where the process is alive but deadlocked or the capture loop has stalled.

**C++ watchdog implementation:**

```cpp
#include <systemd/sd-daemon.h>
#include <thread>

class WatchdogTimer {
    std::thread thread_;
    std::atomic<bool> running_{true};
    uint64_t interval_usec_ = 0;

public:
    void start() {
        int enabled = sd_watchdog_enabled(0, &interval_usec_);
        if (enabled <= 0) return;  // Watchdog not configured

        // Ping at half the interval
        auto ping_interval = std::chrono::microseconds(interval_usec_ / 2);

        thread_ = std::thread([this, ping_interval]() {
            while (running_) {
                sd_notify(0, "WATCHDOG=1");
                std::this_thread::sleep_for(ping_interval);
            }
        });
    }

    void stop() {
        running_ = false;
        if (thread_.joinable()) thread_.join();
    }
};
```

**Better approach -- conditional watchdog pinging:** Instead of a blind timer, only ping the watchdog when frames are actually being captured. This way, a stalled capture loop (no frames arriving) will trigger a watchdog restart.

```cpp
// In the capture loop:
void on_frame_received() {
    sd_notify(0, "WATCHDOG=1");
    // ... process frame ...
}
```

### Logging

Use `sd_journal_print()` for structured logging to the systemd journal, or simply write to stdout/stderr (systemd captures both by default for services).

**Recommendation:** Use stdout/stderr with structured prefixes. This keeps the code portable (works without systemd) and lets journald handle log management.

```ini
# In the service file
StandardOutput=journal
StandardError=journal
SyslogIdentifier=realsense-ego-recorder
```

Query logs: `journalctl -u realsense-ego-recorder -f`

### Resource Limits

| Setting | Value | Rationale |
|---------|-------|-----------|
| `MemoryMax=` | `2G` | Prevents runaway memory from killing the system. 640x480 RGBD at 30fps should use well under 500MB. |
| `CPUQuota=` | `80%` | Leaves headroom for SSH access and monitoring. |
| `LimitNOFILE=` | `4096` | Enough for open files during recording. |
| `IOWeight=` | `500` | Default weight. Increase if I/O contention is an issue. |
| `TasksMax=` | `32` | Reasonable for a multi-threaded capture application. |

### Sources

- [systemd.service official documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html)
- [Implementing Service Recovery in systemd](https://dohost.us/index.php/2025/10/27/implementing-service-recovery-and-restart-policies-in-systemd/)
- [sd_watchdog_enabled manpage](https://www.man7.org/linux/man-pages/man3/sd_notify.3.html)

---

## 6. Managing Output Directory Configuration

**Confidence: HIGH** (official systemd documentation)

### Recommended Approach: StateDirectory + EnvironmentFile

systemd provides managed directories that are automatically created with correct ownership:

```ini
[Service]
StateDirectory=realsense-ego-recorder
# Creates /var/lib/realsense-ego-recorder owned by the service user
```

For configurable output paths (e.g., external USB drives), use an EnvironmentFile:

**/etc/realsense-ego-recorder/env:**
```bash
# Output directory for recordings
OUTPUT_DIR=/var/lib/realsense-ego-recorder/recordings

# Override for external drive
# OUTPUT_DIR=/mnt/usb-drive/recordings

# Session naming
SESSION_PREFIX=ego_capture

# Disk space threshold (GB) -- stop recording below this
MIN_DISK_SPACE_GB=10
```

**Service file:**
```ini
EnvironmentFile=-/etc/realsense-ego-recorder/env
ReadWritePaths=/var/lib/realsense-ego-recorder
# Add external drive paths as needed:
# ReadWritePaths=/mnt/usb-drive/recordings
```

### Application-Level Configuration File

Use a TOML or JSON config file at `/etc/realsense-ego-recorder/config.toml`:

```toml
[output]
directory = "/var/lib/realsense-ego-recorder/recordings"
session_prefix = "ego_capture"
min_disk_space_gb = 10
max_session_duration_sec = 0  # 0 = unlimited
auto_rotate = true            # Start new session file when size exceeds limit
max_file_size_gb = 4          # Per-file size limit

[camera]
serial_number = ""            # Empty = use first available
width = 640
height = 480
fps = 30
enable_depth = true
enable_color = true
enable_imu = true
```

### Directory Structure on Disk

```
/var/lib/realsense-ego-recorder/
  recordings/
    2026-02-19_14-30-00_ego_capture/
      metadata.json        # Session metadata, intrinsics, config
      color.bin             # Compressed color stream
      depth.bin             # Compressed depth stream
      imu.bin               # IMU data
      timestamps.bin        # Per-frame timestamps
    2026-02-19_15-45-00_ego_capture/
      ...
```

### Handling External Drives

For recording to an external USB drive:

1. Mount the drive via fstab or automount
2. Update the config file or environment file with the new path
3. Add `ReadWritePaths=` for the mount point in a systemd drop-in override:

```bash
sudo systemctl edit realsense-ego-recorder
```

```ini
[Service]
ReadWritePaths=/mnt/usb-drive
```

### Sources

- [systemd.exec StateDirectory documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html)
- [Dynamic Users with systemd](https://0pointer.net/blog/dynamic-users-with-systemd.html)
- [Baeldung: systemd Environment Variables](https://www.baeldung.com/linux/systemd-services-environment-variables)

---

## 7. Signal Handling for Graceful Shutdown (SIGTERM from systemd)

**Confidence: HIGH** (well-established C++ pattern, verified by multiple authoritative sources)

### The Problem

systemd sends SIGTERM to stop a service. The application must:
1. Stop accepting new frames
2. Flush all buffered data to disk
3. Close the recording files cleanly (write final metadata, checksums)
4. Release the RealSense pipeline
5. Release the sleep inhibitor lock
6. Exit with code 0

All of this must happen within `TimeoutStopSec` (recommended: 15 seconds).

### Recommended Pattern: Dedicated Signal Thread with sigwait

This is the standard safe approach for multithreaded C++ applications. Using `sigaction()` in multithreaded code is unsafe because the handler can run on any thread and most operations are not async-signal-safe.

```cpp
#include <signal.h>
#include <pthread.h>
#include <atomic>
#include <thread>

class SignalHandler {
    std::thread signal_thread_;
    std::atomic<bool>& shutdown_flag_;  // Shared with capture loop

public:
    SignalHandler(std::atomic<bool>& flag) : shutdown_flag_(flag) {}

    void start() {
        // Block SIGTERM and SIGINT in ALL threads (inherited by child threads)
        sigset_t mask;
        sigemptyset(&mask);
        sigaddset(&mask, SIGTERM);
        sigaddset(&mask, SIGINT);
        pthread_sigmask(SIG_BLOCK, &mask, nullptr);

        // Dedicated thread waits synchronously for signals
        signal_thread_ = std::thread([this, mask]() {
            int sig;
            sigwait(&mask, &sig);  // Blocks until signal received

            // Safe context -- not an async signal handler
            // Can use any synchronization primitive
            shutdown_flag_ = true;

            // Notify systemd of shutdown progress
            #ifdef HAVE_SYSTEMD
            sd_notify(0, "STOPPING=1\nSTATUS=Flushing buffers...");
            #endif
        });
    }

    void join() {
        if (signal_thread_.joinable()) signal_thread_.join();
    }
};

// Usage in main:
int main() {
    std::atomic<bool> shutdown_requested{false};

    SignalHandler signals(shutdown_requested);
    signals.start();

    // ... initialize pipeline, start capture ...

    while (!shutdown_requested) {
        auto frames = pipe.wait_for_frames(1000);
        if (frames) process_frames(frames);
    }

    // Graceful shutdown
    pipe.stop();
    flush_buffers();
    write_final_metadata();
    close_files();

    sd_notify(0, "STATUS=Shutdown complete");
    return 0;
}
```

### Key Requirements

1. **Block signals BEFORE creating any threads.** Signal masks are inherited, so all worker threads will also have SIGTERM/SIGINT blocked.
2. **Only ONE thread calls sigwait.** Multiple threads calling sigwait on the same signal is undefined.
3. **Use std::atomic<bool> for the shutdown flag.** No mutex needed for a single bool.
4. **sd_notify with STOPPING=1** tells systemd the service is shutting down. This extends the grace period if systemd is configured for it.

### systemd Shutdown Sequence

1. systemd sends SIGTERM (configurable via KillSignal=)
2. Immediately follows with SIGCONT (ensures suspended processes wake up)
3. Waits TimeoutStopSec (default 90s, recommend 15s for this service)
4. Sends SIGKILL to remaining processes

### Sources

- [Thomas Trapp: Signal Handlers for Multithreaded C++](https://thomastrapp.com/blog/signal-handlers-for-multithreaded-cpp/)
- [systemd.kill documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html)
- [Systemd killmodes, multithreading and graceful shutdown](https://ihaveabackup.net/2022/01/30/systemd-killmodes-multithreading-and-graceful-shutdown/)

---

## 8. Monitoring the Service Remotely

**Confidence: HIGH** (standard Linux tooling)

### Layer 1: systemd Status and Journal

```bash
# Service status (via SSH)
ssh laptop systemctl status realsense-ego-recorder

# Live log streaming
ssh laptop journalctl -u realsense-ego-recorder -f

# Recent logs
ssh laptop journalctl -u realsense-ego-recorder --since "1 hour ago"
```

### Layer 2: Application Status via sd_notify

The application should update its status string via sd_notify:

```cpp
sd_notifyf(0, "STATUS=Recording session '%s' | %d frames | %.1f GB | %d fps",
    session_name.c_str(), frame_count, disk_usage_gb, current_fps);
```

This appears in `systemctl status` output:

```
Active: active (running) since ...
Status: "Recording session 'ego_capture_001' | 54321 frames | 2.3 GB | 30 fps"
```

### Layer 3: Status File for Programmatic Access

Write a machine-readable status file to the runtime directory:

```cpp
// Write to /run/realsense-ego-recorder/status.json
{
    "state": "recording",
    "session": "ego_capture_001",
    "started_at": "2026-02-19T14:30:00Z",
    "frames_captured": 54321,
    "fps_current": 29.97,
    "fps_target": 30,
    "dropped_frames": 2,
    "disk_usage_bytes": 2468421632,
    "disk_free_bytes": 107374182400,
    "output_path": "/var/lib/realsense-ego-recorder/recordings/2026-02-19_14-30-00",
    "camera_serial": "12345678",
    "camera_connected": true,
    "uptime_seconds": 1810
}
```

Use `RuntimeDirectory=realsense-ego-recorder` in the service file to get `/run/realsense-ego-recorder/` (automatically cleaned up on service stop).

### Layer 4: Disk Space Monitoring

The application should monitor available disk space and take action:

```cpp
#include <sys/statvfs.h>

uint64_t get_free_space_bytes(const std::string& path) {
    struct statvfs stat;
    statvfs(path.c_str(), &stat);
    return stat.f_bavail * stat.f_frsize;
}

// In capture loop:
if (get_free_space_bytes(output_path) < min_free_bytes) {
    log("Disk space critically low, stopping recording");
    // Start new session on different drive, or stop gracefully
}
```

### Layer 5: Optional Lightweight HTTP Status Endpoint

For more sophisticated monitoring, the application could expose a lightweight HTTP endpoint (e.g., on localhost:8080/status) using a minimal embedded HTTP server. However, this adds complexity and attack surface. For most deployments, the status file + journald approach is sufficient.

### Remote Notification Options

For alerting on failures:
- `OnFailure=` unit in systemd to trigger email/webhook on service failure
- systemd journal forwarding to a remote syslog or monitoring system
- Simple cron job checking `systemctl is-active realsense-ego-recorder`

### Sources

- [journalctl usage guide](https://www.digitalocean.com/community/tutorials/how-to-use-journalctl-to-view-and-manipulate-systemd-logs)
- [sd_notify documentation](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html)

---

## 9. Power Management for Sustained USB3 Capture on Battery

**Confidence: MEDIUM** (TLP documentation verified, battery-specific capture behavior based on community reports)

### Problem

Linux power management tools (TLP, power-profiles-daemon) aggressively autosuspend USB devices on battery power. This will kill a RealSense stream.

### TLP Configuration

If TLP is installed (common on Ubuntu laptops), exclude the RealSense from USB autosuspend:

1. Find the device ID:
```bash
lsusb | grep 8086
# Example: Bus 002 Device 003: ID 8086:0b3a Intel Corp. Intel(R) RealSense(TM) Depth Camera 435
```

2. Add to TLP config (`/etc/tlp.conf`):
```ini
USB_DENYLIST="8086:0b3a"
```

Or create a drop-in (`/etc/tlp.d/90-realsense.conf`):
```ini
USB_DENYLIST="8086:0b3a"
```

### Kernel-Level USB Autosuspend Prevention

Even without TLP, the kernel may autosuspend USB devices. Create a udev rule to prevent this:

```udev
# /etc/udev/rules.d/99-realsense-power.rules
# Prevent USB autosuspend for RealSense devices
ACTION=="add", SUBSYSTEM=="usb", ATTRS{idVendor}=="8086", ATTRS{idProduct}=="0b3a", \
    ATTR{power/autosuspend}="-1", ATTR{power/control}="on"
```

### Power Consumption Considerations

The D435 draws approximately 700mA at 5V over USB3 (3.5W). At 640x480@30fps with depth+color+IMU, sustained capture on battery will drain a typical laptop battery (50-80Wh) alongside the laptop's own consumption.

**Rough estimates for a 60Wh laptop battery:**
- Laptop idle with lid closed: ~5-8W
- RealSense D435 streaming: ~3.5W
- Disk I/O for recording: ~1-2W
- Total: ~10-13W
- Battery life: ~4.5-6 hours

### Thermal Considerations

With the lid closed, the laptop has reduced airflow. Sustained USB3 capture generates heat from:
- CPU processing frames
- USB controller
- Disk I/O

**Mitigations:**
- Use CPUQuota in the systemd service to limit thermal output
- Monitor CPU temperature via `/sys/class/thermal/thermal_zone*/temp`
- Consider propping the lid slightly open (not fully closed) for airflow
- Some laptops have BIOS settings to keep fans running with lid closed

### Preventing CPU Power Saving That Impacts Capture

```bash
# Prevent CPU frequency scaling to minimum (optional, trades battery for reliability)
# In /etc/tlp.conf:
CPU_SCALING_GOVERNOR_ON_BAT=performance
# Or more conservatively:
CPU_ENERGY_PERF_POLICY_ON_BAT=balance_performance
```

### Sources

- [TLP USB Devices documentation](https://linrunner.de/tlp/faq/usb.html)
- [TLP USB Settings](https://linrunner.de/tlp/settings/usb.html)
- [Arch Wiki: TLP](https://wiki.archlinux.org/title/TLP)
- [Arch Wiki: Power Management](https://wiki.archlinux.org/title/Power_management)

---

## 10. Structuring C++ for GUI and Headless Mode from the Same Binary

**Confidence: HIGH** (well-established software architecture pattern)

### Recommended Architecture: Strategy Pattern with Runtime Switch

The cleanest approach is a runtime command-line flag (`--headless` or `--gui`) that selects the presentation layer, with the core capture engine completely decoupled.

```
+------------------+     +-------------------+
|   CLI / Args     |---->| Mode Selection    |
+------------------+     +---+----------+----+
                              |          |
                    +---------v--+  +----v--------+
                    | HeadlessUI |  | GuiPresenter|
                    | (no-op/log)|  | (OpenGL/    |
                    +-----+------+  | ImGui)      |
                          |         +------+------+
                          |                |
                    +-----v----------------v------+
                    |      IPresenter interface    |
                    +-------------+----------------+
                                  |
                    +-------------v----------------+
                    |      CaptureEngine           |
                    |  - Pipeline management       |
                    |  - Frame processing          |
                    |  - Disk I/O                  |
                    |  - IMU handling              |
                    +------------------------------+
```

### Interface Definition

```cpp
// presenter.h -- Abstract interface
class IPresenter {
public:
    virtual ~IPresenter() = default;

    // Called once per frame with capture stats
    virtual void on_frame(const FrameData& frame, const CaptureStats& stats) = 0;

    // Called when capture state changes
    virtual void on_state_change(CaptureState state) = 0;

    // Returns false when the user/system requests shutdown
    virtual bool should_continue() = 0;

    // Initialize the presenter (may open windows, or do nothing)
    virtual bool initialize() = 0;

    // Shutdown the presenter
    virtual void shutdown() = 0;
};

// headless_presenter.h
class HeadlessPresenter : public IPresenter {
    std::atomic<bool>& shutdown_flag_;
public:
    HeadlessPresenter(std::atomic<bool>& flag) : shutdown_flag_(flag) {}

    void on_frame(const FrameData&, const CaptureStats& stats) override {
        // Update sd_notify status
        sd_notifyf(0, "WATCHDOG=1\nSTATUS=Recording: %d frames, %.1f fps",
            stats.total_frames, stats.current_fps);
    }

    void on_state_change(CaptureState state) override {
        // Log to journal
        fprintf(stdout, "State: %s\n", to_string(state));
    }

    bool should_continue() override { return !shutdown_flag_; }
    bool initialize() override { return true; }
    void shutdown() override {}
};

// gui_presenter.h
class GuiPresenter : public IPresenter {
    // OpenGL/ImGui window, preview rendering, controls
    // ...
};
```

### Mode Selection in main()

```cpp
int main(int argc, char** argv) {
    // Parse arguments
    bool headless = has_flag(argc, argv, "--headless");
    std::string config_path = get_option(argc, argv, "--config",
        "/etc/realsense-ego-recorder/config.toml");

    std::atomic<bool> shutdown_requested{false};

    // Signal handling (always needed, but especially for headless)
    SignalHandler signals(shutdown_requested);
    signals.start();

    // Select presenter
    std::unique_ptr<IPresenter> presenter;
    if (headless) {
        presenter = std::make_unique<HeadlessPresenter>(shutdown_requested);
        // Acquire sleep inhibitor lock
        SleepInhibitor inhibitor;
        inhibitor.acquire();
        // Notify systemd we are ready
        sd_notify(0, "READY=1");
    } else {
        presenter = std::make_unique<GuiPresenter>(shutdown_requested);
    }

    // Core engine is identical regardless of mode
    CaptureEngine engine(config_path);
    engine.set_presenter(presenter.get());
    engine.run();  // Blocks until shutdown

    return 0;
}
```

### Build System Considerations

**Do NOT use compile-time switches (#ifdef GUI_MODE) for the mode selection.** This forces building two binaries and complicates testing. Use runtime selection.

**Compile-time switches ARE appropriate for optional dependencies:**

```cmake
option(WITH_GUI "Build with GUI support (requires OpenGL, ImGui)" ON)
option(WITH_SYSTEMD "Build with systemd notification support" ON)

if(WITH_GUI)
    find_package(OpenGL REQUIRED)
    find_package(glfw3 REQUIRED)
    target_compile_definitions(${PROJECT_NAME} PRIVATE HAVE_GUI)
    # Add GUI source files
endif()

if(WITH_SYSTEMD)
    pkg_check_modules(SYSTEMD IMPORTED_TARGET libsystemd)
    if(SYSTEMD_FOUND)
        target_compile_definitions(${PROJECT_NAME} PRIVATE HAVE_SYSTEMD)
        target_link_libraries(${PROJECT_NAME} PRIVATE PkgConfig::SYSTEMD)
    endif()
endif()
```

This way, a headless-only build on a minimal system can skip GUI dependencies entirely, but the default build includes both modes.

### Sources

- [Qt Forum: Design pattern for GUI and non-GUI mode](https://forum.qt.io/topic/112443/design-pattern-for-gui-and-non-gui-mode/40)
- [Dear ImGui on GitHub](https://github.com/ocornut/imgui)

---

## 11. Security Hardening for the Service

**Confidence: HIGH** (official systemd documentation)

### Recommended Hardening Options

```ini
[Service]
# Filesystem protection
ProtectSystem=strict          # Read-only filesystem except /dev, /proc, /sys
ProtectHome=yes               # /home, /root, /run/user inaccessible
PrivateTmp=yes                # Isolated /tmp
ReadWritePaths=/var/lib/realsense-ego-recorder

# Privilege restrictions
NoNewPrivileges=yes           # Cannot gain privileges via setuid/setgid
CapabilityBoundingSet=        # Drop all capabilities
AmbientCapabilities=          # No ambient capabilities

# Device access -- CRITICAL: do NOT use PrivateDevices=yes
# PrivateDevices=yes blocks USB camera access entirely
DevicePolicy=auto             # Allow device access via udev rules
DeviceAllow=/dev/bus/usb rw   # Explicitly allow USB bus access

# Network (not needed for this service unless HTTP status endpoint used)
PrivateNetwork=yes            # No network access
RestrictAddressFamilies=AF_UNIX  # Only Unix sockets (for sd_notify and D-Bus)

# System call filtering
SystemCallArchitectures=native
RestrictRealtime=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
```

**Critical note:** `PrivateDevices=yes` is a common hardening recommendation but it MUST NOT be used for this service. It creates a private /dev with only pseudo-devices, preventing all access to USB hardware.

### Sources

- [Options for hardening systemd service units](https://gist.github.com/ageis/f5595e59b1cddb1513d1b425a323db04)
- [Arch Wiki: systemd Sandboxing](https://wiki.archlinux.org/title/Systemd/Sandboxing)
- [Red Hat: Using systemd features to secure services](https://www.redhat.com/en/blog/systemd-secure-services)

---

## Summary: Complete Deployment Checklist

### System Configuration (one-time setup)

1. Install udev rules for RealSense USB access
2. Create system user `realsense-recorder` in `plugdev` group
3. Configure `/etc/systemd/logind.conf` to ignore lid switch
4. Configure TLP (if installed) to exclude RealSense from USB autosuspend
5. Install the service unit file to `/etc/systemd/system/`
6. Create config directory and file at `/etc/realsense-ego-recorder/`
7. Enable the service: `systemctl enable realsense-ego-recorder`

### Application Requirements

1. Accept `--headless` flag for service mode
2. Implement IPresenter interface for GUI/headless separation
3. Use sigwait-based signal handling for graceful SIGTERM shutdown
4. Integrate sd_notify for READY=1, WATCHDOG=1, and STATUS updates
5. Take D-Bus inhibitor lock for sleep prevention
6. Implement device disconnect/reconnect recovery
7. Monitor disk space and stop recording when critically low
8. Write machine-readable status file to /run/ for monitoring

### Monitoring (ongoing operations)

1. `systemctl status realsense-ego-recorder` for quick health check
2. `journalctl -u realsense-ego-recorder -f` for live logs
3. Read `/run/realsense-ego-recorder/status.json` for programmatic monitoring
4. Monitor disk space on recording volume

---

## Implications for Roadmap

### Phase ordering recommendation:

1. **Core capture engine first** (no GUI, no systemd) -- get the pipeline working
2. **Headless mode with signal handling** -- make it stoppable/restartable
3. **systemd integration** (sd_notify, watchdog, inhibitor locks) -- make it a proper service
4. **GUI mode** -- add the IPresenter implementation for interactive use
5. **Production hardening** (disconnect recovery, disk monitoring, security) -- make it reliable

### Research flags:
- **D435 vs D435i clarification needed** -- the project mentions IMU but the D435 does not have one. This is a critical hardware question.
- **Disconnect recovery needs integration testing** -- community reports suggest edge cases with reconnection that cannot be resolved by documentation alone.
- **Lid-close thermal behavior needs empirical testing** -- varies significantly by laptop model.

### Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| systemd service configuration | HIGH | Official docs, well-established patterns |
| Lid-close prevention | HIGH | Two complementary approaches both well-documented |
| udev/USB permissions | HIGH | Official librealsense rules available |
| USB disconnect recovery | MEDIUM | Official API exists but community reports edge cases |
| Signal handling | HIGH | Standard POSIX pattern, well-verified |
| Power management | MEDIUM | TLP docs verified, battery estimates are approximate |
| GUI/headless architecture | HIGH | Standard strategy pattern |
| Security hardening | HIGH | Official systemd sandboxing docs |
