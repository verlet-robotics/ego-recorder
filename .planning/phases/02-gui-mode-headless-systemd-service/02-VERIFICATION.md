---
phase: 02-gui-mode-headless-systemd-service
verified: 2026-02-19T00:00:00Z
status: human_needed
score: 5/5 must-haves verified
re_verification: false
human_verification:
  - test: "GUI shows live RGB+depth at 30fps while simultaneously recording"
    expected: "Window opens with live camera preview, stats overlay shows ~30fps capture FPS, recording can start while preview continues uninterrupted"
    why_human: "Requires physical D435 camera and display. Cannot verify frame rate or visual correctness programmatically. SUMMARY reports 27fps on hardware but this needs current confirmation."
  - test: "Headless service starts on boot, records, and survives lid close"
    expected: "After sudo ./deploy/install.sh + systemctl start ego-recorder, service enters active (running) state and continues after lid is closed"
    why_human: "Requires systemd service installation, physical hardware, and a lid-close event. D-Bus inhibitor lock is wired in code but its effectiveness is environment-dependent."
  - test: "systemctl status shows live recording stats"
    expected: "systemctl status ego-recorder shows Status: Frames: N | FPS: N.N | Free: N MB updating over time"
    why_human: "Requires running service with camera attached. sd_notifyf STATUS= calls are wired in code (confirmed) but cannot verify systemd forwards them without a live service."
  - test: "Camera disconnect/reconnect recovers automatically within 10 seconds"
    expected: "Unplugging camera in headless mode triggers retry loop; reconnect within 2s attempts; new .egorec file created after reconnect"
    why_human: "Requires physical camera disconnect/reconnect test. Code path is correct (confirmed: 500ms + 1500ms per retry = 2s/attempt) but must be verified with actual hardware."
  - test: "Service restarts cleanly after crash (verified via kill -9)"
    expected: "kill -9 $(pidof ego-recorder) causes systemd to restart service within RestartSec=5s; new recording begins"
    why_human: "Requires deployed service. Restart=on-failure is correctly configured in unit file but actual restart behavior requires a live systemd environment."
---

# Phase 02: GUI Mode + Headless systemd Service Verification Report

**Phase Goal:** Add interactive GUI with live preview and recording controls. Add production systemd service for unattended headless recording on a closed laptop.
**Verified:** 2026-02-19
**Status:** human_needed
**Re-verification:** No -- initial verification

## Goal Achievement

All five success criteria from ROADMAP.md map to verifiable code artifacts. Automated checks pass comprehensively. Human verification is required for the runtime/hardware-dependent behaviors.

### Observable Truths (from ROADMAP.md Success Criteria)

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | GUI shows live RGB+depth at 30fps while simultaneously recording | ? UNCERTAIN | `gui_presenter.cpp`: glTexSubImage2D per frame, jet colormap, SIDE_BY_SIDE default. Pipeline: `RS2_FORMAT_RGB8 + Z16 @ 30`. update_frame() mutex-protected. Recording wired via callbacks. Runtime behavior needs hardware. |
| 2 | Headless service starts on boot, records, and survives lid close | ? UNCERTAIN | `deploy/ego-recorder.service`: WantedBy=multi-user.target, install.sh enables service. D-Bus inhibitor wired: `sd_bus_call_method` "Inhibit" "handle-lid-switch:sleep". `deploy/50-ego-recorder-lid.conf` is fallback. Needs deployed test. |
| 3 | `systemctl status` shows live recording stats | ? UNCERTAIN | `headless_presenter.cpp:80-83`: `sd_notifyf(0, "STATUS=Frames: %llu \| FPS: %.1f \| Free: %llu MB", ...)` in watchdog thread at half-WatchdogSec=15s interval. Wired to live stats via `cached_frames_` + `cached_fps_x10_` atomics. Needs live service to confirm. |
| 4 | Camera disconnect/reconnect recovers automatically within 10 seconds | ? UNCERTAIN | `main.cpp:612-641`: `rs2::camera_disconnected_error` caught in capture thread. Headless path: `camera.reset()` + 500ms sleep + `make_unique<RealSensePipeline>()` loop; on rs2::error waits 1500ms more = 2s/attempt. Within 10s target. New file opened post-reconnect. Needs hardware test. |
| 5 | Service restarts cleanly after crash (verified via kill -9) | ? UNCERTAIN | `deploy/ego-recorder.service:10`: `Restart=on-failure`, `RestartSec=5s`. SIGKILL triggers on-failure restart. Config is correct; requires live systemd environment to verify. |

