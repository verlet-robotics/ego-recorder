# Deployment Guide

## Quick Start

### 1. Install

```bash
git clone https://github.com/verlet-robotics/ego-recorder.git
cd ego-recorder
./scripts/setup.sh
```

For headless-only machines (no display):

```bash
./scripts/setup.sh --headless
```

### 2. Set up recordings

```bash
./scripts/setup-recordings.sh
```

This creates the recording directory, sets permissions, and initializes a dataset manifest.

### 3. Record

```bash
./scripts/record.sh
```

This prompts you to pick a dataset (or create a new one), choose headless or GUI mode, and starts recording. Episodes are auto-named (`pick_000.egorec`, `pick_001.egorec`, ...) and registered in `dataset.json`.

---

## Reference

### Prerequisites

- Ubuntu 22.04 or 24.04
- Intel RealSense D435 or D435i
- USB 3.0 port (USB 2.0 cannot sustain 30fps)
- ~150 MB/minute of storage at default settings

### Verify the camera

```bash
# Check USB detection
lsusb | grep 8086
# D435 = 8086:0b07, D435i = 8086:0b3a

# Quick test
ego-recorder -s test -o /tmp
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
ego-recorder -o /var/lib/ego-recorder/pick

# Headless mode
ego-recorder --headless -o /var/lib/ego-recorder/pick

# With explicit session name (overrides auto-indexing)
ego-recorder --headless -o /var/lib/ego-recorder/pick -s my_session
```

GUI controls: **Space** start/stop, **Escape** quit, **V** cycle view.

### Dataset management

```bash
# Inspect dataset (episodes, totals)
ego-recorder dataset info /var/lib/ego-recorder/pick

# Export to RLDS (TFRecord)
ego-recorder export rlds /var/lib/ego-recorder/pick -o ./rlds_output

# Export to LeRobot v3
ego-recorder export lerobot /var/lib/ego-recorder/pick -o ./lerobot_output
```

### Disk usage

| Duration | Size |
|----------|------|
| 1 minute | ~150 MB |
| 1 hour | ~9 GB |
| 8 hours | ~72 GB |

Recording stops automatically when free space drops below `disk_min_mb` (default: 1000 MB).

### CRF tuning (quality vs size)

```bash
ego-recorder --crf 18 -o ~/recordings   # higher quality, larger files
ego-recorder --crf 28 -o ~/recordings   # smaller files, still good for ML
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

The service auto-recovers from camera disconnects and prevents lid-close suspend.

### Updating

```bash
cd ego-recorder
git pull
./scripts/setup.sh
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
