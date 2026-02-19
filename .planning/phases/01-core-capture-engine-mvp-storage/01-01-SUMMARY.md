---
phase: 01-core-capture-engine-mvp-storage
plan: 01
subsystem: infra
tags: [cmake, c++17, librealsense2, libjpeg-turbo, turbojpeg, libzstd, zstd, threading, compression]

# Dependency graph
requires: []
provides:
  - CMake project with realsense2, libturbojpeg, libzstd, cxxopts dependencies
  - CapturedFrame and IMUSample structs with move-only semantics
  - BoundedQueue<T> thread-safe bounded queue with drop-oldest and close semantics
  - JpegCompressor RAII wrapper with pre-allocated buffers and TJFLAG_NOREALLOC
  - ZstdCompressor RAII wrapper with reusable ZSTD_CCtx context
  - Stub source files for pipeline, storage, signal handler, stats
affects: [01-02, 01-03, 01-04]

# Tech tracking
tech-stack:
  added:
    - librealsense2 (from ROS Jazzy /opt/ros/jazzy)
    - libjpeg-turbo 2.1.5 (TurboJPEG API via libturbojpeg0-dev)
    - libzstd 1.5.5
    - cxxopts v3.3.1 (via FetchContent)
  patterns:
    - RAII C-handle ownership: constructor acquires, destructor frees
    - Pre-allocated compression buffers with NOREALLOC semantics
    - Header-only template for threading primitive (BoundedQueue)
    - Local pkg-config prefix injection for non-system-installed libraries

key-files:
  created:
    - CMakeLists.txt
    - src/capture/frame_types.h
    - src/threading/bounded_queue.h
    - src/compression/jpeg_compressor.h
    - src/compression/jpeg_compressor.cpp
    - src/compression/zstd_compressor.h
    - src/compression/zstd_compressor.cpp
    - src/main.cpp (temporary verification test)
    - src/capture/pipeline.cpp (stub)
    - src/storage/file_writer.cpp (stub)
    - src/utils/signal_handler.cpp (stub)
    - src/utils/stats.cpp (stub)
    - .gitignore
  modified: []

key-decisions:
  - "TJSAMP_420 chroma subsampling for JPEG: ~30% smaller files, adequate for ML training"
  - "TJFLAG_NOREALLOC + pre-allocated buffer eliminates per-frame heap allocation in JPEG path"
  - "ZSTD level 1 as default: <0.5ms per frame, 3-4x compression on depth data"
  - "BoundedQueue drop-oldest (not block) so capture thread never stalls"
  - "TURBOJPEG_LOCAL_PREFIX cmake option for systems without libturbojpeg0-dev system-installed"

patterns-established:
  - "RAII C-handle: constructor acquires, destructor frees, non-copyable non-movable for handles"
  - "Pre-allocate worst-case buffers at construction, never reallocate in hot path"
  - "Stub .cpp files with single comment for CMake placeholder pattern"

# Metrics
duration: 5min
completed: 2026-02-19
---

# Phase 1 Plan 01: CMake Foundation + Compression Wrappers Summary

**CMake build system with realsense2/turbojpeg/zstd/cxxopts, CapturedFrame/IMUSample structs, BoundedQueue<T> with drop-oldest, and zero-allocation JPEG/ZSTD compression wrappers verified on synthetic 640x480 data.**

## Performance

- **Duration:** ~5 min
- **Started:** 2026-02-18T23:58:14Z
- **Completed:** 2026-02-19T00:03:00Z
- **Tasks:** 2/2
- **Files modified:** 13

## Accomplishments

- CMake project builds with C++17, linking realsense2 from ROS Jazzy, libturbojpeg, libzstd, and cxxopts v3.3.1 fetched at configure time
- JpegCompressor compresses 640x480 RGB24 to JPEG with pre-allocated buffer and TJFLAG_NOREALLOC (zero per-frame allocation)
- ZstdCompressor compresses depth data with reusable ZSTD_CCtx context at level 1
- BoundedQueue<T> correctly implements drop-oldest (pushed 6, capacity 4, dropped 2, popped 4)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create CMake project, frame types, and bounded queue** - `d4b3ffe` (feat)
2. **Task 2: Implement JPEG and ZSTD compression wrappers** - `5586db1` (feat)

**Plan metadata:** (see below - docs commit)

## Files Created/Modified

