# Phase 3: Optimized Compression + Export Tools - Context

**Gathered:** 2026-02-19
**Status:** Ready for planning

<domain>
## Phase Boundary

Upgrade compression pipeline to Zdepth-style depth + H.264 RGB encoding (~14-19x compression). Build RLDS and LeRobot v3 export tools so recorded data is usable for ML training. Container format v2 with extensible codec IDs.

</domain>

<decisions>
## Implementation Decisions

### Export tool invocation
- Subcommand pattern: `ego-recorder export rlds`, `ego-recorder export lerobot`
- Batch processing supported: multiple .egorec paths or glob patterns, processed sequentially
- Output defaults to same directory as input file (e.g., `file.egorec` → `file_rlds/`), `--output` flag to override
- Progress bar with frame X/Y, ETA, throughput (MB/s); `--quiet` flag for silent mode
- `ego-recorder info file.egorec` subcommand to inspect file metadata (format version, codecs, frame count, duration, resolution, intrinsics)

### Export language & integration
- Export tools written in Python (native tensorflow_datasets + huggingface_hub ecosystem)
- C++ reader exposed as Python extension module via pybind11
- Python scripts in the repo import the C++ extension for reading .egorec frames

### Dataset mapping
- One .egorec file = one RLDS episode (direct 1:1 mapping)
- Dataset named from session name embedded in the .egorec file header
- Batch export creates one dataset with multiple episodes (from session name)
- LeRobot: multiple recordings merge into one dataset by default; `--separate` flag to split into individual datasets

### Export content & quality
- Depth exported as raw uint16 (mm) — original D435 Z16 values preserved exactly, lossless
- Camera intrinsics and extrinsics always included in every export (essential for 3D reconstruction)
- LeRobot MP4 at CRF 23 (balanced quality, suitable for most ML training)
- Per-frame timestamps as relative offset from recording start (0.0, 0.033, 0.066...), not absolute epoch

### V1 file compatibility
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

</decisions>

<specifics>
## Specific Ideas

- Export tools should feel like standard ML ecosystem tooling (tfds conventions for RLDS, HuggingFace conventions for LeRobot)
- Subcommand pattern chosen to keep a single `ego-recorder` binary/entry point
- Python export scripts live in the repo alongside the C++ code (not a separate package)
- Info subcommand is C++ (reads binary header directly, no Python dependency)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 03-optimized-compression-export-tools*
*Context gathered: 2026-02-19*
