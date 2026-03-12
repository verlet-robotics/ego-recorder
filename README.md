# ego-recorder

Record synchronized RGB + depth video from Intel RealSense D435/D435i cameras to `.egorec` files. Optimized for ML training data collection at 1280x720 30fps.

## Features

- **30fps synchronized RGB + depth** at 1280x720
- **H.264 RGB + Zdepth lossless depth** compression (~13-19x vs raw)
- **GUI mode** with live preview, recording controls, and stats overlay
- **Headless mode** for unattended recording via systemd
- **Dataset viewer** (Tauri desktop app) — scrubbable playback, activity analysis, one-click prune/splice, curation pipeline
- **Export to RLDS** (TFRecord) and **LeRobot v3** formats for ML training
- Camera disconnect/reconnect recovery
- Per-frame IMU data (D435i)

## Which Setup Script?

Choose based on your needs:

| Need | Script | Time | Includes |
|------|--------|------|----------|
| Recording + dataset viewer | `./scripts/setup-station.sh` | 3-5 min | `ego-recorder`, `ego-qc`, `viewer-app` (Bun + Tauri) |
| Recorder only (no viewer) | `./scripts/setup-station.sh --no-viewer` | 2-3 min | `ego-recorder`, `ego-qc` |
| Headless (no GUI, no viewer) | `./scripts/setup-station.sh --headless` | 2-3 min | `ego-recorder`, `ego-qc` |
| ML export (RLDS/LeRobot) | `./scripts/setup.sh` | 5+ min | Above + Python + export tools |
| Full pipeline + cloud upload | `./scripts/setup-pipeline.sh` | (see DEPLOYMENT.md) | Systemd services + uploader + R2 setup |

## Quick start

**Before you start — verify your camera:**

```bash
lsusb | grep 8086
# Should show:
#   D435  = 8086:0b07
#   D435i = 8086:0b3a
```

If not detected, try a different USB 3.0 port or cable.

**Then install, record, and review:**

```bash
./scripts/setup-station.sh     # install deps, build recorder + ego-qc + viewer-app
./scripts/setup-recordings.sh  # create dataset directory
./scripts/record.sh            # start recording
./scripts/viewer.sh            # review recordings (scrubbable playback, QC, prune/splice)
```

For headless-only machines (no display): `./scripts/setup-station.sh --headless`

**If you need ML export** (RLDS/LeRobot), use `./scripts/setup.sh` instead of `setup-station.sh`. Run `./scripts/setup.sh --help` for all options or `./scripts/setup.sh --interactive` to choose components individually.

**To verify setup succeeded:**

```bash
./build/ego-recorder --version
./rust/target/release/ego-qc --version
# Viewer (if built): ./scripts/viewer.sh ./datasets/<name>
```

### Manual setup

<details>
<summary>Click to expand manual steps (not needed if you used setup.sh)</summary>

#### Dependencies

```bash
# Ubuntu 22.04 / 24.04 — recorder + ego-qc
sudo apt install cmake g++ pkg-config git curl wget file unzip \
    libzstd-dev libturbojpeg0-dev libclang-dev \
    libavcodec-dev libavutil-dev libswscale-dev libavformat-dev libavdevice-dev libavfilter-dev libswresample-dev \
    libglfw3-dev libopengl-dev \
    libssl-dev libsystemd-dev espeak-ng

# For viewer-app (Tauri desktop): add
sudo apt install libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev
# Plus Bun: curl -fsSL https://bun.sh/install | bash
```

Intel RealSense SDK -- install via **one** of:

```bash
# Option A: Intel apt repo (standalone)
sudo mkdir -p /etc/apt/keyrings
curl -sSf https://librealsense.intel.com/Debian/librealsense.pgp \
    | sudo tee /etc/apt/keyrings/librealsense.pgp > /dev/null
echo "deb [signed-by=/etc/apt/keyrings/librealsense.pgp] \
    https://librealsense.intel.com/Debian/apt-repo $(lsb_release -cs) main" \
    | sudo tee /etc/apt/sources.list.d/librealsense.list
sudo apt update && sudo apt install librealsense2-dev librealsense2-utils

# Option B: ROS 2 Jazzy (if you already have ROS 2 installed)
sudo apt install ros-jazzy-librealsense2
```

#### Build

```bash
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel
```

If using ROS 2 Jazzy for librealsense2, add the prefix path:
```bash
cmake -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH="/opt/ros/jazzy"
```

Build options:
- `-DWITH_GUI=OFF` -- headless only (no ImGui/GLFW/OpenGL dependency)
- `-DWITH_PYTHON=OFF` -- skip Python extension module (no pybind11)
- `-DBUILD_TESTS=OFF` -- skip unit tests

