# librealsense2 C++ SDK Research

**Researched:** 2026-02-19
**SDK Version:** 2.57.6 (latest stable, January 2026)
**Overall Confidence:** HIGH (official docs, source code, GitHub issues)

---

## CRITICAL FINDING: D435 vs D435i -- IMU Availability

**The project spec says "D435" but requires IMU data. The standard D435 does NOT have an IMU.** Only the D435**i** variant includes the Bosch BMI055 6-axis IMU (accelerometer + gyroscope). The cameras are otherwise identical.

**Recommendation:** The project must target the D435i. If the hardware is actually a D435 (no "i"), IMU capture is impossible and that requirement must be dropped or an external IMU must be used. The code should detect at runtime whether the connected device supports IMU streams and gracefully handle both cases.

**Confidence:** HIGH -- confirmed via official Intel specs and multiple sources.

**Sources:**
- [Intel D435i Product Page](https://www.intelrealsense.com/depth-camera-d435i/)
- [D435 vs D435i Comparison](https://www.tegakari.net/en/2019/04/realsense_compare/)
- [librealsense D435i Documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/d435i.md)

---

## 1. Pipeline Configuration for D435(i) at 640x480@30fps

### Basic Pipeline Setup

The `rs2::pipeline` is the recommended high-level API. It handles device discovery, stream configuration, frame synchronization, and delivery.

```cpp
#include <librealsense2/rs.hpp>

rs2::pipeline pipe;
rs2::config cfg;

// RGB stream -- use RS2_FORMAT_RGB8 to avoid BGR conversion overhead
// RS2_FORMAT_BGR8 is also available if feeding directly to OpenCV
cfg.enable_stream(RS2_STREAM_COLOR, 640, 480, RS2_FORMAT_RGB8, 30);

// Depth stream -- Z16 is 16-bit unsigned, native format
cfg.enable_stream(RS2_STREAM_DEPTH, 640, 480, RS2_FORMAT_Z16, 30);

// IMU streams (D435i ONLY -- will throw if device lacks IMU)
cfg.enable_stream(RS2_STREAM_ACCEL, RS2_FORMAT_MOTION_XYZ32F);
cfg.enable_stream(RS2_STREAM_GYRO, RS2_FORMAT_MOTION_XYZ32F);

// Start pipeline -- returns profile with active stream info
rs2::pipeline_profile profile = pipe.start(cfg);
```

### Stream Format Details

| Stream | Format | Bytes/Pixel | Frame Size (640x480) | Notes |
|--------|--------|-------------|----------------------|-------|
| Color | RS2_FORMAT_RGB8 | 3 | 921,600 bytes | Native from sensor |
| Color | RS2_FORMAT_BGR8 | 3 | 921,600 bytes | For OpenCV direct use |
| Color | RS2_FORMAT_YUYV | 2 | 614,400 bytes | Compressed, needs conversion |
| Depth | RS2_FORMAT_Z16 | 2 | 614,400 bytes | 16-bit depth in device units |
| Accel | RS2_FORMAT_MOTION_XYZ32F | 12 | 12 bytes | 3x float32 (x,y,z) |
| Gyro | RS2_FORMAT_MOTION_XYZ32F | 12 | 12 bytes | 3x float32 (x,y,z) |

**Raw throughput at 30fps:** ~46 MB/s for RGB+depth combined (before compression).

### Critical Configuration: Disable Auto-Exposure Priority

The color sensor's auto-exposure can dynamically REDUCE fps to get better exposure in low light. This MUST be disabled for consistent 30fps capture:

```cpp
auto profile = pipe.start(cfg);
auto sensor = profile.get_device().first<rs2::color_sensor>();
sensor.set_option(RS2_OPTION_AUTO_EXPOSURE_PRIORITY, 0.0f);
// 0 = maintain requested FPS, do not reduce for exposure
// 1 (default) = may reduce FPS for better exposure in low light
```

**Confidence:** HIGH -- confirmed in multiple GitHub issues (#2808, #5290) and official docs.

**Sources:**
- [rs2::config Class Reference](http://docs.ros.org/en/kinetic/api/librealsense2/html/classrs2_1_1config.html)
- [rs2::pipeline Class Reference](https://intelrealsense.github.io/librealsense/doxygen/classrs2_1_1pipeline.html)
- [Disable Auto Exposure Priority Issue #5290](https://github.com/IntelRealSense/librealsense/issues/5290)

---

## 2. RGB-Depth Synchronization

### Hardware Sync Limitation

**The D435/D435i does NOT have hardware synchronization between RGB and depth sensors.** The RGB sensor is a separate module with no hardware sync line to the stereo depth module. They run on independent clocks.

The SDK provides **software synchronization** via the pipeline's internal `rs2::syncer`, which matches frames by timestamp. When you call `pipe.wait_for_frames()`, it returns a `rs2::frameset` containing time-matched RGB and depth frames.

### Alignment Options

To get per-pixel RGB-depth correspondence, use `rs2::align`:

```cpp
// Create ONCE before loop -- expensive construction
rs2::align align_to_color(RS2_STREAM_COLOR);

while (running) {
    rs2::frameset frames = pipe.wait_for_frames();

    // Align depth to color viewport
    rs2::frameset aligned = align_to_color.process(frames);

    rs2::video_frame color = aligned.get_color_frame();
    rs2::depth_frame depth = aligned.get_depth_frame();
    // depth is now reprojected to match color pixel grid
}
```

### Alignment Performance Considerations

- `rs2::align` creation is expensive -- create once, reuse in loop
- Per-frame alignment cost: ~5-15ms on desktop x86 (depends on resolution)
- Can be significantly slower on ARM (~50-200ms at higher resolutions)
- CUDA-accelerated alignment available since SDK 2.16.5 (for NVIDIA GPU systems)
- Alignment introduces artifacts from interpolation and occlusion at depth discontinuities

### Alternative: Store Unaligned + Extrinsics

For a recording tool, consider storing raw unaligned RGB + depth frames with their intrinsics and the depth-to-color extrinsic transform. Consumers can then align in post-processing:

```cpp
auto depth_profile = profile.get_stream(RS2_STREAM_DEPTH);
auto color_profile = profile.get_stream(RS2_STREAM_COLOR);
rs2_extrinsics depth_to_color = depth_profile.get_extrinsics_to(color_profile);
// Store depth_to_color.rotation[9] and depth_to_color.translation[3]
```

**Recommendation:** Store raw frames + calibration data. Let consumers align if needed. This avoids per-frame CPU overhead during capture and preserves raw depth fidelity.

**Confidence:** HIGH -- hardware limitation confirmed via Intel Community forums and GitHub issues.

**Sources:**
- [RGB and Depth Sync Issue #774](https://github.com/IntelRealSense/librealsense/issues/774)
- [Hardware Sync Discussion - Intel Community](https://community.intel.com/t5/Items-with-no-label/Hardware-Sync-of-Color-and-Depth-in-D435/m-p/530413)
- [rs-align Example](https://github.com/IntelRealSense/librealsense/blob/master/examples/align/rs-align.cpp)
- [Align Performance on ARM Issue #2257](https://github.com/IntelRealSense/librealsense/issues/2257)

---

## 3. IMU Data Access (D435i Only)

### IMU Hardware Specs (Bosch BMI055)

| Sensor | Data Rate | Format | Range |
|--------|-----------|--------|-------|
| Gyroscope | 200 Hz or 400 Hz | 3x float32 (rad/s) | +/- 2000 deg/s |
| Accelerometer | 63 Hz or 250 Hz | 3x float32 (m/s^2) | +/- 4g |

The gyro and accel are **independent sensors** with different sampling rates. They arrive as separate `rs2::motion_frame` objects.

### IMU Access Pattern with Pipeline Callback

```cpp
rs2::pipeline pipe;
rs2::config cfg;
cfg.enable_stream(RS2_STREAM_COLOR, 640, 480, RS2_FORMAT_RGB8, 30);
cfg.enable_stream(RS2_STREAM_DEPTH, 640, 480, RS2_FORMAT_Z16, 30);
cfg.enable_stream(RS2_STREAM_ACCEL, RS2_FORMAT_MOTION_XYZ32F);
cfg.enable_stream(RS2_STREAM_GYRO, RS2_FORMAT_MOTION_XYZ32F);

auto profile = pipe.start(cfg, [&](rs2::frame frame) {
    // This callback fires for EVERY frame from EVERY stream
    if (auto motion = frame.as<rs2::motion_frame>()) {
        rs2_vector data = motion.get_motion_data();
        double ts = motion.get_timestamp();

        if (motion.get_profile().stream_type() == RS2_STREAM_ACCEL) {
            // data.x, data.y, data.z in m/s^2
            record_accel(data, ts);
        }
        else if (motion.get_profile().stream_type() == RS2_STREAM_GYRO) {
            // data.x, data.y, data.z in rad/s
            record_gyro(data, ts);
        }
    }
    else if (auto fs = frame.as<rs2::frameset>()) {
        // Synchronized RGB+depth frameset
        auto color = fs.get_color_frame();
        auto depth = fs.get_depth_frame();
        record_rgbd(color, depth);
    }
});
```

### IMU Calibration Warning

**The D435i IMU is NOT factory-calibrated.** Intel provides a free calibration tool (`rs-imu-calibration`) as part of the SDK. Without calibration:
- Accelerometer has zero-offset errors
- Gyroscope has bias drift
- Covariance matrices are not meaningful

Calibration results are stored on-device NVRAM. **Recommend running calibration before first use and documenting calibration status in session metadata.**

### IMU Rate Gotcha

When using `pipe.start(cfg, callback)` with IMU-only config, the IMU frame rate may be reduced compared to `pipe.start(cfg)` + `wait_for_frames()`. This is a known issue (#6426, #6424). With mixed RGB+depth+IMU, the callback approach works correctly.

**Confidence:** HIGH -- official rs-motion example, D435i docs, and confirmed issues.

**Sources:**
- [D435i Documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/d435i.md)
- [rs-motion Example](https://github.com/IntelRealSense/librealsense/blob/master/examples/motion/rs-motion.cpp)
- [How-to: Getting IMU Data](https://www.intelrealsense.com/how-to-getting-imu-data-from-d435i-and-t265/)
- [IMU Callback Rate Issue #6426](https://github.com/IntelRealSense/librealsense/issues/6426)

---

## 4. Camera Intrinsics and Metadata Extraction

### Getting Intrinsics

```cpp
auto profile = pipe.start(cfg);

// Depth intrinsics
auto depth_stream = profile.get_stream(RS2_STREAM_DEPTH)
    .as<rs2::video_stream_profile>();
rs2_intrinsics depth_intr = depth_stream.get_intrinsics();
// depth_intr.fx, depth_intr.fy     -- focal length in pixels
// depth_intr.ppx, depth_intr.ppy   -- principal point in pixels
// depth_intr.model                  -- distortion model enum
// depth_intr.coeffs[5]             -- distortion coefficients
// depth_intr.width, depth_intr.height

// Color intrinsics
auto color_stream = profile.get_stream(RS2_STREAM_COLOR)
    .as<rs2::video_stream_profile>();
rs2_intrinsics color_intr = color_stream.get_intrinsics();

// Depth-to-color extrinsics (rotation + translation)
rs2_extrinsics depth_to_color = depth_stream.get_extrinsics_to(color_stream);
// depth_to_color.rotation[9]     -- 3x3 rotation matrix (row-major)
// depth_to_color.translation[3]  -- translation vector in meters
```

### Getting Depth Scale

```cpp
auto depth_sensor = profile.get_device().first<rs2::depth_sensor>();
float depth_scale = depth_sensor.get_depth_scale();
// Typically 0.001 for D435 (1 Z16 unit = 1mm)
// Multiply Z16 value by depth_scale to get meters
```

### Per-Frame Metadata

```cpp
rs2::frameset frames = pipe.wait_for_frames();
auto depth = frames.get_depth_frame();

// Always check support before querying
if (depth.supports_frame_metadata(RS2_FRAME_METADATA_FRAME_COUNTER)) {
    auto frame_number = depth.get_frame_metadata(RS2_FRAME_METADATA_FRAME_COUNTER);
}

if (depth.supports_frame_metadata(RS2_FRAME_METADATA_FRAME_TIMESTAMP)) {
    auto hw_timestamp = depth.get_frame_metadata(RS2_FRAME_METADATA_FRAME_TIMESTAMP);
}

if (depth.supports_frame_metadata(RS2_FRAME_METADATA_ACTUAL_EXPOSURE)) {
    auto exposure_us = depth.get_frame_metadata(RS2_FRAME_METADATA_ACTUAL_EXPOSURE);
}

// Convenience methods (always available)
double timestamp_ms = depth.get_timestamp();
unsigned long long frame_num = depth.get_frame_number();
rs2_timestamp_domain domain = depth.get_frame_timestamp_domain();
```

### Available Metadata Fields

| Field | Description | Always Available |
|-------|-------------|-----------------|
| FRAME_COUNTER | Sequential frame number | Yes (via get_frame_number()) |
| FRAME_TIMESTAMP | Hardware timestamp | Depends on FW |
| SENSOR_TIMESTAMP | Mid-exposure sensor timestamp | Depends on FW |
| ACTUAL_EXPOSURE | Applied exposure (microseconds) | Usually yes |
| GAIN_LEVEL | Sensor gain | Usually yes |
| AUTO_EXPOSURE | AE enabled flag | Usually yes |
| TIME_OF_ARRIVAL | Host arrival time | Yes (software) |
| BACKEND_TIMESTAMP | Backend processing time | Yes (software) |
| ACTUAL_FPS | Measured FPS | Depends on FW |

### Global Time Synchronization

Enable `RS2_OPTION_GLOBAL_TIME_ENABLED` to map device timestamps to host clock:

```cpp
auto sensor = profile.get_device().first<rs2::depth_sensor>();
if (sensor.supports(RS2_OPTION_GLOBAL_TIME_ENABLED)) {
    sensor.set_option(RS2_OPTION_GLOBAL_TIME_ENABLED, 1.0f);
}
```

This periodically compares device and host clocks to compute a linear mapping, giving all frames timestamps in the host time domain. Essential for correlating IMU timestamps with RGB/depth timestamps.

**Confidence:** HIGH -- official API docs, frame_metadata.md, and API How-To.

**Sources:**
- [API How-To (GitHub Wiki)](https://github.com/IntelRealSense/librealsense/wiki/API-How-To)
- [Projection in RealSense SDK 2.0](https://dev.intelrealsense.com/docs/projection-in-intel-realsense-sdk-20)
- [Frame Metadata Documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/frame_metadata.md)

---

## 5. Avoiding Frame Drops Under Sustained Capture

### Core Rule

**Release (or move) `rs2::frame` objects within 1000/fps milliseconds (33ms at 30fps).** If processing takes longer, frames will be dropped. The SDK logs frame drops at DEBUG level.

### Queue Size Configuration

```cpp
// Get the depth sensor and set queue size
auto sensor = profile.get_device().first<rs2::depth_sensor>();
sensor.set_option(RS2_OPTION_FRAMES_QUEUE_SIZE, 2);
// 1 = minimal latency, higher drop risk
// 2 = recommended for depth+color (default pipeline behavior)
// Higher = more buffering, less drops, more latency and memory
```

### Producer-Consumer Threading Pattern (Recommended)

```cpp
#include <queue>
#include <mutex>
#include <condition_variable>

// Thread-safe bounded queue for frame transfer
class FrameQueue {
    std::queue<rs2::frameset> queue_;
    std::mutex mutex_;
    std::condition_variable cv_;
    size_t max_size_;
public:
    explicit FrameQueue(size_t max = 4) : max_size_(max) {}

    void push(rs2::frameset fs) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (queue_.size() >= max_size_) {
            queue_.pop(); // Drop oldest to prevent unbounded growth
        }
        queue_.push(std::move(fs));
        cv_.notify_one();
    }

    rs2::frameset pop() {
        std::unique_lock<std::mutex> lock(mutex_);
        cv_.wait(lock, [this]{ return !queue_.empty(); });
        auto fs = std::move(queue_.front());
        queue_.pop();
        return fs;
    }
};

// Capture thread: minimal work, just move frames
// Compression/write thread: does the heavy lifting
```

### What Causes Frame Drops

| Cause | Symptom | Fix |
|-------|---------|-----|
| Processing too slow in callback | Missed frames | Use producer-consumer pattern |
| USB 3.0 bandwidth saturation | Intermittent drops | Lower resolution or disable streams |
| USB 2.0 connection (accidental) | Heavy drops, low fps | Verify USB 3.0 cable and port |
| Auto-exposure priority enabled | FPS drops in low light | Disable RS2_OPTION_AUTO_EXPOSURE_PRIORITY |
| USB autosuspend (Linux) | Random disconnects | Disable via udev rules |
| Small frame queue | Drops during processing spikes | Increase RS2_OPTION_FRAMES_QUEUE_SIZE |
| keep() method overuse | Memory exhaustion, then drops | Do NOT use keep() for sustained recording |

**Confidence:** HIGH -- frame_lifetime.md, frame buffering wiki, and multiple GitHub issues.

**Sources:**
- [Frame Lifetime Documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/frame_lifetime.md)
- [Frame Buffering Wiki](https://github.com/IntelRealSense/librealsense/wiki/Frame-Buffering-Management-in-RealSense-SDK-2.0)
- [Queue Size Best Practice Issue #5041](https://github.com/IntelRealSense/librealsense/issues/5041)
- [Frame Queue Drops Issue #9022](https://github.com/IntelRealSense/librealsense/issues/9022)

---

## 6. Callback vs Polling Model

### Option A: Polling (wait_for_frames)

```cpp
pipe.start(cfg);
while (running) {
    rs2::frameset frames = pipe.wait_for_frames();  // Blocks up to 5s default
    // Process frames here (must be fast!)
}
```

**Pros:**
- Simple single-threaded model
- Easy to reason about frame ownership
- Good for simple applications

**Cons:**
- If processing takes >33ms, next frame is missed
- wait_for_frames blocks the thread
- Mixing GUI rendering and capture on one thread is fragile

### Option B: Callback (recommended for sustained recording)

```cpp
rs2::frame_queue write_queue(4);  // Bounded buffer

auto profile = pipe.start(cfg, [&](rs2::frame frame) {
    // This runs on internal IO thread -- keep it minimal!
    // NO heavy processing here
    write_queue.enqueue(std::move(frame));
});

// Writer thread (separate from GUI thread)
while (running) {
    rs2::frame f;
    if (write_queue.poll_for_frame(&f)) {
        // Compress and write to disk
        process_frame(f);
    }
}
```

**Pros:**
- Zero heap allocations after stabilization (SDK guarantee)
- Minimal latency -- callback fires from IO thread
- Naturally separates capture from processing
- Frame delivery continues even if processing stalls temporarily

**Cons:**
- Callback must be extremely fast (just move the frame)
- When using callback with `pipe.start()`, `wait_for_frames()` and `poll_for_frames()` throw exceptions
- Requires careful thread-safe data structures

### Option C: Polling + Dedicated Thread (pragmatic middle ground)

```cpp
pipe.start(cfg);
std::atomic<bool> running{true};

// Capture thread
std::thread capture_thread([&]() {
    while (running) {
        try {
            rs2::frameset frames = pipe.wait_for_frames(1000);
            write_queue.push(std::move(frames));
        } catch (const rs2::error& e) {
            // Handle timeout or device error
        }
    }
});

// Writer thread
std::thread writer_thread([&]() {
    while (running) {
        auto frames = write_queue.pop();
        compress_and_write(frames);
    }
});
```

### Recommendation for This Project

**Use Option C: Polling + Dedicated Threads.** Reasons:
1. Simpler to debug than callbacks
2. Capture thread does only `wait_for_frames()` + `move`
3. Writer thread handles compression/IO independently
4. GUI thread (if present) reads a "latest frame" reference for preview
5. Three clear threads: capture, write, GUI

**Confidence:** HIGH -- frame_lifetime.md and frame management docs.

**Sources:**
- [Frame Management](https://dev.intelrealsense.com/docs/frame-management)
- [poll_for_frames vs wait_for_frames Issue #2422](https://github.com/IntelRealSense/librealsense/issues/2422)
- [Async Callback Issue #2647](https://github.com/realsenseai/librealsense/issues/2647)

---

## 7. Memory Management and Zero-Copy Patterns

### Frame Data Access (Zero-Copy When Possible)

```cpp
rs2::frameset frames = pipe.wait_for_frames();
auto color = frames.get_color_frame();
auto depth = frames.get_depth_frame();

// Direct pointer to frame buffer -- NO copy
const void* color_data = color.get_data();
int color_stride = color.get_stride_in_bytes();  // bytes per row
int color_bpp = color.get_bytes_per_pixel();
int width = color.get_width();
int height = color.get_height();

// For depth: direct pointer to uint16_t array
const uint16_t* depth_data =
    reinterpret_cast<const uint16_t*>(depth.get_data());
```

### Frame Ownership Model

- `rs2::frame` is a smart reference (like shared_ptr) to internal buffer
- **Moving** a frame (`std::move(f)`) transfers ownership with NO data copy
- **Copying** a frame increments reference count, still NO data copy
- When last reference dies, buffer returns to SDK's internal pool
- After initial stabilization, SDK reuses a fixed pool -- zero allocations

### Anti-Pattern: Using keep()

```cpp
// DO NOT use keep() for sustained recording
frame.keep();  // Removes frame from pool, forces new allocation next time
// Only suitable for short (<10-30 second) burst captures
```

### Efficient Processing Pattern

```cpp
// In the writer thread:
void compress_and_write(rs2::frameset& frames) {
    auto color = frames.get_color_frame();
    auto depth = frames.get_depth_frame();

    // Get raw pointers -- zero copy
    const uint8_t* rgb = static_cast<const uint8_t*>(color.get_data());
    const uint16_t* z16 = reinterpret_cast<const uint16_t*>(depth.get_data());

    // Compress directly from SDK buffer
    // rgb is 640*480*3 = 921,600 bytes
    // z16 is 640*480*2 = 614,400 bytes
    compress_rgb(rgb, 640, 480, output_file);
    compress_depth(z16, 640, 480, output_file);

    // Frame references drop here, buffers return to pool
}
```

### Iterating Composite Frames

```cpp
rs2::frameset frames = pipe.wait_for_frames();
for (auto&& frame : frames) {
    auto stream_type = frame.get_profile().stream_type();
    if (stream_type == RS2_STREAM_COLOR) { /* ... */ }
    else if (stream_type == RS2_STREAM_DEPTH) { /* ... */ }
}
```

**Confidence:** HIGH -- frame_lifetime.md documentation is explicit about memory semantics.

**Sources:**
- [Frame Lifetime Documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/frame_lifetime.md)
- [Frame Management](https://dev.intelrealsense.com/docs/frame-management)
- [Using keep() Issue #6146](https://github.com/IntelRealSense/librealsense/issues/6146)

---

## 8. Post-Processing Filters (Optional for Recording)

For a recording tool, post-processing is generally NOT recommended during capture (adds CPU overhead, alters raw data). However, these filters are useful for live preview quality:

### Available Filters

| Filter | Purpose | Cost | Use During Capture? |
|--------|---------|------|---------------------|
| Decimation | Reduce resolution | Low | No -- store full res |
| Spatial | Edge-preserving smoothing | Medium | No -- alters data |
| Temporal | Noise reduction via history | Medium | No -- alters data |
| Hole Filling | Fill missing depth pixels | Low | No -- alters data |
| Colorizer | Depth-to-RGB visualization | Low | Preview only |
| Align | Depth-to-color registration | Medium-High | See section 2 |

### Colorizer for Preview Only

```cpp
rs2::colorizer color_map;
// For live depth visualization in GUI:
rs2::frame colorized = color_map.colorize(depth_frame);
// colorized is RS2_FORMAT_RGB8, suitable for GL texture upload
```

### Recommended Filter Chain (If Post-Processing Needed)

```
Depth -> Decimation -> Depth2Disparity -> Spatial -> Temporal
      -> Disparity2Depth -> Hole Filling -> Output
```

Each filter generates a new output frame (does not modify input). Filter pipelines should be per-source (switching sources invalidates temporal filter history).

**Confidence:** HIGH -- official post-processing-filters.md.

**Sources:**
- [Post-Processing Filters Documentation](https://github.com/IntelRealSense/librealsense/blob/master/doc/post-processing-filters.md)
- [rs-post-processing Example](https://github.com/IntelRealSense/librealsense/blob/master/examples/post-processing/rs-post-processing.cpp)

---

## 9. GUI Integration: Dear ImGui

### librealsense Bundles ImGui

The librealsense2 SDK **ships with Dear ImGui** as a third-party dependency. Their own examples (rs-capture, rs-post-processing, etc.) use ImGui + GLFW + OpenGL for rendering. This is the path of least resistance.

### Texture Upload Pattern for Live Preview

```cpp
// Initialize once
GLuint color_tex, depth_tex;
glGenTextures(1, &color_tex);
glGenTextures(1, &depth_tex);

// Per-frame update (in GUI thread)
void upload_frame(GLuint tex, rs2::video_frame frame) {
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB,
                 frame.get_width(), frame.get_height(),
                 0, GL_RGB, GL_UNSIGNED_BYTE, frame.get_data());
    // Use glTexSubImage2D for updates (faster than glTexImage2D)
}

// In ImGui render loop:
ImGui::Image((ImTextureID)(intptr_t)color_tex, ImVec2(640, 480));
ImGui::Image((ImTextureID)(intptr_t)depth_tex, ImVec2(640, 480));
```

### Recommended GUI Architecture

```
Capture Thread        Write Queue        Writer Thread
  |                      |                    |
  | wait_for_frames()    |                    |
  | move to queue ------>|                    |
  |                      |---> pop + write -->|
  |                      |                    |
  | also update "latest" |                    |
  | frame atomically     |                    |
  |                      |                    |
GUI Thread
  |
  | read "latest" frame (lock-free or mutex)
  | upload to GL texture
  | ImGui::Image()
  | render controls, stats overlay
```

The GUI thread should:
- Run at display refresh rate (not necessarily 30fps)
- Read the most recent frame (okay to skip frames for display)
- Use `glTexSubImage2D` for efficient texture updates
- Never block on frame delivery

### GL Processing Acceleration

librealsense2 offers `rs2::gl::processing_block` for GPU-accelerated filter processing. Frames stay in GPU memory as OpenGL textures, avoiding CPU-GPU round trips for visualization.

**Confidence:** HIGH -- librealsense examples use ImGui, SDK bundles it.

**Sources:**
- [librealsense ImGui Third-Party](https://github.com/IntelRealSense/librealsense/blob/master/third-party/imgui/imgui.h)
- [GL Examples](https://github.com/IntelRealSense/librealsense/tree/master/examples/gl)
- [ImGui Image Loading Wiki](https://github.com/ocornut/imgui/wiki/Image-Loading-and-Displaying-Examples)

---

## 10. Common Pitfalls for Long-Running Capture

### Pitfall 1: USB Autosuspend Disconnects (Linux)

**What:** Linux power management suspends USB devices after idle period, causing "Frames didn't arrive within 5 seconds" errors.

**Fix:** Create udev rule to disable autosuspend for RealSense devices:

```bash
# /etc/udev/rules.d/99-realsense-no-suspend.rules
# Intel RealSense vendor ID: 8086
ACTION=="add", SUBSYSTEM=="usb", ATTR{idVendor}=="8086", \
  TEST=="power/control", ATTR{power/control}="on"
```

Or via the librealsense-provided udev rules:
```bash
sudo cp config/99-realsense-libusb.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### Pitfall 2: USB 2.0 Fallback

**What:** If the device falls back to USB 2.0 (bad cable, hub, or port), bandwidth is insufficient for 640x480@30fps RGB+depth. Symptoms: severe frame drops, low actual FPS.

**Detection:**
```cpp
auto dev = profile.get_device();
std::string usb_type = dev.get_info(RS2_CAMERA_INFO_USB_TYPE_DESCRIPTOR);
// Should be "3.x" -- if "2.x", warn user
```

### Pitfall 3: Thermal Drift

**What:** After extended operation, the D435 module heats up, causing slight changes in stereo calibration. Depth accuracy at range (>2m) may degrade after 30+ minutes.

**Mitigation:** Intel recommends allowing 10-15 minutes warm-up before precision-critical capture. For ML training data capture, this is usually acceptable.

### Pitfall 4: Timestamp Domain Confusion

**What:** Timestamps can come from different domains (hardware clock, system clock, global time). Mixing domains gives nonsensical time deltas.

**Fix:** Always check `frame.get_frame_timestamp_domain()` and enable `RS2_OPTION_GLOBAL_TIME_ENABLED` for consistent host-time timestamps across all streams.

### Pitfall 5: Memory Growth from Frame Leaks

**What:** If `rs2::frame` references are held (even accidentally in a closure or container) without being released, the internal frame pool exhausts and the SDK falls back to heap allocation, causing memory growth.

**Detection:** Monitor process RSS over time. Should be flat after first few seconds.

**Fix:** Ensure all frame references are dropped promptly. Use `std::move` aggressively. Never store frames in unbounded containers.

### Pitfall 6: Pipeline Restart After Error

**What:** If the device disconnects (USB glitch, cable bump), the pipeline enters an error state. Simply calling `wait_for_frames()` again will not recover.

**Fix:**
```cpp
try {
    frames = pipe.wait_for_frames(5000);
} catch (const rs2::error& e) {
    // Pipeline error -- must stop and restart
    pipe.stop();
    std::this_thread::sleep_for(std::chrono::seconds(2));
    try {
        profile = pipe.start(cfg);
        // Reconfigure sensors (auto-exposure priority, etc.)
    } catch (...) {
        // Device still unavailable
    }
}
```

### Pitfall 7: Kernel Patches for V4L2 Backend

**What:** On Linux, librealsense2 requires kernel patches (via `librealsense2-dkms` package) for proper UVC support. Without them, metadata timestamps may be unavailable and some features may not work.

**Fix:** Install `librealsense2-dkms` or build the patched kernel module.

**Confidence:** HIGH -- aggregated from multiple GitHub issues and official troubleshooting docs.

**Sources:**
- [Frames Didn't Arrive Issue #13365](https://github.com/realsenseai/librealsense/issues/13365)
- [Heavy Memory Usage Issue #7098](https://github.com/IntelRealSense/librealsense/issues/7098)
- [Frame Rate Issue #4387](https://github.com/IntelRealSense/librealsense/issues/4387)
- [UDEV Rules Issue #4350](https://github.com/IntelRealSense/librealsense/issues/4350)

---

## 11. CMake Integration

### Minimal CMakeLists.txt

```cmake
cmake_minimum_required(VERSION 3.14)
project(realsense-ego-recorder LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Find librealsense2
find_package(realsense2 REQUIRED)

# Find OpenGL + GLFW for GUI
find_package(OpenGL REQUIRED)
find_package(glfw3 3.3 REQUIRED)

add_executable(ego-recorder
    src/main.cpp
    src/capture.cpp
    src/writer.cpp
    src/gui.cpp
    # ImGui sources (vendored or from librealsense third-party)
    third-party/imgui/imgui.cpp
    third-party/imgui/imgui_draw.cpp
    third-party/imgui/imgui_tables.cpp
    third-party/imgui/imgui_widgets.cpp
    third-party/imgui/backends/imgui_impl_glfw.cpp
    third-party/imgui/backends/imgui_impl_opengl3.cpp
)

target_link_libraries(ego-recorder PRIVATE
    ${realsense2_LIBRARY}
    OpenGL::GL
    glfw
)
```

### Installation

```bash
# Ubuntu: Install from Intel APT repository
sudo apt-key adv --keyserver keyserver.ubuntu.com --recv-key ...
sudo add-apt-repository "deb https://librealsense.intel.com/Debian/apt-repo ..."
sudo apt install librealsense2-dkms librealsense2-dev librealsense2-utils

# Or build from source for latest version
git clone https://github.com/IntelRealSense/librealsense.git
cd librealsense && mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release
make -j$(nproc) && sudo make install
```

**Confidence:** HIGH -- official CMake example and installation docs.

**Sources:**
- [Official CMake Example](https://github.com/IntelRealSense/librealsense/blob/master/examples/cmake/CMakeLists.txt)
- [Linux Build Guide](https://dev.intelrealsense.com/docs/compiling-librealsense-for-linux-ubuntu-guide)

---

## 12. Recommended Architecture for This Project

### Thread Model

```
                    +------------------+
                    |  Main Thread     |
                    |  (GUI / ImGui)   |
                    |  - Preview render|
                    |  - Controls      |
                    |  - Stats overlay |
                    +--------+---------+
                             |
                    reads "latest frame" (atomic swap)
                             |
+-------------------+        |        +-------------------+
|  Capture Thread   |        |        |  Writer Thread    |
|  - wait_for_frames|        |        |  - Pop from queue |
|  - Extract IMU    +------->+------->+  - Compress RGB   |
|  - Move to queues |  frame queue    |  - Compress depth |
|  - Update latest  |   (bounded)     |  - Write IMU CSV  |
+-------------------+                 |  - Flush to disk  |
                                      +-------------------+
```

### Key Design Decisions

1. **Capture thread** only does `wait_for_frames()` and moves frames -- never blocks on IO or compression
2. **Writer thread** owns the output file(s) and does all encoding/compression
3. **GUI thread** (main) reads the latest available frame for preview -- okay to skip frames
4. **Bounded queue** between capture and writer prevents unbounded memory growth
5. **IMU data** arrives at higher rates than RGB/depth -- buffer in a separate lock-free ring buffer or dedicated queue
6. **Headless mode** simply omits the GUI thread -- capture and writer threads remain identical

### Data Flow

```
Camera -> USB -> librealsense2 -> wait_for_frames()
                                       |
                            +----------+----------+
                            |                     |
                      rs2::frameset          rs2::motion_frame
                      (RGB + Depth)          (Accel/Gyro)
                            |                     |
                      Frame Queue            IMU Ring Buffer
                            |                     |
                      Writer Thread          Writer Thread
                            |                     |
                   Compress + Write         Append to IMU log
```

**Confidence:** HIGH -- synthesized from official patterns and best practices.

---

## Summary of Recommendations

| Question | Recommendation | Confidence |
|----------|---------------|------------|
| Camera model | Must be D435**i** for IMU | HIGH |
| Pipeline API | `rs2::pipeline` with `rs2::config` | HIGH |
| RGB format | RS2_FORMAT_RGB8 at 640x480@30 | HIGH |
| Depth format | RS2_FORMAT_Z16 at 640x480@30 | HIGH |
| Frame delivery | Polling (`wait_for_frames`) in dedicated capture thread | HIGH |
| Thread model | 3 threads: capture, writer, GUI | HIGH |
| Alignment | Store raw + calibration, skip real-time alignment | MEDIUM |
| Frame queue | Bounded (4-8 frames) between capture and writer | HIGH |
| IMU access | Callback or frameset iteration, separate buffer | HIGH |
| GUI framework | Dear ImGui + GLFW + OpenGL (bundled with SDK) | HIGH |
| Auto-exposure priority | Disable (set to 0) for fixed 30fps | HIGH |
| USB autosuspend | Disable via udev rules | HIGH |
| Post-processing | None during capture, colorizer for preview only | HIGH |
| Global timestamps | Enable RS2_OPTION_GLOBAL_TIME_ENABLED | HIGH |
| Error recovery | Catch exceptions, stop + restart pipeline | MEDIUM |