**Automated score:** 5/5 truths have correct code support. 0 truths have code-level failures. All 5 require human/hardware confirmation.

### Required Artifacts

| Artifact | Min Lines | Status | Details |
|----------|-----------|--------|---------|
| `src/presenter/ipresenter.h` | - | VERIFIED | 45 lines. `class IPresenter` with 6 pure virtual methods + virtual destructor. `#pragma once`. |
| `src/config/config.h` | - | VERIFIED | 37 lines. `struct Config` with 8 fields + `config_path`. `load_config()` declared. |
| `src/config/config.cpp` | - | VERIFIED | 49 lines. `toml::parse_file()` present. All fields read with `value_or()`. `toml::parse_error` caught with stderr warning. |
| `config.toml.example` | - | VERIFIED | 67 lines. All 4 sections ([output], [compression], [recording], [service]) documented with defaults and valid ranges. |
| `CMakeLists.txt` | - | VERIFIED | `FetchContent_Declare(imgui)` present. `imgui_glfw_opengl3` static library target. `option(WITH_GUI)` gate. `HAVE_GUI` define. `tomlplusplus::tomlplusplus` always linked. |
| `src/presenter/gui_presenter.h` | - | VERIFIED | 143 lines. `class GuiPresenter : public IPresenter`. `#ifdef HAVE_GUI` guard. 4 callbacks in constructor. `update_frame()` public API. `ViewMode` enum. All IPresenter methods overridden. |
| `src/presenter/gui_presenter.cpp` | 200 | VERIFIED | 510 lines (exceeds 200 minimum). `ImGui::NewFrame`, `glTexSubImage2D` (twice), `z16_to_jet_rgb`, `WantCaptureKeyboard` guard, `BeginDisabled` on empty session name, `on_reconnect_requested_()` invoked on Reconnect button. |
| `src/presenter/headless_presenter.h` | - | VERIFIED | 111 lines. `class HeadlessPresenter : public IPresenter`. `HAVE_SYSTEMD` guard on `sd_bus*`. `on_request_shutdown_` callback. All IPresenter methods overridden. |
| `src/presenter/headless_presenter.cpp` | 150 | VERIFIED | 374 lines (exceeds 150 minimum). `sd_notify`, `sd_notifyf`, `sd_watchdog_enabled`, `sd_bus_call_method`, `filesystem::space`, `dup()` before `sd_bus_message_unref`, atomic rename for status file. |
| `deploy/ego-recorder.service` | - | VERIFIED | `Type=notify`, `WatchdogSec=30s`, `ExecStart=...--headless`, `Restart=on-failure`, `RestartSec=5s`, `RuntimeDirectory=ego-recorder`. |
| `deploy/99-ego-recorder.rules` | - | VERIFIED | D435 (0x0b07) and D435i (0x0b3a). `ATTR{power/control}="on"` on both. `MODE="0660"`, `GROUP="plugdev"`. |
| `deploy/50-ego-recorder-lid.conf` | - | VERIFIED | `HandleLidSwitch=ignore`, `HandleLidSwitchExternalPower=ignore`. Documented as fallback. |
| `deploy/install.sh` | - | VERIFIED | `set -euo pipefail`. Root check. Idempotent `useradd` (checks `id ego-recorder` first). Installs to all system paths. `systemctl enable` without start. |
| `deploy/config.toml.example` | - | VERIFIED | Present in deploy/. `output` section present. |
| `src/main.cpp` | 250 | VERIFIED | 719 lines (exceeds 250 minimum). `unique_ptr<IPresenter>`, `GuiPresenter`, `HeadlessPresenter`, `load_config`, `rs2::camera_disconnected_error`, `on_reconnect_requested`, `create_directories`. |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| `src/config/config.cpp` | `toml++/toml.hpp` | `toml::parse_file` | WIRED | Line 23: `tbl = toml::parse_file(path)` |
| `CMakeLists.txt` | `imgui_glfw_opengl3` | `FetchContent_Declare + static lib` | WIRED | Lines 92-114: declare, MakeAvailable, static lib, include dirs, link glfw+OpenGL |
| `src/presenter/gui_presenter.cpp` | `imgui.h` | `ImGui::NewFrame` in tick() | WIRED | Line 136: `ImGui::NewFrame()` |
| `src/presenter/gui_presenter.cpp` | OpenGL textures | `glTexSubImage2D` per frame | WIRED | Lines 150, 163: two `glTexSubImage2D` calls (RGB + jet depth) |
| `src/presenter/gui_presenter.h` | `src/presenter/ipresenter.h` | inheritance | WIRED | Line 32: `class GuiPresenter : public IPresenter` |
| `src/presenter/gui_presenter.cpp` | reconnect callback | `on_reconnect_requested_()` invoked | WIRED | Line 272: `if (on_reconnect_requested_) on_reconnect_requested_()` |
| `src/presenter/headless_presenter.cpp` | `systemd/sd-daemon.h` | `sd_notify` calls | WIRED | Lines 73, 80, 83, 99, 138, 170, 192, 206: all lifecycle states covered |
| `src/presenter/headless_presenter.cpp` | `systemd/sd-bus.h` | `sd_bus_call_method` inhibitor | WIRED | Line 238: `sd_bus_call_method` to `org.freedesktop.login1.Manager.Inhibit` |
| `src/presenter/headless_presenter.h` | `src/presenter/ipresenter.h` | inheritance | WIRED | Line 29: `class HeadlessPresenter : public IPresenter` |
| `deploy/ego-recorder.service` | `/usr/local/bin/ego-recorder` | `ExecStart` with `--headless` | WIRED | Line 9: `ExecStart=/usr/local/bin/ego-recorder --headless --config ...` |
| `src/main.cpp` | `src/presenter/ipresenter.h` | `unique_ptr<IPresenter>` | WIRED | Line 468: `std::unique_ptr<IPresenter> presenter` |
| `src/main.cpp` | `src/presenter/gui_presenter.h` | `GuiPresenter` instantiation | WIRED | Line 542: `presenter = std::make_unique<GuiPresenter>(...)` |
| `src/main.cpp` | `src/presenter/headless_presenter.h` | `HeadlessPresenter` instantiation | WIRED | Line 475: `presenter = std::make_unique<HeadlessPresenter>(...)` |
| `src/main.cpp` | `src/config/config.h` | `load_config` call | WIRED | Line 257: `Config config = load_config(config_path)` |
| `src/main.cpp` | camera reconnect | destroy+recreate pipeline | WIRED | Lines 506-521 (GUI lambda), 623-636 (headless auto-retry loop) |
| `src/main.cpp` | `rs2::camera_disconnected_error` | catch in capture thread | WIRED | Line 612: `catch (const rs2::camera_disconnected_error& e)` |

