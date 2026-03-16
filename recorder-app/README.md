# Ego Recorder App

Unified Tauri 2 desktop app for recording, reviewing, and uploading `.egorec` files. Replaces the fragmented workflow of separate C++ recorder GUI, Python upload daemon, and viewer app with a single interface.

## Architecture

```
recorder-app/
  src-tauri/src/         Rust backend (Tauri 2)
    recorder/            Spawn/stop C++ ego-recorder subprocess, parse stats
    video/               Axum MP4 streaming server (ported from viewer-app)
    commands/            Tauri IPC command handlers
    config.rs            TOML config at ~/.config/ego-recorder-app/config.toml
    disk.rs              statvfs disk space monitoring
  src/                   React 19 frontend
    components/record/   Recording UI with countdown, live stats, disk bar
    components/library/  File browser, scrubbable video player, metadata panel
    components/settings/ Settings page with first-run setup wizard
    stores/              Zustand state management
```

The app does **not** rewrite the C++ recorder. It spawns the existing `ego-recorder` binary as a subprocess, parses its stderr for live stats, and manages its lifecycle via signals (SIGINT to stop).

## Prerequisites

### Required

| Tool | Version | Install |
|------|---------|---------|
| Rust | 1.75+ | [rustup.rs](https://rustup.rs) |
| Bun | 1.0+ | `curl -fsSL https://bun.sh/install \| bash` |
| CMake | 3.16+ | `sudo apt install cmake` |
| pkg-config | any | `sudo apt install pkg-config` |

### System Libraries (for the C++ ego-recorder binary)

```bash
# Ubuntu/Debian
sudo apt install \
  librealsense2-dev \
  libavcodec-dev libavutil-dev libswscale-dev \
  libturbojpeg0-dev \
  libzstd-dev \
  libglfw3-dev

# The C++ build also fetches zdepth, cxxopts, toml++, imgui, and googletest via CMake FetchContent.
```

### Tauri 2 System Dependencies (Linux)

```bash
# Ubuntu/Debian
sudo apt install \
  libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  patchelf \
  libxcb-render0-dev \
  libxcb-shape0-dev \
  libxcb-xfixes0-dev \
  libxkbcommon-x11-dev \
  libdbus-1-dev
```

See [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for other distros.

## Quick Start

### One-command setup

```bash
cd recorder-app
./setup.sh
```

This idempotent script:
1. Checks all system dependencies
2. Builds the C++ `ego-recorder` binary (headless mode, no GUI deps needed)
3. Installs frontend packages (`bun install`)
4. Builds the Tauri desktop app

Other modes:

```bash
./setup.sh --check   # Only verify dependencies
./setup.sh --dev     # Install deps, skip full build (for development)
```

### Development

```bash
cd recorder-app
bun install
bun run tauri dev
```

This starts both the Vite dev server (port 1422) and the Tauri Rust backend with hot-reload.

### Production Build

```bash
bun run tauri build
```

Binary output: `src-tauri/target/release/ego-recorder-app`

Optional install:

```bash
./setup.sh --install   # Copies to ~/.local/bin/ego-recorder-app
```

## Configuration

All configuration is stored in a single TOML file:

```
~/.config/ego-recorder-app/config.toml
```

On first launch the app opens a setup wizard that walks through each section. You can also edit the file directly:

```toml
[recorder]
binary_path = "/usr/local/bin/ego-recorder"   # or leave blank for auto-detect
default_crf = 23                               # H.264 quality (0=lossless, 51=worst)
warmup_frames = 30                             # frames to discard for auto-exposure

[storage]
output_dir = "/home/user/recordings"           # default output directory
disk_threshold_mb = 500                        # warn when free space drops below this

[upload]
endpoint = "https://xxx.r2.cloudflarestorage.com"
bucket = "ego-recordings"
access_key = "your-access-key"
secret_key = "your-secret-key"
region = "auto"
auto_upload = false
```

### Binary Auto-Detection

The app searches for the `ego-recorder` binary in this order:
1. `$PATH` (i.e. if installed system-wide)
2. `../build/ego-recorder` (relative to app binary — the default CMake build output)
3. `../../build/ego-recorder`

Override with the `binary_path` config key or via the Settings page.

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `RUST_LOG` | Rust log level for the Tauri backend | `warn` |
| `HOME` | Used to locate `~/.config/ego-recorder-app/` | system default |
| `TAURI_DEV_HOST` | Bind Vite dev server to a specific host (for remote dev) | `localhost` |

The app uses **no other environment variables** at runtime. All S3/R2 credentials and paths are stored in the TOML config file and configured through the Settings page (not env vars). This is intentional — the app is designed for non-technical operators who shouldn't need to set environment variables.

### Debug Logging

```bash
RUST_LOG=debug bun run tauri dev        # Verbose backend logging
RUST_LOG=ego_recorder_app_lib=trace     # Trace only the app crate
```

## Features

### Phase 1 (current)

- **Record Page** — Configure session (output dir, name, CRF), 3-second audio countdown, start/stop recording (Space shortcut), live stats (frames, FPS, file size, elapsed time), disk space monitoring with threshold warning
- **Library Page** — Browse `.egorec` files in a directory, scrubbable H.264 video playback via in-memory MP4 muxing, full metadata panel (camera intrinsics, codec info, timestamps)
- **Settings Page** — First-run setup wizard, recorder binary config, storage paths, R2/S3 upload credentials
- **Lid-Close Safe Mode** — D-Bus inhibitor lock (`handle-lid-switch:sleep:idle`) so recording continues with the laptop lid closed

### Planned

- **Phase 2** — File watcher for new recordings, QC analysis integration
- **Phase 3** — Rust-native S3 multipart upload with recording-aware throttling, persistent upload queue
- **Phase 4** — Dataset manifest CRUD, "record into dataset" flow

## Tech Stack

| Layer | Stack |
|-------|-------|
| Desktop framework | Tauri 2 |
| Backend | Rust, axum 0.8, tokio, zbus (D-Bus) |
| Frontend | React 19, Vite 7, Tailwind 4, shadcn/ui, Zustand 5 |
| Video playback | In-memory H.264 Annex-B to MP4 muxing, HTTP range requests |
| Recording | C++ ego-recorder subprocess (RealSense + H.264 + Zdepth) |
| Config | TOML (`~/.config/ego-recorder-app/config.toml`) |

## Project Structure

```
recorder-app/
├── setup.sh                    # One-command build script
├── package.json                # Frontend deps (Bun)
├── vite.config.ts              # Vite config (port 1422)
├── tsconfig.json
├── index.html
├── src/
│   ├── main.tsx                # React entry point
│   ├── app.tsx                 # Page router + init
│   ├── lib/
│   │   ├── tauri.ts            # Tauri command bindings
│   │   ├── types.ts            # TypeScript interfaces
│   │   ├── audio.ts            # Web Audio countdown beeps
│   │   └── utils.ts            # cn() helper
│   ├── stores/
│   │   ├── app-store.ts        # Page, config, first-run state
│   │   └── recorder-store.ts   # Recording + library state
│   ├── components/
│   │   ├── ui/                 # shadcn/ui primitives (ported from viewer-app)
│   │   ├── layout/sidebar.tsx  # Navigation sidebar
│   │   ├── record/             # Record page components
│   │   ├── library/            # Library page + video player
│   │   └── settings/           # Settings + setup wizard
│   └── styles/globals.css      # Tailwind + Verlet design tokens
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json
    └── src/
        ├── lib.rs              # Tauri setup, command registration
        ├── main.rs             # Binary entry point
        ├── state.rs            # AppState, DTOs
        ├── config.rs           # TOML config load/save
        ├── disk.rs             # Disk space monitoring
        ├── recorder/
        │   ├── subprocess.rs   # Spawn/kill C++ process, parse stderr
        │   ├── status.rs       # RecorderStatus DTOs
        │   └── inhibitor.rs    # D-Bus lid-close inhibitor lock
        ├── video/
        │   ├── video_server.rs # Axum MP4 streaming
        │   ├── h264_annex_b.rs # NAL unit parser
        │   └── mp4_mux.rs      # ISO BMFF muxer
        └── commands/
            ├── recorder_commands.rs
            ├── library_commands.rs
            ├── settings_commands.rs
            └── dialog_commands.rs
```
