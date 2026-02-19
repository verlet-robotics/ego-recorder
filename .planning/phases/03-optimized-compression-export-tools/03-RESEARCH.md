# Phase 3: Optimized Compression + Export Tools - Research

**Researched:** 2026-02-19
**Domain:** C++ compression (Zdepth, H.264/FFmpeg), Python C-extension (nanobind), RLDS/TFDS, LeRobot v3
**Confidence:** MEDIUM-HIGH (stack verified; some LeRobot v3 API details from source inspection only)

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Export tool invocation
- Subcommand pattern: `ego-recorder export rlds`, `ego-recorder export lerobot`
- Batch processing supported: multiple .egorec paths or glob patterns, processed sequentially
- Output defaults to same directory as input file (e.g., `file.egorec` → `file_rlds/`), `--output` flag to override
- Progress bar with frame X/Y, ETA, throughput (MB/s); `--quiet` flag for silent mode
- `ego-recorder info file.egorec` subcommand to inspect file metadata (format version, codecs, frame count, duration, resolution, intrinsics)

#### Export language & integration
- Export tools written in Python (native tensorflow_datasets + huggingface_hub ecosystem)
- C++ reader exposed as Python extension module via pybind11
- Python scripts in the repo import the C++ extension for reading .egorec frames

#### Dataset mapping
- One .egorec file = one RLDS episode (direct 1:1 mapping)
- Dataset named from session name embedded in the .egorec file header
- Batch export creates one dataset with multiple episodes (from session name)
- LeRobot: multiple recordings merge into one dataset by default; `--separate` flag to split into individual datasets

#### Export content & quality
- Depth exported as raw uint16 (mm) — original D435 Z16 values preserved exactly, lossless
- Camera intrinsics and extrinsics always included in every export (essential for 3D reconstruction)
- LeRobot MP4 at CRF 23 (balanced quality, suitable for most ML training)
- Per-frame timestamps as relative offset from recording start (0.0, 0.033, 0.066...), not absolute epoch

#### V1 file compatibility
- V1 .egorec files (ZSTD+JPEG) NOT supported by export tools — v2 only
- Recorder drops v1 writing entirely after upgrade — clean break, no legacy format flag
- V2 container format uses extensible codec IDs per stream (e.g., DEPTH_ZDEPTH=2, RGB_H264=2) so future codecs add new enum values without format version bump

### Claude's Discretion
- Zdepth porting/adaptation strategy (port vs adapt catid/Zdepth)
- H.264 encoding library choice (libx264 vs FFmpeg)
- pybind11 vs nanobind for C++ Python bindings
- Progress bar library choice for Python CLI
- Exact TFRecord schema design
- Exact Parquet column layout for LeRobot

### Deferred Ideas (OUT OF SCOPE)
None — discussion stayed within phase scope
</user_constraints>

---

## Summary

Phase 3 involves four interconnected workstreams: (1) replacing ZSTD raw-depth compression with Zdepth-style block-prediction encoding, (2) replacing per-frame JPEG with H.264 video streaming, (3) exposing the C++ .egorec reader to Python via a C extension module, and (4) building RLDS and LeRobot v3 export tools. Each workstream has a well-defined solution with verified APIs. The biggest integration risk is the Zdepth library's bundled zstd conflicting with the project's existing system zstd dependency; this must be resolved explicitly in CMake. The second risk is LeRobot v3 API instability — the v3 format only landed in lerobot 0.4.x and requires installation from the main branch or a venv.

The C++ side (Zdepth + H.264 + container v2) and the Python side (reader extension + export tools) can be developed in parallel. The container format v2 changes (codec IDs in the header) are a prerequisite for everything else — finalize `binary_format.h` first so both the C++ encoder and Python reader agree on wire format.

**Primary recommendation:** Use catid/Zdepth via CMake FetchContent (guard against bundled zstd clash); use FFmpeg libavcodec (already installed: version 6.1.1 with libx264 backend) for H.264 encode via CRF 23; use pybind11 via FetchContent for the C extension (honors the locked decision); use tqdm for progress bars; target lerobot 0.4.x in a venv; tensorflow-datasets 4.9.9 for RLDS.

---

## Standard Stack

### Core — C++ side

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| catid/Zdepth | master | Depth block-prediction + ZSTD | Purpose-built for RealSense Z16; 7.8-11.9:1 ratio; BSD-3 |
| libavcodec (FFmpeg) | 6.1.1 (system) | H.264 video encode/decode | Already installed; no new dep; CRF mode for quality target |
| libswscale (FFmpeg) | 7.5.100 (system) | RGB24 → YUV420P conversion | Required before H.264 encode; same FFmpeg package |
| libavutil (FFmpeg) | 58.29.100 (system) | AVFrame/AVPacket utilities | Same FFmpeg package |
| pybind11 | 2.13+ via FetchContent | C++ → Python extension module | Locked user decision |

### Core — Python side

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tqdm | 4.x (install in venv) | Progress bar with ETA + throughput | Lightweight; bytes/s support; quiet flag easy |
| tensorflow-datasets | 4.9.9 | RLDS/TFRecord building | Standard for RLDS ecosystem; tfds build command |
| lerobot | 0.4.x (from main) | LeRobot v3 dataset creation | Only version with v3 format (LeRobotDataset.create) |
| numpy | 1.26.4 (system) | Array handling in bindings | Already available |
| pyarrow | via lerobot | Parquet writing | Pulled in by lerobot |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| pybind11 | nanobind | nanobind is faster/smaller but pybind11 is the locked decision |
| FFmpeg libavcodec | libx264 directly | libx264 requires more boilerplate; FFmpeg wraps it with AVCodecContext |
| tqdm | rich | rich is heavier; tqdm has native `--bytes` unit scaling and is already present (v0.0.0 stub) |
| catid/Zdepth via FetchContent | vendored copy | FetchContent preserves upstream tracking; vendor if API stability needed |

