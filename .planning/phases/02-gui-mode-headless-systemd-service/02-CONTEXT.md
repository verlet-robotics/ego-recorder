# Phase 2: GUI Mode + Headless Systemd Service - Context

**Gathered:** 2026-02-19
**Status:** Ready for planning

<domain>
## Phase Boundary

Add two presentation modes to the existing capture engine: an interactive Dear ImGui GUI for desktop recording with live preview, and a headless systemd service for unattended recording on a closed laptop. Includes USB disconnect recovery, deployment artifacts (systemd unit, udev rules, install script), and a configuration file.

</domain>

<decisions>
## Implementation Decisions

### GUI Layout & Preview
- Tabbed/toggle view between RGB-only, depth-only, and side-by-side — switchable with a hotkey
- Depth colorized using jet colormap (blue=near, red=far) — classic RealSense Viewer style
- Stats overlay (FPS, frame count, dropped frames, disk usage, elapsed time) rendered semi-transparently on top of the video feed
- Window is resizable, preview scales to fit the window

### Recording Workflow
- Camera preview starts automatically on GUI launch — no manual "start preview" step
- Session name is required before recording starts — text input field, must be filled
- No pause/resume — stop ends the recording, start begins a new file
- Keyboard shortcuts: Space = start/stop recording, Escape = quit, Tab = toggle view mode

### Headless Deployment
- Auto-record on boot — service starts, camera opens, recording begins immediately
- Date-based folder organization: output_dir/YYYY/MM/DD/ subdirectories created automatically
- Single install script (sudo ./install.sh) that creates system user, copies files, enables service
- Auto-generated session names in headless mode (timestamp-based, since no user to type a name)

### USB Recovery
- GUI mode: notify user of disconnect, show reconnect button — user clicks to retry
- Headless mode: auto-retry reconnect loop (e.g., every 2 seconds) until camera comes back
- On reconnect: start a new recording file (close old one cleanly, fresh file for new session)

### Disk Full Handling
- Stop everything cleanly — finalize current file, exit process
- Let systemd Restart=on-failure or user restart when disk is cleared

### Claude's Discretion
- Config file format (TOML vs JSON) — pick based on dependency and usability trade-offs
- Exact keyboard shortcut mappings beyond Space/Escape/Tab
- Stats overlay styling (font size, opacity, position)
- Install script internals (user creation, file paths, permission setup)
- Disk space threshold default value

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 02-gui-mode-headless-systemd-service*
*Context gathered: 2026-02-19*
