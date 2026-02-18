# Depth Data Compression and Storage Formats

**Project:** RealSense Ego Recorder
**Researched:** 2026-02-19
**Overall confidence:** HIGH (multiple verified sources, well-established algorithms)

---

## Executive Summary

Depth compression is a solved problem with excellent options, but no single dominant standard. The best approach for this project is a **hybrid strategy**: use ZSTD-compressed depth frames with delta encoding inside a custom lightweight binary container, alongside H.264-encoded RGB. This achieves 8-12x lossless compression on depth (vs raw), real-time encoding at 30fps on laptop hardware, and produces files roughly **10-15x smaller than ROS bags** while remaining trivially convertible to RLDS/LeRobot formats that VLM training pipelines consume.

The key insight from the research: **lossless depth compression at 8-10x is achievable and fast enough for real-time**. Going lossy (via video codecs or hue-colorization) can push to 20-80x but introduces artifacts near depth edges that may harm downstream VLM training on manipulation tasks where precise object boundaries matter.

**Recommendation:** Lossless Zdepth-style compression (quantization + prediction + ZSTD) for depth, H.264/H.265 for RGB, in a custom MKV or flat binary container with per-frame index for random access.

---

## 1. Raw Data Budget

At 640x480 @ 30fps with RealSense D435:

| Stream | Per Frame | Per Second | Per Minute | Per Hour |
|--------|-----------|------------|------------|----------|
| Depth (Z16, 16-bit) | 614.4 KB | 18.0 MB/s | 1.08 GB | 64.8 GB |
| RGB (8-bit, 3ch) | 921.6 KB | 27.0 MB/s | 1.62 GB | 97.2 GB |
| IMU (accel+gyro) | ~48 bytes | ~9.6 KB/s | ~0.58 MB | ~34.6 MB |
| **Total raw** | **1.54 MB** | **45.0 MB/s** | **2.7 GB** | **162 GB** |

**Source:** Direct calculation from RealSense D435 specs. D435 outputs Z16 format (unsigned 16-bit) with default depth scale of 0.001 (1mm per unit), giving a theoretical range of 0-65.535m. Practical range is 0.17m to ~10m.
**Confidence:** HIGH

---

## 2. Depth Compression Methods Compared

### 2.1 Lossless Per-Frame Methods

| Method | Compression Ratio | Speed (compress) | Speed (decompress) | Complexity | Notes |
|--------|-------------------|-------------------|---------------------|------------|-------|
| PNG 16-bit gray | 1.8:1 | ~15ms/frame | ~8ms/frame | Low | Universally supported but poor ratio |
| LZ4 on raw Z16 | 2-3:1 | <1ms/frame | <0.5ms/frame | Low | Blazing fast, poor ratio |
| ZSTD on raw Z16 | 3-4:1 | ~1ms/frame | <1ms/frame | Low | Good speed/ratio balance |
| RVL (Run-length + VLC) | 4-5:1 | <1ms/frame | <0.5ms/frame | Very Low | ~100 lines of C, depth-specific |
| ZSTD on split planes | 5:1 | ~1.5ms/frame | <1ms/frame | Low | Split hi/lo bytes, compress separately |
| RVL + ZSTD | 5.5-6.3:1 | ~1.5ms/frame | <1ms/frame | Low | Combining domain-specific + general |
| Zdepth (keyframe) | 8-11:1 | ~1.7ms/frame | <1ms/frame | Medium | Block prediction + quantization + ZSTD |
| FFV1 (16-bit gray) | 2-4:1 | ~5-10ms/frame | ~3ms/frame | Medium | Lossless video codec, needs ffmpeg |
| FitDepth | ~5:1 | <1ms/frame | <0.5ms/frame | Medium | Spline-based, parallelizable |

### 2.2 Lossless Temporal Methods (Video-Style)

| Method | Compression Ratio | Speed | Complexity | Notes |
|--------|-------------------|-------|------------|-------|
| Zdepth (P-frames) | 9-12:1 | ~1.5ms/frame | Medium | Inter-frame prediction using ZSTD |
| Temporal RVL | ~20:1 | Fast | Low | Technically lossy, header-only C++ |
| FFV1 in MKV | 3-5:1 | ~5ms/frame | Medium | Standard tool, via ffmpeg/libav |