**Installation (venv recommended for Python tools):**
```bash
# System dependencies (already installed on dev machine)
# libavcodec-dev 6.1.1, libswscale-dev, libavutil-dev, libx264-dev, python3-dev, libzstd-dev

# Python venv for export tools
python3 -m venv .venv
source .venv/bin/activate
pip install tqdm numpy tensorflow-datasets apache-beam
pip install "git+https://github.com/huggingface/lerobot.git"  # for v3 support
```

---

## Architecture Patterns

### Recommended Project Structure

```
ego-recorder/
├── src/
│   ├── compression/
│   │   ├── zdepth_compressor.h/.cpp   # Wraps catid/Zdepth DepthCompressor
│   │   ├── h264_encoder.h/.cpp        # Wraps AVCodecContext for H.264
│   │   ├── jpeg_compressor.h/.cpp     # KEEP for reference/fallback
│   │   └── zstd_compressor.h/.cpp     # KEEP (used by Zdepth indirectly)
│   ├── storage/
│   │   └── binary_format.h            # V2: add FILE_MAGIC v2, update codec enums
│   └── python/
│       └── egorec_reader.cpp          # pybind11 module: EgorecReader class
├── python/
│   ├── egorec_reader.so               # Built by CMake, imported by scripts
│   ├── export_rlds.py                 # RLDS export entry point
│   └── export_lerobot.py             # LeRobot export entry point
└── CMakeLists.txt                     # Updated with Zdepth + pybind11 + avcodec
```

### Pattern 1: Container Format V2 Header Update

**What:** Bump the file magic version byte from 0x01 to 0x02 and document that `rgb_codec` and `depth_codec` fields are extensible enum values, not version-locked constants.

**Codec ID values (uint8 in FileHeader):**
```
rgb_codec:   0=raw, 1=JPEG, 2=H264
depth_codec: 0=raw, 1=ZSTD,  2=Zdepth
```

The existing `binary_format.h` already defines these fields (`rgb_codec`, `depth_codec`). V2 simply means the recorder always writes `rgb_codec=2` and `depth_codec=2`. The reader checks these fields per-frame rather than assuming a single global codec.

**FILE_MAGIC update:**
```cpp
// In binary_format.h
static constexpr uint8_t FILE_MAGIC[8] = {'E','G','O','R','E','C', 0x02, 0x00};
//                                                                   ^^^^ v2
```

### Pattern 2: Zdepth Integration via FetchContent

**What:** Fetch catid/Zdepth at configure time. Guard against its bundled zstd conflicting with system zstd (critical: Zdepth does `if (NOT TARGET zstd) add_subdirectory(zstd) endif()` — it will skip its bundled zstd if `zstd` target already exists).

**CMake pattern:**
```cmake
# IMPORTANT: Must create the 'zstd' target BEFORE fetching Zdepth
# so Zdepth skips its bundled copy and uses ours.
# pkg_check_modules already creates PkgConfig::ZSTD, but Zdepth
# checks for TARGET named exactly 'zstd'. Create an alias:
add_library(zstd ALIAS PkgConfig::ZSTD)

FetchContent_Declare(
    zdepth
    GIT_REPOSITORY https://github.com/catid/Zdepth.git
    GIT_TAG        master  # pin to a specific commit SHA in practice
)
FetchContent_MakeAvailable(zdepth)

target_link_libraries(ego-recorder PRIVATE zdepth::zdepth)

# For the pybind11 module:
pybind11_add_module(egorec_reader src/python/egorec_reader.cpp)
target_link_libraries(egorec_reader PRIVATE zdepth::zdepth PkgConfig::ZSTD)
```

**WARNING:** The ALIAS approach may not work with all CMake versions if PkgConfig::ZSTD is an IMPORTED target — test this. Alternative: vendor Zdepth source in the repo (copy the src/ + include/ into third_party/zdepth/) and just add_subdirectory, defining a stub zstd CMake target first.

### Pattern 3: H.264 Encoder (FFmpeg avcodec send/receive API)

**What:** One `H264Encoder` class wraps an `AVCodecContext` and provides a `encode(rgb24_ptr, width, height) → encoded_packets` interface. Internally converts RGB24 → YUV420P via `libswscale`, then calls `avcodec_send_frame` / `avcodec_receive_packet`.

```cpp
// Source: FFmpeg official encode_video.c example + libswscale docs
// Headers needed:
extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/opt.h>
#include <libavutil/imgutils.h>
#include <libswscale/swscale.h>
}

class H264Encoder {
public:
    H264Encoder(int width, int height, int fps, int crf = 23);
    ~H264Encoder();
    // Returns compressed H.264 NAL units for one input RGB frame.
    // May return empty vector on buffered frames (B-frames).
    // Call with rgb=nullptr to flush remaining frames.
    std::vector<uint8_t> encode(const uint8_t* rgb24, int width, int height);
    std::vector<uint8_t> flush();  // call at end of recording

private:
    AVCodecContext* ctx_{nullptr};
    AVFrame*        yuv_frame_{nullptr};
    AVPacket*       pkt_{nullptr};
    SwsContext*     sws_{nullptr};
};
```

