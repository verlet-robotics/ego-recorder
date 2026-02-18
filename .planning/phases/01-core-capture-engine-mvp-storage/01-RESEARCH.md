# Phase 1: Core Capture Engine + MVP Storage - Research

**Researched:** 2026-02-19
**Domain:** C++17 real-time RGBD capture, compression, and binary file I/O
**Confidence:** HIGH

## Summary

This phase builds a headless-capable capture engine that records synchronized RGB+depth from a RealSense D435 into a compressed custom binary file with constant memory usage. The implementation-specific questions center on eight areas: JPEG library choice, ZSTD API usage, binary format wire design, CMake dependency management, CLI parsing, thread-safe queues, JPEG quality tradeoffs, and pipeline warmup handling.

The core finding is that **libjpeg-turbo (TurboJPEG API)** is the correct JPEG library -- it provides SIMD-accelerated in-memory compression at 3-7x the speed of alternatives, uses a simple C API (`tjCompress2`), and is available as a system package on Ubuntu 22.04+. For ZSTD, the **one-shot `ZSTD_compressCCtx()` API at level 1-3** is ideal for per-frame depth compression, giving sub-millisecond encode times on 614KB depth frames. The binary container format should use a **fixed-size header, length-prefixed frame blocks with magic-tagged boundaries, and an append-at-close index table** with recovery via sequential frame header scanning.

**Primary recommendation:** Use system-installed librealsense2, libjpeg-turbo, and libzstd via `find_package`/`pkg_check_modules`. Use FetchContent only for header-only libraries (cxxopts). Build a three-thread pipeline (capture, compress+write, signal handler) with a mutex+condvar bounded queue of 4-8 frames. Skip the first 30 frames for auto-exposure warmup.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| librealsense2 | 2.57.x | Camera capture, frame delivery, intrinsics | Only SDK for RealSense D435; system package via Intel APT repo |
| libzstd | 1.5.x | Lossless depth frame compression | Facebook's standard real-time compressor; 338+ MB/s at level 1 |
| libjpeg-turbo (TurboJPEG API) | 2.1.x+ | JPEG RGB compression | SIMD-accelerated, 3-7x faster than libjpeg; in-memory API |
| cxxopts | 3.3.1 | CLI argument parsing | Header-only, MIT, C++11+, 1 file to vendor |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| libsystemd | system | sd_notify for headless mode | Only if building with systemd support (optional) |
| pthreads | system | Signal handling (sigwait) | Always -- POSIX signal thread pattern |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| libjpeg-turbo (TurboJPEG) | stb_image_write.h | stb is simpler (single header, no deps) but 3-5x slower, no SIMD, designed for "compactness not performance" |
| libjpeg-turbo (TurboJPEG) | OpenCV imencode | Adds massive OpenCV dependency for one function |
| cxxopts | CLI11 | CLI11 is more feature-rich but heavier; cxxopts is sufficient for simple flag parsing |
| cxxopts | getopt (POSIX) | getopt works but requires manual option registration, no type safety, no help generation |
| Custom bounded queue | Boost lockfree queue | Boost is a massive dependency; mutex+condvar queue is 50 lines and perfectly adequate at 30fps |

**Installation:**
```bash
# System packages (Ubuntu 22.04+)
sudo apt install librealsense2-dev librealsense2-dkms libturbojpeg0-dev libzstd-dev

# cxxopts is vendored via FetchContent or copied as single header
```

## Architecture Patterns

### Recommended Project Structure
```
CMakeLists.txt
src/
  main.cpp              # Entry point, CLI parsing, signal setup, thread orchestration
  capture/
    pipeline.h/.cpp     # RealSense pipeline wrapper, warmup, intrinsics extraction
    frame_types.h       # CapturedFrame struct (timestamp, compressed buffers)
  compression/
    jpeg_compressor.h/.cpp    # TurboJPEG wrapper (reusable tjhandle)
    zstd_compressor.h/.cpp    # ZSTD wrapper (reusable ZSTD_CCtx)
  storage/
    binary_format.h     # Format constants, magic bytes, struct definitions
    file_writer.h/.cpp  # Sequential frame writer + index table
    file_reader.h/.cpp  # (Future) Reader for playback/export
  threading/
    bounded_queue.h     # Thread-safe bounded queue template
  utils/
    signal_handler.h/.cpp   # sigwait-based SIGTERM/SIGINT handler
    stats.h/.cpp            # Frame counter, FPS tracker, dropped frame counter
```

