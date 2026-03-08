---
phase: 03-optimized-compression-export-tools
plan: 02
subsystem: pipeline-integration
tags: [h264, zdepth, compression, pipeline, cli, subcommands, ffmpeg, libavcodec]

# Dependency graph
requires:
  - phase: 03-optimized-compression-export-tools
    plan: 01
    provides: "ZdepthCompressor, H264Encoder RAII wrappers, binary_format.h v2"
  - phase: 01-core-capture-engine-mvp-storage
    provides: "FileWriter, main.cpp pipeline, BoundedQueue, Stats"
provides:
  - "Writer thread with H.264 RGB + Zdepth depth compression (replaces JPEG+ZSTD)"
  - "FileHeader with rgb_codec=2 (H264) and depth_codec=2 (Zdepth)"
  - "write_trailing_codec_data() for safe H.264 flush without index table corruption"
  - "ego-recorder info subcommand for .egorec metadata inspection"
  - "ego-recorder export rlds/lerobot dispatch to Python scripts"
  - "Config h264_crf field with --crf CLI flag"
affects: [03-03, 03-04, 03-05, python-reader, export-scripts]

# Tech tracking
tech-stack:
  added: []
  patterns: [trailing-codec-data-flush, subcommand-dispatch-before-cxxopts, execvp-python-dispatch]

key-files:
  created: []
  modified:
    - src/main.cpp
    - src/config/config.h
    - src/config/config.cpp
    - src/storage/file_writer.h
    - src/storage/file_writer.cpp

key-decisions:
  - "write_trailing_codec_data() instead of write_frame() for H.264 flush -- prevents index table corruption by writing raw bytes without FrameBlockHeader/IndexEntry"
  - "Subcommand dispatch before cxxopts -- info/export intercepted at argc/argv level before options parsing, avoiding conflicts"
  - "H.264 encoder reset on stop_recording -- enables clean state for reconnect scenarios without encoder artifacts"
  - "Zdepth GOP=30 via frame_number modulo -- keyframe every 30 frames matching camera FPS for 1-second seek granularity"

patterns-established:
  - "Trailing codec data: write raw bytes between last frame block and index table for codec flush data (H.264 NAL units)"
  - "Subcommand interception: check argv[1] before cxxopts parsing for subcommands that don't share CLI options"
  - "Export dispatch: execvp to Python with PYTHONPATH set to binary directory for finding .so modules"

# Metrics
duration: 3min
completed: 2026-03-08
---

# Phase 3 Plan 2: Pipeline Integration + CLI Subcommands Summary

**H.264+Zdepth compression wired into writer thread replacing JPEG+ZSTD, with safe flush design, plus info/export CLI subcommands for metadata inspection and Python script dispatch**

## Performance

- **Duration:** 3 min
- **Started:** 2026-03-08T05:22:24Z
- **Completed:** 2026-03-08T05:26:16Z
- **Tasks:** 2/2
- **Files modified:** 5

## Accomplishments
- Writer thread now encodes RGB via H264Encoder and depth via ZdepthCompressor instead of JPEG+ZSTD
- FileHeader writes rgb_codec=2 (H264) and depth_codec=2 (Zdepth) for v2 container format
- H.264 flush uses write_trailing_codec_data() to write NAL units without corrupting index table
- ego-recorder info subcommand reads and prints .egorec file metadata (tested on real capture files)
- ego-recorder export rlds/lerobot dispatches to Python scripts via execvp with PYTHONPATH

## Task Commits

Each task was committed atomically:

1. **Task 1: Replace JPEG+ZSTD with H.264+Zdepth in writer thread** - `977afe4` (feat)
2. **Task 2: Info subcommand and export subcommand dispatch** - `2fa2dc3` (feat)

## Files Created/Modified
- `src/main.cpp` - H.264+Zdepth compression in writer thread, info/export subcommands, --crf CLI flag
- `src/config/config.h` - Added h264_crf field to Config struct
- `src/config/config.cpp` - Load h264_crf from TOML [compression] section
- `src/storage/file_writer.h` - Added write_trailing_codec_data() declaration
- `src/storage/file_writer.cpp` - Implemented write_trailing_codec_data() for raw byte writes without index entries

## Decisions Made
- **Trailing codec data flush:** write_trailing_codec_data() writes raw bytes to file without creating FrameBlockHeader or IndexEntry. This prevents H.264 flush data from corrupting the index table. The reader recovers trailing data by reading bytes between the last indexed frame block's end and the footer's index_offset.
- **Subcommand dispatch before cxxopts:** info and export are intercepted at the top of main() before cxxopts::Options is declared. This avoids requiring subcommand-aware option parsing and keeps the existing CLI structure intact.
- **GOP=30 for Zdepth:** Keyframes are emitted every 30 frames via `frame.frame_number % 30 == 0`, matching camera FPS for ~1 second seek granularity.
- **H.264 encoder reset:** h264.reset() is called in stop_recording after flush, ensuring clean encoder state for reconnect scenarios.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Missing #include <unistd.h> for execvp**
- **Found during:** Task 2 (export subcommand implementation)
- **Issue:** execvp() not declared in scope -- plan specified `<cerrno>` and `<cstdlib>` but not `<unistd.h>` which defines execvp on POSIX systems
- **Fix:** Added `#include <unistd.h>` to main.cpp includes
- **Files modified:** src/main.cpp
- **Verification:** Build succeeds, export subcommand works
- **Committed in:** 2fa2dc3 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Missing include was the only issue. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviation above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Recording pipeline now produces .egorec v2 files with H.264+Zdepth compression
- Info subcommand provides C++-only metadata inspection
- Export dispatch framework ready -- Python scripts (export_rlds.py, export_lerobot.py) need to be created in plans 04/05
- pybind11 reader module (egorec_reader.so) needed for Python scripts to read .egorec files (plan 03)
- JPEG and ZSTD compressor code still exists in codebase but is no longer used in recording pipeline

## Self-Check: PASSED

All 5 modified files verified present. Both task commits verified in git log.

---
*Phase: 03-optimized-compression-export-tools*
*Completed: 2026-03-08*
