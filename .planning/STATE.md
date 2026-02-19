# Project State: RealSense Ego Recorder

**Last updated:** 2026-02-19
**Current phase:** 01-core-capture-engine-mvp-storage
**Current plan:** 4 of 4 complete (PHASE COMPLETE)
**Next action:** Begin phase 02 (`/gsd:execute-phase 02-gui-headless-service`)

---

## Progress

| Phase | Status | Description |
|-------|--------|-------------|
| 1 | COMPLETE (4/4 plans done) | Core capture engine + MVP storage |
| 2 | NOT STARTED | GUI mode + headless systemd service |
| 3 | NOT STARTED | Optimized compression + export tools |

---

## Key Decisions Made

| Decision | Choice | Date |
|----------|--------|------|
| Language | C++ (C++17) | 2026-02-19 |
| Resolution | 640x480 @ 30fps | 2026-02-19 |
| Camera | D435 (no IMU), runtime IMU detection for D435i | 2026-02-19 |
| MVP compression | ZSTD depth + JPEG RGB | 2026-02-19 |
| Optimized compression | Zdepth depth + H.264 RGB | 2026-02-19 |
| Container | Custom binary (not ROS bags, not HDF5) | 2026-02-19 |
| GUI framework | Dear ImGui + GLFW + OpenGL | 2026-02-19 |
| Headless | systemd Type=notify with watchdog | 2026-02-19 |
| Export formats | RLDS TFRecord + LeRobot v3 | 2026-02-19 |
| Workflow mode | YOLO, Quick depth, Parallel execution | 2026-02-19 |
| Data strategy | Build recorder first, annotations later | 2026-02-19 |
| JPEG subsampling | TJSAMP_420 (4:2:0) -- 30% smaller, adequate for ML training | 2026-02-19 |
| ZSTD level | Level 1 (fastest, <0.5ms/frame, ~3-4x on depth) | 2026-02-19 |
| Queue policy | BoundedQueue drop-oldest so capture thread never blocks | 2026-02-19 |
| turbojpeg install | TURBOJPEG_LOCAL_PREFIX cmake option for extracted .deb | 2026-02-19 |
| Pipeline-storage coupling | No binary_format.h in pipeline.h; getters expose raw metadata for main.cpp to assemble FileHeader | 2026-02-19 |
| IMU detection | rs2::error try/catch: D435i gets accel+gyro, D435 falls back to RGB+depth only | 2026-02-19 |
| FileWriter write buffer | 256KB pubsetbuf() reduces syscall frequency on sequential frame appends | 2026-02-19 |
| Stats atomic ordering | memory_order_relaxed for stats counters -- stale reads within display interval are acceptable | 2026-02-19 |
| sigwait signal handling | Detached sigwait thread with pthread_sigmask -- avoids all async-signal-safety issues vs signal()/sigaction() | 2026-02-19 |
| FileHeader assembly location | main.cpp assembles FileHeader from pipeline getters; no pipeline-to-storage coupling | 2026-02-19 |
| Compression thread placement | JPEG+ZSTD compression happens in writer thread; capture thread stays minimal (poll+copy+enqueue) | 2026-02-19 |
| End-to-end verification | Verified on physical D435: 617 frames, 29.9fps, 0 dropped, EGOREC magic confirmed | 2026-02-19 |

---

## Research Completed

- [x] Depth compression methods and benchmarks → `research/depth-compression.md`
- [x] VLM training data formats and market analysis → `research/vlm-data-formats.md`
- [x] librealsense2 C++ API patterns → `research/librealsense-api.md`
- [x] Headless systemd deployment → `research/headless-systemd.md`
- [x] Synthesis → `research/SUMMARY.md`

---

## Execution Sessions

| Session | Plan | Duration | Tasks | Commits |
|---------|------|----------|-------|---------|
| 2026-02-19 | 01-01 | ~5 min | 2/2 | d4b3ffe, 5586db1 |
| 2026-02-19 | 01-03 | ~2 min | 1/1 | 14795a3 |
| 2026-02-19 | 01-02 | ~4 min | 2/2 | c6d20f8, 651a086 |
| 2026-02-19 | 01-04 | ~15 min | 2/2 | a02dd98 |

---

## Blockers

None.

---

## Notes

- D435 has no IMU. Code will detect IMU at runtime for future D435i support.
- Raw ego video without annotations has low VLA training value. Annotation strategy deferred.
- Depth is stored but not yet used by current VLMs -- forward-looking differentiator.
- `libturbojpeg0-dev` not system-installed on dev machine. Use `TURBOJPEG_LOCAL_PREFIX` cmake option or `sudo apt install libturbojpeg0-dev`. See 01-01-SUMMARY.md for workaround details.
- realsense2 is installed via ROS Jazzy at `/opt/ros/jazzy`. Pass `-DCMAKE_PREFIX_PATH="/opt/ros/jazzy"` to cmake.
- Phase 1 complete: ego-recorder binary records 640x480 RGB+depth at sustained 30fps to .egorec files. All 4 plans executed, physical camera verified.
