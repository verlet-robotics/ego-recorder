---
phase: 02-gui-mode-headless-systemd-service
plan: 01
subsystem: infra
tags: [cmake, imgui, tomlplusplus, glfw, opengl, config, interface]

# Dependency graph
requires:
  - phase: 01-core-capture-engine-mvp-storage
    provides: Stats class (used in IPresenter::update_stats signature)
provides:
  - IPresenter pure virtual interface defining presenter lifecycle (start/tick/shutdown/camera events/update_stats)
  - Config struct with 9 settings fields and TOML file loading via toml++
  - CMakeLists.txt with toml++ v3.4.0 (always), Dear ImGui v1.92.6 + GLFW + OpenGL (WITH_GUI option)
  - config.toml.example with all settings documented
affects:
  - 02-02-PLAN (GuiPresenter implements IPresenter, uses Config)
  - 02-03-PLAN (HeadlessPresenter implements IPresenter, uses Config)
  - 02-04-PLAN (main.cpp integrates Config + IPresenter)

# Tech tracking
tech-stack:
  added:
    - tomlplusplus v3.4.0 (FetchContent, header-only TOML parser)
    - Dear ImGui v1.92.6 (FetchContent, gated behind WITH_GUI=ON)
    - GLFW (system package, gated behind WITH_GUI=ON)
    - OpenGL (system package, gated behind WITH_GUI=ON)
  patterns:
    - Strategy pattern: IPresenter abstracts GUI vs headless presenter
    - Optional dependency gating: WITH_GUI cmake option and HAVE_GUI compile definition
    - TOML config with value_or() fallbacks -- missing keys silently use defaults

key-files:
  created:
    - src/presenter/ipresenter.h
    - src/config/config.h
    - src/config/config.cpp
    - config.toml.example
  modified:
    - CMakeLists.txt

key-decisions:
  - "IPresenter uses Stats by const-ref in update_stats() -- presenter reads stats, never writes them"
  - "Config struct has config_path field tracking which file was loaded (empty = defaults only)"
  - "toml++ always linked (headless needs config too); imgui gated behind WITH_GUI option"
  - "WITH_GUI defaults to ON but build verified clean with OFF -- existing binary unaffected"
  - "imgui_glfw_opengl3 static library includes misc/cpp/imgui_stdlib.cpp for std::string support in ImGui widgets"

patterns-established:
  - "Strategy interface pattern: IPresenter with start/tick/shutdown lifecycle + out-of-band events"
  - "TOML config: [output], [compression], [recording], [service] sections with value_or() fallbacks"
  - "CMake optional deps: option(WITH_GUI) -> if(WITH_GUI) -> find_package + FetchContent + add_compile_definitions(HAVE_GUI)"

# Metrics
duration: 2min
completed: 2026-02-19
---

# Phase 02 Plan 01: Foundation Infrastructure Summary

**IPresenter Strategy interface + TOML config system via toml++ v3.4.0 + CMake optional Dear ImGui v1.92.6 / GLFW / OpenGL dependencies**

## Performance

- **Duration:** ~2 min
- **Started:** 2026-02-19T06:42:57Z
- **Completed:** 2026-02-19T06:45:09Z
- **Tasks:** 2/2
- **Files modified:** 5 (4 created, 1 modified)

## Accomplishments

- IPresenter pure virtual interface with 6 lifecycle methods (start, tick, shutdown, on_camera_disconnect, on_camera_reconnect, update_stats) + virtual destructor
- Config struct with 9 fields and load_config() using toml++ value_or() fallbacks; parse errors emit stderr warning and return defaults
- CMakeLists.txt extended with toml++ v3.4.0 (always) and Dear ImGui v1.92.6 + GLFW + OpenGL behind WITH_GUI option; existing ego-recorder binary verified unchanged with WITH_GUI=OFF
- config.toml.example documents all settings with defaults and valid ranges

## Task Commits

Each task was committed atomically:

1. **Task 1: IPresenter interface and TOML config system** - `92fa579` (feat)
2. **Task 2: Update CMakeLists.txt with Dear ImGui, toml++, GLFW, OpenGL** - `2708cf4` (feat)

**Plan metadata:** _(docs commit follows)_

## Files Created/Modified

- `src/presenter/ipresenter.h` - Pure virtual IPresenter interface; includes utils/stats.h for update_stats signature
- `src/config/config.h` - Config struct with 9 fields and load_config() declaration
- `src/config/config.cpp` - TOML parsing via toml::parse_file(); value_or() for all fields; parse_error -> stderr warning + defaults
- `config.toml.example` - All settings documented with defaults, valid ranges, and usage tips
- `CMakeLists.txt` - Added tomlplusplus FetchContent + WITH_GUI block (imgui FetchContent, static lib, find_package GLFW/OpenGL, HAVE_GUI define); added src/config/config.cpp to sources

## Decisions Made

- IPresenter uses Stats by const-ref in update_stats() -- presenter reads stats, never writes them
- Config struct tracks config_path (empty string = defaults only, no file loaded)
- toml++ always linked to ego-recorder; imgui/GLFW/OpenGL gated behind WITH_GUI option
- WITH_GUI defaults to ON but headless build (-DWITH_GUI=OFF) verified clean -- no regression to existing binary
- imgui_glfw_opengl3 static library includes misc/cpp/imgui_stdlib.cpp for std::string widget support

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- IPresenter interface ready for GuiPresenter (plan 02-02) and HeadlessPresenter (plan 02-03) to implement
- Config system ready for main.cpp integration (plan 02-04)
- WITH_GUI=ON cmake path ready to be tested once gui_presenter.cpp exists in plan 02-02
- Existing ego-recorder binary unaffected -- phase 1 functionality preserved

---
*Phase: 02-gui-mode-headless-systemd-service*
*Completed: 2026-02-19*
