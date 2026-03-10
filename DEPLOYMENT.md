# Deployment Guide

## Quick Start

### 1. Install

```bash
git clone https://github.com/verlet-robotics/ego-recorder.git
cd ego-recorder
./scripts/setup-station.sh
```

This builds two tools:

| Tool | Location | Purpose |
|------|----------|---------|
| `ego-recorder` | `build/ego-recorder` | C++ RGBD capture from Intel RealSense cameras |
| `ego-qc` | `rust/target/release/ego-qc` | Rust CLI for QC, pruning, splicing, and MP4 extraction |

For headless-only machines (no display):

```bash
./scripts/setup-station.sh --headless
```

To deploy as a systemd service in one step:

```bash
./scripts/setup-station.sh --headless --with-systemd
```

> **Need RLDS/LeRobot export?** Run `./scripts/setup.sh` instead for the full install (adds `ego-convert` for format conversion, Python bindings, tests).

### 2. Set up recordings

```bash
./scripts/setup-recordings.sh
```

Creates a dataset directory at `./datasets/<name>/`, sets permissions, and initializes a `dataset.json` manifest.

### 3. Record

```bash
./scripts/record.sh
```

Prompts you to pick a dataset (or create a new one), choose headless or GUI mode, and starts recording. Episodes are saved to `./datasets/<name>/` and auto-named (`pick_000.egorec`, `pick_001.egorec`, ...).

### 4. Review in the viewer

```bash
cd viewer
bun install                                  # first time only
bun run dev -- --dir ../datasets/pick        # start on :4200
```

Opens a web UI at `http://localhost:4200` with RGB + depth video playback, activity analysis, and one-click prune/splice controls.

> If ego-qc is not on your PATH, pass it explicitly:
> `bun run dev -- --dir ../datasets/pick --qc ../rust/target/release/ego-qc`

### 5. QC and prune

```bash
# Analyze activity (read-only)
./rust/target/release/ego-qc analyze ./datasets/pick -v

# Prune idle episodes (dry run, then apply)
./rust/target/release/ego-qc prune ./datasets/pick
./rust/target/release/ego-qc prune ./datasets/pick --apply
```

> **Tip:** Add `rust/target/release/` to your PATH to use `ego-qc` directly:
> `export PATH="$PWD/rust/target/release:$PATH"`

### 6. Upload to cloud (optional)

```bash
# Interactive — pick dataset, choose options, upload
python3 python/ego_uploader.py --config deploy/upload_config.toml -i

# One-shot upload of everything pending
python3 python/ego_uploader.py --config deploy/upload_config.toml --once
```

