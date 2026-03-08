# ego-recorder

Record synchronized RGB + depth video from Intel RealSense D435/D435i cameras to `.egorec` files. Optimized for ML training data collection at 1280x720 30fps.

## Features

- **30fps synchronized RGB + depth** at 1280x720
- **H.264 RGB + Zdepth lossless depth** compression (~13-19x vs raw)
- **GUI mode** with live preview, recording controls, and stats overlay
- **Headless mode** for unattended recording via systemd
- **Export to RLDS** (TFRecord) and **LeRobot v3** formats for ML training
- Camera disconnect/reconnect recovery
- Per-frame IMU data (D435i)

## Quick start

```bash
./scripts/setup.sh             # install deps, build, install binary
./scripts/setup-recordings.sh  # create recording directory + dataset
./scripts/record.sh            # choose dataset, mode, start recording
```

For headless-only machines (no display): `./scripts/setup.sh --headless`

Run `./scripts/setup.sh --help` for all options or `./scripts/setup.sh --interactive` to choose components individually.

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

</details>

### Record

```bash
./scripts/record.sh            # interactive: pick dataset, mode, go
./scripts/record.sh pick       # pre-select dataset
```

Or run directly:

```bash
# GUI mode
ego-recorder -o /var/lib/ego-recorder/pick

# Headless mode
ego-recorder --headless -o /var/lib/ego-recorder/pick -d 300
```

Episodes are auto-named `pick_000.egorec`, `pick_001.egorec`, etc. Use `-s name` to override.

GUI controls: **Space** start/stop, **Escape** quit, **V** cycle view.

### Inspect a recording

```bash
./build/ego-recorder info recording.egorec
```

Shows format version, codecs, frame count, duration, resolution, and camera intrinsics.

### Export

```bash
# RLDS (TFRecord)
ego-recorder export rlds recording.egorec -o ./rlds_output

# LeRobot v3
ego-recorder export lerobot recording.egorec -o ./lerobot_output

# Export entire dataset (preserves manifest metadata)
ego-recorder export rlds /var/lib/ego-recorder/pick -o ./rlds_output
```

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

Record directly into the dataset directory -- episodes are auto-indexed and auto-registered:

```bash
ego-recorder --headless -o ./my_dataset -d 30
# creates my_dataset/2026/03/09/my_dataset_000.egorec
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
      "filename": "2026/03/09/kitchen-pick_000.egorec",
      "session_name": "kitchen-pick_000",
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
| Session name | `-s name` | auto | Filename (headless auto-generates as `{dataset}_{NNN}`) |
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
