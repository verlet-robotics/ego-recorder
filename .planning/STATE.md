# Project State: RealSense Ego Recorder

**Last updated:** 2026-02-19
**Current phase:** 02-gui-mode-headless-systemd-service
**Current plan:** 4 of 4 (tasks 1-2 complete, awaiting human verification checkpoint Task 3)
**Next action:** Human verification of GUI and headless modes with physical camera (02-04 Task 3 checkpoint)

---

## Progress

| Phase | Status | Description |
|-------|--------|-------------|
| 1 | COMPLETE (4/4 plans done) | Core capture engine + MVP storage |
| 2 | IN PROGRESS (4/4 code complete, human-verify checkpoint pending) | GUI mode + headless systemd service |
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
| IPresenter interface | Strategy pattern: start/tick/shutdown lifecycle + camera events + update_stats | 2026-02-19 |
| Config loading | toml++ value_or() fallbacks; parse_error -> stderr warning + defaults | 2026-02-19 |
| CMake GUI gating | WITH_GUI=ON: Dear ImGui v1.92.6 + GLFW + OpenGL; toml++ always linked | 2026-02-19 |
| GuiPresenter frame buffer | std::mutex + memcpy per tick: minimal lock hold, adequate for 30fps capture vs 60fps render | 2026-02-19 |
| Jet colormap orientation | blue=near (t=0), red=far (t=1) -- standard jet, invalid depth=black | 2026-02-19 |
| ImGui flag correction | NoBringToDisplayFront does not exist in v1.92.6; correct is NoBringToFrontOnFocus | 2026-02-19 |
| gui_presenter.cpp cmake | Compiled via target_sources inside if(WITH_GUI) block -- not unconditionally with HAVE_GUI guard | 2026-02-19 |
| HeadlessPresenter shutdown callback | on_request_shutdown_ callback passed at construction -- disk-full signaling without coupling presenter to main.cpp | 2026-02-19 |
| Status file write | Atomic rename (write to .tmp then rename) -- reader never sees partial JSON | 2026-02-19 |
| FPS atomic storage | Stored as uint64_t (fps * 10) -- avoids atomic<double> which is not lock-free on x86 | 2026-02-19 |
| deploy disk_min_mb | Production config.toml.example uses 1000 MB vs 500 MB interactive -- more conservative for unattended | 2026-02-19 |
| presenter polymorphism | unique_ptr<IPresenter>: GuiPresenter or HeadlessPresenter selected at runtime via --headless flag | 2026-02-19 |
| on_reconnect_requested wiring | Lambda in main captures camera by ref: destroy+500ms+recreate+presenter notify+new recording file | 2026-02-19 |
| Headless USB auto-retry | camera.reset() + sleep(500ms) + make_unique<RealSensePipeline>() loop every 2s in capture thread | 2026-02-19 |
| Config+CLI merge | load_config() first, then cxxopts count() > 0 guards CLI overrides; 0-sentinel for numeric flags | 2026-02-19 |

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
| 2026-02-19 | 02-01 | ~2 min | 2/2 | 92fa579, 2708cf4 |
| 2026-02-19 | 02-02 | ~8 min | 1/1 | 90a9dd8 |
| 2026-02-19 | 02-03 | ~3 min | 2/2 | 2dfae82, 30f4b67 |
| 2026-02-19 | 02-04 | ~3 min | 2/3 (checkpoint) | 17f77a9, 8e6e3e5 |

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