Or set up automatic upload as a systemd service — see [Cloud upload](#cloud-upload-r2-sync) below.

> R2 credentials can be configured during any setup script (prompted at the end), or manually in `.env`. See [Upload configuration](#upload-configuration).

---

## Viewer

The viewer is a Bun web app that serves `.egorec` recordings for browser playback. It uses `ego-qc` for MP4 conversion (RGB + turbo-colormapped depth) and activity analysis.

### Prerequisites

- [Bun](https://bun.sh) runtime
- `ego-qc` binary (built by `setup-station.sh` to `rust/target/release/ego-qc`)

### Running

```bash
cd viewer
bun install                                          # first time only
bun run dev -- --dir ../datasets/pick                # start on :4200
```

The `--dir` argument points to a directory containing `.egorec` files (supports nested subdirectories). To point at all datasets at once:

```bash
bun run dev -- --dir ../datasets
```

### Options

```bash
# Use a specific ego-qc binary (default: ego-qc on PATH)
bun run dev -- --dir ../datasets/pick --qc ../rust/target/release/ego-qc
```

### Features

- **RGB playback** — streams H.264 directly from `.egorec` via remux (instant, no conversion needed)
- **Depth playback** — converts zdepth to turbo-colormapped MP4 via `ego-qc mp4` (on-demand, cached)
- **Activity analysis** — runs `ego-qc analyze` across all episodes, shows verdicts with color-coded scores
- **One-click prune** — moves low-activity episodes to `.pruned/` via `ego-qc prune`
- **One-click splice** — extracts active segments via `ego-qc splice`
- **Metadata panel** — camera intrinsics, session info, frame counts, duration

---

## Recommended Workflow

For a typical collection station running recorder, QC, and uploader:

```bash
# 1. Record a batch of episodes
./scripts/record.sh

# 2. Review in the viewer (optional — for visual inspection)
cd viewer && bun run dev -- --dir ../datasets/pick

# 3. Analyze quality
ego-qc analyze ./datasets/pick -v

# 4. Prune dead episodes
ego-qc prune ./datasets/pick          # dry run
ego-qc prune ./datasets/pick --apply   # execute

# 5. Splice long recordings into focused segments (optional)
ego-qc splice ./datasets/pick --replace-original

# 6. Upload (happens automatically if uploader service is running,
#    or trigger manually)
python3 python/ego_uploader.py --config deploy/upload_config.toml -i
```

> Steps 3-5 assume `ego-qc` is on your PATH. If not, use the full path: `./rust/target/release/ego-qc`.

---

## Reference

### Prerequisites

- Ubuntu 22.04 or 24.04
- Intel RealSense D435 or D435i
- USB 3.0 port (USB 2.0 cannot sustain 30fps)
- ~435 MB/minute of storage at default settings (720p)

### Verify the camera

```bash
# Check USB detection
lsusb | grep 8086
# D435 = 8086:0b07, D435i = 8086:0b3a

# Quick test
./build/ego-recorder -s test -o /tmp
```

If not detected: try a different USB 3.0 port or cable, check `dmesg | tail -20`.

### Recording modes

**Interactive (via script):**

```bash
./scripts/record.sh          # fully interactive
./scripts/record.sh pick     # pre-select dataset
```

**Direct CLI:**

```bash
# GUI mode
./build/ego-recorder -o ./datasets/pick

# Headless mode
./build/ego-recorder --headless -o ./datasets/pick

# With explicit session name (overrides auto-indexing)
./build/ego-recorder --headless -o ./datasets/pick -s my_session
```

GUI controls: **Space** start/stop, **Escape** quit, **V** cycle view.

### Dataset management

```bash
# Inspect dataset (episodes, totals)
./build/ego-recorder dataset info ./datasets/pick

# Extract MP4 from .egorec (RGB + depth visualization)
ego-qc mp4 ./datasets/pick/*.egorec -o ./mp4_output

# Export to RLDS (TFRecord) — requires ego-convert (full install)
ego-convert rlds ./datasets/pick/*.egorec -o ./rlds_output

# Export to LeRobot v3 — requires ego-convert (full install)
ego-convert lerobot ./datasets/pick/*.egorec -o ./lerobot_output
```

### Disk usage

| Duration | Size |
|----------|------|
| 1 minute | ~435 MB |
| 1 hour | ~26 GB |
| 8 hours | ~208 GB |

Recording stops automatically when free space drops below `disk_min_mb` (default: 1000 MB).

### CRF tuning (quality vs size)

```bash
./build/ego-recorder --crf 18 -o ./datasets/pick   # higher quality, larger files
./build/ego-recorder --crf 28 -o ./datasets/pick   # smaller files, still good for ML
```

### Systemd service (unattended headless)

For dedicated recording machines (robot-mounted laptops, etc.):

```bash
# Deploy service files
sudo bash deploy/install.sh

# Edit config
sudo nano /etc/ego-recorder/config.toml

# Start
sudo systemctl enable --now ego-recorder.service

# Monitor
journalctl -fu ego-recorder.service

# Stop
sudo systemctl stop ego-recorder.service
```

The service records to `/var/lib/ego-recorder/recordings/` and auto-recovers from camera disconnects. It also prevents lid-close suspend.

### Cloud upload (R2 sync)

The uploader runs as a background service alongside the recorder. It watches the recordings directory, detects completed `.egorec` files, and uploads them to Cloudflare R2 when connectivity is available. Files in `.pruned/` directories are automatically skipped.

#### Full pipeline setup (recorder + QC + uploader)

```bash
sudo ./scripts/setup-pipeline.sh
```

This builds the recorder and `ego-qc`, installs both to `/usr/local/bin/`, installs both systemd services, sets up Python for the uploader, and prompts for R2 credentials. Both services start on boot.

During setup you'll be prompted to configure R2 credentials. You can either paste an entire `.env` block or enter each credential individually:

```
  1) Paste a .env block (all credentials at once)
  2) Enter each credential individually
  3) Skip (configure later)
```

The paste option accepts a block like:

```env
R2_ENDPOINT=https://your-account-id.r2.cloudflarestorage.com
R2_BUCKET=your-bucket-name
R2_ACCESS_KEY_ID=your-key
R2_SECRET_ACCESS_KEY=your-secret
```

The script also auto-detects facility servers on the local network by scanning the /24 subnet for port 8100. If found, it prompts for confirmation; otherwise, you can enter the IP manually or skip.

> **Note:** `setup.sh` and `setup-station.sh` also offer an optional R2 configuration step at the end of their build process ("Configure R2 cloud upload? [y/N]"), so you can configure upload credentials from any setup entry point.

```bash
sudo ./scripts/setup-pipeline.sh --no-build      # skip build (binary already compiled)
sudo ./scripts/setup-pipeline.sh --recorder-only  # skip uploader, recorder only
```

To stop and disable:

```bash
sudo ./scripts/stop.sh
sudo ./scripts/stop.sh --purge   # also remove binaries, configs, venv
```

#### Upload configuration

The uploader reads from two files:

| File | Purpose |
|------|---------|
| `/etc/ego-recorder/upload_config.toml` | Bucket, prefix, poll interval, facility API |
| `/etc/ego-recorder/.env` | R2 credentials (see below) |

**Environment variables** (in `.env`):

| Variable | Required | Description |
|----------|----------|-------------|
| `R2_ENDPOINT` | Yes | Cloudflare R2 endpoint URL |
| `R2_BUCKET` | No | Bucket name (overrides config, default: `ego-data-verlet`) |
| `R2_ACCESS_KEY_ID` | Yes | R2 access key |
| `R2_SECRET_ACCESS_KEY` | Yes | R2 secret key |
| `FACILITY_URL` | No | Facility API URL (alternative to config file) |

> `R2_ENDPOINT_URL` and `R2_BUCKET_NAME` are also accepted as aliases during setup.

Key settings in `upload_config.toml`:

```toml
[cloud]
bucket = "ego-data-verlet"       # override via R2_BUCKET env var
prefix = "device-01/"            # optional key prefix per device

[upload]
episodes_dir = "/var/lib/ego-recorder/recordings"
poll_interval_s = 30             # how often to scan for new episodes
file_settle_s = 10               # skip files modified within this window
delete_after_upload = false      # delete local .egorec after R2-verified upload

[facility]
enabled = true                   # register episodes with facility server
url = "http://192.168.1.100:8100"
dataset_name = "kitchen-01"
```

#### Running the uploader manually

```bash
# Interactive mode — pick dataset, choose options, upload
python3 python/ego_uploader.py --config deploy/upload_config.toml -i

# Continuous mode (foreground, useful for debugging)
python3 python/ego_uploader.py --config deploy/upload_config.toml -v

# Single pass (upload everything pending, then exit)
python3 python/ego_uploader.py --config deploy/upload_config.toml --once

# Upload a specific dataset only
python3 python/ego_uploader.py --config deploy/upload_config.toml --once --dataset pick

# Upload and delete local files after R2-verified upload
python3 python/ego_uploader.py --config deploy/upload_config.toml --once --delete
```

Interactive mode (`-i`) discovers datasets in the recordings directory, shows pending episode counts and sizes, and lets you select which dataset to upload with an option to delete local files after verified upload.

#### How the uploader works

- Scans `episodes_dir` for `.egorec` files not yet in `.upload_manifest.json`
- Skips files in `.pruned/` directories and files still being written
- Checks network connectivity before each upload (3-tier: nmcli, sysfs, ip route)
- Uploads via boto3 multipart with progress logging (speed, ETA, percentage)
- SHA-256 checksums computed before upload for verification
- Retries with exponential backoff (capped at 5 minutes), resets on reconnect
- If facility mode is enabled, registers each episode with the facility API so it appears in the manager dashboard
- If `delete_after_upload` is enabled, verifies the object exists on R2 (head_object size check) before deleting the local file

#### Upload manifest

The uploader maintains `.upload_manifest.json` in the recordings directory. This tracks which files have been uploaded with their SHA-256 checksums, so uploads are idempotent across restarts.

```bash
# Check upload status
cat /var/lib/ego-recorder/recordings/.upload_manifest.json | python3 -m json.tool

# Force re-upload of a file (remove its entry from the manifest)
# then restart the uploader
sudo systemctl restart ego-uploader
```

### ego-qc reference

`ego-qc` is built by `setup-station.sh` to `rust/target/release/ego-qc`. The `setup-pipeline.sh` script installs it to `/usr/local/bin/ego-qc` (on PATH).

**For non-systemd setups:** Replace `ego-qc` with `./rust/target/release/ego-qc` or add `rust/target/release/` to your PATH:

```bash
export PATH="$PWD/rust/target/release:$PATH"
```

#### Validate

```bash
ego-qc validate ./datasets/pick
```

#### Analyze (read-only)

```bash
ego-qc analyze ./datasets/pick          # summary
ego-qc analyze ./datasets/pick -v        # verbose with reasons
ego-qc analyze ./datasets/pick --report report.json
```

Verdicts: **KEEP** (active), **PRUNE_CONFIDENT** (definitely idle), **PRUNE_SUGGESTED** (likely idle), **REVIEW** (ambiguous).

#### Calibrate a station profile (optional, recommended)

A station profile aggregates statistics across many episodes for more stable activity thresholds.

```bash
ego-qc calibrate ./datasets/pick --format csv -o features.csv
ego-qc calibrate ./datasets/pick --save-profile station.json
```

Then pass `--profile station.json` to analyze, prune, or splice.

#### Prune

```bash
ego-qc prune ./datasets/pick                    # dry run
ego-qc prune ./datasets/pick --threshold 0.3    # custom threshold
ego-qc prune ./datasets/pick --profile station.json  # with profile
ego-qc prune ./datasets/pick --apply             # execute
```

Pruned files are moved to `.pruned/` (never deleted). An `audit.jsonl` log records every operation.

#### Splice

```bash
ego-qc splice ./datasets/pick                    # preview segments
ego-qc splice ./datasets/pick --min-gap 5 --min-duration 3
ego-qc splice ./datasets/pick --replace-original  # move originals to .pruned/
```

Output: `{original}_seg000.egorec`, `{original}_seg001.egorec`, etc.

#### Restore

```bash
ego-qc restore ./datasets/pick episode_042.egorec
```

#### MP4 extraction

```bash
ego-qc mp4 recording.egorec                         # outputs .mp4 + .depth.mp4 + .meta.json
ego-qc mp4 ./datasets/pick/*.egorec -o ./mp4_output
ego-qc mp4 recording.egorec -q                      # quiet (no progress bar)
```

### Updating

```bash
cd ego-recorder
git pull
./scripts/setup-station.sh            # rebuilds recorder + QC tools
sudo systemctl restart ego-recorder.service   # if using systemd
```

### Troubleshooting

| Problem | Fix |
|---------|-----|
| Camera not detected | Use USB 3.0 port, try different cable, check `lsusb \| grep 8086` |
| Service won't start | Check `journalctl -u ego-recorder.service -n 50` |
| Frames dropping | Increase `queue_size` to 8+, increase `h264_crf`, use SSD |
| Laptop suspends on lid close | Verify `/etc/systemd/logind.conf.d/50-ego-recorder-lid.conf` exists, reboot |
| `librealsense2.so` not found | Run `scripts/setup.sh` (registers library path) or manually: `echo /opt/ros/jazzy/lib/x86_64-linux-gnu \| sudo tee /etc/ld.so.conf.d/ros-jazzy.conf && sudo ldconfig` |
| Viewer can't find ego-qc | Pass `--qc ../rust/target/release/ego-qc` or add `rust/target/release/` to PATH |
| Depth not showing in viewer | Click the file to trigger conversion; check terminal for ego-qc errors |