</details>

### Record

**Interactive (recommended):**

```bash
./scripts/record.sh            # interactive: pick dataset, mode, go
./scripts/record.sh pick       # pre-select dataset
```

**Direct CLI:**

```bash
# GUI mode (saves to ./datasets/pick/)
./build/ego-recorder -o ./datasets/pick

# Headless mode
./build/ego-recorder --headless -o ./datasets/pick -d 300
```

Episodes are auto-named `pick_000.egorec`, `pick_001.egorec`, etc. Use `-s name` to override.

GUI controls: **Space** start/stop, **Escape** quit, **V** cycle view.

### Inspect a recording

```bash
./build/ego-recorder info recording.egorec
```

Shows format version, codecs, frame count, duration, resolution, and camera intrinsics.

### Export

> **Note:** Export requires `./scripts/setup.sh` (full install). If you only ran `setup-station.sh`, run `setup.sh` now.

```bash
# RLDS (TFRecord)
./build/ego-recorder export rlds recording.egorec -o ./rlds_output

# LeRobot v3
./build/ego-recorder export lerobot recording.egorec -o ./lerobot_output

# Export entire dataset (preserves manifest metadata)
./build/ego-recorder export rlds ./datasets/pick -o ./rlds_output
```

## Dataset management

Organize multiple recordings into a dataset with metadata for structured ML export.

### Initialize a dataset

Use the interactive setup script (recommended):

```bash
./scripts/setup-recordings.sh
```

This creates `./datasets/<name>/` with auto-timestamped subdirectories: `2026/MM/DD/`.

Or use the CLI directly:

```bash
./build/ego-recorder dataset init -o ./my_dataset --name "kitchen-pick" \
  --description "Picking objects from a kitchen shelf" \
  --tags "manipulation,kitchen,pick-and-place"
```

Creates a `dataset.json` manifest in the directory. Use `--force` to overwrite an existing one.

### Record into a dataset

Record directly into the dataset directory -- episodes are auto-timestamped and auto-registered:

```bash
./build/ego-recorder --headless -o ./my_dataset -d 30
# creates: my_dataset/2026/03/09/my_dataset_000.egorec
```

Episodes are auto-organized by date in `YYYY/MM/DD/` subdirectories. Each episode's metadata (session name, timestamp, duration, frame count) is automatically extracted from the `.egorec` file header and footer and registered in `dataset.json`.

### Add existing recordings

```bash
./build/ego-recorder dataset add ./my_dataset recording1.egorec recording2.egorec
```

Duplicate filenames are silently skipped (idempotent).

### Inspect a dataset

```bash
./build/ego-recorder dataset info ./my_dataset
```

Shows dataset name, description, tags, episode count, total duration, and total frames.

### Remove an episode

```bash
./build/ego-recorder dataset remove ./my_dataset 2026/03/09/recording_000.egorec
```

### Export a dataset

Pass the dataset directory (instead of individual files) to preserve manifest metadata in the export:

```bash
# RLDS
./build/ego-recorder export rlds ./my_dataset -o ./rlds_output

# LeRobot v3
./build/ego-recorder export lerobot ./my_dataset -o ./lerobot_output
```

> **Note:** Export requires `./scripts/setup.sh`. If you only ran `setup-station.sh`, run it now.

The exporter reads `dataset.json`, resolves all episode paths, and passes the dataset name, description, and tags through to the exported format.

### Manifest format

The `dataset.json` manifest:

```json
{
  "version": 1,
  "name": "kitchen-pick",
  "description": "Picking objects from a kitchen shelf",
  "tags": ["manipulation", "kitchen", "pick-and-place"],
  "created": "2026-03-08T12:00:00Z",
  "episodes": [
    {
      "filename": "2026/03/09/kitchen-pick_000.egorec",
      "session_name": "kitchen-pick_000",
      "recorded_at": "2026-03-08T12:00:00Z",
      "duration_s": 30.5,
      "frames": 915
    }
  ]
}
```

## Benchmark datasets

### EgoPAT3D sample

