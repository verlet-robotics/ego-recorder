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
./setup.sh          # interactive -- prompts for build options
./setup.sh --all    # full build (GUI + Python + tests)
./setup.sh --headless  # headless only (no GUI)
```

The setup script installs all system dependencies (including the Intel RealSense SDK), builds the project, and optionally installs Python export tools and the systemd service. Run `./setup.sh --help` for all options.

### Manual setup

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
pip install tensorflow-datasets numpy tqdm
python python/export_rlds.py recording.egorec -o ./rlds_output
```

Or via the binary:
```bash
./build/ego-recorder export rlds recording.egorec -o ./rlds_output
```

Options: `--quiet` to suppress progress bar. Accepts multiple files for batch export.

### Export to LeRobot v3

```bash
pip install lerobot numpy tqdm
python python/export_lerobot.py recording.egorec -o ./lerobot_output
```

Or via the binary:
```bash
./build/ego-recorder export lerobot recording.egorec -o ./lerobot_output
```

Options: `--separate` to create one dataset per file (default merges all into one). `--quiet` to suppress progress bar.

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

## Headless / systemd deployment

```bash
sudo deploy/install.sh
sudo systemctl start ego-recorder
```

This installs the binary, config, systemd unit, udev rules, and lid-close prevention. See `deploy/install.sh` for details.

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
