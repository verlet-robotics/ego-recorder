# Research Synthesis

**Project:** RealSense Ego Recorder
**Date:** 2026-02-19
**Researchers:** 4 parallel agents (depth compression, VLM data formats, librealsense API, headless systemd)

---

## Critical Findings

### 1. D435 Does NOT Have an IMU
The standard D435 lacks an IMU. Only the **D435i** has a Bosch BMI055 6-axis IMU. Code must detect IMU support at runtime and handle both variants.

### 2. Raw Ego Video Has Near-Zero VLA Fine-Tuning Value
Current robotics VLMs (OpenVLA, Octo, RT-2, Pi0) train on **robot teleoperation data with action labels**, not raw ego video. The data value hierarchy:
- **Tier S**: Teleoperated robot data with actions + language ($$$)
- **Tier A**: Ego video with dense annotations (hand tracking, language) ($$)
- **Tier B**: Ego video with language only ($)
- **Tier C**: Raw ego RGBD (near $0 for VLA fine-tuning)

**Implication**: The recording tool is solid, but the data product strategy needs annotations and/or robot integration to be sellable.

### 3. RLDS/TFRecord is the Dominant Format
Every major VLM project uses RLDS. LeRobot v3 is the rising PyTorch alternative. Must ship conversion tools for both.

### 4. Lossless Depth Compression at 8-10:1 is Achievable
Zdepth-style compression (quantization + prediction + ZSTD) compresses 16-bit depth frames at 8-10:1 losslessly in <2ms per frame. This makes a custom binary container ~14x smaller than raw and ~10x smaller than ROS bags.

### 5. Current VLMs Train on RGB Only (Depth is a Future Differentiator)
OpenVLA, Octo, RT-2, Pi0 all use RGB at 224x224. Depth is stored by datasets (DROID, Bridge V2, ManiSkill) but not yet consumed by models. Storing depth is forward-looking differentiation.

---

## Architecture Decisions (from Research)

| Decision | Research Recommendation | Confidence |
|----------|------------------------|------------|
| Depth compression | Zdepth-style lossless (8-10:1), ZSTD for MVP (3.5:1) | HIGH |
| RGB compression | H.264 lossy via libx264 (30-50:1) | HIGH |
| Container format | Custom binary with per-frame index (not ROS bags, not HDF5) | MEDIUM |
| Thread model | 3 threads: capture, writer, GUI (bounded queue between them) | HIGH |
| GUI framework | Dear ImGui + GLFW + OpenGL (bundled with librealsense) | HIGH |
| Alignment | Store raw unaligned frames + calibration data | MEDIUM |
| IMU support | Runtime detection, optional (support D435 and D435i) | HIGH |
| Headless mode | systemd Type=notify with watchdog, Strategy pattern for GUI/headless | HIGH |
| Signal handling | Dedicated thread with sigwait (SIGTERM) | HIGH |
| Lid-close | logind.conf + programmatic D-Bus inhibitor lock | HIGH |
| Export formats | RLDS TFRecord + LeRobot v3 conversion tools | HIGH |

---

## Compression Budget (640x480 @ 30fps)

| Stream | Raw/min | MVP (ZSTD+JPEG) | Optimized (Zdepth+H.264) |
|--------|---------|------------------|--------------------------|
| Depth | 1.08 GB | 310 MB (3.5:1) | 108-135 MB (8-10:1) |
| RGB | 1.62 GB | 162 MB (10:1 JPEG) | 32-54 MB (30-50:1 H.264) |
| IMU | 0.58 MB | 0.58 MB | 0.58 MB |
| **Total** | **2.7 GB** | **~470 MB** | **~140-190 MB** |

1-hour recording: Raw 162 GB → MVP ~28 GB → Optimized ~8-12 GB

---

## Key Dependencies

| Library | Purpose | License | Phase |
|---------|---------|---------|-------|
| librealsense2 | Camera capture | Apache-2.0 | 1 |
| zstd | Depth compression (MVP) | BSD-3 | 1 |
| Dear ImGui + GLFW | GUI | MIT | 2 |
| libx264/FFmpeg | RGB video encoding | GPL-2/LGPL-2.1 | 3 |
| catid/Zdepth | Optimized depth compression | BSD-3 | 3 |
| libsystemd | Service integration | LGPL-2.1 | 2 |

---

## Open Questions for User

1. Is the camera a D435 or D435i? (IMU only available on D435i)
2. What annotation strategy for the data product? (raw video has near-zero VLA training value)
3. Is a robot arm in scope for data collection? (moves data from Tier C to Tier S value)
