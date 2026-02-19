---
phase: 02-gui-mode-headless-systemd-service
plan: 02
subsystem: ui
tags: [imgui, glfw, opengl, gui, presenter, colormap, texture, live-preview]

# Dependency graph
requires:
  - phase: 02-gui-mode-headless-systemd-service
    plan: 01
    provides: IPresenter pure virtual interface, Config struct, imgui_glfw_opengl3 cmake target
  - phase: 01-core-capture-engine-mvp-storage
    provides: Stats class (used in update_stats signature)
provides:
  - GuiPresenter class: full IPresenter implementation with GLFW window, ImGui render loop, OpenGL texture upload
  - update_frame() thread-safe API for capture thread to push RGB+depth frames to GUI
  - Jet colormap (z16_to_jet_rgb): blue=near, red=far, invalid depth=black
  - Three view modes: RGB_ONLY, DEPTH_ONLY, SIDE_BY_SIDE cycled by Tab
  - Session name InputText + Start/Stop button (disabled when name empty)
  - Camera disconnect banner with Reconnect button wired to on_reconnect_requested callback
  - Semi-transparent stats overlay (alpha 0.5) showing FPS, frame counts, bytes, elapsed time
  - Space/Escape/Tab keyboard shortcuts guarded by !io.WantCaptureKeyboard
affects:
  - 02-04-PLAN (main.cpp integration: construct GuiPresenter, call update_frame, wire callbacks)

# Tech tracking
tech-stack:
  added:
    - Dear ImGui v1.92.6 render loop (ImGui_ImplGlfw + ImGui_ImplOpenGL3 backends, imgui_stdlib for InputText)
    - OpenGL texture management: glGenTextures, glTexImage2D (once), glTexSubImage2D (per-frame)
    - GLFW window creation with glfwWindowHint OpenGL 3.0, vsync via glfwSwapInterval(1)
  patterns:
    - Double-buffer via local copies: frame_mutex_ protects shared rgb_buf_/depth_buf_; tick() copies under lock, renders without lock
    - ImTextureID cast: (ImTextureID)(intptr_t)tex for strict aliasing safety
    - ImGui::BeginDisabled/EndDisabled for conditionally disabled buttons
    - Jet colormap: per-pixel normalize depth to [0,1], apply r/g/b = clamp(1.5 - |4t-N|, 0, 1) formulas

key-files:
  created:
    - src/presenter/gui_presenter.h
    - src/presenter/gui_presenter.cpp
  modified:
    - CMakeLists.txt

key-decisions:
  - "update_frame() uses std::mutex + memcpy: minimal lock hold time (~1ms), no complex ring buffer needed at 30fps capture vs 60fps render"
  - "Jet colormap: blue=near (t=0), red=far (t=1) -- standard jet orientation per user decision"
  - "ImGuiWindowFlags_NoBringToDisplayFront does not exist in v1.92.6 -- correct flag is NoBringToFrontOnFocus"
  - "gui_presenter.cpp placed in target_sources inside if(WITH_GUI) block -- cleaner than unconditional add with HAVE_GUI guard"
  - "Controls panel fixed at 120px bottom strip; preview fills remaining height with aspect-ratio scaling"

patterns-established:
  - "IPresenter concrete implementation: override all 6 virtual methods, wrap in #ifdef HAVE_GUI"
  - "OpenGL texture lifecycle: glGenTextures + glTexImage2D(nullptr) in start(), glTexSubImage2D per tick(), glDeleteTextures in shutdown()"
  - "Keyboard shortcut guard: check !ImGui::GetIO().WantCaptureKeyboard before IsKeyPressed()"

# Metrics
duration: ~8min
completed: 2026-02-19
---

# Phase 02 Plan 02: GuiPresenter Implementation Summary

**Dear ImGui + GLFW + OpenGL3 GUI presenter with live RGB/depth preview, jet colormap, three view modes, session controls, and semi-transparent stats overlay**

## Performance

- **Duration:** ~8 min
- **Started:** 2026-02-19T06:47:32Z
- **Completed:** 2026-02-19T06:55:00Z
- **Tasks:** 1/1
- **Files modified:** 3 (2 created, 1 modified)

## Accomplishments

