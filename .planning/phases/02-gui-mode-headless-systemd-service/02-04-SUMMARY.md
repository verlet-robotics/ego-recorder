---
phase: 02-gui-mode-headless-systemd-service
plan: 04
subsystem: main-integration
tags: [main, presenter, ipresenter, gui, headless, usb-recovery, config, cmake, cxxopts, filesystem]

# Dependency graph
requires:
  - phase: 02-gui-mode-headless-systemd-service
    plan: 01
    provides: IPresenter interface, Config struct, load_config function
  - phase: 02-gui-mode-headless-systemd-service
    plan: 02
    provides: GuiPresenter with four callbacks including on_reconnect_requested
  - phase: 02-gui-mode-headless-systemd-service
    plan: 03
    provides: HeadlessPresenter with on_request_shutdown callback
  - phase: 01-core-capture-engine-mvp-storage
    provides: RealSensePipeline, FileWriter, Stats, BoundedQueue, JpegCompressor, ZstdCompressor

provides:
  - Unified single binary supporting both GUI and headless modes via --headless flag
  - Config file loading (--config) with CLI override merge logic
  - USB disconnect/reconnect recovery in both modes (GUI: user-triggered, headless: auto-retry)
  - Date-based output directories for headless mode (YYYY/MM/DD)
  - Auto-generated timestamp session name for headless mode
  - on_reconnect_requested callback wired to pipeline destroy+recreate+new-file sequence

affects:
  - Phase 3: uses ego-recorder binary with headless mode for automated recording

# Tech tracking
tech-stack:
  added:
    - std::filesystem::create_directories (date-based dir creation in headless mode)
    - rs2::camera_disconnected_error catch (USB disconnect recovery in capture thread)
    - cxxopts count() for explicit vs default CLI flag detection
  patterns:
    - Config-first then CLI-override: load_config() then cxxopts count() guard
    - Presenter polymorphism: unique_ptr<IPresenter> with GUI or Headless impl
    - Lambda-based callbacks wired at presenter construction time
    - on_reconnect_requested: destroy-sleep-recreate-pipeline pattern in main thread
    - Headless auto-retry: camera.reset() + sleep(500ms) + make_unique<RealSensePipeline>() loop
    - recording_active atomic guards frame routing in capture thread
    - stop_recording lambda: close queue, join writer thread, finalize file
    - make_file_header() extracted to helper: preserves pipeline-storage decoupling

key-files:
  created: []
  modified:
    - src/main.cpp

key-decisions:
  - "presenter = unique_ptr<IPresenter>: GuiPresenter or HeadlessPresenter at runtime"
  - "on_reconnect_requested lambda captures camera unique_ptr by ref: destroy+recreate inside lambda"
  - "Headless session name: auto-generate capture_YYYYMMDD_HHMMSS if session-name not explicitly set"
  - "Date dirs: make_date_dir() creates output_dir/YYYY/MM/DD/ with filesystem::create_directories"
  - "GUI mode capture thread: continues running during disconnect, sets camera_disconnected flag"
  - "Headless mode capture thread: auto-retries every 2s with camera.reset()+make_unique loop"
  - "stop_recording is a reusable lambda: called on shutdown, reconnect, and GUI Stop button"
  - "make_file_header() helper keeps FileHeader assembly in main.cpp, not in pipeline or storage"

# Metrics
duration: ~3min
completed: 2026-02-19
---

# Phase 02 Plan 04: main.cpp Integration Summary

**Single binary with IPresenter polymorphism: GUI mode (Dear ImGui preview + recording controls) and headless mode (auto-record, date dirs, 2s USB auto-retry) via --headless flag; on_reconnect_requested wired to pipeline destroy+recreate with new recording file**

## Performance

- **Duration:** ~3 min
- **Started:** 2026-02-19T06:54:21Z
- **Completed:** 2026-02-19T06:57:41Z
- **Tasks:** 2/3 (Task 3 is human verification checkpoint)
- **Files modified:** 1

## Accomplishments

- main.cpp refactored from 339 lines to 420+ lines: IPresenter integration, dual-mode operation, USB recovery
- Config file loaded via load_config() first, then CLI flags override via cxxopts count() detection
- GUI mode: four callbacks (on_start_recording, on_stop_recording, on_session_name_changed, on_reconnect_requested) wired at GuiPresenter construction; on_reconnect_requested lambda destroys pipeline, sleeps 500ms, recreates, calls presenter->on_camera_reconnect(), opens new recording file
- Headless mode: auto-generates capture_YYYYMMDD_HHMMSS session name; creates date-based YYYY/MM/DD output directory; starts recording immediately after camera init
- USB recovery in capture thread: GUI mode sets disconnected flag (user triggers reconnect), headless mode auto-retries every 2 seconds with camera.reset()+make_unique<RealSensePipeline>() loop
- Disk full: HeadlessPresenter.tick() returns false -> shutdown_flag set -> clean shutdown
- Both WITH_GUI=ON and WITH_GUI=OFF builds compile and link; headless-only build rejects GUI mode with clear error message

## Task Commits

Each task was committed atomically:

1. **Task 1: Refactor main.cpp with IPresenter, USB recovery, config integration** - `17f77a9` (feat)
2. **Task 2: End-to-end verification preparation** - `8e6e3e5` (chore)
3. **Task 3: Human verification checkpoint** - pending user verification

**Plan metadata:** _(docs commit follows)_

## Files Created/Modified

- `src/main.cpp` - Full refactor: IPresenter polymorphism, --headless/--config flags, config-first+CLI-override merge, GUI four-callback wiring, headless auto-record with date dirs, USB recovery in capture thread (GUI: flag+presenter notify, headless: auto-retry), disk-full via presenter->tick() returning false

## Decisions Made

- presenter uses unique_ptr<IPresenter> created conditionally on config.headless flag
- on_reconnect_requested lambda captures camera by reference to enable destroy+recreate sequence from main thread
- Headless session name auto-generated only if session-name not explicitly provided on CLI (checks args.count("session-name") == 0)
- stop_recording extracted as lambda shared by GUI Stop callback, reconnect sequence, and shutdown path
- make_file_header() extracted to static helper to keep FileHeader assembly in main.cpp (Phase 1 decoupling decision preserved)
- GUI capture thread continues running during disconnect (sets camera_disconnected flag); reconnect handled in on_reconnect_requested callback from main thread

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - Task 3 (human verification) is the checkpoint. User needs to:
1. Run `./build/ego-recorder` with physical camera + display (GUI mode)
2. Run `./build/ego-recorder --headless --output /tmp --duration 10` (headless mode)
3. Test config file with --config flag
4. Verify USB reconnect behavior in both modes

## Next Phase Readiness

- Both build configurations (WITH_GUI=ON/OFF) verified to compile and link
- Binary --help shows all new flags in both builds
- Headless-only build correctly rejects GUI mode with clear error
- All critical wiring patterns confirmed in source: on_reconnect_requested, load_config, create_directories, camera_disconnected_error catch
- Pending: human verification of visual/functional behavior with real hardware (Task 3 checkpoint)

---
*Phase: 02-gui-mode-headless-systemd-service*
*Completed: 2026-02-19*