**CMake integration:**
```cmake
find_package(PkgConfig REQUIRED)
pkg_check_modules(AVCODEC  REQUIRED IMPORTED_TARGET libavcodec)
pkg_check_modules(AVUTIL   REQUIRED IMPORTED_TARGET libavutil)
pkg_check_modules(SWSCALE  REQUIRED IMPORTED_TARGET libswscale)

target_link_libraries(ego-recorder PRIVATE
    PkgConfig::AVCODEC
    PkgConfig::AVUTIL
    PkgConfig::SWSCALE
)
```

**Key codec context setup (CRF 23):**
```cpp
// Source: FFmpeg encode_video.c official example
const AVCodec* codec = avcodec_find_encoder_by_name("libx264");
AVCodecContext* c = avcodec_alloc_context3(codec);
c->width     = width;
c->height    = height;
c->time_base = {1, fps};
c->framerate = {fps, 1};
c->pix_fmt   = AV_PIX_FMT_YUV420P;
c->gop_size  = fps;       // one keyframe per second
c->max_b_frames = 0;      // disable B-frames for real-time / seeking
av_opt_set(c->priv_data, "preset", "fast", 0);  // real-time trade-off
av_opt_set(c->priv_data, "crf",    "23",   0);  // CRF 23 per decision
avcodec_open2(c, codec, nullptr);
```

**Encode loop (send/receive pattern):**
```cpp
// Source: FFmpeg official API overview
avcodec_send_frame(ctx_, yuv_frame_);  // send YUV frame

std::vector<uint8_t> out;
while (true) {
    int ret = avcodec_receive_packet(ctx_, pkt_);
    if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) break;
    if (ret < 0) throw std::runtime_error("encode error");
    // append pkt_->data[0..pkt_->size] to out
    av_packet_unref(pkt_);
}
return out;
```

**RGB24 → YUV420P (required before encoding):**
```cpp
// Source: FFmpeg libswscale docs
SwsContext* sws = sws_getContext(
    width, height, AV_PIX_FMT_RGB24,
    width, height, AV_PIX_FMT_YUV420P,
    SWS_FAST_BILINEAR, nullptr, nullptr, nullptr);

const uint8_t* src_data[1] = { rgb24 };
int src_linesize[1] = { width * 3 };
sws_scale(sws, src_data, src_linesize, 0, height,
          yuv_frame_->data, yuv_frame_->linesize);
```

### Pattern 4: pybind11 EgorecReader Extension Module

**What:** A C++ pybind11 module (`egorec_reader`) that opens a .egorec v2 file and iterates frames, returning decoded data as Python dicts with numpy arrays.

**CMake setup:**
```cmake
FetchContent_Declare(
    pybind11
    GIT_REPOSITORY https://github.com/pybind/pybind11.git
    GIT_TAG        v2.13.6
)
FetchContent_MakeAvailable(pybind11)

pybind11_add_module(egorec_reader src/python/egorec_reader.cpp)
target_link_libraries(egorec_reader PRIVATE zdepth::zdepth PkgConfig::AVCODEC PkgConfig::ZSTD)
target_include_directories(egorec_reader PRIVATE src)
```

**Python module API design:**
```cpp
// Source: pybind11 official docs (pybind11.readthedocs.io/en/stable/basics.html)
#include <pybind11/pybind11.h>
#include <pybind11/numpy.h>
#include <pybind11/stl.h>

namespace py = pybind11;

PYBIND11_MODULE(egorec_reader, m) {
    py::class_<EgorecFile>(m, "EgorecFile")
        .def(py::init<const std::string&>())
        .def("header", &EgorecFile::header_dict)  // returns dict with metadata
        .def("frame_count", &EgorecFile::frame_count)
        .def("frames", &EgorecFile::frames_iterator)  // returns iterable
        ;
}
```

**Frame dict returned to Python:**
```python
{
    "timestamp_relative_s": float,      # offset from start, seconds
    "frame_number":         int,
    "rgb":   np.ndarray(dtype=uint8,  shape=(H, W, 3)),  # decoded from H264
    "depth": np.ndarray(dtype=uint16, shape=(H, W)),     # decoded from Zdepth, mm
}
```

**Header dict returned to Python:**
```python
{
    "session_name":      str,
    "format_version":    int,   # 2 for v2
    "frame_count":       int,
    "duration_s":        float,
    "start_ts_us":       int,
    "depth_width":       int,
    "depth_height":      int,
    "color_width":       int,
    "color_height":      int,
    "depth_fx":          float,
    "depth_fy":          float,
    "depth_ppx":         float,
    "depth_ppy":         float,
    "depth_distortion":  list[float],  # 5 coefficients
    "color_fx":          float,
    # ... color intrinsics ...
    "extrinsic_R":       list[float],  # 9 floats, row-major 3x3
    "extrinsic_t":       list[float],  # 3 floats
    "rgb_codec":         int,   # 2=H264
    "depth_codec":       int,   # 2=Zdepth
}
```

### Pattern 5: RLDS Export

**What:** Python script that reads .egorec frames via `egorec_reader`, builds a TFDS episode, yields it through `_generate_examples()`.

