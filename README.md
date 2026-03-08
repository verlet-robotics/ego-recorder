# ego-recorder

Record synchronized RGB + depth video from Intel RealSense D435/D435i cameras to `.egorec` files. Optimized for ML training data collection with ~13-19x compression over raw frames.

## Features

- **30fps synchronized RGB + depth** at 640x480
- **H.264 RGB + Zdepth lossless depth** compression (~13-19x vs raw)
- **GUI mode** with live preview, recording controls, and stats overlay
- **Headless mode** for unattended recording via systemd
- **Export to RLDS** (TFRecord) and **LeRobot v3** formats for ML training
- Camera disconnect/reconnect recovery
- Per-frame IMU data (D435i)

## Quick start

### One-line setup

```bash
./setup.sh             # full install (deps + build + Python export tools)
./setup.sh --headless  # headless only (no GUI, no Python)
```

The setup script installs all system dependencies (including the Intel RealSense SDK), builds the project, and installs Python export tools. Run `./setup.sh --help` for all options or `./setup.sh --interactive` to choose components individually.

### Manual setup

<details>
<summary>Click to expand manual steps (not needed if you used setup.sh)</summary>

#### Dependencies

```bash
# Ubuntu 22.04 / 24.04
sudo apt install cmake g++ pkg-config git \
    libzstd-dev libturbojpeg0-dev \
    libavcodec-dev libavutil-dev libswscale-dev \
    libglfw3-dev libopengl-dev \
    python3-dev libsystemd-dev
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

#### Python export dependencies

```bash
pip install tensorflow-datasets numpy tqdm   # for RLDS export
pip install lerobot numpy tqdm               # for LeRobot export
```

</details>

### Record (GUI)

```bash
./build/ego-recorder -s my_session -o ./recordings
```

- **Space** -- start/stop recording
- **Escape** -- stop recording (if active) or quit
- **V** -- cycle view: RGB only / depth only / side-by-side

### Record (headless)

```bash
./build/ego-recorder --headless -o ./recordings -d 300
```

Records for 300 seconds (omit `-d` for unlimited). Session name is auto-generated from timestamp.

### Inspect a recording

```bash
./build/ego-recorder info recording.egorec
```

Shows format version, codecs, frame count, duration, resolution, and camera intrinsics.

### Export to RLDS (TFRecord)

```bash
./build/ego-recorder export rlds recording.egorec -o ./rlds_output
```

Or call the Python script directly:
```bash
python python/export_rlds.py recording.egorec -o ./rlds_output
```

Options: `--quiet` to suppress progress bar. Accepts multiple files for batch export.

### Export to LeRobot v3

```bash
./build/ego-recorder export lerobot recording.egorec -o ./lerobot_output
```

Or call the Python script directly:
```bash
python python/export_lerobot.py recording.egorec -o ./lerobot_output
```

Options: `--separate` to create one dataset per file (default merges all into one). `--quiet` to suppress progress bar.

## Dataset management

Organize multiple recordings into a dataset with metadata for structured ML export.

### Initialize a dataset

Use the interactive setup script:

```bash
./scripts/setup-recordings.sh
```

Or use the CLI directly:

```bash
./build/ego-recorder dataset init -o ./my_dataset --name "kitchen-pick" \
  --description "Picking objects from a kitchen shelf" \
  --tags "manipulation,kitchen,pick-and-place"
```

Creates a `dataset.json` manifest in the directory. Use `--force` to overwrite an existing one.

### Record into a dataset

Record directly into the dataset directory -- episodes are auto-registered into the manifest when recording stops:

```bash
# GUI mode
./build/ego-recorder -o ./my_dataset -s pick_001

# Headless mode
./build/ego-recorder --headless -o ./my_dataset -d 30
```

Each episode's metadata (session name, timestamp, duration, frame count) is automatically extracted from the `.egorec` file header and footer.

### Add existing recordings

```bash
./build/ego-recorder dataset add ./my_dataset recording1.egorec recording2.egorec
```

Duplicate filenames are silently skipped (idempotent).

### Inspect a dataset

```bash
./build/ego-recorder dataset info ./my_dataset
```

Shows dataset name, description, tags, per-episode details, and totals.

### Remove an episode

```bash
./build/ego-recorder dataset remove ./my_dataset recording1.egorec
```

### Export a dataset

Pass the dataset directory (instead of individual files) to preserve manifest metadata in the export:

```bash
# RLDS
./build/ego-recorder export rlds ./my_dataset -o ./rlds_output

# LeRobot v3
./build/ego-recorder export lerobot ./my_dataset -o ./lerobot_output
```

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
      "filename": "pick_001_20260308_120000.egorec",
      "session_name": "pick_001",
      "recorded_at": "2026-03-08T12:00:00Z",
      "duration_s": 30.5,
      "frames": 915
    }
  ]
}
```

## Configuration

Pass `--config path/to/config.toml` or use CLI flags. CLI flags override config file values.

| Setting | CLI flag | Default | Description |
|---------|----------|---------|-------------|
| H.264 CRF | `--crf 23` | 23 | Video quality (0=lossless, 51=worst). 28-30 for smaller files |
| Output dir | `-o ./dir` | `.` | Where .egorec files are saved |
| Session name | `-s name` | `capture` | Filename prefix (headless auto-generates from timestamp) |
| Duration | `-d 300` | unlimited | Max recording seconds |
| Queue size | `--queue-size 4` | 4 | Writer queue depth (2-16) |
| Warmup | `--warmup 30` | 30 | Frames to skip for auto-exposure |

See `deploy/config.toml.example` for the full TOML config reference.

## Production deployment

See [DEPLOYMENT.md](DEPLOYMENT.md) for the full production setup guide covering GUI mode, headless systemd service, udev rules, lid-close prevention, monitoring, updating, and troubleshooting.

## File format

`.egorec` v2 binary format:
- **Header**: magic bytes, format version, resolution, codec IDs, camera intrinsics/extrinsics
- **Frame blocks**: sequential H.264 NAL units (RGB) + Zdepth-compressed depth (Z16) + optional IMU
- **Index table**: byte offsets for random access to any frame
- **Footer**: total frame count, duration, index offset

Depth compression is **lossless** (bit-exact round-trip). RGB uses H.264 CRF encoding (lossy, visually near-lossless at CRF 23).

## Compression tuning

| CRF | Approx ratio | Quality | Use case |
|-----|-------------|---------|----------|
| 18 | ~10x | Visually lossless | Archival |
| 23 | ~13-15x | Near-lossless | ML training (default) |
| 28 | ~20-25x | Good | Storage-constrained |
| 33 | ~30-40x | Acceptable | Quick previews |

Depth (Zdepth) is always lossless at ~8-10:1. These ratios are combined RGB+depth.

## License

MIT