- Full GuiPresenter implementing IPresenter: GLFW window (1280x720), ImGui context, OpenGL3 backend, vsync
- Per-frame texture upload via glTexSubImage2D for both RGB and jet-colorized depth (no re-allocation per frame)
- Thread-safe update_frame() with std::mutex: capture thread pushes frames, render thread copies under lock and renders unlocked
- Jet colormap converter (z16_to_jet_rgb): normalizes Z16 depth to metres, maps to blue=near/red=far jet palette, black for invalid pixels
- Three view modes (RGB_ONLY, DEPTH_ONLY, SIDE_BY_SIDE default) with aspect-ratio-preserving scaling; cycled by Tab
- Controls panel: InputText session name + Start/Stop recording button (ImGui::BeginDisabled when name empty)
- Camera disconnect banner with red text + Reconnect button wired to on_reconnect_requested_ callback
- Semi-transparent stats overlay (alpha=0.5, top-right) showing capture/write FPS, frame counts, bytes (MB/GB), elapsed time
- Keyboard shortcuts guarded by !io.WantCaptureKeyboard: Space=toggle record, Tab=cycle view, Escape=quit

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement GuiPresenter with live preview, controls, and stats overlay** - `90a9dd8` (feat)

**Plan metadata:** _(docs commit follows)_

## Files Created/Modified

- `src/presenter/gui_presenter.h` - GuiPresenter class declaration wrapped in #ifdef HAVE_GUI; inherits IPresenter; update_frame() and getter APIs; private ViewMode enum, GL handles, mutex, frame buffers, stats cache
- `src/presenter/gui_presenter.cpp` - 510-line implementation: start()/tick()/shutdown() lifecycle, z16_to_jet_rgb colormap, update_frame() with mutex, update_stats() stats cache, on_camera_disconnect/reconnect flags
- `CMakeLists.txt` - gui_presenter.cpp added via target_sources inside if(WITH_GUI) block (linter-improved pattern vs unconditional add)

## Decisions Made

- `update_frame()` uses `std::mutex` + `memcpy` approach: lock hold time is a single memcpy (~1ms for 640x480x5 bytes). No complex double-buffering needed at 30fps capture vs 60fps render cadence.
- Jet colormap orientation: blue=near, red=far (standard jet, per user-locked decision from research phase).
- `gui_presenter.cpp` compiled only inside `if(WITH_GUI)` cmake block via `target_sources()` -- cleaner than always compiling with `#ifdef HAVE_GUI` guard.
- Controls panel is a fixed 120px strip at the bottom; preview ImGui window fills the rest with aspect-ratio-fit scaling per view mode.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed invalid ImGui flag name for v1.92.6**
- **Found during:** Task 1 build verification
- **Issue:** `ImGuiWindowFlags_NoBringToDisplayFront` does not exist in ImGui v1.92.6 -- compiler error "was not declared in this scope"
- **Fix:** Replaced with `ImGuiWindowFlags_NoBringToFrontOnFocus` which is the correct flag name in this version
- **Files modified:** `src/presenter/gui_presenter.cpp`
- **Verification:** Build passed after fix: `[100%] Built target ego-recorder`
- **Committed in:** `90a9dd8` (part of Task 1 commit)

---

**Total deviations:** 1 auto-fixed (Rule 1 - Bug)
**Impact on plan:** Necessary for correct compilation. No scope creep. The flag name mismatch was a research artifact -- documentation referenced a flag name not present in the fetched version.

## Issues Encountered

- ImGui v1.92.6 API difference: `ImGuiWindowFlags_NoBringToDisplayFront` flag referenced in plan does not exist; correct name is `ImGuiWindowFlags_NoBringToFrontOnFocus`. Fixed inline before commit.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- GuiPresenter ready for main.cpp integration (plan 02-04): construct with Config + callbacks, call update_frame() from capture thread, call tick() in main loop
- update_frame() API is public so main.cpp can pass it as a lambda/bound function to the capture thread
- WITH_GUI=ON build verified clean end-to-end: imgui fetched, compiled, linked, ego-recorder binary produced
- HeadlessPresenter (plan 02-03) can proceed in parallel -- GuiPresenter does not affect it

## Self-Check: PASSED

- FOUND: src/presenter/gui_presenter.h
- FOUND: src/presenter/gui_presenter.cpp
- FOUND: .planning/phases/02-gui-mode-headless-systemd-service/02-02-SUMMARY.md
- FOUND: commit 90a9dd8

---
*Phase: 02-gui-mode-headless-systemd-service*
*Completed: 2026-02-19*
