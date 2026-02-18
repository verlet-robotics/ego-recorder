# Project State: RealSense Ego Recorder

**Last updated:** 2026-02-19
**Current phase:** Not started
**Next action:** `/gsd:plan-phase 1`

---

## Progress

| Phase | Status | Description |
|-------|--------|-------------|
| 1 | NOT STARTED | Core capture engine + MVP storage |
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

---

## Research Completed

- [x] Depth compression methods and benchmarks → `research/depth-compression.md`
- [x] VLM training data formats and market analysis → `research/vlm-data-formats.md`
- [x] librealsense2 C++ API patterns → `research/librealsense-api.md`
- [x] Headless systemd deployment → `research/headless-systemd.md`
- [x] Synthesis → `research/SUMMARY.md`

---

## Blockers

None.

---

## Notes

- D435 has no IMU. Code will detect IMU at runtime for future D435i support.
- Raw ego video without annotations has low VLA training value. Annotation strategy deferred.
- Depth is stored but not yet used by current VLMs — forward-looking differentiator.
