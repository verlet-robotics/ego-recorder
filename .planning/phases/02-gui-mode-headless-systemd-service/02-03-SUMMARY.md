---
phase: 02-gui-mode-headless-systemd-service
plan: 03
subsystem: headless-service
tags: [headless, systemd, sd_notify, sd_bus, dbus, inhibitor, watchdog, udev, deploy]

# Dependency graph
requires:
  - phase: 02-gui-mode-headless-systemd-service
    plan: 01
    provides: IPresenter interface, Config struct
  - phase: 01-core-capture-engine-mvp-storage
    provides: Stats class used in update_stats

provides:
  - HeadlessPresenter implementing IPresenter with full systemd integration
  - deploy/ directory with 5 production deployment artifacts

affects:
  - 02-04-PLAN (main.cpp selects HeadlessPresenter when --headless flag present)

# Tech tracking
tech-stack:
  added:
    - libsystemd sd_notify API (READY/WATCHDOG/STATUS/STOPPING lifecycle)
    - sd-bus C API (D-Bus inhibitor lock via org.freedesktop.login1.Manager.Inhibit)
    - std::filesystem::space (disk monitoring)
    - systemd RuntimeDirectory (auto-creates /run/ego-recorder with correct ownership)
  patterns:
    - HAVE_SYSTEMD guard: graceful degradation without libsystemd
    - Atomic rename for status file (write to .tmp, then rename for consistency)
    - dup() before sd_bus_message_unref to preserve inhibitor fd ownership
    - Watchdog at half-interval (sd_watchdog_enabled returns WatchdogSec in usec)
    - on_request_shutdown_ callback signals main.cpp without presenter knowing about main

key-files:
  created:
    - src/presenter/headless_presenter.h
    - src/presenter/headless_presenter.cpp
    - deploy/ego-recorder.service
    - deploy/99-ego-recorder.rules
    - deploy/50-ego-recorder-lid.conf
    - deploy/install.sh
    - deploy/config.toml.example
  modified:
    - CMakeLists.txt

key-decisions:
  - "HeadlessPresenter takes on_request_shutdown callback -- decouples disk-full handling from main.cpp details"
  - "Status file uses atomic rename (write to .tmp then rename) -- reader never sees partial JSON"
  - "FPS stored as atomic<uint64_t> fps_x10 (FPS * 10) -- avoids atomic<double> which is not lock-free on x86"
  - "gui_presenter.cpp gated behind target_sources WITH_GUI conditional -- prevents broken builds before plan 02-02"
  - "deploy/config.toml.example disk_min_mb=1000 (production) vs 500 (interactive) -- more conservative for unattended"
  - "install.sh enables but does not start service -- user reviews config first"

# Metrics
duration: ~3min
completed: 2026-02-19
---

# Phase 02 Plan 03: HeadlessPresenter + Deployment Artifacts Summary

**HeadlessPresenter with sd_notify READY/WATCHDOG/STATUS/STOPPING lifecycle, sd-bus D-Bus inhibitor blocking lid-close, disk monitoring with configurable threshold, plus 5 production deployment files (systemd unit, udev rules, logind drop-in, install script, headless config)**

## Performance

- **Duration:** ~3 min
- **Started:** 2026-02-19T06:47:27Z
- **Completed:** 2026-02-19T06:50:00Z
- **Tasks:** 2/2
- **Files modified:** 8 (7 created, 1 modified)

## Accomplishments

- HeadlessPresenter fully implements IPresenter (6 methods: start, tick, shutdown, on_camera_disconnect, on_camera_reconnect, update_stats)
- sd_notify lifecycle: READY=1 after camera opens, WATCHDOG=1 at half-interval, STATUS= with frames+FPS+disk free, STOPPING=1 before exit
- D-Bus inhibitor lock via sd_bus_call_method to org.freedesktop.login1.Manager.Inhibit; dup() of fd before message unref (research pitfall 5 addressed)
- Disk space monitoring via std::filesystem::space every 30 ticks; stops recording and calls on_request_shutdown_ below threshold
- Atomic JSON status file at /run/ego-recorder/status written via rename for consistency
- All systemd/sd-bus calls gated behind HAVE_SYSTEMD for graceful degradation on non-systemd builds
- 5 deployment artifacts in deploy/: systemd unit (Type=notify, WatchdogSec=30s, RuntimeDirectory), udev rules (D435/D435i autosuspend disable), logind drop-in (fallback, documented as such), install script (set -euo pipefail, root check, idempotent user creation), headless config (disk_min_mb=1000, headless=true)
- CMakeLists.txt: gui_presenter.cpp properly gated via target_sources conditional

