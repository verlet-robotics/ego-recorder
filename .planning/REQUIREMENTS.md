# Requirements: RealSense Ego Recorder

**Version:** 0.1.0
**Date:** 2026-02-19
**Hardware:** Intel RealSense D435 (no IMU — runtime detection for D435i future support)

---

## Functional Requirements

### FR-1: Core Capture Pipeline
- **FR-1.1**: Capture synchronized RGB (640x480@30fps, RS2_FORMAT_RGB8) and depth (640x480@30fps, RS2_FORMAT_Z16) streams from RealSense D435
- **FR-1.2**: Extract and store camera intrinsics (fx, fy, ppx, ppy, distortion model, coefficients) and depth scale per session
- **FR-1.3**: Extract and store depth-to-color extrinsic transform (rotation + translation) per session
- **FR-1.4**: Record per-frame timestamps using global time synchronization (RS2_OPTION_GLOBAL_TIME_ENABLED)
- **FR-1.5**: Detect IMU capability at runtime; capture accel+gyro if available (D435i), gracefully skip if not (D435)
- **FR-1.6**: Detect USB connection type and warn if USB 2.0 (insufficient bandwidth)

### FR-2: Storage Format
- **FR-2.1**: Store recordings in a custom binary container with file header (magic bytes, version, intrinsics, stream descriptors, session metadata), sequential frame blocks, and a seekable index table
- **FR-2.2**: Compress depth frames losslessly using ZSTD (MVP) with upgrade path to Zdepth-style compression
- **FR-2.3**: Compress RGB frames using JPEG (MVP) with upgrade path to H.264 video encoding
- **FR-2.4**: Store per-frame: timestamp (uint64 microseconds), compressed RGB data, compressed depth data, IMU samples (if available)
- **FR-2.5**: Write index table at end of file for random access by timestamp or frame number
- **FR-2.6**: Support recovery of index table by scanning frame headers if recording ends abnormally

### FR-3: GUI Mode
- **FR-3.1**: Display live RGB and colorized depth preview side-by-side using Dear ImGui + GLFW + OpenGL
- **FR-3.2**: Provide recording controls: start, stop, pause/resume, session naming
- **FR-3.3**: Display real-time stats overlay: current FPS, frame count, dropped frames, disk usage, elapsed time, disk free space
- **FR-3.4**: Disable auto-exposure priority for consistent 30fps
- **FR-3.5**: Graceful handling of camera disconnect/reconnect during GUI session

### FR-4: Headless Mode
- **FR-4.1**: Run as a systemd service (Type=notify) with watchdog support (WatchdogSec=30)
- **FR-4.2**: Accept `--headless` flag to run without display
- **FR-4.3**: Survive laptop lid close via logind.conf configuration + programmatic D-Bus sleep inhibitor lock
- **FR-4.4**: Implement graceful shutdown on SIGTERM using dedicated signal thread with sigwait
- **FR-4.5**: Write machine-readable status file to /run/ for remote monitoring
- **FR-4.6**: Update sd_notify STATUS with frame count, FPS, and disk usage
- **FR-4.7**: Monitor disk space and stop recording when below configurable threshold

### FR-5: Export/Conversion (Future Phase)
- **FR-5.1**: Convert custom format to RLDS/TFRecord (uint16 depth + decoded RGB + metadata)
- **FR-5.2**: Convert custom format to LeRobot v3 (MP4 + Parquet)
- **FR-5.3**: Support batch conversion of multiple session files

---

## Non-Functional Requirements

### NFR-1: Memory Efficiency
- **NFR-1.1**: Constant RAM footprint regardless of recording duration (no frame buffering beyond bounded queue)
- **NFR-1.2**: Bounded frame queue (max 4-8 frames) between capture and writer threads
- **NFR-1.3**: Zero-copy frame access from librealsense2 SDK buffers (use get_data() pointers directly)
- **NFR-1.4**: Never use rs2::frame::keep() for sustained recording

### NFR-2: Performance
- **NFR-2.1**: Sustain 30fps capture with zero frame drops under normal conditions on laptop hardware
- **NFR-2.2**: Per-frame compression must complete within 33ms (frame interval at 30fps)
- **NFR-2.3**: Three-thread architecture: capture thread (wait_for_frames only), writer thread (compress + disk IO), GUI thread (preview + controls)

### NFR-3: Compression Targets
- **NFR-3.1**: MVP: ~5-7x total compression vs raw (~350-470 MB/min at 640x480@30fps)
- **NFR-3.2**: Optimized: ~14-19x total compression vs raw (~140-190 MB/min)
- **NFR-3.3**: Depth must remain lossless (zero precision loss)
- **NFR-3.4**: RGB lossy compression acceptable (standard practice in robotics ML datasets)

### NFR-4: Reliability
- **NFR-4.1**: Handle USB disconnect/reconnect without crashing (recreate pipeline + context if needed)
- **NFR-4.2**: Handle disk full gracefully (stop recording, log, update status)
- **NFR-4.3**: Recordings must be recoverable even if the process crashes mid-write (scannable frame headers)
- **NFR-4.4**: Disable USB autosuspend for RealSense devices via udev rules

### NFR-5: Build & Platform
- **NFR-5.1**: C++17, CMake build system
- **NFR-5.2**: Linux (Ubuntu 22.04+) as primary platform
- **NFR-5.3**: Optional compile-time dependencies: GUI (OpenGL, GLFW, ImGui), systemd (libsystemd)
- **NFR-5.4**: Single binary supporting both GUI and headless mode via runtime flag

---

## Out of Scope (v0.1)
- Playback/review within the tool
- Cloud upload
- Real-time inference or processing
- Multi-camera synchronization
- Audio capture
- Annotation workflow (deferred)
- Robot action recording
- Real-time depth alignment (store raw + calibration instead)
