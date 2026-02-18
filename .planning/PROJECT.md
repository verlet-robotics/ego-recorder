# RealSense Ego Recorder

## What This Is

A high-performance C++ tool for capturing synchronized RGBD + IMU data from Intel RealSense D435 cameras, storing it in a compact compressed format optimized for robotics VLM fine-tuning. Supports both an interactive GUI mode with live preview and a headless systemd service mode for unattended recording on closed laptops. Designed to produce data sold to robotics companies training vision-language models for manipulation and ego-centric understanding.

## Core Value

Reliably capture synchronized, compressed RGBD+IMU streams without dropping frames or overloading RAM, producing data in a format robotics ML teams can directly ingest for VLM fine-tuning.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] Capture synchronized RGB (640x480@30fps) + depth (640x480@30fps) + IMU streams from RealSense D435
- [ ] Store data in a compressed format significantly smaller than ROS bags or HDF5 (research needed on optimal depth compression)
- [ ] Per-frame timestamps with RGB-depth synchronization
- [ ] Camera intrinsics (focal length, distortion coefficients, depth scale) saved per session
- [ ] IMU data (accelerometer + gyroscope) captured and stored alongside frames
- [ ] GUI mode: live RGB+depth preview, start/stop/pause controls, session naming, FPS/disk/frame stats overlay
- [ ] Headless mode: runs as systemd service, records without display, survives laptop lid close
- [ ] Memory-efficient streaming pipeline — constant RAM usage regardless of recording duration
- [ ] Flexible output targets: local SSD, external USB drive, configurable output path
- [ ] Session metadata: scene description labels, mounting type (ego/fixed), recording parameters

### Out of Scope

- Playback/review of recordings within the tool — use external viewers
- Cloud upload — manual transfer for now
- ROS bag or HDF5 as primary format — too large, researching compressed alternatives
- Real-time processing or inference on captured data
- Multi-camera synchronization — single D435 only
- Audio capture

## Context

- Intel RealSense D435 has RGB camera, stereo depth, and built-in IMU
- Target data consumers are robotics companies fine-tuning VLM backbones (RT-2, Octo, OpenVLA, etc.) for manipulation tasks and general ego-centric understanding
- Data needs to be in formats these companies actually use or can easily convert to
- Depth data is inherently compressible (smooth surfaces, limited range) — research needed on best compression approaches (e.g., lossless depth codecs, quantized depth, video-based depth compression)
- Camera will be used both body-mounted (ego-centric) and on fixed tripods (workspace view)
- Recording sessions range from 30-second task demos to multi-hour ambient capture
- Must handle sustained 30fps writes without frame drops on laptop-class hardware

## Constraints

- **Hardware**: Intel RealSense D435 camera via librealsense2 SDK
- **Language**: C++ for maximum performance and memory control
- **Platform**: Linux (Ubuntu) — laptop deployment, must work with lid closed
- **Resolution**: 640x480 @ 30fps for both RGB and depth streams
- **Memory**: Constant RAM footprint — stream-to-disk, no frame buffering in memory
- **Storage**: Must achieve significant compression vs raw frames (~30MB/s raw at 640x480 RGBD@30fps)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| C++ over Python | Maximum performance for sustained capture, direct librealsense2 access, memory control | — Pending |
| 640x480 @ 30fps | Good balance of quality and file size for ML training | — Pending |
| Compressed custom format over ROS bags/HDF5 | ROS bags and HDF5 are bloated for this use case; depth is highly compressible | — Pending |
| Systemd service for headless | Standard Linux daemon management, survives lid close, auto-start on boot | — Pending |

---
*Last updated: 2026-02-19 after initialization*
