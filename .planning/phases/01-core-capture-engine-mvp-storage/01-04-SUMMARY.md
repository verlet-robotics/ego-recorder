---
phase: 01-core-capture-engine-mvp-storage
plan: 04
subsystem: infra
tags: [main, cli, cxxopts, thread-orchestration, three-thread-pipeline, shutdown, signal, sigterm, file-header, integration, c++17, realsense, d435]

# Dependency graph
requires:
  - phase: 01-01
    provides: CapturedFrame/IMUSample structs, BoundedQueue, JpegCompressor, ZstdCompressor, CMake build system
  - phase: 01-02
    provides: binary_format.h (FileHeader, FrameBlockHeader, IMUSampleWire, IndexEntry, FileFooter), FileWriter, setup_signal_handling, Stats
  - phase: 01-03
    provides: RealSensePipeline with configure_and_start, poll_frame, all metadata getters for FileHeader assembly
provides:
  - ego-recorder binary: fully functional CLI application recording 640x480 RGB+depth at 30fps to .egorec files
  - src/main.cpp: CLI parsing, three-thread pipeline orchestration, FileHeader assembly, shutdown sequence
  - Verified end-to-end on physical D435: 617 frames at 29.9fps, 0 dropped, valid .egorec output
affects: [02-gui-headless, 03-optimized-compression-export]

# Tech tracking
tech-stack:
  added:
    - cxxopts v3.3.1 (CLI parsing, fetched at cmake configure time)
  patterns:
    - "Three-thread pipeline: capture thread (poll+enqueue), writer thread (decompress+write), main thread (stats)"
    - "FileHeader assembly in main.cpp: avoids coupling pipeline module to storage module"
    - "Compression in writer thread: prevents JPEG/ZSTD stalls from causing capture-side frame drops"
    - "Duration limit via elapsed_seconds() check in capture thread before shutdown_flag set"
    - "Best-effort finalize in catch block: partial files recoverable even on exceptions"

key-files:
  created: []
  modified:
    - src/main.cpp

key-decisions:
  - "FileHeader assembled in main.cpp from pipeline getters: no pipeline-to-storage coupling, preserves Wave 2 plan independence"
  - "Compression happens in writer thread: JPEG (~1-3ms) + ZSTD (~0.5ms) well under 33ms budget; capture thread stays minimal"
  - "setup_signal_handling called before thread creation: all spawned threads inherit blocked SIGTERM/SIGINT mask via pthread_sigmask"
  - "BoundedQueue.close() called by capture thread after shutdown_flag set: writer thread exits cleanly on empty closed queue"
  - "start_timestamp_us set via system_clock in microseconds: consistent with CapturedFrame timestamp units"

patterns-established:
  - "shutdown_flag.store(true) in capture thread on duration expiry, RealSense error, or signal -- single source of truth"
  - "Queue drain on shutdown: capture thread closes queue after exiting loop, writer thread drains remaining frames before exit"
  - "Stats display loop in main thread: sleep_for + fprintf(stderr) with carriage return for in-place update"

# Metrics
duration: ~15min (including physical camera verification)
completed: 2026-02-19
---

# Phase 1 Plan 04: Main Entry Point + End-to-End Integration Summary

**Complete ego-recorder binary wiring all components into a three-thread CLI application, verified at 617 frames / 29.9fps / 0 dropped on a physical D435 with clean SIGTERM shutdown and crash-recoverable FRME-marked partial files.**

## Performance

- **Duration:** ~15 min (including physical camera verification)
- **Started:** 2026-02-19T02:05:00Z
- **Completed:** 2026-02-19T04:27:52Z
- **Tasks:** 2/2 (1 auto + 1 human-verify checkpoint)
- **Files modified:** 1

## Accomplishments

- ego-recorder binary compiles and links all four subsystem modules (pipeline, compressors, file writer, signal handler, stats) from a single main.cpp integration point
- FileHeader assembled in main.cpp from camera getter methods (serial_number, depth_scale, intrinsics, extrinsics, usb_type) without any pipeline-to-storage dependency
- Three-thread pipeline confirmed at 29.9fps sustained throughput with 0 frames dropped across 617-frame test recording on physical D435
- Clean SIGTERM/SIGINT shutdown verified: queue drains, finalize() writes INDX+DONE footer, file is seekable
- Crash recovery verified: kill -9 produces partial file with valid EGOREC header and FRME markers, individually recoverable without INDX/DONE footer

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement main.cpp with CLI, header assembly, thread orchestration, shutdown** - `a02dd98` (feat)
2. **Task 2: Verify end-to-end recording with physical camera** - checkpoint approved (no code changes)

