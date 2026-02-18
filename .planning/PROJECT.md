# RealSense Ego Recorder

## What This Is

A high-performance C++ tool for capturing synchronized RGBD data from Intel RealSense D435/D435i cameras, storing it in a compact compressed custom binary format (~14x smaller than raw, ~10x smaller than ROS bags). Supports both an interactive GUI mode (Dear ImGui) with live preview and a headless systemd service mode for unattended recording on closed laptops. Includes conversion tools for RLDS and LeRobot v3 formats used by robotics VLM training pipelines.

## Core Value

Reliably capture synchronized, losslessly-compressed RGBD streams at 30fps without dropping frames or overloading RAM, producing data that robotics ML teams can convert to RLDS/LeRobot for VLM fine-tuning.

## Requirements

See `REQUIREMENTS.md` for full specification.

### Active

- [ ] Capture synchronized RGB+depth at 640x480@30fps from D435/D435i
- [ ] Lossless depth compression (ZSTD MVP → Zdepth optimized, 8-10:1)
- [ ] Lossy RGB compression (JPEG MVP → H.264 optimized, 30-50:1)
- [ ] Custom binary container with seekable index
- [ ] Per-frame timestamps with global time sync
- [ ] Camera intrinsics + depth-to-color extrinsics per session
- [ ] Runtime IMU detection (capture if D435i, skip if D435)
- [ ] GUI mode: ImGui live preview, controls, stats overlay
- [ ] Headless mode: systemd service, lid-close safe, watchdog
- [ ] Memory-efficient 3-thread pipeline (capture → queue → writer)
- [ ] RLDS + LeRobot v3 export tools

### Out of Scope (v0.1)

- Playback/review within the tool
- Cloud upload
- Real-time inference or processing
- Multi-camera synchronization
- Audio capture
- Annotation workflow (deferred — important for data product value)
- Robot action recording
- Real-time depth alignment (store raw + calibration instead)

## Context

- **Hardware**: D435 (no IMU). Code auto-detects D435i for future IMU support.
- **D435 does NOT have an IMU** — only D435i does. Confirmed by research.
- **Current VLMs train on RGB only** at 224x224. Depth is stored by foundational datasets (DROID, Bridge V2, ManiSkill) but not yet consumed by models. Depth is a forward-looking differentiator.
- **Raw ego video without annotations has low VLA training value.** The data hierarchy: teleoperated robot data >>> annotated ego video >>> raw ego video. Annotation strategy deferred.
- **RLDS/TFRecord is the dominant format** (OpenVLA, Octo, RT-X, Pi0). LeRobot v3 is the rising PyTorch alternative. Must ship both.
- **Lossless depth compression at 8-10:1 is proven** (Zdepth algorithm). Lossy approaches introduce edge artifacts harmful to manipulation training.
- Camera used both body-mounted (ego) and on fixed tripods (workspace view)
- Sessions range from 30-second task demos to multi-hour ambient capture
- Raw throughput: ~45 MB/s (2.7 GB/min, 162 GB/hr) at 640x480 RGBD@30fps

## Constraints

- **Hardware**: Intel RealSense D435 via librealsense2 SDK
- **Language**: C++17, CMake build system
- **Platform**: Linux (Ubuntu 22.04+), laptop deployment, must work with lid closed
- **Resolution**: 640x480 @ 30fps for both RGB and depth streams
- **Memory**: Constant RAM footprint (<200MB), bounded frame queue (4-8 frames)
- **Compression**: MVP ~350-470 MB/min (5-7x), Optimized ~140-190 MB/min (14-19x)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| C++17 | Maximum performance, direct librealsense2, memory control | Confirmed |
| 640x480 @ 30fps | Good quality/size balance; VLMs train at 224x224 anyway | Confirmed |
| Custom binary container | ROS bags ~2x bloat, HDF5 heavy; custom is minimal overhead | Confirmed |
| ZSTD depth (MVP) | 3.5:1 lossless, <1ms/frame, zero domain-specific code | Confirmed |
| Zdepth depth (optimized) | 8-10:1 lossless, <2ms/frame, proven on Kinect | Confirmed |
| H.264 for RGB | 30-50:1 lossy, standard practice (LeRobot uses AV1 CRF 30) | Confirmed |
| Dear ImGui for GUI | Bundled with librealsense2 SDK, ImGui+GLFW+OpenGL | Confirmed |
| systemd Type=notify | Proper readiness signaling, watchdog for hang detection | Confirmed |
| Store raw unaligned frames | Avoid per-frame alignment CPU cost, preserve raw depth | Confirmed |
| Runtime IMU detection | Support both D435 and D435i from single binary | Confirmed |
| RLDS + LeRobot v3 export | RLDS is dominant, LeRobot rising; ship both | Confirmed |

---
*Last updated: 2026-02-19 after research phase*