- `CMakeLists.txt` - C++17 project with realsense2, turbojpeg, zstd, cxxopts, optional systemd; TURBOJPEG_LOCAL_PREFIX option for non-system installs
- `src/capture/frame_types.h` - CapturedFrame (timestamp, frame_number, rgb_data, depth_data, imu_samples) and IMUSample (timestamp, accel[3], gyro[3]); move-only
- `src/threading/bounded_queue.h` - Header-only BoundedQueue<T> with push(drop-oldest), pop(blocking), close(), dropped(), size()
- `src/compression/jpeg_compressor.h/.cpp` - JpegCompressor RAII TurboJPEG wrapper; tjInitCompress + pre-allocated tjAlloc buffer + TJFLAG_NOREALLOC|TJFLAG_FASTDCT|TJSAMP_420
- `src/compression/zstd_compressor.h/.cpp` - ZstdCompressor RAII ZSTD wrapper; ZSTD_createCCtx + ZSTD_compressBound pre-allocation + ZSTD_compressCCtx at level 1
- `src/main.cpp` - Temporary verification test (replaced by real main in Plan 04)
- `src/capture/pipeline.cpp` - Stub (implemented in plan 02)
- `src/storage/file_writer.cpp` - Stub (implemented in plan 03)
- `src/utils/signal_handler.cpp` - Stub (implemented in plan 04)
- `src/utils/stats.cpp` - Stub (implemented in plan 04)
- `.gitignore` - Excludes build/, *.deb, *.o, *.a

## Decisions Made

- **TJSAMP_420:** 4:2:0 chroma subsampling chosen per research recommendation (~30% smaller vs 4:4:4, adequate for ML training at 224x224)
- **TJFLAG_NOREALLOC:** Pre-allocate buffer at construction via tjBufSize, pass it to tjCompress2 with NOREALLOC to eliminate per-frame malloc
- **ZSTD level 1:** Fastest compression level (<0.5ms per 614KB frame per research), reusable CCtx eliminates context allocation per frame
- **BoundedQueue drop-oldest:** Producer (capture thread) must never block; oldest frames are sacrificed rather than stalling capture
- **TURBOJPEG_LOCAL_PREFIX cmake option:** libturbojpeg0-dev was not system-installed; cmake generates a corrected pkg-config file in the build directory

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] libturbojpeg0-dev not system-installed; extracted from .deb**
- **Found during:** Task 1 (CMake configure)
- **Issue:** `libturbojpeg0-dev` provides `turbojpeg.h` but was not installed on the system. `libjpeg-turbo8-dev` (installed) only provides the older libjpeg API without `turbojpeg.h`. CMake pkg-config found no `libturbojpeg`.
- **Fix:** Downloaded `libturbojpeg0-dev` and `libturbojpeg` .deb packages via `apt-get download`, extracted to `/tmp/turbojpeg-extract/` via `dpkg-deb -x`. Added `TURBOJPEG_LOCAL_PREFIX` cmake option that auto-generates a corrected pkg-config file in the build directory with absolute paths, then injects it via `PKG_CONFIG_PATH`.
- **Files modified:** CMakeLists.txt
- **Verification:** CMake finds libturbojpeg 2.1.5, build succeeds, binary runs and produces JPEG output
- **Committed in:** `d4b3ffe` (Task 1), `5586db1` (Task 2 refined the cmake approach)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Required workaround for missing dev package. CMakeLists.txt now has clean option for local prefix. Actual developer setup note: install `libturbojpeg0-dev` via `sudo apt install libturbojpeg0-dev` and the TURBOJPEG_LOCAL_PREFIX option becomes unnecessary.

## Issues Encountered

- CMake cached stale `TURBOJPEG_INCLUDE_DIRS` from first configure run (before local prefix was set). Required `rm -rf build/` and fresh configure. Added to documentation.
- The .deb-extracted pkg-config file hardcodes `prefix=/usr` rather than using relative paths, requiring generation of a patched .pc file in the build directory.

## User Setup Required

**To build on a system with `libturbojpeg0-dev` installed (normal case):**
```bash
cmake -B build -DCMAKE_PREFIX_PATH="/opt/ros/jazzy" .
cmake --build build -j$(nproc)
```

**Without system libturbojpeg0-dev (workaround used during development):**
```bash
apt-get download libturbojpeg0-dev libturbojpeg
dpkg-deb -x libturbojpeg0-dev_*.deb /tmp/turbojpeg-extract
dpkg-deb -x libturbojpeg_*.deb /tmp/turbojpeg-extract
cmake -B build \
  -DCMAKE_PREFIX_PATH="/opt/ros/jazzy" \
  -DTURBOJPEG_LOCAL_PREFIX="/tmp/turbojpeg-extract/usr" .
LD_LIBRARY_PATH="/tmp/turbojpeg-extract/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH" ./build/ego-recorder
```

## Next Phase Readiness

- Plan 02 (capture pipeline) can include `frame_types.h` and `bounded_queue.h` immediately
- Plan 03 (storage) can use ZstdCompressor and JpegCompressor directly
- Stub files are in place for CMake; replacing them with real implementations in later plans will compile automatically
- realsense2 headers at `/opt/ros/jazzy/include/librealsense2/` are accessible via the cmake prefix path

---
*Phase: 01-core-capture-engine-mvp-storage*
*Completed: 2026-02-19*