**Schema for our dataset:**
```python
# Source: kpertsch/rlds_dataset_builder + maniskill RLDS example (tensorflow.org)
import tensorflow_datasets as tfds

features = tfds.features.FeaturesDict({
    'steps': tfds.features.Dataset({
        'observation': tfds.features.FeaturesDict({
            'image':  tfds.features.Image(shape=(480, 640, 3), dtype=np.uint8, encoding_format='jpeg'),
            'depth':  tfds.features.Image(shape=(480, 640, 1), dtype=np.uint16, encoding_format='png'),
            # Camera intrinsics/extrinsics (same for all steps, from header)
            'depth_intrinsics': tfds.features.Tensor(shape=(4,), dtype=np.float32),   # fx,fy,ppx,ppy
            'color_intrinsics': tfds.features.Tensor(shape=(4,), dtype=np.float32),
            'extrinsic_R':      tfds.features.Tensor(shape=(3,3), dtype=np.float32),
            'extrinsic_t':      tfds.features.Tensor(shape=(3,), dtype=np.float32),
        }),
        'timestamp': tfds.features.Scalar(dtype=np.float64),  # relative seconds
        'is_first':  tfds.features.Scalar(dtype=np.bool_),
        'is_last':   tfds.features.Scalar(dtype=np.bool_),
        'is_terminal': tfds.features.Scalar(dtype=np.bool_),
    }),
    'episode_metadata': tfds.features.FeaturesDict({
        'file_path':    tfds.features.Text(),
        'session_name': tfds.features.Text(),
        'duration_s':   tfds.features.Scalar(dtype=np.float64),
    }),
})
```

**Depth uint16 encoding note (CRITICAL):** D435 Z16 values are in mm (0–65535). The standard RLDS depth encoding is `depth_in_meters = value / 1024`. Our values are in mm, so `depth_in_meters = value / 1000`. Document this in the dataset description string — do NOT rescale the stored values.

**_generate_examples pattern:**
```python
# Source: kpertsch/rlds_dataset_builder template
def _generate_examples(self, paths):
    for episode_path in paths:
        reader = egorec_reader.EgorecFile(str(episode_path))
        header = reader.header()
        steps = []
        for i, frame in enumerate(reader.frames()):
            depth_hw1 = frame["depth"][:, :, np.newaxis]  # add channel dim
            steps.append({
                'observation': {
                    'image': frame["rgb"],
                    'depth': depth_hw1,
                    # intrinsics same for all steps
                    'depth_intrinsics': np.array([header['depth_fx'], ...], dtype=np.float32),
                    ...
                },
                'timestamp': frame["timestamp_relative_s"],
                'is_first': i == 0,
                'is_last': i == len_frames - 1,
                'is_terminal': i == len_frames - 1,
            })
        yield episode_path, {
            'steps': steps,
            'episode_metadata': {'file_path': str(episode_path), ...}
        }
```

### Pattern 6: LeRobot v3 Export

**What:** Python script using `LeRobotDataset.create()` to build a dataset from one or more .egorec files.

**API (verified against lerobot 0.4.x source):**
```python
# Source: huggingface/lerobot examples/port_datasets/port_droid.py
from lerobot.datasets.lerobot_dataset import LeRobotDataset

features = {
    "observation.images.rgb": {"dtype": "video", "shape": (480, 640, 3), "names": ["height", "width", "channel"]},
    "observation.depth":      {"dtype": "float32", "shape": (480, 640), "names": ["height", "width"]},
    # depth stored as float32 (mm → meters divide by 1000) for LeRobot convention
    "observation.state":      {"dtype": "float32", "shape": (6,), "names": None},
    # state = [fx, fy, ppx, ppy, extrinsic placeholder...] or leave minimal
    "timestamp":              {"dtype": "float64", "shape": (1,), "names": None},
    "task_index":             {"dtype": "int64",   "shape": (1,), "names": None},
}

dataset = LeRobotDataset.create(
    repo_id="local/ego-recording",   # local-only until push_to_hub
    fps=30,
    features=features,
    robot_type="realsense_d435",
)

for frame in reader.frames():
    dataset.add_frame({
        "observation.images.rgb": frame["rgb"],
        "observation.depth":      frame["depth"].astype(np.float32) / 1000.0,
        "timestamp":              np.array([frame["timestamp_relative_s"]]),
        "task_index":             np.array([0], dtype=np.int64),
    })

dataset.save_episode()
dataset.finalize()          # CRITICAL: closes parquet writers, writes footers
```

**Video encoding (MP4 at CRF 23):** LeRobot handles MP4 encoding internally for `"dtype": "video"` features. The default codec in recent lerobot is `libsvtav1` — override to H.264 if needed via `lerobot-edit-dataset --operation.vcodec h264 --operation.crf 23`. Alternatively, pre-encode with FFmpeg and pass paths. Verify in lerobot 0.4.x whether `LeRobotDataset.create()` accepts a `video_codec` parameter.

**CRITICAL: finalize() is mandatory** before any dataset inspection — skipping it produces corrupt parquet files.

### Pattern 7: Subcommand Dispatch in main.cpp

cxxopts does NOT have native subcommand support. The standard pattern with cxxopts is to capture the first positional argument as the subcommand name, then route:

```cpp
// Source: cxxopts GitHub README + cxxsubs pattern
// main.cpp updated to dispatch subcommands:

int main(int argc, char* argv[]) {
    if (argc < 2) { print_usage(); return 1; }

    std::string_view cmd = argv[1];

    if (cmd == "record") {
        return cmd_record(argc - 1, argv + 1);  // existing logic
    } else if (cmd == "export") {
        return cmd_export(argc - 1, argv + 1);  // calls Python via subprocess
    } else if (cmd == "info") {
        return cmd_info(argc - 1, argv + 1);    // C++ reads header, prints
    } else {
        fprintf(stderr, "Unknown command: %s\n", argv[1]);
        print_usage();
        return 1;
    }
}
```

