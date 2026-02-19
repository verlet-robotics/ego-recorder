---
phase: 01-core-capture-engine-mvp-storage
plan: 03
subsystem: capture
tags: [librealsense2, rs2, pipeline, camera, imu, intrinsics, extrinsics, d435, d435i, realsense, c++17]

# Dependency graph
requires:
  - phase: 01-01
    provides: CapturedFrame and IMUSample structs in frame_types.h; CMakeLists.txt with realsense2 linked
provides:
  - RealSensePipeline class: configure_and_start, poll_frame, stop, and all camera metadata getters
  - IMU runtime detection (try/catch D435i fallback to D435)
  - Camera intrinsics, extrinsics, depth scale, serial number, USB type exposed for FileHeader assembly
  - Warmup sequence (30 frames dropped), auto-exposure priority disabled, global time enabled
affects: [01-04]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Non-copyable/non-movable wrapper around rs2::pipeline (owns SDK resource)
    - IMU try/catch detection: attempt with accel+gyro first, catch rs2::error, retry without
    - Timestamp conversion: SDK milliseconds (double) to microseconds (uint64_t) via * 1000.0
    - Copy-on-exit: pixel data memcpy'd into vectors before frameset goes out of scope

key-files:
  created:
    - src/capture/pipeline.h
    - src/capture/pipeline.cpp
  modified: []

key-decisions:
  - "No binary_format.h dependency in pipeline: getters expose raw metadata for main.cpp to assemble FileHeader"
  - "IMU fallback via rs2::error catch: single binary handles D435 and D435i transparently at runtime"
  - "foreach_rs for IMU collection: handles variable IMU frame rate (200-400Hz) bundled in synchronized frameset"

patterns-established:
  - "Non-copyable pipeline wrapper: delete copy/move to prevent accidental rs2::pipeline duplication"
  - "data copy before frameset release: assign() from get_data() pointer while frameset is still alive"

# Metrics
duration: 2min
completed: 2026-02-19
---

# Phase 1 Plan 03: RealSense Pipeline Wrapper Summary

**RealSensePipeline wrapping rs2::pipeline with D435/D435i IMU detection, auto-exposure disable, global time enable, intrinsics/extrinsics extraction, 30-frame warmup, and CapturedFrame production via poll_frame().**

## Performance

- **Duration:** ~2 min
- **Started:** 2026-02-19T00:05:52Z
- **Completed:** 2026-02-19T00:07:27Z
- **Tasks:** 1/1
- **Files modified:** 2

## Accomplishments

- RealSensePipeline class implements the full initialization sequence: configure RGB8+Z16 streams at 640x480@30fps, IMU detection, auto-exposure priority disable, global time enable, depth scale + intrinsics + extrinsics extraction, 30-frame warmup
- IMU runtime detection via try/catch: enables RS2_STREAM_ACCEL + RS2_STREAM_GYRO for D435i, falls back to RGB+depth-only config for D435 when rs2::error is thrown
- poll_frame() blocks on wait_for_frames(), copies pixel data into CapturedFrame vectors before frameset goes out of scope (no dangling pointers), collects IMU samples via foreach_rs
- Getter methods (has_imu, serial_number, usb_type, depth_scale, depth_intrinsics, color_intrinsics, depth_to_color_extrinsics) expose all camera metadata for main.cpp to assemble FileHeader without creating pipeline-to-storage coupling
- Build succeeds with zero warnings against librealsense2 from ROS Jazzy

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement RealSense pipeline wrapper with full configuration** - `14795a3` (feat)

**Plan metadata:** (docs commit follows)

## Files Created/Modified

- `src/capture/pipeline.h` - RealSensePipeline class declaration: constructor, configure_and_start, stop, poll_frame, accessor getters, private rs2 members; no binary_format.h dependency
- `src/capture/pipeline.cpp` - Full initialization sequence (9 steps), IMU try/catch fallback, USB 2.0 warning, poll_frame with vector-copy pixel data and foreach_rs IMU collection

## Decisions Made

- **No binary_format.h dependency:** Getter methods expose raw rs2_intrinsics/rs2_extrinsics structs and primitive types so main.cpp (Plan 04) can populate FileHeader. This preserves Wave 2 independence between Plan 02 (binary_format) and Plan 03 (pipeline).
- **IMU detection via rs2::error catch:** The D435 does not advertise IMU capability until a start() attempt with motion streams fails. The try/catch pattern is the canonical approach per librealsense2 patterns.
- **foreach_rs for IMU collection:** IMU frames arrive at 200-400Hz bundled into the synchronized frameset. foreach_rs iterates all frames in the set to collect any motion frames, handling the variable rate naturally.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required. Build uses the same cmake settings as Plan 01 (TURBOJPEG_LOCAL_PREFIX, CMAKE_PREFIX_PATH for ROS Jazzy realsense2).

## Next Phase Readiness

- Plan 04 (main.cpp orchestration) can now call `pipeline.configure_and_start()` and `pipeline.poll_frame()` to obtain CapturedFrame structs
- All camera metadata getters are available for assembling FileHeader (depth_scale, intrinsics, extrinsics, serial, usb_type)
- Build already links against librealsense2 -- no cmake changes needed for Plan 04
- Full runtime verification requires a physical D435/D435i camera (deferred to Plan 04 integration)

---
*Phase: 01-core-capture-engine-mvp-storage*
*Completed: 2026-02-19*

## Self-Check: PASSED

- src/capture/pipeline.h: FOUND
- src/capture/pipeline.cpp: FOUND
- .planning/phases/01-core-capture-engine-mvp-storage/01-03-SUMMARY.md: FOUND
- Commit 14795a3: FOUND