### Pattern 1: Three-Thread Pipeline
**What:** Capture thread polls `pipe.wait_for_frames()`, compresses frames, and enqueues to a bounded queue. Writer thread dequeues and writes to disk. Signal thread waits for SIGTERM/SIGINT.
**When to use:** Always -- this is the core architecture.
**Why three threads not two:** Compression (JPEG + ZSTD) takes 2-5ms per frame. Writing to disk is variable (0.1ms cached, 10ms+ on fsync). Separating them prevents disk stalls from causing frame drops. However, for MVP simplicity, compression can happen in the writer thread since combined compress+write fits in the 33ms budget.

```
Capture Thread              Writer Thread           Signal Thread
  |                           |                       |
  | pipe.wait_for_frames()    |                       | sigwait(SIGTERM)
  | extract data pointers     |                       | set shutdown=true
  | enqueue to bounded queue  |                       |
  |                           | dequeue frame          |
  |                           | JPEG compress RGB      |
  |                           | ZSTD compress depth    |
  |                           | write to file          |
```

**Simplified MVP variant (two worker threads):**
```cpp
// Capture thread: poll + enqueue raw frame data
// Writer thread: dequeue + compress + write
// This works because compress+write < 33ms at 640x480
```

### Pattern 2: Reusable Compression Contexts
**What:** Create `tjhandle` and `ZSTD_CCtx*` once at startup, reuse for every frame.
**When to use:** Always -- context creation is expensive, reuse is free.

```cpp
// Created once in writer thread initialization:
tjhandle jpeg_compressor = tjInitCompress();
ZSTD_CCtx* zstd_ctx = ZSTD_createCCtx();

// Pre-allocate output buffers:
std::vector<uint8_t> jpeg_buf(tjBufSize(640, 480, TJSAMP_420));
std::vector<uint8_t> zstd_buf(ZSTD_compressBound(640 * 480 * 2));

// Used per-frame (zero allocation):
unsigned long jpeg_size = 0;
unsigned char* jpeg_ptr = jpeg_buf.data();
tjCompress2(jpeg_compressor, rgb_data, 640, 0, 480,
            TJPF_RGB, &jpeg_ptr, &jpeg_size,
            TJSAMP_420, 90, TJFLAG_FASTDCT);

size_t zstd_size = ZSTD_compressCCtx(zstd_ctx,
    zstd_buf.data(), zstd_buf.size(),
    depth_data, 640 * 480 * 2,
    1);  // level 1 for speed

// Destroyed at shutdown:
tjDestroy(jpeg_compressor);
ZSTD_freeCCtx(zstd_ctx);
```

### Pattern 3: CapturedFrame Data Transfer Struct
**What:** A struct that owns compressed data moved from capture to writer via queue.
**When to use:** For transferring frame data between threads without copying raw buffers.

```cpp
struct CapturedFrame {
    uint64_t timestamp_us;          // Global time, microseconds
    uint64_t frame_number;          // Sequential frame counter

    // Raw data pointers (valid only while rs2::frameset is alive)
    // Used if compression happens in writer thread
    std::vector<uint8_t> rgb_data;  // Copy of RGB (921,600 bytes)
    std::vector<uint8_t> depth_data; // Copy of depth (614,400 bytes)

    // IMU samples accumulated since last frame
    struct IMUSample {
        uint64_t timestamp_us;
        float accel[3];  // m/s^2
        float gyro[3];   // rad/s
    };
    std::vector<IMUSample> imu_samples;
};
```