Download sample episodes from the [EgoPAT3D](https://ai4ce.github.io/EgoPAT3D/) egocentric manipulation dataset and convert them to `.egorec` format for benchmarking the activity detector.

**Prerequisites:**

1. Request access to [EgoPAT3Dv2 on HuggingFace](https://huggingface.co/datasets/qianlima/EgoPAT3Dv2)
2. Authenticate — one of:
   ```bash
   # Option A: huggingface-cli (writes ~/.huggingface/token)
   pip install huggingface_hub && huggingface-cli login

   # Option B: environment variable
   export HF_TOKEN=hf_your_token_here

   # Option C: pass directly
   python scripts/download_egopat3d.py --token hf_your_token_here
   ```
3. (Optional) Build ego-convert for `.egorec` import/export: `cd rust && cargo build -p ego-convert --release`

**Download and convert:**

```bash
# Full pipeline: download 3 episodes (30s each), convert to .egorec, validate + analyze
python scripts/download_egopat3d.py

# Fewer frames (faster download, ~7s per episode)
python scripts/download_egopat3d.py --frames 200

# Download only (no .egorec conversion)
python scripts/download_egopat3d.py --skip-convert

# Custom output directory
python scripts/download_egopat3d.py --output /tmp/egopat3d
```

The script downloads 3 episodes from different scenes (kitchen counter, drawer, desk), extracts `rgb_video.mp4` + 16-bit depth PNGs via HTTP range requests (~1-2 GB per episode), converts each to `.egorec`, writes a `dataset.json` manifest, then validates and analyzes the result.

Output structure:
```
datasets/egopat3d/
  kitchenCounter_3/       # raw extracted data
    rgb_video.mp4
    d2rgb/1.png ... 900.png
  drawer_1/
  desk_2/
  egorec/                 # converted dataset
    kitchenCounter_3.egorec
    drawer_1.egorec
    desk_2.egorec
    dataset.json
```

**Permissions:** On stations running the ego-recorder systemd service, the `datasets/` directory is owned by the `ego-recorder` system user. The script will automatically request `sudo` to create the directory and grant your user write access via POSIX ACLs (same mechanism as `setup-recordings.sh`). If `sudo` is not available, run manually:

```bash
sudo mkdir -p datasets/egopat3d
sudo setfacl -R -m u:$USER:rwx datasets/egopat3d
sudo setfacl -R -d -m u:$USER:rwx datasets/egopat3d
```

**Importing custom video + depth:** The `ego-convert import` command works with any MP4 + depth PNG directory:

```bash
ego-convert import \
  --video path/to/rgb.mp4 \
  --depth-dir path/to/depth_pngs/ \
  --output output.egorec \
  --width 1280 --height 720 --fps 30 \
  --session-name my_recording
```

Depth PNGs must be 1-indexed (`1.png`, `2.png`, ...) 16-bit grayscale. The converter handles resolution mismatches via nearest-neighbor scaling.

## Configuration

Pass `--config path/to/config.toml` or use CLI flags. CLI flags override config file values.

| Setting | CLI flag | Default | Description |
|---------|----------|---------|-------------|
| H.264 CRF | `--crf 23` | 23 | Video quality (0=lossless, 51=worst). 28-30 for smaller files |
| Output dir | `-o ./dir` | `.` | Where .egorec files are saved |
| Session name | `-s name` | auto | Filename (headless auto-generates as `{dataset}_{NNN}`) |
| Duration | `-d 300` | unlimited | Max recording seconds |
| Queue size | `--queue-size 4` | 4 | Writer queue depth (2-16) |
| Warmup | `--warmup 30` | 30 | Frames to skip for auto-exposure |

See `deploy/config.toml.example` for the full TOML config reference.

## Next steps

- **Want to review recordings before uploading?** See viewer setup in [DEPLOYMENT.md](DEPLOYMENT.md#viewer)
- **Want automatic cloud sync?** See [DEPLOYMENT.md](DEPLOYMENT.md#cloud-upload-r2-sync). All setup scripts can optionally configure R2 credentials and auto-detect your facility server.
- **Want QC automation (prune idle episodes)?** See [DEPLOYMENT.md](DEPLOYMENT.md#ego-qc-reference)
- **Deploying to production?** See [DEPLOYMENT.md](DEPLOYMENT.md) for full systemd service setup

## File format

`.egorec` v2 binary format:
- **Header**: magic bytes, format version, resolution, codec IDs, camera intrinsics/extrinsics
- **Frame blocks**: sequential H.264 NAL units (RGB) + Zdepth-compressed depth (Z16) + optional IMU
- **Index table**: byte offsets for random access to any frame
- **Footer**: total frame count, duration, index offset

Depth compression is **lossless** (bit-exact round-trip). RGB uses H.264 CRF encoding (lossy, visually near-lossless at CRF 23).

## Disk usage

At default settings (1280x720, CRF 23):

| Duration | Size |
|----------|------|
| 1 minute | ~435 MB |
| 1 hour | ~26 GB |
| 8 hours | ~208 GB |

Recording stops automatically when free space drops below `disk_min_mb` (default: 1000 MB).

### CRF tuning

| CRF | Quality | Use case |
|-----|---------|----------|
| 18 | Visually lossless | Archival |
| 23 | Near-lossless | ML training (default) |
| 28 | Good | Storage-constrained |

Depth (Zdepth) is always lossless.

## License

MIT