**`ego-recorder export` delegates to Python:** The C++ `cmd_export()` function locates the Python script alongside the binary and calls it via `execvp` or `system()`, or directly prints usage telling the user to run the Python script. Keep C++ `cmd_export` thin — its role is just to forward args.

**`ego-recorder info` is pure C++:** Reads the FileHeader binary, then prints formatted output. No Python dependency.

### Anti-Patterns to Avoid

- **Reading H.264 stream without demuxer context:** H.264 NAL units stored raw in .egorec need proper decoder initialization. Use `AV_CODEC_ID_H264` with `avcodec_find_decoder` in the pybind11 reader. Do NOT try to treat the raw byte blob as displayable without decoding.
- **B-frames in real-time encoder:** B-frames require lookahead and cause frame reordering, making the writer-thread pipeline more complex. Set `c->max_b_frames = 0` for real-time recording.
- **Forgetting to flush the H.264 encoder:** At recording stop, call `encode(nullptr, 0, 0)` (send null frame) to drain buffered P-frames. Missing this loses the last ~0.5s of video.
- **Forgetting to flush the H.264 decoder:** In the Python reader, call `avcodec_send_packet(ctx, nullptr)` after the last packet to drain.
- **Zdepth zstd symbol collision:** If Zdepth compiles its bundled zstd AND the project links system libzstd, you get multiple definition linker errors or silent ABI mismatches. The CMake `zstd` alias guard above must be in place BEFORE `FetchContent_MakeAvailable(zdepth)`.
- **Not calling LeRobot finalize():** Parquet writers buffer data; without `finalize()`, footer metadata is not written and the file is corrupt.
- **Storing absolute timestamps in LeRobot:** LeRobot expects `timestamp` as relative offset from episode start (seconds). Divide by 1e6 from the header's `start_timestamp_us` baseline.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Depth block prediction | Custom predictor | catid/Zdepth | Handles zig-zag encoding, 8×8 block context, 6 encode modes for D435 range |
| H.264 NAL unit generation | Custom bitstream | libavcodec (FFmpeg) | Rate control (CRF), B-frame management, Annex B headers are all non-trivial |
| RGB→YUV color conversion | Manual conversion formula | libswscale | Handles chroma subsampling, YCbCr coefficients correctly for BT.601/709 |
| TFRecord episode packing | Custom protobuf | tensorflow-datasets tfds.features | Handles nested Dataset, encoding (PNG for uint16), sharding |
| Parquet writing | pandas to_parquet | lerobot LeRobotDataset | Handles chunk sizing, episode offsets, metadata Parquet, video shard naming |
| Progress ETA + throughput | Time-since-start / frame_count | tqdm | Handles adaptive ETA, bytes/s display, quiet mode, terminal detection |
| Python numpy array wrapping | ctypes + memoryview | pybind11 py::array_t | Handles ownership lifetime, shape/stride metadata, refcounting |

**Key insight:** The "simple" problems in this domain (H.264 CRF, YUV subsampling, TFRecord schema validation, Parquet footer) all have subtle correctness traps that the established libraries handle correctly. Custom implementations reliably hit these traps at the worst time (data corruption in a 2-hour recording).

---

## Common Pitfalls

### Pitfall 1: Zdepth Bundled Zstd Conflict
**What goes wrong:** CMake links both the bundled zstd (compiled as part of Zdepth) and system libzstd, causing duplicate symbol errors or silent function pointer mixups.
**Why it happens:** Zdepth does `add_subdirectory(zstd)` if no target named `zstd` exists. The system libzstd from `pkg_check_modules(ZSTD ...)` creates target `PkgConfig::ZSTD`, not `zstd`. So Zdepth doesn't see it and adds its bundled one.
**How to avoid:** Add `add_library(zstd ALIAS PkgConfig::ZSTD)` BEFORE `FetchContent_MakeAvailable(zdepth)`. Verify this works with your CMake version — if ALIAS of IMPORTED target fails, vendor Zdepth instead (copy src/ + include/ to third_party/zdepth/, create stub CMakeLists that defines TARGET zstd before adding Zdepth).
**Warning signs:** Linker errors mentioning `ZSTD_compress` defined multiple times, or `zstd.h` version mismatch at runtime.

### Pitfall 2: H.264 Width/Height Must Be Even
**What goes wrong:** `avcodec_open2` returns error if width or height is odd.
**Why it happens:** YUV420P chroma plane is half-resolution; odd dimensions are undefined.
**How to avoid:** Assert `width % 2 == 0 && height % 2 == 0` at encoder construction. D435 default 640×480 is fine, but guard anyway.
**Warning signs:** `avcodec_open2` returns `-22` (EINVAL).

### Pitfall 3: H.264 Frame Timestamp Must Increase Monotonically
**What goes wrong:** Decoder produces frames in wrong order or drops frames.
**Why it happens:** `frame->pts` must increment by 1 per frame (with `time_base = {1, fps}`). If you forget to increment or reset pts, libx264 silently drops frames.
**How to avoid:** Maintain a frame counter in H264Encoder, set `frame->pts = frame_counter_++` on every encode call.
**Warning signs:** Decoded video is shorter than expected; ffprobe shows duplicate PTS.

