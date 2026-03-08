---
phase: 03-optimized-compression-export-tools
plan: 01
subsystem: compression
tags: [zdepth, h264, ffmpeg, libavcodec, libswscale, pybind11, cmake, fetchcontent]

# Dependency graph
requires:
  - phase: 01-core-capture-engine-mvp-storage
    provides: "binary_format.h v1, CMakeLists.txt with ZSTD+turbojpeg"
  - phase: 02-gui-mode-headless-systemd-service
    provides: "Complete ego-recorder binary with GUI/headless modes"
provides:
  - "binary_format.h v2 with 0x02 magic byte and extensible codec ID documentation"
  - "ZdepthCompressor class wrapping catid/Zdepth for D435 Z16 depth compression"
  - "H264Encoder class wrapping FFmpeg libavcodec for RGB24 H.264 encoding"
  - "CMakeLists.txt with Zdepth FetchContent, FFmpeg pkg-config, pybind11 FetchContent"
affects: [03-02, 03-03, pipeline-integration, python-reader]

# Tech tracking
tech-stack:
  added: [catid/Zdepth, libavcodec 60.31.102, libavutil 58.29.100, libswscale 7.5.100, pybind11 v2.13.6]
  patterns: [pimpl-for-c-libs, interface-zstd-target-guard, fetchcontent-with-export-fix]

key-files:
  created:
    - src/compression/zdepth_compressor.h
    - src/compression/zdepth_compressor.cpp
    - src/compression/h264_encoder.h
    - src/compression/h264_encoder.cpp
  modified:
    - CMakeLists.txt
    - src/storage/binary_format.h

key-decisions:
  - "Zdepth encode mode: kNotQuantized8191mm for D435 (lossless up to 8191mm, covers typical indoor use)"
  - "INTERFACE zstd target (not ALIAS) to prevent Zdepth bundled zstd clash and support Zdepth's export(TARGETS)"
  - "Pimpl pattern for both compressors to isolate Zdepth/FFmpeg headers from consumers"
  - "H264Encoder: CRF 23, preset fast, max_b_frames=0, gop_size=fps for real-time"

patterns-established:
  - "Pimpl for C library wrappers: Impl struct hidden in .cpp, unique_ptr in header, isolates C headers"
  - "FetchContent zstd guard: add_library(zstd INTERFACE) + install(TARGETS zstd EXPORT zdepth) before Zdepth"

# Metrics
duration: 5min
completed: 2026-03-08
---

# Phase 3 Plan 1: Compression Building Blocks Summary

**V2 container format, Zdepth depth compressor (kNotQuantized8191mm), H.264 RGB encoder (CRF 23 via FFmpeg libavcodec), and all CMake dependencies (Zdepth FetchContent + FFmpeg pkg-config + pybind11)**

## Performance

- **Duration:** 5 min
- **Started:** 2026-03-08T04:17:39Z
- **Completed:** 2026-03-08T04:22:56Z
- **Tasks:** 3/3
- **Files modified:** 6

## Accomplishments
- Container format bumped to v2 (0x02 magic byte) with extensible codec ID documentation
- ZdepthCompressor wraps catid/Zdepth with compress/decompress, GOP=30, kNotQuantized8191mm mode
- H264Encoder wraps FFmpeg libavcodec with encode/flush/reset, CRF 23, no B-frames, RGB24->YUV420P via libswscale
- CMakeLists.txt integrates Zdepth FetchContent (with zstd clash guard), FFmpeg pkg-config, and pybind11 FetchContent

## Task Commits

Each task was committed atomically:

1. **Task 1: Container format v2 + CMake dependencies** - `d1dcfa4` (feat)
2. **Task 2: ZdepthCompressor RAII wrapper** - `0343e3a` (feat)
3. **Task 3: H264Encoder RAII wrapper** - `65dd038` (feat)

## Files Created/Modified
- `src/storage/binary_format.h` - FILE_MAGIC bumped to v2 (0x02), codec ID documentation added
- `src/compression/zdepth_compressor.h` - ZdepthCompressor class declaration with pimpl
- `src/compression/zdepth_compressor.cpp` - Zdepth DepthCompressor wrapper: compress/decompress, kNotQuantized8191mm
- `src/compression/h264_encoder.h` - H264Encoder class declaration with pimpl
- `src/compression/h264_encoder.cpp` - FFmpeg avcodec H.264 encode: RGB24->YUV420P->H.264, CRF 23, no B-frames
- `CMakeLists.txt` - Added Zdepth FetchContent, FFmpeg pkg-config, pybind11 FetchContent, WITH_PYTHON option

## Decisions Made
- **Zdepth encode mode:** `kNotQuantized8191mm` -- lossless for D435 Z16 values 0-8191mm. Values >= 8192mm (rare at ~10m max range) are clipped to 0. This covers typical indoor use where D435 accuracy degrades beyond ~4m anyway.
- **zstd target strategy:** INTERFACE target (not ALIAS) because Zdepth's CMake does `export(TARGETS zdepth zstd ...)` which requires a non-ALIAS target. Combined with `install(TARGETS zstd EXPORT zdepth)` to satisfy the export set.
- **Pimpl pattern:** Both ZdepthCompressor and H264Encoder use pimpl to isolate zdepth.hpp and FFmpeg C headers from header consumers. This prevents include pollution and allows changing internals without recompiling dependents.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Missing #include <memory> in stub headers**
- **Found during:** Task 1 (initial build of stub files)
- **Issue:** Both zdepth_compressor.h and h264_encoder.h used std::unique_ptr but did not include <memory>
- **Fix:** Added `#include <memory>` to both headers
- **Files modified:** src/compression/zdepth_compressor.h, src/compression/h264_encoder.h
- **Verification:** Build succeeds
- **Committed in:** d1dcfa4 (Task 1 commit)

**2. [Rule 3 - Blocking] Zdepth CMake zstd ALIAS target incompatibility**
- **Found during:** Task 1 (CMake configure)
- **Issue:** Plan suggested `add_library(zstd ALIAS PkgConfig::ZSTD)` but Zdepth's CMake does `export(TARGETS zdepth zstd ...)` which fails with ALIAS targets
- **Fix:** Used INTERFACE target instead: `add_library(zstd INTERFACE)` + `target_link_libraries(zstd INTERFACE PkgConfig::ZSTD)` + `install(TARGETS zstd EXPORT zdepth)`
- **Files modified:** CMakeLists.txt
- **Verification:** CMake configure and build succeed, no zstd symbol conflicts
- **Committed in:** d1dcfa4 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for compilation. The zstd target strategy was anticipated as a risk in the plan and research docs. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required. System FFmpeg and libzstd packages were already installed.

## Next Phase Readiness
- ZdepthCompressor and H264Encoder are compiled and linked but not yet wired into the recording pipeline
- Next plan (03-02) should integrate these into the writer thread, replacing JPEG+ZSTD with H264+Zdepth
- pybind11 FetchContent declared but module not yet created (planned for later plan)
- WITH_PYTHON=ON requires Python3 dev headers (available on dev machine)

## Self-Check: PASSED

All 7 files verified present. All 3 task commits verified in git log.

---
*Phase: 03-optimized-compression-export-tools*
*Completed: 2026-03-08*