## Task Commits

Each task was committed atomically:

1. **Task 1: HeadlessPresenter with sd_notify, D-Bus inhibitor, disk monitoring** - `2dfae82` (feat)
2. **Task 2: Deployment artifacts (systemd unit, udev rules, logind drop-in, install script)** - `30f4b67` (feat)

**Plan metadata:** _(docs commit follows)_

## Files Created/Modified

- `src/presenter/headless_presenter.h` - HeadlessPresenter class declaration; HAVE_SYSTEMD guards on sd_bus* member; atomic fps_x10 for lock-free FPS storage
- `src/presenter/headless_presenter.cpp` - 267-line implementation; private helpers: take_inhibitor_lock, release_inhibitor_lock, write_status_file, disk_free_mb
- `deploy/ego-recorder.service` - Type=notify, WatchdogSec=30s, User=ego-recorder, RuntimeDirectory=ego-recorder, Restart=on-failure, OOMScoreAdjust=-100
- `deploy/99-ego-recorder.rules` - D435 (0x0b07) and D435i (0x0b3a) rules: MODE=0660, GROUP=plugdev, ATTR{power/control}=on
- `deploy/50-ego-recorder-lid.conf` - [Login] HandleLidSwitch=ignore; documented as fallback to D-Bus inhibitor
- `deploy/install.sh` - Full install script: set -euo pipefail, root check, idempotent useradd, plugdev+video groups, config preservation on upgrade, systemctl enable without start
- `deploy/config.toml.example` - Production defaults: output.dir=/var/lib/ego-recorder/recordings, disk_min_mb=1000, headless=true
- `CMakeLists.txt` - gui_presenter.cpp moved to target_sources(WITH_GUI) conditional block

## Decisions Made

- HeadlessPresenter receives on_request_shutdown_ as constructor callback -- disk-full signaling without coupling to main.cpp internals
- FPS stored as uint64_t (fps * 10) in atomic to ensure lock-free operation without atomic<double>
- Status file written atomically: temp file + rename -- no partial JSON visible to readers
- deploy/config.toml.example disk_min_mb=1000 vs root config.toml.example 500 -- more conservative for production unattended use
- install.sh enables service but does not start it -- operator reviews config before first run
- CMakeLists.txt gui_presenter.cpp gated via target_sources() rather than comment -- prevents linter from adding it unconditionally (which would break builds before plan 02-02)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] CMakeLists.txt linter kept adding gui_presenter.cpp unconditionally**
- **Found during:** Task 2
- **Issue:** An automated tool added `src/presenter/gui_presenter.cpp` directly to the add_executable() sources list, breaking WITH_GUI=OFF builds since the file does not yet exist
- **Fix:** Moved gui_presenter.cpp to a `target_sources(ego-recorder PRIVATE ...)` call inside `if(WITH_GUI)` block so it is only compiled when the GUI is enabled
- **Files modified:** CMakeLists.txt
- **Commit:** 30f4b67 (included in Task 2 commit)

## Issues Encountered

None beyond the auto-fixed CMakeLists.txt deviation above.

## User Setup Required

None -- all implementation is complete. Deployment requires the binary to be compiled and install.sh run as root.

## Next Phase Readiness

- HeadlessPresenter ready for integration in plan 02-04 (main.cpp presenter selection)
- deploy/ artifacts ready for system installation
- WITH_GUI=OFF build verified clean with both headless_presenter.cpp and updated CMakeLists.txt
- Plan 02-02 (GuiPresenter) can proceed independently -- gated via WITH_GUI=ON

---
*Phase: 02-gui-mode-headless-systemd-service*
*Completed: 2026-02-19*