### Pitfall 4: Zdepth EncodeMode Must Match D435 Range
**What goes wrong:** Decompressed depth values are wrong (wrong scale/quantization).
**Why it happens:** Zdepth has 6 encode modes for different quantization levels. `kAzureKinectQuantized` targets Azure Kinect ranges which differ from D435.
**How to avoid:** For D435 with max range ~10m (10000mm), use `EncodeMode::kNotQuantized8191mm` (or `kNotQuantized4095mm` for indoor use). Set via `compressor.set_encode_mode(EncodeMode::kNotQuantized8191mm)` before first `Compress()` call. The exact mode choice affects compression ratio but not correctness for uint16 lossless.
**Warning signs:** Decoded depth values are systematically off by a constant factor.

### Pitfall 5: pybind11 Python Module Placement for Import
**What goes wrong:** `import egorec_reader` fails at runtime even though the .so exists.
**Why it happens:** Python looks for the .so in directories on `sys.path`. CMake builds it into `build/` by default, not alongside the Python scripts.
**How to avoid:** Either (a) add a `cmake --install` step that places the .so in `python/`, or (b) set `OUTPUT_NAME` and `LIBRARY_OUTPUT_DIRECTORY` in CMake to put the .so in `python/`, or (c) instruct users to run scripts with `PYTHONPATH=build/ python python/export_rlds.py`.
**Warning signs:** `ModuleNotFoundError: No module named 'egorec_reader'`.

### Pitfall 6: LeRobot v3 Not in PyPI Stable
**What goes wrong:** `pip install lerobot` installs 0.3.x which lacks `LeRobotDataset.create()` with v3 format support.
**Why it happens:** LeRobot v3 landed in lerobot 0.4.x, which at research time was only available from the main branch.
**How to avoid:** Install from main: `pip install "git+https://github.com/huggingface/lerobot.git"`. Pin to a specific commit SHA once the version stabilizes. Document this in the repo's `python/README.md` or `requirements.txt`.
**Warning signs:** `AttributeError: type object 'LeRobotDataset' has no attribute 'create'` or missing v3 directory layout.