### 2.3 Lossy Methods

| Method | Compression Ratio | Quality (PSNR) | Complexity | Notes |
|--------|-------------------|-----------------|------------|-------|
| Hue Codec + WebP q50 | 44:1 | ~45 dB | Medium | Maps 16-bit to 11-bit hue, then lossy codec |
| Hue Codec + H.264 | 30-50:1 | ~40-45 dB | Medium | Standard HW-accelerated encode |
| Intel colorization + WebP | up to 80:1 | >70 dB at 40x | Medium | Intel official approach |
| Temporal RVL (aggressive) | ~20:1 | Good edges | Low | Lossy temporal smoothing |
| H.265 (16-bit mono) | 10-30:1 | Variable | High | Immature 16-bit support in x265 |

**Sources:**
- Zdepth benchmarks: [catid/Zdepth](https://github.com/catid/Zdepth) (BSD-3)
- RVL paper: [Wilson, 2017 - Fast Lossless Depth Image Compression](https://dl.acm.org/doi/10.1145/3132272.3134144)
- Temporal RVL: [Jun & Bailenson, 2020](https://github.com/hanseuljun/temporal-rvl)
- Hue Codec: [jdtremaine/hue-codec](https://github.com/jdtremaine/hue-codec)
- Intel colorization whitepaper: [Intel RealSense docs](https://dev.intelrealsense.com/docs/depth-image-compression-by-colorization-for-intel-realsense-depth-cameras)
- FFV1: [RFC 9043](https://datatracker.ietf.org/doc/html/draft-ietf-cellar-ffv1-16), supports gray16
- LZ4/ZSTD benchmarks from [librealsense issue #8117](https://github.com/IntelRealSense/librealsense/issues/8117)
**Confidence:** HIGH (benchmarks from multiple independent implementations)

---

## 3. Deep Dive: Recommended Approach

### 3.1 Zdepth-Style Compression (PRIMARY RECOMMENDATION)

The Zdepth algorithm from [catid/Zdepth](https://github.com/catid/Zdepth) is the gold standard for lossless depth compression. It was designed for Azure Kinect DK but works on any 16-bit depth data.

**How it works:**
1. **Quantization based on sensor accuracy** -- at longer distances, depth sensors have lower precision (error scales with Z^2 for stereo). Quantize to actual sensor precision at each depth.
2. **Zero-run encoding** -- large portions of depth frames are zero (no reading). Encode zero runs with ZSTD.
3. **Block-based prediction** -- 8x8 blocks with optimal predictor selection (similar to PNG prediction modes).
4. **Residual encoding** -- zig-zag encoding and 12-bit packing of prediction residuals.
5. **Context separation** -- edge residuals stored separately from surface residuals.
6. **ZSTD final pass** -- compressed with Zstandard for entropy coding.

**Benchmark results (from Zdepth README, Azure Kinect 640x576):**

| Scene | Keyframe Ratio | P-frame Ratio | Bitrate @ 30fps |
|-------|---------------|---------------|-----------------|
| Room | 8.15:1 | 9.86:1 | 4.49-5.43 Mbps |
| Ceiling | 11.17:1 | 11.90:1 | 3.72-3.96 Mbps |
| Person | 7.79:1 | 9.29:1 | 4.76-5.68 Mbps |

**Speed:** Compress in 1-2ms, decompress in 0.7-0.8ms per frame.

**For our 640x480 depth frames (614.4 KB raw):**
- Keyframe: ~75 KB (8:1)
- P-frame: ~62 KB (10:1)
- Effective sustained bitrate: ~15 Mbps for both depth + RGB combined

**Dependencies:** Modified ZSTD library (included), pure C++, BSD-3 license.

**Confidence:** HIGH -- working code, published benchmarks, widely referenced.

### 3.2 Why Not Simpler Alternatives?

**RVL alone (4-5:1):** Too low ratio. At 640x480, that is ~130 KB/frame depth. With RGB it adds up.

**ZSTD on raw (3-4:1):** Does not exploit depth-specific structure. Leaving 2-3x on the table.

**PNG (1.8:1):** Terrible for depth. PNG prediction filters are designed for natural images, not 16-bit depth with large zero regions and smooth surfaces.

**LZ4 (2-3:1):** Only justified if CPU is extremely constrained (Raspberry Pi). On laptop hardware, ZSTD is fast enough and compresses 2x better.

### 3.3 Why Not Lossy Video Codecs for Depth?

The Intel colorization whitepaper shows 80:1 is achievable with HUE encoding + lossy codecs. However:

**Problems for robotics VLM training data:**
1. **Edge artifacts** -- Lossy compression creates "flying pixels" near depth discontinuities (object edges). These are exactly where manipulation VLMs need precise depth.
2. **Quantization to ~11 bits** -- Hue encoding maps 16-bit to 1531 hue values (~11 bits). For manipulation at close range (0.2-2m), the D435 has mm precision that matters.
3. **Non-deterministic reconstruction** -- Different decoders may produce slightly different values.
4. **Chroma subsampling loss** -- H.264/H.265 with yuv420p loses chroma resolution. Even yuv444p has encoding artifacts.

**When lossy IS acceptable:**
- Navigation/mapping tasks (not close-range manipulation)
- Visualization/preview streams
- If storage is severely constrained and you can tolerate 1-2mm error at close range

**Recommendation:** Use lossless for depth (our primary product is training data -- precision matters). Use lossy H.264/H.265 for RGB (visual data tolerates lossy well; LeRobot uses AV1 at CRF 30 with no training degradation).

**Confidence:** HIGH (Intel own whitepaper documents the edge artifacts)

---

## 4. Disparity vs. Depth Storage

### 4.1 The D435 Depth Pipeline

The D435 is a stereo camera. Internally it computes disparity, then converts to depth:
```
Z = f * B / d
```
Where f = focal length (pixels), B = baseline (meters), d = disparity (pixels).

### 4.2 Compression Benefits of Disparity

| Property | Depth (Z) | Disparity (1/Z) |
|----------|-----------|-----------------|
| Distribution | Linear, wide range | More uniform, compressed range |
| Near-field precision | Many values (0-2000mm uses 2000 values) | Many values (high disparity) |
| Far-field precision | Many values wasted (2000-65535mm) | Few values (low disparity, appropriate) |
| Smoothness | Smooth surfaces = smooth gradients | Also smooth, more uniform gradients |
| Zero regions | Invalid = 0 | Invalid = 0 |
| Compressibility | Good | Slightly better for mixed-range scenes |

**Intel colorization whitepaper explicitly recommends:**
> "Using direct colorization of depth was well suited for short range spans (0.5m to 2.0m), but for ranges spanning from 0m to 65m, colorization of disparity maps was recommended instead."

### 4.3 Recommendation

**Store as depth (Z16), not disparity.** Reasons:
1. The D435 already outputs Z16 (depth in mm). Converting to disparity adds a processing step and requires storing camera intrinsics for recovery.
2. For our range (0.17m - ~3m for manipulation, up to ~6m for room scenes), the depth distribution is already reasonable.
3. Downstream consumers (RLDS, VLM pipelines) expect depth in millimeters. Disparity requires camera-specific conversion.
4. Lossless compression does not benefit significantly from the representation change -- the entropy is the same.
5. The quantization in Zdepth already adapts to sensor precision at different ranges, achieving the same effect as disparity storage.

**Exception:** If you later add support for ranges >10m (outdoor), revisit disparity storage.

**Confidence:** MEDIUM (the disparity vs depth compression advantage is well-documented for lossy schemes, but marginal for lossless)

---

## 5. Container Format Options

### 5.1 Requirements

- Multi-stream: RGB video + depth frames + IMU data + metadata
- Synchronized timestamps per frame
- Compact (no bloat like ROS bags)
- Seekable (random access to any frame)
- Streamable (write in real-time without buffering entire session)
- Simple to implement in C++

### 5.2 Options Evaluated

| Container | Multi-stream | Timestamps | Seekable | Overhead | C++ Library | Notes |
|-----------|-------------|------------|----------|----------|-------------|-------|
| **ROS bag** | Yes | Yes | Yes | HIGH (~2x bloat) | Complex | Explicitly excluded by user |
| **HDF5** | Yes | Yes | Yes | HIGH | libhdf5 | Explicitly excluded by user |
| **MKV (Matroska)** | Yes | Yes | Yes | Low | libmatroska | RFC 9559 standard, multi-track |
| **MP4** | Yes | Yes | Yes | Low | libavformat | Less flexible than MKV |
| **Custom binary** | Yes | Yes | With index | Minimal | None needed | Maximum control |
| **EBML-based custom** | Yes | Yes | Yes | Low | libebml | Binary XML, Matroska is built on this |
| **Multiple files** | Yes | Manual | Per-file | Low | None | Simplest, but fragile |

### 5.3 Recommendation: Custom Binary with MKV-style Index

**Primary approach:** A custom binary format with these properties:

```
File Layout:
  [File Header]
    - Magic bytes + version
    - Camera intrinsics (focal length, principal point, distortion, depth scale)
    - Stream descriptors (RGB codec, depth codec, IMU format)
    - Session metadata (scene label, mounting type, recording params)
  [Frame Blocks] (repeated for each timestamp)
    - Timestamp (uint64, microseconds)
    - RGB compressed frame (H.264 NAL unit or JPEG)
    - Depth compressed frame (Zdepth or ZSTD-compressed Z16)
    - IMU samples since last frame (variable count, ~7-8 at 200Hz)
  [Index Table] (written at end, or periodically)
    - Array of (timestamp, file_offset) for seeking
  [Footer]
    - Index table offset
    - Frame count
    - Duration
```

**Why custom over MKV:**
1. **Simpler** -- MKV parsing is complex (EBML nested elements). Custom format needs ~200 lines of C++.
2. **No dependency** -- No libmatroska/libebml required.
3. **Optimized** -- Can interleave depth+RGB+IMU per-timestamp naturally.
4. **Small overhead** -- Header is a few hundred bytes. Per-frame overhead is ~20 bytes (timestamp + sizes).

**Why custom over multiple files:**
1. One file per session is easier to manage, transfer, and catalog.
2. Synchronization is embedded, not inferred from filenames.
3. Atomic -- no partial sessions from crashed writes (write index at end).

**Alternative approach (MKV):**
If downstream tooling compatibility matters more than simplicity, use MKV with:
- Track 1: H.264 RGB video
- Track 2: FFV1 or custom codec ID for compressed depth
- Track 3: Subtitle track for IMU data (JSON per sample)
- Attachment: Camera intrinsics as JSON

MKV adds ~50 KB overhead per file and requires libmatroska, but gives you ffmpeg/VLC compatibility for the RGB stream.

**Confidence:** MEDIUM (no standard exists; this is an engineering judgment call)

---

## 6. What the VLM Training Ecosystem Actually Consumes

### 6.1 RLDS (Reinforcement Learning Datasets)

The de facto standard for robotics VLA training (used by OpenVLA, Octo, Open X-Embodiment).

**Storage structure:**
- Episodes stored as TFRecord files (Tensorflow protocol buffers)
- Observations include: `image_primary` (RGB, uint8), `depth_primary` (uint16 or float32)
- Actions, proprioceptive state as tensors

**Depth format in RLDS:** Raw uint16 tensors, millimeter values. No compression beyond TFRecord built-in gzip.

**Implication:** Our recorder does NOT need to output RLDS directly. A separate conversion tool can read our compressed format and produce RLDS. The recording format should optimize for compact storage and fast capture; the conversion tool handles format translation.

### 6.2 LeRobot (Hugging Face)

Emerging standard, gaining traction rapidly.

**Storage structure:**
- MP4 videos (AV1 codec, CRF 30, yuv420p, GOP=2) for RGB
- Parquet files for state/action data
- **Depth maps: NOT yet supported** (explicitly listed as future work)

**Depth format:** When included in source datasets, stored as unencoded images. LeRobot blog states: "we did not look into video encoding with depth maps."

**Implication:** We are ahead of the curve. Our depth compression solution could become a reference implementation if LeRobot adds depth support.

### 6.3 Robo-DM (Berkeley, 2025)

Newest approach, published May 2025. Uses EBML container (same as MKV) with video compression.

**Results:** 70x compression over RLDS (lossy), 3.5x (lossless). 50x faster data loading than LeRobot.

**Approach:** Video encoding for visual data, EBML container for multi-stream.

**Implication:** Validates the approach of video-compressed RGB + custom container. Their EBML choice is interesting but their implementation is complex (cloud-oriented).

**Sources:**
- [OpenVLA paper](https://arxiv.org/abs/2406.09246) -- uses RLDS format
- [LeRobot video encoding blog](https://huggingface.co/blog/video-encoding) -- depth explicitly not addressed
- [Robo-DM paper](https://arxiv.org/abs/2505.15558) -- EBML container, 70x compression
- [RLDS GitHub](https://github.com/google-research/rlds)
**Confidence:** HIGH (based on published papers and official documentation)

---

## 7. Compression Ratio Summary: What is Achievable?

For a 640x480 D435 stream at 30fps:

| Approach | Depth Ratio | RGB Ratio | Total Size/Min | vs Raw (2.7 GB/min) |
|----------|-------------|-----------|-----------------|----------------------|
| Raw (no compression) | 1:1 | 1:1 | 2.7 GB | 1x |
| ROS bag (typical) | ~1.5:1 | ~1.5:1 | ~1.8 GB | 0.67x |
| PNG depth + JPEG RGB | 1.8:1 + 10:1 | - | ~500 MB | 5.4x smaller |
| ZSTD depth + H.264 RGB | 3.5:1 + 30:1 | - | ~350 MB | 7.7x smaller |
| Zdepth + H.264 RGB | 8:1 + 30:1 | - | ~190 MB | 14x smaller |
| Zdepth + H.265 RGB | 10:1 + 50:1 | - | ~140 MB | 19x smaller |
| Temporal RVL + H.264 RGB | 20:1 (lossy) + 30:1 | - | ~80 MB | 34x smaller |
| Hue codec + H.264 (both lossy) | 40:1 + 30:1 | - | ~50 MB | 54x smaller |

**Recommended target: 140-200 MB per minute** (Zdepth lossless depth + H.264/H.265 lossy RGB).

For a 1-hour recording:
- Raw: 162 GB
- Our format: **8.4-12 GB**
- ROS bag: ~108 GB

That is **~14x smaller than raw and ~10x smaller than ROS bags**.

**Confidence:** HIGH for depth ratios (from Zdepth benchmarks). MEDIUM for combined estimates (depends on scene complexity for RGB).

---

## 8. Precision Requirements for Robotics VLM Training

### 8.1 D435 Actual Precision

The D435 depth error scales quadratically with distance:
- At 0.5m: ~0.5mm RMS error
- At 1.0m: ~2mm RMS error
- At 2.0m: ~8mm RMS error
- At 4.0m: ~32mm RMS error

**Noise model coefficients** (from [Ahn & Chae, 2019](https://ieeexplore.ieee.org/document/8768489/)):
c0 = 0.001063, c1 = 0.0007278, c2 = 0.003949

### 8.2 What Precision Does VLM Training Need?

For manipulation tasks (the primary use case):
- **Close-range (0.2-1m):** mm precision matters for grasping, insertion tasks
- **Mid-range (1-3m):** cm precision sufficient for navigation, spatial reasoning
- **Far-range (>3m):** Depth is supplementary context, precision less critical

VLM training datasets (SpatialBot, RT-X) store depth as uint16 in millimeters directly. No quantization beyond the sensor native output.

### 8.3 Acceptable Compression Loss

| Method | Max Error | Acceptable For |
|--------|-----------|---------------|
| Lossless (Zdepth, RVL, ZSTD) | 0 mm | Everything -- always preferred |
| Sensor-aware quantization | 0 mm effective | Same as sensor noise floor |
| Temporal RVL (default) | ~1-2 mm | Most manipulation tasks |
| Hue encoding (11-bit) | ~32 mm at close range | NOT acceptable for manipulation |
| H.264 on raw depth | Variable, up to 10mm | Risky for close-range |

**Recommendation:** Stay lossless for depth. The compression ratio difference between lossless (8-10:1) and lossy (20-40:1) is only 2-4x, but lossless guarantees zero precision loss. For a 1-hour recording, lossless depth costs ~5 GB vs ~2.5 GB lossy. Not worth the risk.

**Confidence:** HIGH (sensor specs + VLM pipeline requirements well-documented)

---

## 9. Implementation Roadmap

### Phase 1: MVP Compression
- **Depth:** ZSTD (level 3) on raw Z16 frames. Ratio: ~3.5:1. Zero domain-specific code.
- **RGB:** JPEG at quality 90 per frame. Ratio: ~10:1. Simple, fast.
- **Container:** Custom binary with header + sequential frames + index.
- **Effort:** 1-2 days. Gets you recording immediately.

### Phase 2: Optimized Compression
- **Depth:** Port Zdepth algorithm (or simplified version: split planes + delta encoding + ZSTD). Ratio: 8-10:1.
- **RGB:** H.264 via FFmpeg/libx264 pipe or API. Ratio: 30-50:1.
- **Container:** Same custom binary, upgraded codecs.
- **Effort:** 3-5 days. Major size reduction.

### Phase 3: Export/Conversion
- **RLDS converter:** Read our format, output TFRecord with raw depth + decoded RGB.
- **LeRobot converter:** Read our format, output MP4 + Parquet.
- **Effort:** 2-3 days per format.

### Phase 4 (Optional): Advanced Compression
- **Temporal depth compression:** Zdepth P-frames for 10-12:1.
- **AV1 for RGB:** Better ratio than H.264 at same quality.
- **Effort:** 3-5 days. Diminishing returns.

---

## 10. Key Libraries and Dependencies

| Library | Purpose | License | Size Impact |
|---------|---------|---------|-------------|
| [facebook/zstd](https://github.com/facebook/zstd) | Depth frame compression | BSD-3 | ~300 KB |
| [catid/Zdepth](https://github.com/catid/Zdepth) | Depth-specific compression | BSD-3 | ~50 KB (includes modified ZSTD) |
| libx264 or FFmpeg | RGB video encoding | GPL-2 / LGPL-2.1 | ~2 MB |
| librealsense2 | Camera capture | Apache-2.0 | Already required |

**Minimal dependency set:** ZSTD only (for Phase 1). Add libx264/FFmpeg for Phase 2.

Note: Zdepth bundles a modified ZSTD. If using Zdepth directly, it brings its own ZSTD.

---

## 11. Emerging Standards and Future Directions

### 11.1 No Dominant Standard Exists

There is no established standard for compressed RGBD storage in robotics/CV. The field is fragmented:
- RLDS: TFRecords (uncompressed depth, gzip on container)
- LeRobot: MP4 for RGB only, depth not addressed
- Robo-DM: EBML/video compression (newest, not widely adopted)
- ROS bags: ROS-specific, bloated
- Aivero 3DQ: Proprietary, GStreamer-based
- Record3D: iOS-specific, LZFSE compression

### 11.2 Likely Evolution

Based on the trend (Robo-DM 2025, LeRobot v3 2025):
1. **Video compression for RGB** is now standard (H.264/AV1)
2. **Depth compression** is the unsolved piece -- everyone acknowledges it but no one has standardized
3. **EBML/MKV-based containers** are gaining traction (Robo-DM chose EBML)
4. **Lossless depth** preferred for training data (LeRobot explicitly avoids lossy depth)

### 11.3 Opportunity

By building a clean, efficient depth compression format with open conversion tools, this project could become a reference for RGBD dataset recording. The gap is real and widely acknowledged.

**Confidence:** MEDIUM (trend analysis, not verified standard)

---

## 12. Critical Pitfalls

### Pitfall 1: Using Video Codecs Directly on 16-bit Depth
**What goes wrong:** H.264/H.265 are designed for 8-bit YUV. They do not natively handle 16-bit single-channel well. x265 16-bit support is "not mature." Forcing 16-bit depth through video codecs introduces chroma subsampling artifacts, quantization noise, and edge blurring.
**Prevention:** Use depth-specific compression (Zdepth, RVL+ZSTD), not video codecs for depth.

### Pitfall 2: Assuming PNG is Good Enough for Depth
**What goes wrong:** PNG achieves only 1.8:1 on 16-bit depth. For 30fps capture, PNG compression alone is ~340 KB/frame, adding up to 600 MB/min for depth alone. Also, PNG encode is slow (~15ms/frame) -- may cause frame drops at 30fps with other processing.
**Prevention:** Use ZSTD (3.5:1, <1ms) or Zdepth (8:1, <2ms) instead.

### Pitfall 3: Not Implementing Seekability
**What goes wrong:** A recording format that requires reading from the beginning to reach any frame makes conversion to RLDS/LeRobot painfully slow for multi-hour recordings.
**Prevention:** Write an index table (frame offset array) at the end of the file. If the recording crashes, the index can be rebuilt by scanning frame headers.

### Pitfall 4: Buffering Frames in Memory
**What goes wrong:** At 45 MB/s raw, even a 1-second buffer is 45 MB. Compressing inline and writing to disk immediately is essential for constant memory usage.
**Prevention:** Compress-and-write pipeline: capture thread -> compress thread -> write thread. Ring buffer of 2-3 frames max.

### Pitfall 5: Choosing Lossy Depth to Save 2x More Space
**What goes wrong:** Lossy depth compression introduces systematic errors at object edges. VLM models trained on such data learn incorrect depth boundaries, potentially harming manipulation performance. The space savings (8 GB vs 5 GB per hour) is not worth the quality risk for premium training data.
**Prevention:** Stay lossless for depth. Compress RGB lossily (it is the accepted standard in the field).

### Pitfall 6: Ignoring the Conversion Step
**What goes wrong:** Recording in a maximally compressed format is useless if buyers cannot load it. RLDS and LeRobot are what training pipelines consume.
**Prevention:** Plan conversion tools from day one. The recording format optimizes for capture speed and compression; conversion tools handle translation to RLDS/LeRobot/raw frames.

---

## Sources

### Primary (HIGH confidence)
- [Zdepth GitHub - catid/Zdepth](https://github.com/catid/Zdepth) -- BSD-3 C++ depth compressor with benchmarks
- [RVL Paper - Wilson 2017](https://dl.acm.org/doi/10.1145/3132272.3134144) -- Original RVL algorithm
- [Temporal RVL - Jun & Bailenson 2020](https://github.com/hanseuljun/temporal-rvl) -- ~20x depth compression
- [Hue Codec - jdtremaine/hue-codec](https://github.com/jdtremaine/hue-codec) -- 16-bit to RGB encoding
- [Intel Depth Colorization Whitepaper](https://dev.intelrealsense.com/docs/depth-image-compression-by-colorization-for-intel-realsense-depth-cameras)
- [LeRobot Video Encoding Blog](https://huggingface.co/blog/video-encoding) -- AV1, depth not addressed
- [Zstandard - facebook/zstd](https://github.com/facebook/zstd) -- Real-time compression
- [FFV1 - RFC 9043](https://datatracker.ietf.org/doc/html/draft-ietf-cellar-ffv1-16) -- Lossless video codec
- [Matroska - RFC 9559](https://datatracker.ietf.org/doc/rfc9559/) -- Container format spec
- [RLDS - google-research/rlds](https://github.com/google-research/rlds) -- Dataset format

### Secondary (MEDIUM confidence)
- [Robo-DM Paper](https://arxiv.org/abs/2505.15558) -- EBML container, 70x compression
- [FitDepth - Springer 2023](https://link.springer.com/article/10.1186/s13640-023-00606-z) -- Spline-based depth compression
- [PDM Format](https://sr.ht/~sotirisp/pdm/) -- Portable Depth Map format spec
- [D435 Noise Model - Ahn & Chae 2019](https://ieeexplore.ieee.org/document/8768489/)
- [librealsense depth compression discussion](https://github.com/IntelRealSense/librealsense/issues/8117)
- [SpatialBot - depth in VLMs](https://arxiv.org/html/2406.13642v1)
- [Aivero 3DQ whitepaper](https://aivero.com/whitepaper/) -- Proprietary depth compression

### Tertiary (LOW confidence -- cited for completeness)
- [Adapting Standard Video Codecs for Depth Streaming - UCL](http://reality.cs.ucl.ac.uk/projects/depth-streaming/depth-streaming.pdf)
- [3D-HEVC standard](https://ieeexplore.ieee.org/document/6694184/) -- Depth-aware HEVC extension
