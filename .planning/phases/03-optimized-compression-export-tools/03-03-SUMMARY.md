---
phase: 03-optimized-compression-export-tools
plan: 03
subsystem: python-bridge
tags: [pybind11, python, egorec-reader, h264-decoder, zdepth, numpy, ffmpeg, libavcodec]

# Dependency graph
requires:
  - phase: 03-optimized-compression-export-tools
    plan: 01
    provides: "ZdepthCompressor, H264Encoder RAII wrappers, binary_format.h v2"
  - phase: 03-optimized-compression-export-tools
    plan: 02
    provides: "H.264+Zdepth pipeline, write_trailing_codec_data(), container v2 format"
provides:
  - "egorec_reader.so Python extension module for reading .egorec v2 files"
  - "EgorecFile class with header(), frame_count(), frames() API"
  - "Decoded RGB (numpy uint8 H,W,3) and depth (numpy uint16 H,W) per frame"
  - "Stateful H.264 decoding with P-frame dependencies and multi-frame output queue"
  - "Trailing H.264 flush data recovery at end-of-iteration"
  - "V1 .egorec rejection with descriptive error"
affects: [03-04, 03-05, export-rlds, export-lerobot]

# Tech tracking
tech-stack:
  added: [pybind11-module]
  patterns: [pybind11-numpy-zero-copy, h264-decoder-state-machine, frame-iterator-with-keep-alive]

key-files:
  created:
    - src/python/egorec_reader.cpp
  modified:
    - CMakeLists.txt

key-decisions:
  - "ZdepthCompressor as unique_ptr member -- avoids non-movable constraint since EgorecFile itself is non-copyable/non-movable"
  - "decoded_rgb_queue_ as deque -- handles H.264 decoder outputting multiple frames from a single packet (P-frame buffering)"
  - "trailing_flushed_ flag -- ensures flush-trailing-and-decoder is called exactly once at end-of-iteration"
  - "zdepth_compressor.cpp compiled directly into pybind11 module -- our RAII wrapper symbols must be in the .so, not just the zdepth library"

patterns-established:
  - "pybind11 keep_alive: frames() returns iterator with py::keep_alive<0, 1>() to prevent EgorecFile destruction while iterating"
  - "H.264 decode loop: avcodec_receive_frame in a while loop to drain all buffered frames per packet"
  - "Trailing codec data recovery: read bytes between last indexed frame block end and index_offset, feed to decoder before final flush"

# Metrics
duration: 3min
completed: 2026-03-08
---

# Phase 3 Plan 3: Python Reader Module (pybind11) Summary

**pybind11 C extension module (egorec_reader.so) reading .egorec v2 files with stateful H.264 decoding and Zdepth decompression, returning decoded RGB/depth as numpy arrays**

## Performance

- **Duration:** 3 min
- **Started:** 2026-03-08T05:28:37Z
- **Completed:** 2026-03-08T05:32:17Z
- **Tasks:** 2/2
- **Files modified:** 2

## Accomplishments
- egorec_reader.so Python extension module builds and imports successfully
- EgorecFile reads .egorec v2 header/footer/index and returns full metadata as Python dict
- FrameIterator decodes H.264 RGB to numpy uint8 (H,W,3) and Zdepth depth to numpy uint16 (H,W)
- H.264 decoder handles P-frame state with multi-frame output queue for deferred decode output
- Trailing codec flush data recovered at end-of-iteration (bytes between last frame block and index table)
- V1 .egorec files rejected with descriptive error: "V1 .egorec files are not supported by export tools. Re-record with v2 format."

## Task Commits

Each task was committed atomically:

1. **Task 1: EgorecFile class with header/footer/index parsing and raw frame reading** - `901b25d` (feat)
2. **Task 2: Frame iterator with H.264 decode, Zdepth decompress, trailing flush recovery, numpy output, and CMake integration** - `e05f5fc` (feat)

## Files Created/Modified
- `src/python/egorec_reader.cpp` - pybind11 module: EgorecFile class with H.264 decoder, ZdepthCompressor, FrameIterator, numpy array output
- `CMakeLists.txt` - pybind11_add_module target inside WITH_PYTHON block, linking zdepth/zstd/avcodec/avutil/swscale

## Decisions Made
- **ZdepthCompressor as unique_ptr:** The ZdepthCompressor class is non-copyable and non-movable (pimpl pattern). Using unique_ptr as a member allows deferred initialization in the constructor after reading the header's depth dimensions.
- **decoded_rgb_queue_ as deque:** The H.264 decoder can produce zero, one, or multiple frames from a single send_packet call. A deque buffers these and pops one per read_frame() call, keeping frame-to-frame correspondence with depth data.
- **zdepth_compressor.cpp compiled into .so:** Our RAII wrapper's symbols (ZdepthCompressor constructor/destructor/decompress) are not in the zdepth library -- they're our own code wrapping it. Including the source directly in pybind11_add_module ensures the symbols are available in the extension module.
- **trailing_flushed_ flag:** Prevents double-flushing the H.264 decoder. The flush is triggered once when current_frame_ exceeds total_frames_, and the flag prevents re-entry from the FrameIterator's next() method.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Missing ZdepthCompressor symbols in shared module**
- **Found during:** Task 2 (CMake integration and import test)
- **Issue:** egorec_reader.so failed to import with `undefined symbol: _ZN16ZdepthCompressorC1Eii` -- our RAII wrapper's constructor is in zdepth_compressor.cpp, not in the zdepth library
- **Fix:** Added `src/compression/zdepth_compressor.cpp` as a source in pybind11_add_module
- **Files modified:** CMakeLists.txt
- **Verification:** Module imports successfully, EgorecFile class accessible
- **Committed in:** e05f5fc (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Missing source file was the only issue. Required adding one line to CMake. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviation above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- egorec_reader.so is importable from Python via PYTHONPATH=build_v2
- EgorecFile exposes header(), frame_count(), frames() -- the complete API needed by export scripts
- Plans 03-04 (RLDS export) and 03-05 (LeRobot export) can now import egorec_reader and iterate decoded frames
- No v2 .egorec test files exist yet (existing captures are v1) -- export scripts will need a v2 recording to fully test end-to-end

## Self-Check: PASSED

All 2 created/modified files verified present. egorec_reader.so built successfully. Both task commits verified in git log.

---
*Phase: 03-optimized-compression-export-tools*
*Completed: 2026-03-08*