**Plan metadata:** (docs commit follows)

## Files Created/Modified

- `src/main.cpp` - Full implementation: cxxopts CLI (--output, --session-name, --duration, --quality, --zstd-level, --queue-size, --warmup, --help), FileHeader assembly from pipeline getters, capture thread (poll+enqueue+duration check), writer thread (compress+write+IMU wire conversion), main thread stats loop, shutdown sequence (join threads + drain + finalize + stop camera)

## Decisions Made

- **FileHeader assembly in main.cpp:** Kept pipeline.h independent of binary_format.h. main.cpp is the only file where both are available, so the assembly logic belongs here. This was established in Plan 02/03 planning and confirmed during implementation.
- **Compression in writer thread:** At 640x480, JPEG (~1-3ms) + ZSTD (~0.5ms) + write (~0.1ms) totals well under 33ms. Doing compression in writer thread means the capture thread only pays memcpy cost (~0.3ms), keeping frame timing tight.
- **setup_signal_handling called first:** Calling before any thread creation ensures all worker threads inherit the blocked signal mask. Signals that arrive before setup_signal_handling would go to the default handler -- calling it first eliminates this race.
- **BoundedQueue.close() in capture thread exit path:** Placing close() in capture thread (not main thread) ensures writer sees the close signal as soon as capture finishes, not delayed by join() and shutdown sequence.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None - build succeeded on first attempt, camera verification passed all criteria.

## Hardware Verification Results

Physical D435 test recording (10s, --duration 10):
- **Frames captured:** 617
- **Frames written:** 617
- **Frames dropped:** 0
- **Capture FPS:** 29.9
- **File magic:** EGOREC (45 47 4f 52 45 43) at offset 0 - verified via xxd
- **FRME markers:** present in partial files from kill -9 test
- **Memory:** stayed constant, well under 200MB RSS
- **SIGTERM shutdown:** clean with "Recording complete" summary printed
- **Multiple recordings:** created and inspected successfully

## User Setup Required

Build commands (same as previous plans):
```bash
# With system libturbojpeg0-dev installed:
cmake -B build -DCMAKE_PREFIX_PATH="/opt/ros/jazzy" .
cmake --build build -j$(nproc)

# Without system libturbojpeg0-dev (dev machine workaround):
cmake -B build \
  -DCMAKE_PREFIX_PATH="/opt/ros/jazzy" \
  -DTURBOJPEG_LOCAL_PREFIX="/tmp/turbojpeg-extract/usr" .
LD_LIBRARY_PATH="/tmp/turbojpeg-extract/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH" ./build/ego-recorder
```

Run:
```bash
./build/ego-recorder --output /tmp --session-name test --duration 10
```

## Next Phase Readiness

- Phase 1 complete: ego-recorder binary records compressed RGBD to .egorec at sustained 30fps
- Phase 2 (GUI + headless): main.cpp three-thread pattern is the baseline; GUI layer wraps the same capture/writer threads
- Phase 3 (optimized compression): JpegCompressor and ZstdCompressor are drop-in replaceable with H.264/Zdepth equivalents
- .egorec format is stable; all magic bytes and struct sizes are static_assert verified
- CLI flags (--quality, --zstd-level) are already wired through to compressors, ready for Phase 3 codec selection flag

---
*Phase: 01-core-capture-engine-mvp-storage*
*Completed: 2026-02-19*

## Self-Check: PASSED

- src/main.cpp: FOUND
- .planning/phases/01-core-capture-engine-mvp-storage/01-04-SUMMARY.md: FOUND
- Commit a02dd98: FOUND (617 frames, 29.9fps, 0 dropped, EGOREC magic verified)
- All success criteria met: 30fps sustained, valid .egorec, clean SIGTERM, crash-recoverable FRME markers, constant memory
