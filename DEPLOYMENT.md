# Production Deployment Guide

This guide covers deploying ego-recorder on a dedicated machine for robust, unattended RGBD recording. By the end you will have both GUI mode (interactive sessions) and headless mode (systemd service) working.

## Prerequisites

- Ubuntu 22.04 or 24.04 (desktop install recommended for GUI mode)
- Intel RealSense D435 or D435i camera
- USB 3.0 port (USB 2.0 cannot sustain 30fps)
- Sufficient storage for recordings (~150 MB/minute at default settings)

## 1. Build from source

```bash
git clone https://github.com/verlet-robotics/ego-recorder.git
cd ego-recorder
./setup.sh --all
```

This installs all system dependencies, the Intel RealSense SDK, and builds with GUI + Python + tests. All 66 tests should pass (no camera required for tests).

If you only need headless mode (e.g., a server with no display):

```bash
./setup.sh --headless
```

## 2. Verify the camera works

Plug in the RealSense camera and confirm it is detected:

```bash
# If you installed librealsense2-utils
rs-enumerate-devices | head -20

# Or just try running ego-recorder
./build/ego-recorder -s test -o /tmp
```

If the camera is not detected:
- Ensure USB 3.0 (blue port). USB 2.0 hubs will not work.
- Check `dmesg | tail -20` for USB errors after plugging in.
- Try a different cable -- RealSense cameras are sensitive to cable quality.

## 3. Set up the recording directory

Choose where recordings will be stored. This directory needs enough free space for your recording sessions.

```bash
# For interactive (GUI) use -- any directory your user can write to
mkdir -p ~/recordings

# For the systemd service -- needs to be writable by the ego-recorder system user
sudo mkdir -p /var/lib/ego-recorder/recordings
sudo chown ego-recorder:ego-recorder /var/lib/ego-recorder/recordings
```

Disk usage at default settings (CRF 23):

| Duration | Approximate size |
|----------|-----------------|
| 1 minute | ~150 MB |
| 1 hour | ~9 GB |
| 8 hours | ~72 GB |

Recording stops automatically when free disk space drops below `disk_min_mb` (default: 1000 MB in production config).

## 4. GUI mode (interactive recording)

Run directly from the build directory:

```bash
./build/ego-recorder -s my_session -o ~/recordings
```

Controls:
- **Space** -- start/stop recording
- **Escape** -- stop recording (if active) or quit
- **V** -- cycle view: RGB only / depth only / side-by-side

The live preview shows FPS, frame count, disk usage, and elapsed time.

For production use, you can tune the CRF (quality vs size tradeoff):

```bash
# Higher quality, larger files (~10x compression)
./build/ego-recorder -s session --crf 18 -o ~/recordings

# Smaller files, still good for ML training (~20-25x compression)
./build/ego-recorder -s session --crf 28 -o ~/recordings
```

### Installing the binary system-wide (optional)

If you want to run `ego-recorder` from anywhere without the build path:

```bash
sudo install -m 755 build/ego-recorder /usr/local/bin/ego-recorder
```

Then just run `ego-recorder -s my_session -o ~/recordings` from any directory.

## 5. Headless mode (systemd service)

Headless mode runs ego-recorder as a background service that starts recording automatically when the camera is connected. This is the recommended setup for dedicated recording machines (laptops mounted on robots, etc.).

### 5a. Run the deployment script

```bash
sudo bash deploy/install.sh
```

This installs:

| File | Location |
|------|----------|
| Binary | `/usr/local/bin/ego-recorder` |
| Config | `/etc/ego-recorder/config.toml` |
| systemd unit | `/etc/systemd/system/ego-recorder.service` |
| udev rules | `/etc/udev/rules.d/99-ego-recorder.rules` |
| logind drop-in | `/etc/systemd/logind.conf.d/50-ego-recorder-lid.conf` |

The script creates a dedicated `ego-recorder` system user with access to USB and video devices. The service is **not** auto-enabled -- you must explicitly start it.

### 5b. Configure

Edit the production config:

```bash
sudo nano /etc/ego-recorder/config.toml
```

Key settings to verify:

```toml
[output]
dir = "/var/lib/ego-recorder/recordings"   # must exist and be writable

[compression]
h264_crf = 23          # 23 = default, good for ML training

[recording]
disk_min_mb = 1000     # stop recording at 1 GB free
queue_size = 4         # frame queue depth

[service]
headless = true        # must be true for systemd
```

Make sure the output directory exists:

```bash
sudo mkdir -p /var/lib/ego-recorder/recordings
sudo chown ego-recorder:ego-recorder /var/lib/ego-recorder/recordings
```

### 5c. Start the service

```bash
# Start now (one-time)
sudo systemctl start ego-recorder.service

# Or enable to start on every boot + start now
sudo systemctl enable --now ego-recorder.service
```

Only enable auto-start when you have verified the service works correctly with your camera. If the camera is not connected, the service will fail and restart every 5 seconds until it finds one.

### 5d. Monitor

```bash
# Check status
sudo systemctl status ego-recorder.service

# Follow logs in real time
journalctl -fu ego-recorder.service

# Check disk usage of recordings
du -sh /var/lib/ego-recorder/recordings/
```

### 5e. Stop the service

```bash
# Stop recording
sudo systemctl stop ego-recorder.service

# Disable auto-start on boot
sudo systemctl disable ego-recorder.service
```

## 6. What the deployment files do

### udev rules (`99-ego-recorder.rules`)

- Grants the `plugdev` group access to RealSense D435/D435i USB devices
- Disables USB autosuspend for the camera (prevents the OS from power-cycling it mid-recording)

### logind drop-in (`50-ego-recorder-lid.conf`)

Prevents the laptop from suspending when the lid is closed. This is a machine-wide fallback -- the application also holds a D-Bus inhibitor lock at runtime that is scoped to the process. The drop-in is only needed if D-Bus is unavailable.

If you don't want this behavior, remove it:

```bash
sudo rm /etc/systemd/logind.conf.d/50-ego-recorder-lid.conf
sudo systemctl restart systemd-logind   # caution: may disrupt graphical sessions
```

### systemd service (`ego-recorder.service`)

- Runs as the `ego-recorder` system user (no shell, no home directory)
- Uses `Type=notify` with a 30-second watchdog -- if the process hangs, systemd kills and restarts it
- `Restart=on-failure` with 5-second backoff -- auto-recovers from camera disconnects
- `OOMScoreAdjust=-100` -- less likely to be killed by the OOM killer

## 7. Updating

After pulling new code:

```bash
cd ego-recorder
git pull
./setup.sh --all               # rebuild
sudo bash deploy/install.sh    # reinstall binary + service files
sudo systemctl restart ego-recorder.service   # if running headless
```

The install script preserves your existing `/etc/ego-recorder/config.toml` and saves the new example as `config.toml.example` so you can compare.

## 8. Troubleshooting

### Camera not detected

```bash
# Check if the device is visible
lsusb | grep 8086
# D435 shows as 8086:0b07, D435i as 8086:0b3a

# Check kernel messages
dmesg | grep -i realsense

# Test with the RealSense viewer (if installed)
realsense-viewer
```

Common fixes:
- Use a USB 3.0 port (not USB 2.0, not through a hub)
- Try a different USB cable
- Unplug and replug the camera
- Check `lsusb -t` to verify the camera is on a USB 3.0 bus (xhci_hcd)

### Service fails to start

```bash
# Check the logs
journalctl -u ego-recorder.service --no-pager -n 50

# Common causes:
# - Camera not connected
# - Output directory doesn't exist or isn't writable
# - Another process (GUI instance) is already using the camera
```

### Frames are being dropped

```bash
# Check journalctl for "dropped" messages
journalctl -u ego-recorder.service | grep -i drop
```

Frame drops happen when the writer thread can't keep up with the capture thread. Fixes:
- Increase `queue_size` to 8 or 16 (uses more RAM but absorbs burst latency)
- Increase `h264_crf` to 28+ (faster to encode, smaller output)
- Use a faster disk (SSD recommended, avoid network mounts)

### Not enough disk space

Recording stops automatically at `disk_min_mb`. To change:

```bash
sudo nano /etc/ego-recorder/config.toml
# Set disk_min_mb = 500 (or lower if you're sure)
sudo systemctl restart ego-recorder.service
```

### Service keeps restarting without a camera

This is expected behavior -- `Restart=on-failure` retries every 5 seconds. If you don't want this:

```bash
# Stop and disable until you're ready
sudo systemctl disable --now ego-recorder.service
```

### Laptop suspends when lid is closed

Check that the logind drop-in is installed:

```bash
cat /etc/systemd/logind.conf.d/50-ego-recorder-lid.conf
```

If it's there but not working, verify with:

```bash
loginctl show-session $(loginctl | grep $(whoami) | awk '{print $1}') -p HandleLidSwitch
```

You may need to reboot for the logind config to take effect.