**Important note on rs2::frame lifetime:** The SDK reuses frame buffers. You MUST copy the data out before the next `wait_for_frames()` call returns. Use `memcpy` into pre-allocated vectors, or use `rs2::frame_queue` to hold references (but this uses the SDK's internal pool slots).

### Anti-Patterns to Avoid
- **Using `rs2::frame::keep()`:** Prevents the SDK from reusing internal buffers, causing heap allocations and memory growth during sustained recording.
- **Compressing in the capture thread:** Any stall in compression blocks the next `wait_for_frames()`, causing frame drops.
- **Unbounded queue:** Without a max size, the queue grows if the writer falls behind, consuming all RAM.
- **Shared tjhandle across threads:** TurboJPEG handles are NOT thread-safe. One handle per thread.
- **Opening/closing the file per frame:** Keep the file open for the entire session. Use buffered I/O (`fwrite` with large buffer or `std::ofstream` with `rdbuf()->pubsetbuf()`).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| JPEG encoding | Custom DCT/huffman | libjpeg-turbo `tjCompress2()` | SIMD-optimized, handles edge cases, quality control |
| General compression | LZ-style compressor | libzstd `ZSTD_compressCCtx()` | Decades of optimization, level knobs, proven |
| CLI parsing | Manual argc/argv walking | cxxopts | Help text generation, type checking, short/long opts |
| Thread-safe queue | Lock-free ring buffer | Mutex+condvar bounded queue | Lock-free is harder to debug, mutex is fast enough at 30fps |
| Frame synchronization | Manual timestamp matching | `rs2::pipeline.wait_for_frames()` | SDK handles sync internally |
| Binary endianness | Manual byte swapping | Write in host byte order (little-endian on x86) | Target is Linux x86 only; document LE assumption |

**Key insight:** At 30fps, "real-time" means 33ms per frame. JPEG compress is ~1-3ms. ZSTD compress is ~0.5ms. File write is ~0.1ms (buffered). There is massive headroom. Do not over-engineer concurrency or compression -- the simple approach works.

## Common Pitfalls

### Pitfall 1: Auto-Exposure Warmup Frames
**What goes wrong:** The first 10-30 frames after pipeline start have wildly incorrect exposure, producing dark/overexposed RGB images.
**Why it happens:** The D435 color sensor's auto-exposure algorithm needs several frames to converge.
**How to avoid:** Drop the first 30 frames after `pipe.start()`. This is Intel's official recommendation.
**Warning signs:** First few frames in recording are visually much darker or lighter than the rest.

```cpp
// Warmup: drop first 30 frames
auto profile = pipe.start(cfg);
for (int i = 0; i < 30; ++i) {
    pipe.wait_for_frames();
}
// NOW start recording
```

### Pitfall 2: TurboJPEG Buffer Reallocation
**What goes wrong:** If you pass `*jpegBuf = nullptr` to `tjCompress2`, it allocates a new buffer every call (via `tjAlloc`). This causes ~900 KB allocation per frame at 30fps.
**Why it happens:** Default TurboJPEG behavior is to allocate on the caller's behalf.
**How to avoid:** Pre-allocate using `tjBufSize()` and pass the pre-allocated buffer. Set `TJFLAG_NOREALLOC` to prevent reallocation.

```cpp
// Pre-allocate worst-case buffer
unsigned long buf_size = tjBufSize(640, 480, TJSAMP_420);
unsigned char* jpeg_buf = tjAlloc(buf_size);
unsigned long jpeg_size = buf_size;

// Compress with no-realloc flag
tjCompress2(handle, rgb, 640, 0, 480, TJPF_RGB,
            &jpeg_buf, &jpeg_size, TJSAMP_420, 90,
            TJFLAG_FASTDCT | TJFLAG_NOREALLOC);
// jpeg_size now contains actual compressed size
// jpeg_buf was NOT reallocated
```

### Pitfall 3: ZSTD Error Checking
**What goes wrong:** `ZSTD_compress*` returns the compressed size on success OR an error code. The error code is also a `size_t` value, so you must check explicitly.
**Why it happens:** ZSTD uses the top bit of size_t for error signaling.
**How to avoid:** Always check with `ZSTD_isError()`.

```cpp
size_t result = ZSTD_compressCCtx(ctx, dst, dst_cap, src, src_size, level);
if (ZSTD_isError(result)) {
    fprintf(stderr, "ZSTD error: %s\n", ZSTD_getErrorName(result));
    // Handle error
}
// result is the compressed size
```

### Pitfall 4: rs2::frame Data Pointer Invalidation
**What goes wrong:** You save `frame.get_data()` pointer, then call `wait_for_frames()` again. The old pointer now points to new frame data (or garbage).
**Why it happens:** The SDK reuses a fixed pool of frame buffers. When you release a frame (by letting the `rs2::frame` object go out of scope), its buffer returns to the pool and may be overwritten immediately.
**How to avoid:** Copy the data before releasing the frame. In the capture thread, `memcpy` the raw bytes into a `CapturedFrame` struct, then let the `rs2::frameset` go out of scope.

```cpp
while (running) {
    rs2::frameset frames = pipe.wait_for_frames();
    auto color = frames.get_color_frame();
    auto depth = frames.get_depth_frame();

    CapturedFrame cf;
    cf.timestamp_us = static_cast<uint64_t>(color.get_timestamp() * 1000.0);
    cf.rgb_data.assign(
        static_cast<const uint8_t*>(color.get_data()),
        static_cast<const uint8_t*>(color.get_data()) + color.get_stride_in_bytes() * color.get_height());
    cf.depth_data.assign(
        static_cast<const uint8_t*>(depth.get_data()),
        static_cast<const uint8_t*>(depth.get_data()) + depth.get_stride_in_bytes() * depth.get_height());

    queue.push(std::move(cf));
    // frames goes out of scope here -- buffers return to pool
}
```

### Pitfall 5: Forgetting to Disable Auto-Exposure Priority
**What goes wrong:** In low-light conditions, the camera silently reduces FPS from 30 to 15 or lower to get better exposure. Recording appears to work but has half the expected frames.
**Why it happens:** `RS2_OPTION_AUTO_EXPOSURE_PRIORITY` defaults to 1 (allow FPS reduction).
**How to avoid:** Set to 0 immediately after pipeline start.

```cpp
auto profile = pipe.start(cfg);
auto color_sensor = profile.get_device().first<rs2::color_sensor>();
color_sensor.set_option(RS2_OPTION_AUTO_EXPOSURE_PRIORITY, 0.0f);
```

### Pitfall 6: Index Table Loss on Crash
**What goes wrong:** The index table is written at the end of the file. If the process crashes, there is no index table, and the file appears corrupt.
**Why it happens:** Index-at-end is the simplest design for streaming writes.
**How to avoid:** Design frame blocks with recognizable magic bytes and a length prefix so the file can be scanned sequentially to rebuild the index. Document this recovery procedure.

### Pitfall 7: Bounded Queue Drop Policy
**What goes wrong:** If the writer thread falls behind, the queue fills. With a naive "block producer" policy, the capture thread blocks on `push()`, causing frame drops in the SDK's internal queue.
**Why it happens:** Disk I/O spike or compression stall.
**How to avoid:** Use a "drop oldest" policy: if the queue is full, pop the oldest frame and push the new one. Log the dropped frame. This maintains real-time behavior at the cost of occasional frame loss.

## Code Examples

### Complete TurboJPEG Compression (Verified Pattern)
```cpp
// Source: libjpeg-turbo official docs + tjCompress2 API reference
#include <turbojpeg.h>
#include <vector>
#include <stdexcept>

class JpegCompressor {
    tjhandle handle_;
    unsigned char* buf_;
    unsigned long buf_size_;
    int quality_;

public:
    JpegCompressor(int width, int height, int quality = 90)
        : quality_(quality) {
        handle_ = tjInitCompress();
        if (!handle_) throw std::runtime_error("tjInitCompress failed");

        // Pre-allocate worst-case buffer
        buf_size_ = tjBufSize(width, height, TJSAMP_420);
        buf_ = tjAlloc(buf_size_);
    }

    ~JpegCompressor() {
        if (buf_) tjFree(buf_);
        if (handle_) tjDestroy(handle_);
    }

    // Returns (pointer, size) -- pointer valid until next compress() call
    std::pair<const uint8_t*, size_t> compress(
            const uint8_t* rgb, int width, int height) {
        unsigned long compressed_size = buf_size_;
        unsigned char* out_buf = buf_;

        int ret = tjCompress2(handle_, rgb, width, 0, height,
                              TJPF_RGB, &out_buf, &compressed_size,
                              TJSAMP_420, quality_,
                              TJFLAG_FASTDCT | TJFLAG_NOREALLOC);
        if (ret != 0) {
            throw std::runtime_error(
                std::string("JPEG compress failed: ") + tjGetErrorStr2(handle_));
        }
        return {buf_, compressed_size};
    }

    // Non-copyable, non-movable (owns C handle)
    JpegCompressor(const JpegCompressor&) = delete;
    JpegCompressor& operator=(const JpegCompressor&) = delete;
};
```

### Complete ZSTD One-Shot Compression (Verified Pattern)
```cpp
// Source: facebook/zstd manual -- ZSTD_compressCCtx API
#include <zstd.h>
#include <vector>
#include <stdexcept>

class ZstdCompressor {
    ZSTD_CCtx* ctx_;
    std::vector<uint8_t> buf_;
    int level_;

public:
    ZstdCompressor(size_t max_input_size, int level = 1)
        : level_(level) {
        ctx_ = ZSTD_createCCtx();
        if (!ctx_) throw std::runtime_error("ZSTD_createCCtx failed");
        buf_.resize(ZSTD_compressBound(max_input_size));
    }

    ~ZstdCompressor() {
        if (ctx_) ZSTD_freeCCtx(ctx_);
    }

    // Returns (pointer, size) -- pointer valid until next compress() call
    std::pair<const uint8_t*, size_t> compress(
            const void* src, size_t src_size) {
        size_t result = ZSTD_compressCCtx(ctx_,
            buf_.data(), buf_.size(),
            src, src_size,
            level_);
        if (ZSTD_isError(result)) {
            throw std::runtime_error(
                std::string("ZSTD compress failed: ") + ZSTD_getErrorName(result));
        }
        return {buf_.data(), result};
    }

    ZstdCompressor(const ZstdCompressor&) = delete;
    ZstdCompressor& operator=(const ZstdCompressor&) = delete;
};
```

### Thread-Safe Bounded Queue
```cpp
// Source: Standard C++17 mutex+condvar pattern
#include <queue>
#include <mutex>
#include <condition_variable>
#include <optional>

template <typename T>
class BoundedQueue {
    std::queue<T> queue_;
    mutable std::mutex mutex_;
    std::condition_variable not_empty_;
    std::condition_variable not_full_;
    size_t max_size_;
    bool closed_ = false;
    size_t dropped_ = 0;

public:
    explicit BoundedQueue(size_t max_size) : max_size_(max_size) {}

    // Push with drop-oldest policy (never blocks producer)
    void push(T item) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (closed_) return;
        if (queue_.size() >= max_size_) {
            queue_.pop();  // Drop oldest
            ++dropped_;
        }
        queue_.push(std::move(item));
        not_empty_.notify_one();
    }

    // Pop with blocking wait (returns nullopt when closed)
    std::optional<T> pop() {
        std::unique_lock<std::mutex> lock(mutex_);
        not_empty_.wait(lock, [this] {
            return !queue_.empty() || closed_;
        });
        if (queue_.empty()) return std::nullopt;  // Closed
        T item = std::move(queue_.front());
        queue_.pop();
        not_full_.notify_one();
        return item;
    }

    void close() {
        std::lock_guard<std::mutex> lock(mutex_);
        closed_ = true;
        not_empty_.notify_all();
    }

    size_t dropped() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return dropped_;
    }
};
```

### Custom Binary Format: Wire Layout
```cpp
// Source: Designed based on requirements FR-2.1 through FR-2.6

// --- File-level constants ---
static constexpr uint8_t MAGIC[8] = {'E','G','O','R','E','C', 0x01, 0x00};
// 'EGOREC' + version major (1) + version minor (0)

static constexpr uint32_t FRAME_MAGIC = 0x46524D45; // 'FRME' little-endian

// --- File Header (fixed size, written once at start) ---
struct FileHeader {
    uint8_t  magic[8];              // "EGOREC\x01\x00"
    uint32_t header_size;           // Size of this header in bytes (for forward compat)
    uint32_t flags;                 // Bit 0: has_imu, Bit 1: has_index, etc.

    // Camera info
    char     serial_number[32];     // Null-terminated
    float    depth_scale;           // Z16 units to meters (typically 0.001)

    // Depth intrinsics
    uint32_t depth_width;
    uint32_t depth_height;
    float    depth_fx, depth_fy;
    float    depth_ppx, depth_ppy;
    uint32_t depth_distortion_model;
    float    depth_distortion_coeffs[5];

    // Color intrinsics
    uint32_t color_width;
    uint32_t color_height;
    float    color_fx, color_fy;
    float    color_ppx, color_ppy;
    uint32_t color_distortion_model;
    float    color_distortion_coeffs[5];

    // Depth-to-color extrinsics
    float    extrinsic_rotation[9];     // 3x3 row-major
    float    extrinsic_translation[3];  // meters

    // Session metadata
    char     session_name[128];     // Null-terminated
    uint64_t start_timestamp_us;    // Unix epoch, microseconds
    char     usb_type[8];           // "3.2", "2.0", etc.

    // Compression info
    uint8_t  rgb_codec;             // 0=raw, 1=JPEG, 2=H264
    uint8_t  depth_codec;           // 0=raw, 1=ZSTD, 2=Zdepth
    uint8_t  rgb_quality;           // JPEG quality (0-100)
    uint8_t  zstd_level;            // ZSTD compression level

    uint8_t  reserved[128];         // Future use, zero-filled
};
// Total: ~512 bytes (pad to round number)

// --- Frame Block (repeated, variable size) ---
struct FrameBlockHeader {
    uint32_t magic;                 // FRAME_MAGIC (0x46524D45)
    uint32_t block_size;            // Total size of this block including header
    uint64_t timestamp_us;          // Frame timestamp, microseconds
    uint64_t frame_number;          // Sequential counter
    uint32_t rgb_compressed_size;   // Size of compressed RGB data
    uint32_t depth_compressed_size; // Size of compressed depth data
    uint16_t imu_sample_count;      // Number of IMU samples (0 if no IMU)
    uint16_t flags;                 // Reserved
};
// Followed by:
//   uint8_t rgb_data[rgb_compressed_size];
//   uint8_t depth_data[depth_compressed_size];
//   IMUSample imu_data[imu_sample_count];  (if imu_sample_count > 0)

struct IMUSample {
    uint64_t timestamp_us;
    float    accel_x, accel_y, accel_z;  // m/s^2
    float    gyro_x, gyro_y, gyro_z;     // rad/s
};
// 32 bytes per sample

// --- Index Table (written at end of file) ---
struct IndexEntry {
    uint64_t timestamp_us;
    uint64_t file_offset;           // Byte offset of FrameBlockHeader
    uint64_t frame_number;
};
// 24 bytes per entry

// --- Footer (last bytes of file) ---
struct FileFooter {
    uint32_t magic;                 // 'INDX' = 0x58444E49
    uint64_t index_offset;          // Byte offset where index table starts
    uint32_t index_entry_count;     // Number of IndexEntry items
    uint64_t total_frames;          // Redundant but useful
    uint64_t total_duration_us;     // Last timestamp - first timestamp
    uint32_t footer_magic;          // 'DONE' = 0x454E4F44
};
```

### Recovery Procedure
```
1. Open file, read FileHeader (first 512 bytes)
2. If footer is present (last 28 bytes have DONE magic), use index_offset to read index table
3. If footer is missing (crash):
   a. Seek to byte 512 (after header)
   b. Scan for FRAME_MAGIC (0x46524D45) at each position
   c. Read FrameBlockHeader, validate block_size is reasonable
   d. Skip block_size bytes to find next frame
   e. Build index from recovered frames
   f. Last frame may be truncated -- discard if block_size extends past EOF
```

### CMakeLists.txt (Complete)
```cmake
cmake_minimum_required(VERSION 3.16)
project(ego-recorder VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# --- Dependencies: System packages ---
find_package(realsense2 REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(TURBOJPEG REQUIRED IMPORTED_TARGET libturbojpeg)
pkg_check_modules(ZSTD REQUIRED IMPORTED_TARGET libzstd)

# --- Dependencies: Header-only (FetchContent) ---
include(FetchContent)
FetchContent_Declare(
    cxxopts
    GIT_REPOSITORY https://github.com/jarro2783/cxxopts.git
    GIT_TAG        v3.3.1
)
FetchContent_MakeAvailable(cxxopts)

# --- Optional: systemd support ---
option(WITH_SYSTEMD "Build with systemd notification support" ON)
if(WITH_SYSTEMD)
    pkg_check_modules(SYSTEMD IMPORTED_TARGET libsystemd)
    if(SYSTEMD_FOUND)
        add_compile_definitions(HAVE_SYSTEMD)
    endif()
endif()

# --- Main executable ---
add_executable(ego-recorder
    src/main.cpp
    src/capture/pipeline.cpp
    src/compression/jpeg_compressor.cpp
    src/compression/zstd_compressor.cpp
    src/storage/file_writer.cpp
    src/utils/signal_handler.cpp
    src/utils/stats.cpp
)

target_include_directories(ego-recorder PRIVATE src)

target_link_libraries(ego-recorder PRIVATE
    realsense2::realsense2
    PkgConfig::TURBOJPEG
    PkgConfig::ZSTD
    cxxopts::cxxopts
    pthread
)

if(WITH_SYSTEMD AND SYSTEMD_FOUND)
    target_link_libraries(ego-recorder PRIVATE PkgConfig::SYSTEMD)
endif()

# --- Install ---
install(TARGETS ego-recorder DESTINATION bin)
```

### CLI Interface
```cpp
// Source: cxxopts v3.3.1 API
#include <cxxopts.hpp>

cxxopts::Options options("ego-recorder", "RealSense D435 RGBD recorder");
options.add_options()
    ("o,output", "Output directory", cxxopts::value<std::string>()->default_value("."))
    ("s,session-name", "Session name", cxxopts::value<std::string>()->default_value("capture"))
    ("d,duration", "Max recording duration in seconds (0=unlimited)",
        cxxopts::value<int>()->default_value("0"))
    ("q,quality", "JPEG quality (1-100)", cxxopts::value<int>()->default_value("90"))
    ("z,zstd-level", "ZSTD compression level (1-22)",
        cxxopts::value<int>()->default_value("1"))
    ("headless", "Run without display")
    ("h,help", "Print usage");

auto result = options.parse(argc, argv);
if (result.count("help")) {
    std::cout << options.help() << std::endl;
    return 0;
}
```

### Signal Handler
```cpp
// Source: POSIX sigwait pattern (from project-level research)
#include <signal.h>
#include <pthread.h>
#include <atomic>
#include <thread>

void setup_signal_handling(std::atomic<bool>& shutdown_flag) {
    // Block SIGTERM/SIGINT in ALL threads (before creating any other threads)
    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGTERM);
    sigaddset(&mask, SIGINT);
    pthread_sigmask(SIG_BLOCK, &mask, nullptr);

    // Dedicated signal thread
    std::thread([&shutdown_flag, mask]() {
        int sig;
        sigwait(&mask, &sig);
        shutdown_flag.store(true, std::memory_order_release);
        // stderr is fine here -- not an async signal handler
        fprintf(stderr, "\nReceived signal %d, shutting down...\n", sig);
    }).detach();
}
```

## JPEG Quality vs Compression Ratio Analysis

For 640x480 RGB images (921,600 bytes raw):

| Quality | Approx JPEG Size | Compression Ratio | Visual Quality | Recommendation |
|---------|-------------------|-------------------|----------------|----------------|
| 95 | ~120-180 KB | ~6:1 | Near-lossless | Overkill for ML training |
| 90 | ~60-100 KB | ~10-15:1 | Excellent | **Recommended default** |
| 85 | ~40-70 KB | ~13-20:1 | Very good | Good if disk-constrained |
| 80 | ~30-50 KB | ~18-30:1 | Good, minor artifacts | Acceptable for most ML |
| 75 | ~25-40 KB | ~23-37:1 | Visible artifacts on edges | Not recommended |

**Recommendation:** Quality 90 with TJSAMP_420 (4:2:0 chroma subsampling). This matches LeRobot's approach (which uses AV1 at CRF 30, approximately equivalent quality). For robotics ML training data, the chroma subsampling has negligible impact since models typically resize and augment anyway.

**TJSAMP_420 vs TJSAMP_444:**
- 4:2:0 reduces chroma resolution by 2x in each dimension. ~30% smaller files.
- 4:4:4 preserves full chroma. Better for color-critical tasks.
- For manipulation VLM training at 640x480, 4:2:0 is standard and sufficient.

## ZSTD Level Selection for Depth

For 640x480 Z16 depth frames (614,400 bytes):

| Level | Approx Ratio | Compress Time | Decompress Time | Notes |
|-------|--------------|---------------|-----------------|-------|
| 1 | ~3.0-3.5:1 | <0.5ms | <0.3ms | **Fastest, recommended for MVP** |
| 3 (default) | ~3.5-4.0:1 | ~0.5-1.0ms | <0.3ms | Slightly better ratio |
| 5 | ~3.8-4.2:1 | ~1.5ms | <0.3ms | Diminishing returns |
| 9 | ~4.0-4.5:1 | ~5ms | <0.3ms | Too slow for real-time |

**Recommendation:** Level 1 for MVP. The ratio difference between level 1 and 3 is ~10-15%, but level 1 is 2x faster. At 30fps, every millisecond counts when compression shares a thread with disk I/O.

**No dictionary training needed for MVP:** Dictionary training is useful when compressing many small (<1KB) similar items. Our depth frames are 614KB each -- large enough that ZSTD's internal model adapts well per-frame. Dictionary training adds complexity without meaningful ratio improvement for this data size.

## Pipeline Warmup and First Frames

**Official Intel recommendation:** Drop the first 30 frames after `pipe.start()`.

**Sequence:**
1. `pipe.start(cfg)` -- pipeline begins streaming
2. Set `RS2_OPTION_AUTO_EXPOSURE_PRIORITY` to 0
3. Enable `RS2_OPTION_GLOBAL_TIME_ENABLED` on depth sensor
4. Loop: `pipe.wait_for_frames()` x30 (discard results)
5. Begin recording loop

**Why 30 frames:** At 30fps, this is 1 second of warmup. Auto-exposure typically converges in 10-15 frames, but 30 provides margin for:
- Depth sensor warmup (stereo matching stabilization)
- USB bandwidth negotiation settling
- Global time synchronization establishing its clock mapping

**What if the user wants instant recording?** Document that the first ~1s of recording may have suboptimal exposure. Consider a `--skip-warmup` flag for testing, but default to 30-frame warmup.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| libjpeg (no SIMD) | libjpeg-turbo (SIMD) | 2010+ | 3-7x faster JPEG encode/decode |
| ZSTD level 3 default | ZSTD level 1 for real-time | Stable | Prefer speed for streaming |
| `rs2::frame::keep()` for buffering | Move-based frame pipeline | SDK 2.x | Constant memory possible |
| Multiple files per session | Single binary container | Current best practice | Atomic sessions, easier transfer |
| PNG for depth | ZSTD direct on Z16 | 2020+ | 2x better ratio, 15x faster |
| getopt for CLI | cxxopts/CLI11 | 2018+ | Type-safe, auto-help, modern C++ |

**Deprecated/outdated:**
- `tjCompress` (old API): Use `tjCompress2` (supports pixel format enum)
- `ZSTD_compress()` without context: Use `ZSTD_compressCCtx()` to reuse context
- librealsense 1.x API: Completely different from 2.x

## Open Questions

1. **Exact JPEG size at quality 90 for D435 RGB output**
   - What we know: General estimates of 60-100KB based on typical images
   - What's unclear: D435 RGB at 640x480 has specific noise/texture characteristics that affect JPEG ratio
   - Recommendation: Run a quick benchmark with 100 actual D435 frames at quality 85/90/95 during implementation to set the default

2. **ZSTD level 1 vs 3 on actual D435 depth data**
   - What we know: General ZSTD benchmarks on various data types
   - What's unclear: D435 depth frames have unique characteristics (large zero regions, smooth gradients) that may compress differently
   - Recommendation: Benchmark both on actual D435 depth frames during implementation

3. **Optimal bounded queue size**
   - What we know: 4-8 is the recommended range from project-level research
   - What's unclear: Exact size depends on disk I/O variability of the target system
   - Recommendation: Start with 4, make it a command-line parameter for tuning

4. **rs2::frame_queue vs custom queue**
   - What we know: librealsense provides `rs2::frame_queue` with bounded capacity and `poll_for_frame()` / `enqueue()`. It integrates naturally with the SDK's frame pool.
   - What's unclear: Whether using the SDK's queue avoids the memcpy of raw data (zero-copy transfer). If frames are moved via `rs2::frame_queue`, the SDK may keep the buffer alive without pool return until the consumer releases it.
   - Recommendation: Consider using `rs2::frame_queue(4)` instead of a custom queue to potentially avoid the 1.5MB memcpy per frame. Benchmark both approaches.

## Sources

### Primary (HIGH confidence)
- [ZSTD Manual](http://facebook.github.io/zstd/zstd_manual.html) -- ZSTD_compress, ZSTD_compressCCtx, ZSTD_compressBound, compression levels
- [libjpeg-turbo/TurboJPEG API docs](https://gensoft.pasteur.fr/docs/libjpeg-turbo/2.0.4/group___turbo_j_p_e_g.html) -- tjCompress2 signature, buffer management
- [libjpeg-turbo Performance](https://libjpeg-turbo.org/About/Performance) -- 3-7x speedup over libjpeg with SIMD
- [cxxopts GitHub](https://github.com/jarro2783/cxxopts) -- v3.3.1, MIT, header-only
- [zstd CMake README](https://github.com/facebook/zstd/blob/dev/build/cmake/README.md) -- FetchContent pattern, target names
- [librealsense auto-exposure issue #2269](https://github.com/IntelRealSense/librealsense/issues/2269) -- 30-frame warmup recommendation
- [rs2::frame_queue docs](https://intelrealsense.github.io/librealsense/doxygen/classrs2_1_1frame__queue.html) -- bounded queue with poll/enqueue
- Project-level research: `.planning/research/librealsense-api.md` -- pipeline config, threading, zero-copy patterns
- Project-level research: `.planning/research/depth-compression.md` -- ZSTD benchmarks, container format design
- Project-level research: `.planning/research/headless-systemd.md` -- signal handling patterns

### Secondary (MEDIUM confidence)
- [stb_image_write performance comparison](https://blog.gibson.sh/2015/03/23/comparing-performance-stb_image-vs-libjpeg-turbo-libpng-and-lodepng/) -- stb slower than libjpeg-turbo (2015 benchmarks)
- [JPEG quality comparison](https://sirv.com/help/articles/jpeg-quality-comparison/) -- 90% quality threshold guidance
- [Thread-safe queue patterns](https://www.justsoftwaresolutions.co.uk/threading/implementing-a-thread-safe-queue-using-condition-variables.html) -- Anthony Williams' definitive guide
- [Zstandard benchmarks](https://facebook.github.io/zstd/) -- Official speed/ratio table

### Tertiary (LOW confidence)
- JPEG file size estimates at 640x480 are approximate -- actual sizes depend heavily on image content (scene complexity, texture, noise). Must be validated with real D435 frames.
- ZSTD compression ratios on depth data are interpolated from general benchmarks. Real D435 depth data may compress better (large zero regions) or worse (noisy depth edges).

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- libjpeg-turbo, ZSTD, and cxxopts are well-established with verified APIs
- Architecture: HIGH -- three-thread pipeline is proven pattern from librealsense documentation
- Binary format: MEDIUM -- custom design, no existing standard to follow; based on engineering judgment
- Pitfalls: HIGH -- documented in librealsense issues and library documentation
- JPEG quality tradeoffs: MEDIUM -- estimates need validation with actual D435 frames
- ZSTD level selection: MEDIUM -- needs benchmarking on actual depth data

**Research date:** 2026-02-19
**Valid until:** 2026-03-19 (stable libraries, not fast-moving)