### Requirements Coverage

All plan-level success criteria are satisfied by the verified artifacts:

| Requirement | Status | Evidence |
|-------------|--------|----------|
| IPresenter interface defines lifecycle contract | SATISFIED | 6 pure virtual methods in `ipresenter.h` |
| Config loads TOML with sensible defaults | SATISFIED | `value_or()` for all 8 fields in `config.cpp` |
| GuiPresenter implements complete IPresenter | SATISFIED | All 6 methods overridden in 510-line `gui_presenter.cpp` |
| HeadlessPresenter implements complete IPresenter | SATISFIED | All 6 methods overridden in 374-line `headless_presenter.cpp` |
| sd_notify READY/WATCHDOG/STATUS/STOPPING lifecycle | SATISFIED | All four states wired in `headless_presenter.cpp` |
| D-Bus inhibitor blocks lid-close | SATISFIED | `sd_bus_call_method` "handle-lid-switch:sleep" + `dup()` before unref |
| Watchdog at half-interval | SATISFIED | `ping_interval_us = interval_us / 2` then `sleep_for(ping_interval_us)` |
| Status file at /run/ego-recorder/status | SATISFIED | Atomic rename pattern in `write_status_file()` |
| Disk monitoring stops recording below threshold | SATISFIED | `filesystem::space` every 30 ticks; calls `on_request_shutdown_` |
| All deploy artifacts production-ready | SATISFIED | 5 files verified with correct content |
| Both WITH_GUI=ON and WITH_GUI=OFF build configs | SATISFIED | `gui_presenter.cpp` gated in `target_sources(WITH_GUI)` block |
| USB reconnect: GUI user-triggered via button | SATISFIED | `on_reconnect_requested_()` in disconnect banner; lambda in main wires destroy+recreate |
| USB reconnect: headless auto-retry every 2s | SATISFIED | `camera.reset() + 500ms + make_unique + 1500ms` = 2s/cycle |
| New file created on reconnect | SATISFIED | `start_recording()` called after reconnect in both GUI and headless paths |
| Date-based dirs in headless mode | SATISFIED | `make_date_dir()` via `filesystem::create_directories` at line 331 |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| `headless_presenter.h:98` | 98 | `void* bus_{nullptr}; // placeholder; never used` | INFO | Non-HAVE_SYSTEMD stub member. Not a blocker -- it is intentional graceful degradation documented by comment. The real `sd_bus*` is used on all HAVE_SYSTEMD builds which is the target platform. |