### Pitfall 7: Forgetting H.264 Decoder in Python Reader
**What goes wrong:** The pybind11 reader stores raw H.264 NAL bytes per frame (from FrameBlockHeader's `rgb_compressed_size`), but can't decode them without a stateful H.264 decoder context.
**Why it happens:** H.264 is a video codec with inter-frame dependencies (P-frames reference previous frames). Each frame's bytes cannot be decoded in isolation — the decoder must process frames in order from a keyframe.
**How to avoid:** The pybind11 reader must maintain an `AVCodecContext` decoder for the lifetime of the file iteration, and feed each frame's bytes to `avcodec_send_packet` / `avcodec_receive_frame` in sequence. Initialize the decoder once when the file is opened.
**Warning signs:** First decoded frame looks OK (keyframe), subsequent frames are corrupted.

### Pitfall 8: RLDS uint16 Depth — Channel Dimension Required
**What goes wrong:** `tfds.features.Image(shape=(480, 640), dtype=np.uint16)` errors at build time.
**Why it happens:** TFDS Image feature requires an explicit channel dimension: shape must be `(H, W, C)`.
**How to avoid:** Add channel axis before yielding: `depth_hwc = depth_hw[:, :, np.newaxis]`. Use `shape=(480, 640, 1)`.
**Warning signs:** `ValueError: Invalid shape for Image feature`.

---

## Code Examples

### Zdepth Compress + Decompress
```cpp
// Source: github.com/catid/Zdepth README + include/zdepth.hpp
#include "zdepth.hpp"

zdepth::DepthCompressor compressor;
// For D435 indoor scenes (max ~4m)
compressor.set_encode_mode(zdepth::EncodeMode::kNotQuantized4095mm);
compressor.set_gop(30);  // keyframe every 30 frames (~1s at 30fps)

std::vector<uint8_t> compressed;
const uint16_t* z16_data = /* frame.depth_data.data() cast to uint16_t* */;
zdepth::DepthResult r = compressor.Compress(640, 480, z16_data, compressed, /*keyframe=*/false);
if (r != zdepth::DepthResult::Success) { /* handle error */ }
// compressed is ready to write to FrameBlockHeader's depth block

// Decompress (for Python reader or benchmark):
zdepth::DepthCompressor decompressor;  // separate instance
int w, h;
std::vector<uint16_t> depth_out;
zdepth::DepthResult dr = decompressor.Decompress(compressed, w, h, depth_out);
```

### H.264 Encode Init + Frame Encode
```cpp
// Source: FFmpeg encode_video.c official example (ffmpeg.org/doxygen/trunk/)
extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/opt.h>
#include <libavutil/imgutils.h>
#include <libswscale/swscale.h>
}

// Init:
const AVCodec* codec = avcodec_find_encoder_by_name("libx264");
AVCodecContext* c = avcodec_alloc_context3(codec);
c->width  = 640; c->height = 480;
c->time_base = {1, 30}; c->framerate = {30, 1};
c->pix_fmt  = AV_PIX_FMT_YUV420P;
c->gop_size = 30;
c->max_b_frames = 0;
av_opt_set(c->priv_data, "preset", "fast", 0);
av_opt_set(c->priv_data, "crf",    "23",   0);
avcodec_open2(c, codec, nullptr);

// Per-frame encode:
frame->pts = frame_number++;
// [fill yuv_frame_ via sws_scale from RGB24]
avcodec_send_frame(c, frame);
AVPacket* pkt = av_packet_alloc();
while (avcodec_receive_packet(c, pkt) == 0) {
    // pkt->data[0..pkt->size] = H.264 NAL units for this frame
    av_packet_unref(pkt);
}
```

### RLDS Export Skeleton
```python
# Source: kpertsch/rlds_dataset_builder template
import tensorflow_datasets as tfds
import numpy as np
import sys
sys.path.insert(0, "/path/to/build")  # or set PYTHONPATH
import egorec_reader

class EgoRecDataset(tfds.core.GeneratorBasedBuilder):
    VERSION = tfds.core.Version('1.0.0')

    def _info(self):
        return tfds.core.DatasetInfo(
            builder=self,
            features=tfds.features.FeaturesDict({
                'steps': tfds.features.Dataset({
                    'observation': tfds.features.FeaturesDict({
                        'image': tfds.features.Image(
                            shape=(480, 640, 3), dtype=np.uint8,
                            encoding_format='jpeg'),
                        'depth': tfds.features.Image(
                            shape=(480, 640, 1), dtype=np.uint16,
                            encoding_format='png'),
                    }),
                    'timestamp':   tfds.features.Scalar(dtype=np.float64),
                    'is_first':    tfds.features.Scalar(dtype=np.bool_),
                    'is_last':     tfds.features.Scalar(dtype=np.bool_),
                    'is_terminal': tfds.features.Scalar(dtype=np.bool_),
                }),
                'episode_metadata': tfds.features.FeaturesDict({
                    'file_path':    tfds.features.Text(),
                    'session_name': tfds.features.Text(),
                }),
            }),
        )

    def _generate_examples(self, paths):
        for path in paths:
            f = egorec_reader.EgorecFile(str(path))
            header = f.header()
            all_frames = list(f.frames())
            n = len(all_frames)
            steps = [
                {
                    'observation': {
                        'image': frame['rgb'],
                        'depth': frame['depth'][:, :, np.newaxis],  # add channel dim
                    },
                    'timestamp':   frame['timestamp_relative_s'],
                    'is_first':    i == 0,
                    'is_last':     i == n - 1,
                    'is_terminal': i == n - 1,
                }
                for i, frame in enumerate(all_frames)
            ]
            yield str(path), {
                'steps': steps,
                'episode_metadata': {
                    'file_path':    str(path),
                    'session_name': header['session_name'],
                },
            }
```

### tqdm Progress Bar for Export CLI
```python
# Source: tqdm.github.io docs
from tqdm import tqdm

def export_with_progress(reader, quiet=False):
    all_frames = list(reader.frames())  # or use a generator with known total
    total_bytes = sum(f['_raw_size'] for f in all_frames)  # if tracked

    bar = tqdm(
        all_frames,
        total=len(all_frames),
        unit='frame',
        desc='Exporting',
        disable=quiet,
    )
    bytes_written = 0
    for frame in bar:
        process_frame(frame)
        bytes_written += frame.get('_raw_size', 0)
        bar.set_postfix({'MB/s': f'{bytes_written/1e6/bar.format_dict["elapsed"]:.1f}'})
```

### LeRobot v3 Export Skeleton
```python
# Source: huggingface/lerobot port_droid.py example
from lerobot.datasets.lerobot_dataset import LeRobotDataset
import numpy as np
import egorec_reader

def export_lerobot(egorec_paths, repo_id, root="./lerobot_output"):
    features = {
        "observation.images.rgb": {
            "dtype": "video",
            "shape": (480, 640, 3),
            "names": ["height", "width", "channel"],
        },
        "observation.depth_mm": {
            "dtype": "float32",
            "shape": (480, 640),
            "names": ["height", "width"],
        },
        "timestamp": {"dtype": "float64", "shape": (1,), "names": None},
        "task_index": {"dtype": "int64",   "shape": (1,), "names": None},
    }
    dataset = LeRobotDataset.create(
        repo_id=repo_id, fps=30, features=features, robot_type="realsense_d435",
        root=root,
    )
    for path in egorec_paths:
        reader = egorec_reader.EgorecFile(str(path))
        for frame in reader.frames():
            dataset.add_frame({
                "observation.images.rgb":  frame["rgb"],
                "observation.depth_mm":    frame["depth"].astype(np.float32),
                "timestamp":               np.array([frame["timestamp_relative_s"]]),
                "task_index":              np.array([0], dtype=np.int64),
            })
        dataset.save_episode()
    dataset.finalize()   # MUST be called before any read/push
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Per-frame JPEG for video | H.264 inter-frame video codec | Phase 3 | ~10-15x vs JPEG for video; enables temporal compression |
| Raw ZSTD on depth pixels | Zdepth block prediction + ZSTD | Phase 3 | ~8-12x for depth specifically vs ~2-3x for ZSTD alone |
| Episode-per-file (LeRobot v2) | Multi-episode Parquet/MP4 files (v3) | lerobot 0.4.x | Scales to millions of episodes; faster Hub initialization |
| `avcodec_encode_video2` (FFmpeg < 3.1) | `avcodec_send_frame / receive_packet` | FFmpeg 3.1 (2016) | Old API deprecated; new API handles buffered frames correctly |
| RLDS Apache Beam default | Single-threaded default, Beam optional | Still current | Use single-threaded for small datasets; add `--runner DirectRunner` for Beam |

**Deprecated/outdated:**
- `avcodec_encode_video2()`: Removed in FFmpeg 5.x — use send/receive API only.
- LeRobot `meta/episodes.jsonl` (v2 format): Replaced by chunked Parquet in v3. Do not reference any v2 structure.

---

## Open Questions

1. **Zdepth zstd ALIAS target compatibility**
   - What we know: Zdepth CMakeLists checks `if (NOT TARGET zstd)` and will skip its bundled zstd if a target named `zstd` exists.
   - What's unclear: Whether `add_library(zstd ALIAS PkgConfig::ZSTD)` works when `PkgConfig::ZSTD` is an IMPORTED target in all CMake 3.16+ versions on the dev machine.
   - Recommendation: Test immediately with a minimal CMakeLists. If ALIAS fails, switch to vendoring Zdepth's src/ directory and defining a `zstd` interface target manually.

2. **LeRobot v3 API stability (lerobot 0.4.x)**
   - What we know: `LeRobotDataset.create()`, `add_frame()`, `save_episode()`, `finalize()` pattern is confirmed from the port_droid.py example. The API changed between 0.3.x and 0.4.x.
   - What's unclear: Whether `"dtype": "video"` features in the features dict work without additional configuration (e.g., a separate video encoder path). Whether `root=` parameter is correct spelling vs `local_dir=`.
   - Recommendation: Spike the LeRobot create + finalize path with a 10-frame synthetic dataset before writing the full export tool. Pin to a specific commit SHA.

3. **H.264 Annex B vs MP4 container for raw storage**
   - What we know: The .egorec file stores raw compressed bytes per frame (no container). For H.264, this means raw Annex B NAL units (with start codes 0x00 0x00 0x00 0x01).
   - What's unclear: Whether libavcodec produces Annex B by default when using raw H.264 codec (without libavformat muxer), or whether it produces AVCC format (length-prefixed). The decoder in the pybind11 reader must know which format to expect.
   - Recommendation: Set `c->flags |= AV_CODEC_FLAG_GLOBAL_HEADER` to control this. Spike with a test file and hexdump the first 8 bytes to confirm start codes.

4. **Zdepth EncodeMode for D435 exact range**
   - What we know: D435 maximum range is ~10m (10000mm). `kNotQuantized8191mm` mode covers 0-8191mm without quantization loss.
   - What's unclear: Whether scenes with targets >8.191m cause clipping or wrapping in `kNotQuantized8191mm` mode vs `kAzureKinectQuantized`.
   - Recommendation: Run Zdepth benchmark on a captured D435 file with depth values from 0-10000mm and verify lossless roundtrip. Use `kNotQuantized8191mm` unless benchmark shows >10m values are common.

---

## Sources

### Primary (HIGH confidence)
- `catid/Zdepth` GitHub — API signatures from `include/zdepth.hpp` (fetched directly), CMakeLists.txt structure confirmed
- FFmpeg doxygen `encode_video.c` official example — `avcodec_send_frame` / `avcodec_receive_packet` pattern
- LeRobot docs `lerobot-dataset-v3` (huggingface.co) — directory layout, Parquet/MP4 structure, `finalize()` requirement
- LeRobot `port_droid.py` example — `LeRobotDataset.create()` + `add_frame()` + `save_episode()` pattern
- TensorFlow Datasets ManiSkill RLDS catalog — `tfds.features.Image(dtype=uint16, encoding='png')` confirmed for depth
- pybind11 docs (pybind11.readthedocs.io) — CMake integration, `pybind11_add_module`, Python.h detection
- System package versions confirmed: libavcodec-dev 6.1.1, libswscale-dev 7.5.100, libx264-dev 0.164

### Secondary (MEDIUM confidence)
- nanobind docs (nanobind.readthedocs.io) — ndarray ownership, nb::numpy pattern; not used (pybind11 locked)
- kpertsch/rlds_dataset_builder — `_generate_examples()` yield pattern, features dict structure
- LeRobot `using_dataset_tools` doc — confirmed `finalize()` is mandatory, dataset tools API
- tqdm.github.io — `unit='frame'`, `set_postfix()` for MB/s display

### Tertiary (LOW confidence — validate before use)
- Zdepth encode mode for D435 range: inference from EncodeMode enum values vs D435 specs; needs runtime benchmark
- H.264 Annex B vs AVCC output from raw encoder: research confirms this is codec-context-flag-dependent, needs spike
- LeRobot 0.4.x `root=` parameter spelling in `create()`: inferred from examples, not from official API docs

---

## Metadata

**Confidence breakdown:**
- Standard stack (C++ side): HIGH — all libraries verified present on system (FFmpeg 6.1.1, libx264, libzstd, python3-dev)
- Standard stack (Python side): MEDIUM — lerobot v3 requires venv + git install; tensorflow-datasets 4.9.9 confirmed available
- Zdepth integration: MEDIUM — CMakeLists structure verified; zstd alias approach needs runtime test
- H.264 encoder pattern: HIGH — from official FFmpeg examples
- RLDS schema: MEDIUM — verified pattern from ManiSkill catalog; uint16 depth with PNG encoding confirmed
- LeRobot v3 API: MEDIUM — confirmed from port_droid.py; API may shift in 0.4.x pre-release
- Pitfalls: HIGH for known FFmpeg/Zdepth gotchas; MEDIUM for LeRobot API details

**Research date:** 2026-02-19
**Valid until:** 2026-03-19 (LeRobot v3 API: 2026-03-05 — fast-moving, re-verify before planning)
