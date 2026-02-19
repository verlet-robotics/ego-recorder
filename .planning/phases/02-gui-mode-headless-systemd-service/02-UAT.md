---
status: complete
phase: 02-gui-mode-headless-systemd-service
source: 02-01-SUMMARY.md, 02-02-SUMMARY.md, 02-03-SUMMARY.md, 02-04-SUMMARY.md
started: 2026-02-19T09:10:00Z
updated: 2026-02-19T09:35:00Z
---

## Current Test

[testing complete]

## Tests

### 1. GUI launches with live camera preview
expected: Run `./build/ego-recorder`. Window opens with side-by-side RGB + jet-colorized depth. Blue=near, red=far, black=invalid. Smooth ~30fps preview.
result: pass
note: Fixed colormap during testing — replaced fixed-range jet (0.1-10m) with histogram-equalized turbo colormap. Black noise pixels are normal D435 stereo holes.

### 2. View mode cycling with Tab
expected: While GUI is running, press Tab. View cycles: side-by-side -> RGB only -> depth only -> side-by-side. Each mode fills the available preview area with aspect-ratio scaling.
result: pass

### 3. Stats overlay displays live metrics
expected: Semi-transparent overlay appears in the top-right corner showing: capture FPS, write FPS, frame counts (captured/written), dropped frames, bytes written (MB/GB), and elapsed time. Values update in real-time.
result: pass

### 4. Session name + recording controls
expected: Type a session name in the text input field. Start Recording button becomes enabled (was disabled/grayed when name was empty). Press Space -- recording starts, button changes to "Stop Recording (Space)". Press Space again -- recording stops. A .egorec file is created in the output directory.
result: pass

### 5. Escape quits the GUI
expected: Press Escape while GUI is running. Window closes cleanly. If recording was active, the .egorec file is finalized (not corrupted). Terminal shows "Recording complete" with final stats.
result: pass

### 6. Headless mode with auto-record and date directories
expected: Run `./build/ego-recorder --headless --output /tmp --duration 10`. Recording starts immediately with auto-generated session name (capture_YYYYMMDD_HHMMSS). Output file created in date-based path `/tmp/YYYY/MM/DD/capture_....egorec`. Stats print to stderr. Clean shutdown after ~10 seconds.
result: pass

### 7. Config file loading with CLI override
expected: Copy `config.toml.example` to `/tmp/test.toml`. Edit `output.dir` to a custom path (e.g., `/tmp/cfg-test`). Run `./build/ego-recorder --headless --config /tmp/test.toml --duration 5`. Recording uses the config file's output directory. CLI --duration overrides any config default.
result: pass

### 8. Headless-only build rejects GUI mode
expected: Run `./build_headless/ego-recorder` (without --headless flag). Prints an error message about GUI not being available (built without WITH_GUI). Exits with non-zero code. Running with `--headless` works normally.
result: pass

### 9. Deploy artifacts are complete and correct
expected: `ls deploy/` shows 5 files: ego-recorder.service, 99-ego-recorder.rules, 50-ego-recorder-lid.conf, install.sh, config.toml.example. Service file has Type=notify and WatchdogSec=30s. Install script has set -euo pipefail and root check.
result: pass
note: Verified programmatically — all 5 files present, service file and install script correct.

## Summary

total: 9
passed: 9
issues: 0
pending: 0
skipped: 0

## Gaps

[none yet]
