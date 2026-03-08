# Roadmap: RealSense Ego Recorder

**Version:** 0.1.0
**Date:** 2026-02-19
**Depth:** Quick (3 phases)

---

## Phase 1: Core Capture Engine + MVP Storage

**Goal:** Record synchronized RGB+depth from D435 to a compressed custom binary file with constant memory usage. Headless-capable (no GUI yet).

**Requirements covered:** FR-1.1, FR-1.2, FR-1.3, FR-1.4, FR-1.5, FR-1.6, FR-2.1, FR-2.2 (ZSTD), FR-2.3 (JPEG), FR-2.4, FR-2.5, FR-2.6, NFR-1.*, NFR-2.*, NFR-3.1, NFR-5.1, NFR-5.2

**Key deliverables:**
- CMake project structure with librealsense2 + zstd dependencies
- Three-thread capture pipeline: capture → bounded queue → writer
- Custom binary container format with header, sequential frames, and index table
- ZSTD-compressed depth (lossless, ~3.5:1) + JPEG-compressed RGB (~10:1)
- Per-frame timestamps with global time sync
- Camera intrinsics + extrinsics stored in file header
- Runtime IMU detection (capture if D435i, skip if D435)
- USB type detection + auto-exposure priority disabled
- Signal handling (SIGTERM/SIGINT) for clean shutdown
- CLI interface: `./ego-recorder --output /path --session-name foo`

**Estimated compression:** ~350-470 MB/min (5-7x vs raw)

**Plans:** 4 plans

Plans:
- [x] 01-01-PLAN.md -- CMake project scaffold, frame types, bounded queue, compression wrappers
- [x] 01-02-PLAN.md -- Binary container format, file writer, signal handler, stats tracker
- [x] 01-03-PLAN.md -- RealSense pipeline wrapper with IMU detection and intrinsics
- [x] 01-04-PLAN.md -- Main orchestration, CLI, thread wiring, end-to-end verification

**Success criteria:**
- [x] Records 640x480 RGB+depth at sustained 30fps with 0 dropped frames
- [x] Memory usage stays constant (<200MB RSS) during 10-minute recording
- [x] Output file is seekable and recoverable after simulated crash
- [x] Clean shutdown on SIGTERM with complete file written

---

## Phase 2: GUI Mode + Headless Systemd Service

**Goal:** Add interactive GUI with live preview and recording controls. Add production systemd service for unattended headless recording on a closed laptop.

**Requirements covered:** FR-3.*, FR-4.*, NFR-4.*, NFR-5.3, NFR-5.4

**Key deliverables:**
- IPresenter interface (Strategy pattern) for GUI/headless mode selection
- **GUI mode (GuiPresenter):**
  - Dear ImGui + GLFW + OpenGL window
  - Side-by-side RGB + colorized depth preview
  - Start/stop controls, session naming input
  - Stats overlay: FPS, frame count, dropped frames, disk usage, elapsed time
- **Headless mode (HeadlessPresenter):**
  - systemd Type=notify service with WatchdogSec=30
  - sd_notify integration (READY, WATCHDOG, STATUS updates)
  - D-Bus sleep inhibitor lock for lid-close prevention
  - Machine-readable status file at /run/
  - Disk space monitoring with configurable threshold
- **Deployment artifacts:**
  - systemd unit file
  - udev rules for USB permissions + autosuspend prevention
  - logind.conf drop-in for lid-close prevention
  - Install script for system user creation and file placement
- USB disconnect/reconnect recovery (destroy + recreate pipeline)
- Configuration file support (TOML via toml++)

**Plans:** 4 plans

Plans:
- [ ] 02-01-PLAN.md -- IPresenter interface, TOML config system, CMake updates (ImGui, toml++)
- [ ] 02-02-PLAN.md -- GuiPresenter with live preview, controls, stats overlay, keyboard shortcuts
- [ ] 02-03-PLAN.md -- HeadlessPresenter with sd_notify, D-Bus inhibitor, deploy artifacts
- [ ] 02-04-PLAN.md -- Main.cpp integration, USB recovery, --headless flag, end-to-end verification

**Success criteria:**
- [ ] GUI shows live RGB+depth at 30fps while simultaneously recording
- [ ] Headless service starts on boot, records, and survives lid close
- [ ] `systemctl status` shows live recording stats
- [ ] Camera disconnect/reconnect recovers automatically within 10 seconds
- [ ] Service restarts cleanly after crash (verified via kill -9)

---

## Phase 3: Optimized Compression + Export Tools

**Goal:** Upgrade to Zdepth-style depth compression and H.264 RGB encoding for ~14-19x compression. Build RLDS and LeRobot conversion tools so the data product is sellable.

**Requirements covered:** FR-2.2 (Zdepth upgrade), FR-2.3 (H.264 upgrade), FR-5.*, NFR-3.2

**Key deliverables:**
- **Zdepth-style depth compression:**
  - Port or adapt catid/Zdepth algorithm for D435 Z16 frames
  - Block prediction + quantization + ZSTD (8-10:1 lossless)
  - Benchmark on real D435 scenes
- **H.264 RGB encoding:**
  - Integrate libx264 (or FFmpeg) for real-time video encoding
  - Target CRF 23-28 for good quality at 30-50:1 compression
  - Replace per-frame JPEG with intra-stream video encoding
- **RLDS export tool:**
  - Read custom binary format, decode all frames
  - Output TFRecord with uint16 depth + decoded RGB + metadata
  - Use rlds_dataset_builder template
- **LeRobot v3 export tool:**
  - Output MP4 (RGB) + Parquet (metadata/timestamps)
  - Compatible with HuggingFace Hub upload
- Container format v2: updated codec IDs, backward-compatible header

**Estimated compression:** ~140-190 MB/min (14-19x vs raw, ~10x smaller than ROS bags)

**Plans:** 5 plans

Plans:
- [x] 03-01-PLAN.md -- Container format v2, Zdepth compressor, H.264 encoder, CMake deps
- [ ] 03-02-PLAN.md -- Wire Zdepth+H.264 into recording pipeline, info subcommand
- [ ] 03-03-PLAN.md -- Python reader module (pybind11 egorec_reader extension)
- [ ] 03-04-PLAN.md -- RLDS TFRecord export tool
- [ ] 03-05-PLAN.md -- LeRobot v3 export tool

**Success criteria:**
- [ ] 1-hour recording fits in ~8-12 GB (vs 162 GB raw)
- [ ] Depth compression maintains bit-exact lossless (verified round-trip test)
- [ ] Per-frame encode time stays under 33ms (no frame drops)
- [ ] RLDS export loads cleanly with `tfds.load()`
- [ ] LeRobot export uploads to HuggingFace Hub successfully

---

## Future Milestones (Not Planned)

- **Annotation workflow**: Language description prompts, hand tracking via MediaPipe, episode segmentation
- **Robot integration**: Action label recording for teleoperated robot data (Tier S value)
- **Temporal depth compression**: Zdepth P-frames for 10-12:1 (diminishing returns)
- **AV1 for RGB**: Better ratio than H.264 at same quality
- **Multi-camera support**: Synchronized capture from multiple D435s
- **Cloud upload**: Auto-upload to S3/GCS after recording