No blocker or warning-level anti-patterns found. The one INFO-level item is an intentional design decision for cross-platform graceful degradation.

### Human Verification Required

#### 1. GUI 30fps Live Preview With Simultaneous Recording

**Test:** Build with `cmake -B build -DCMAKE_PREFIX_PATH="/opt/ros/jazzy" -DWITH_GUI=ON . && cmake --build build -j$(nproc)`. Run `./build/ego-recorder`. Observe live preview, press Tab to cycle views, verify depth is blue-near/red-far jet colormap, enter session name, press Space to start recording, verify stats overlay updates while preview continues.
**Expected:** Window shows live frames at ~30fps, stats overlay shows non-zero capture/write FPS, recording does not pause the preview.
**Why human:** Requires physical D435 and display. FPS value and visual correctness of colormap cannot be verified from source inspection.

#### 2. Headless Boot + Lid Close Survival

**Test:** `sudo ./deploy/install.sh` then `systemctl start ego-recorder.service`. Close laptop lid. Wait 30 seconds. Open lid, run `systemctl status ego-recorder.service`.
**Expected:** Service remains active (running), journal shows continued recording, no "stopped" entries during lid-close period.
**Why human:** D-Bus inhibitor effectiveness depends on the desktop session manager and hardware ACPI configuration. Code is wired correctly but environmental factors apply.

#### 3. systemctl status Live Stats

**Test:** With service running and camera connected, run `systemctl status ego-recorder.service` multiple times 15 seconds apart.
**Expected:** "Status:" line shows `Frames: N | FPS: N.N | Free: N MB` and frame count increases between observations.
**Why human:** Requires live service with watchdog enabled (NOTIFY_SOCKET must be set by systemd). sd_notifyf STATUS= calls are present in code but only visible via systemd.

#### 4. Camera Disconnect/Reconnect Within 10 Seconds

**Test:** Start `./build/ego-recorder --headless --output /tmp`, then unplug D435 USB, wait 5 seconds, plug back in. Watch stderr output.
**Expected:** "Camera disconnected" message appears, then "Camera reconnected" appears within 10 seconds, new .egorec file created in /tmp.
**Why human:** Requires physical USB disconnect/reconnect. Code retry loop is 2s/attempt (500ms + 1500ms) which is well within 10s, but USB re-enumeration time is hardware-dependent.

#### 5. kill -9 Service Restart

**Test:** With deployed service running: `sudo kill -9 $(pidof ego-recorder)`, then `systemctl status ego-recorder.service` after 6+ seconds.
**Expected:** Service shows "active (running)" again, new PID, new recording file started.
**Why human:** Requires deployed service under systemd. `Restart=on-failure` configuration is correct in unit file.

### Gaps Summary

No code-level gaps found. All artifacts exist, are substantive (well above minimum line counts), and are fully wired. All key links are verified with exact line references.

The human_needed status reflects that all five ROADMAP.md success criteria require physical hardware (RealSense D435) and/or a deployed systemd environment to confirm. The code provides correct and complete implementations for every requirement. The SUMMARY reports human testing was completed during plan 02-04 (GUI: ~27fps, headless: ~25fps, 4 start/stop cycles, 0 dropped frames), which provides confidence, but that testing was performed at implementation time and this verification documents what requires re-confirmation by the operator.

---

_Verified: 2026-02-19_
_Verifier: Claude (gsd-verifier)_
