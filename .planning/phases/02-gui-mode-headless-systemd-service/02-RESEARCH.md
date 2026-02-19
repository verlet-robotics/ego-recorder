# Phase 2: GUI Mode + Headless Systemd Service - Research

**Researched:** 2026-02-19
**Domain:** Dear ImGui + GLFW + OpenGL3 (GUI), libsystemd sd_notify + D-Bus inhibitor (headless), toml++ config, librealsense USB reconnect
**Confidence:** HIGH (core stack), MEDIUM (D-Bus inhibitor C API), HIGH (systemd unit patterns)

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### GUI Layout & Preview
- Tabbed/toggle view between RGB-only, depth-only, and side-by-side — switchable with a hotkey
- Depth colorized using jet colormap (blue=near, red=far) — classic RealSense Viewer style
- Stats overlay (FPS, frame count, dropped frames, disk usage, elapsed time) rendered semi-transparently on top of the video feed
- Window is resizable, preview scales to fit the window

#### Recording Workflow
- Camera preview starts automatically on GUI launch — no manual "start preview" step
- Session name is required before recording starts — text input field, must be filled
- No pause/resume — stop ends the recording, start begins a new file
- Keyboard shortcuts: Space = start/stop recording, Escape = quit, Tab = toggle view mode

#### Headless Deployment
- Auto-record on boot — service starts, camera opens, recording begins immediately
- Date-based folder organization: output_dir/YYYY/MM/DD/ subdirectories created automatically
- Single install script (sudo ./install.sh) that creates system user, copies files, enables service
- Auto-generated session names in headless mode (timestamp-based, since no user to type a name)

#### USB Recovery
- GUI mode: notify user of disconnect, show reconnect button — user clicks to retry
- Headless mode: auto-retry reconnect loop (e.g., every 2 seconds) until camera comes back
- On reconnect: start a new recording file (close old one cleanly, fresh file for new session)

#### Disk Full Handling
- Stop everything cleanly — finalize current file, exit process
- Let systemd Restart=on-failure or user restart when disk is cleared

### Claude's Discretion
- Config file format (TOML vs JSON) — pick based on dependency and usability trade-offs
- Exact keyboard shortcut mappings beyond Space/Escape/Tab
- Stats overlay styling (font size, opacity, position)
- Install script internals (user creation, file paths, permission setup)
- Disk space threshold default value

### Deferred Ideas (OUT OF SCOPE)
None — discussion stayed within phase scope
</user_constraints>

---

## Summary

Phase 2 adds two distinct execution modes on top of the existing three-thread pipeline: an interactive Dear ImGui + GLFW + OpenGL3 window for desktop use, and a systemd Type=notify service with watchdog for unattended operation. The core pipeline code from Phase 1 (capture thread, bounded queue, writer thread) is preserved unchanged; what changes is the "presenter" layer that owns startup, display, and shutdown.

The standard approach is a pure-virtual `IPresenter` interface (Strategy pattern) with two concrete implementations — `GuiPresenter` and `HeadlessPresenter`. The GUI presenter owns the GLFW window, OpenGL texture handles, and the ImGui render loop. The headless presenter owns the sd_notify watchdog keepalive thread and a D-Bus inhibitor file descriptor. Both presenters share the same signal-handling and shutdown machinery already in Phase 1.

The config file recommendation is TOML using toml++ v3.4.0 (header-only, C++17, FetchContent-compatible). TOML is human-editable, supports comments, and maps cleanly to the structured settings needed (output directory, JPEG quality, disk threshold). JSON has no comments, making it harder for users to annotate a config file they hand-edit.

**Primary recommendation:** Compile Dear ImGui's GLFW+OpenGL3 backend as a static CMake target via FetchContent. Wrap the Phase 1 pipeline in an `IPresenter` interface. Use sd_notify + sd_watchdog_enabled for the headless service. Use sd-bus for the D-Bus inhibitor lock. Use `std::filesystem::space` for disk monitoring.

---

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| Dear ImGui | v1.92.6 (Feb 2025) | Immediate-mode GUI, all widgets, rendering | Industry-standard for C++ tool UIs; no external dependencies beyond backend |
| GLFW | 3.x (system) | OpenGL window, input events | ImGui's recommended backend; widely packaged |
| OpenGL | 3.0+ (system) | GPU rendering, texture management | Available on all Linux desktop systems |
| toml++ | v3.4.0 | TOML config file parsing | Header-only, C++17, FetchContent-compatible, supports comments |
| libsystemd | system | sd_notify, sd_watchdog_enabled, sd-bus | Ships with systemd; already conditionally linked in Phase 1 CMake |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| imgui_stdlib.h | bundled with ImGui | std::string InputText wrapper | Always — avoids manual char[] buffer for session name input |
| std::filesystem | C++17 stdlib | Disk space check, directory creation | Date-based subdirectory creation, `space()` for free space monitoring |
| pthread | system | Signal thread for headless sigwait | Already linked in Phase 1 |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| toml++ | nlohmann/json | JSON has no comment support; worse user experience for hand-edited configs |
| toml++ | toml11 | Both are solid; toml++ is slightly smaller API surface and more commonly referenced in C++ projects |
| OpenGL3 backend | Vulkan/SDL2 | GLFW+OpenGL3 is the simplest path; Vulkan adds complexity without benefit for this use case |
| sd-bus (libsystemd) | dbus-1 (libdbus) | sd-bus is included in libsystemd already linked; avoids an extra dependency |

### Installation (system packages, Ubuntu/Debian)

```bash
sudo apt install libglfw3-dev libgl1-mesa-dev libsystemd-dev
```

### CMake FetchContent additions

```cmake
# Dear ImGui (no native CMake support — build manually)
FetchContent_Declare(
    imgui
    GIT_REPOSITORY https://github.com/ocornut/imgui.git
    GIT_TAG        v1.92.6
)
FetchContent_MakeAvailable(imgui)

add_library(imgui_glfw_opengl3 STATIC
    ${imgui_SOURCE_DIR}/imgui.cpp
    ${imgui_SOURCE_DIR}/imgui_draw.cpp
    ${imgui_SOURCE_DIR}/imgui_tables.cpp
    ${imgui_SOURCE_DIR}/imgui_widgets.cpp
    ${imgui_SOURCE_DIR}/backends/imgui_impl_glfw.cpp
    ${imgui_SOURCE_DIR}/backends/imgui_impl_opengl3.cpp
    ${imgui_SOURCE_DIR}/misc/cpp/imgui_stdlib.cpp   # std::string InputText
)
find_package(OpenGL REQUIRED)
find_package(glfw3 REQUIRED)
target_link_libraries(imgui_glfw_opengl3 PUBLIC glfw OpenGL::GL)
target_include_directories(imgui_glfw_opengl3 PUBLIC
    ${imgui_SOURCE_DIR}
    ${imgui_SOURCE_DIR}/backends
    ${imgui_SOURCE_DIR}/misc/cpp
)

# toml++
FetchContent_Declare(
    tomlplusplus
    GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
    GIT_TAG        v3.4.0
)
FetchContent_MakeAvailable(tomlplusplus)

# Optional GUI build flag
option(WITH_GUI "Build Dear ImGui GUI mode" ON)
if(WITH_GUI)
    add_compile_definitions(HAVE_GUI)
endif()
```

---

## Architecture Patterns

### Recommended Project Structure

```
src/
├── capture/             # Phase 1 — unchanged
│   ├── pipeline.h
│   └── pipeline.cpp
├── compression/         # Phase 1 — unchanged
├── storage/             # Phase 1 — unchanged
├── threading/           # Phase 1 — unchanged
├── utils/               # Phase 1 — unchanged
├── presenter/           # NEW: Strategy pattern implementations
│   ├── ipresenter.h         # Pure virtual interface
│   ├── gui_presenter.h
│   ├── gui_presenter.cpp
│   ├── headless_presenter.h
│   └── headless_presenter.cpp
├── config/              # NEW: Config file support
│   ├── config.h
│   └── config.cpp
└── main.cpp             # Modified: --headless flag, presenter selection
deploy/
├── ego-recorder.service     # systemd unit file
├── 99-ego-recorder.rules    # udev rules (USB permissions + autosuspend)
├── 50-ego-recorder-lid.conf # logind.conf drop-in
└── install.sh               # Installation script
```

### Pattern 1: IPresenter Strategy Interface

**What:** Pure-virtual interface expressing the lifecycle hooks that main.cpp calls. Concrete implementations handle mode-specific behavior.
**When to use:** Always — this is the central architectural seam between the recording engine and the display/service layer.

```cpp
// Source: standard C++ abstract class pattern
// src/presenter/ipresenter.h
class IPresenter {
public:
    virtual ~IPresenter() = default;

    // Called once after camera and writer are initialized.
    // Returns false if presenter cannot start (e.g., no display for GUI).
    virtual bool start() = 0;

    // Called in a loop by main. Returns false when the presenter wants to quit.
    // GUI: renders one frame. Headless: sends watchdog ping, checks disk.
    virtual bool tick() = 0;

    // Called by main when shutdown is complete. Flush final status, destroy window.
    virtual void shutdown() = 0;

    // Notifies presenter that the camera disconnected.
    virtual void on_camera_disconnect() = 0;

    // Notifies presenter that the camera reconnected.
    virtual void on_camera_reconnect() = 0;
};
```

### Pattern 2: Dear ImGui GLFW+OpenGL3 Render Loop

**What:** Standard immediate-mode render loop. Each call to `tick()` polls events, begins a new ImGui frame, renders all widgets, then swaps buffers.
**When to use:** Inside `GuiPresenter::tick()`.

```cpp
// Source: https://github.com/ocornut/imgui/blob/master/examples/example_glfw_opengl3/main.cpp
bool GuiPresenter::tick() {
    if (glfwWindowShouldClose(window_)) return false;

    glfwPollEvents();
    ImGui_ImplOpenGL3_NewFrame();
    ImGui_ImplGlfw_NewFrame();
    ImGui::NewFrame();

    // --- Begin fullscreen dockspace or single window ---
    render_preview_window();   // ImGui::Image() with texture
    render_controls_panel();   // Session name input, start/stop button
    render_stats_overlay();    // Semi-transparent overlay

    // --- Keyboard shortcuts (only when InputText not focused) ---
    ImGuiIO& io = ImGui::GetIO();
    if (!io.WantCaptureKeyboard) {
        if (ImGui::IsKeyPressed(ImGuiKey_Space))  toggle_recording();
        if (ImGui::IsKeyPressed(ImGuiKey_Tab))    cycle_view_mode();
        if (ImGui::IsKeyPressed(ImGuiKey_Escape)) return false;
    }

    ImGui::Render();
    int w, h;
    glfwGetFramebufferSize(window_, &w, &h);
    glViewport(0, 0, w, h);
    glClear(GL_COLOR_BUFFER_BIT);
    ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
    glfwSwapBuffers(window_);
    return true;
}
```

### Pattern 3: OpenGL Texture Upload for Live Preview

**What:** Create one `GLuint` texture per stream (RGB, colorized depth). Update it each frame with `glTexSubImage2D` for efficiency. Display via `ImGui::Image()`.
**When to use:** Inside `GuiPresenter` — upload the latest frame from a `std::atomic<std::shared_ptr<FrameSnapshot>>` updated by the capture thread.

```cpp
// Source: https://github.com/ocornut/imgui/wiki/Image-Loading-and-Displaying-Examples
// Initial creation (call once after GL context exists):
void GuiPresenter::create_textures() {
    // RGB texture
    glGenTextures(1, &rgb_tex_);
    glBindTexture(GL_TEXTURE_2D, rgb_tex_);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 640, 480,
                 0, GL_RGB, GL_UNSIGNED_BYTE, nullptr);

    // Depth colorized texture (same dims, RGB output after colormap)
    glGenTextures(1, &depth_tex_);
    // ... same setup with GL_RGB ...
}

// Per-frame update (call inside tick() when new frame available):
void GuiPresenter::upload_rgb(const uint8_t* rgb_data) {
    glBindTexture(GL_TEXTURE_2D, rgb_tex_);
    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
    glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 640, 480,
                    GL_RGB, GL_UNSIGNED_BYTE, rgb_data);
}

// Display:
ImGui::Image((ImTextureID)(intptr_t)rgb_tex_,
             ImVec2(preview_w_, preview_h_));
```

### Pattern 4: Jet Colormap for Z16 Depth

**What:** Convert uint16_t Z16 depth to RGB using the jet colormap (blue=near, red=far) before uploading to GPU. No OpenCV needed.
**When to use:** In the GUI presenter when preparing the depth texture.

```cpp
// Source: verified independently — standard jet colormap math
// Normalize depth value [min_depth, max_depth] to [0.0, 1.0], then map:
static void z16_to_jet_rgb(const uint16_t* depth, uint8_t* out_rgb,
                            int width, int height,
                            float depth_scale,
                            float near_m = 0.1f, float far_m = 10.0f) {
    for (int i = 0; i < width * height; ++i) {
        float d_m = depth[i] * depth_scale;
        float t = 0.0f;
        if (depth[i] != 0 && d_m >= near_m && d_m <= far_m) {
            t = (d_m - near_m) / (far_m - near_m);
            t = std::clamp(t, 0.0f, 1.0f);
        }
        // Jet colormap: blue(0.0) -> cyan -> green -> yellow -> red(1.0)
        float r = std::clamp(1.5f - std::abs(4.0f * t - 3.0f), 0.0f, 1.0f);
        float g = std::clamp(1.5f - std::abs(4.0f * t - 2.0f), 0.0f, 1.0f);
        float b = std::clamp(1.5f - std::abs(4.0f * t - 1.0f), 0.0f, 1.0f);
        out_rgb[i * 3 + 0] = static_cast<uint8_t>(r * 255.0f);
        out_rgb[i * 3 + 1] = static_cast<uint8_t>(g * 255.0f);
        out_rgb[i * 3 + 2] = static_cast<uint8_t>(b * 255.0f);
    }
}
```

### Pattern 5: Session Name Input with imgui_stdlib

**What:** Use `ImGui::InputText` with `std::string` via `misc/cpp/imgui_stdlib.h`. Validates non-empty before enabling record button.
**When to use:** In the controls panel of `GuiPresenter`.

```cpp
// Source: https://github.com/ocornut/imgui/blob/master/misc/cpp/imgui_stdlib.cpp
#include "imgui_stdlib.h"

std::string session_name_;
bool recording_ = false;

void render_controls_panel() {
    ImGui::InputText("Session Name", &session_name_);
    bool can_record = !session_name_.empty();
    if (!can_record) ImGui::BeginDisabled();
    if (ImGui::Button(recording_ ? "Stop (Space)" : "Start (Space)"))
        toggle_recording();
    if (!can_record) ImGui::EndDisabled();
}
```

### Pattern 6: systemd sd_notify Watchdog

**What:** Send READY=1 at startup, then send WATCHDOG=1 every half-interval on a dedicated thread. Send STATUS= with live stats. Send STOPPING=1 before exit.
**When to use:** `HeadlessPresenter` after camera is open and recording has started.

```cpp
// Source: https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html
//         https://www.freedesktop.org/software/systemd/man/latest/sd_watchdog_enabled.html
#include <systemd/sd-daemon.h>

void HeadlessPresenter::start_watchdog_thread() {
    uint64_t interval_us = 0;
    int enabled = sd_watchdog_enabled(0, &interval_us);
    if (enabled <= 0) return;  // No watchdog configured

    uint64_t ping_interval_us = interval_us / 2;  // Per spec: ping at half-interval

    sd_notify(0, "READY=1");

    watchdog_thread_ = std::thread([this, ping_interval_us]() {
        while (!shutdown_.load()) {
            sd_notify(0, "WATCHDOG=1");
            sd_notifyf(0, "STATUS=Frames: %llu | FPS: %.1f | Free: %llu MB",
                       (unsigned long long)stats_.written(),
                       stats_.write_fps(),
                       disk_free_mb());
            std::this_thread::sleep_for(
                std::chrono::microseconds(ping_interval_us));
        }
        sd_notify(0, "STOPPING=1");
    });
}
```

### Pattern 7: D-Bus Sleep Inhibitor Lock (sd-bus)

**What:** Call `org.freedesktop.login1.Manager.Inhibit` via sd-bus to block lid-close and sleep events. Hold the returned file descriptor; close it to release the lock.
**When to use:** `HeadlessPresenter::start()` — take before recording begins, release on shutdown.

```cpp
// Source: https://systemd.io/INHIBITOR_LOCKS/
//         https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html
// Note: sd_bus_call_method signature verified from systemd source patterns.
// Confidence: MEDIUM — no single authoritative C code sample found; derived from
// method signature "ssss" -> "h" and systemd's inhibit.c tool.
#include <systemd/sd-bus.h>

int inhibitor_fd_ = -1;
sd_bus* bus_ = nullptr;

bool HeadlessPresenter::take_inhibitor_lock() {
    int r = sd_bus_open_system(&bus_);
    if (r < 0) return false;

    sd_bus_message* reply = nullptr;
    sd_bus_error error = SD_BUS_ERROR_NULL;

    r = sd_bus_call_method(
        bus_,
        "org.freedesktop.login1",           // service
        "/org/freedesktop/login1",          // object path
        "org.freedesktop.login1.Manager",   // interface
        "Inhibit",                          // method
        &error,                             // error out
        &reply,                             // reply out
        "ssss",                             // signature: 4 strings in
        "handle-lid-switch:sleep",          // what to block
        "ego-recorder",                     // who (app name)
        "Recording in progress",            // why
        "block"                             // mode
    );

    if (r < 0) {
        sd_bus_error_free(&error);
        return false;
    }

    // Extract the returned file descriptor
    r = sd_bus_message_read(reply, "h", &inhibitor_fd_);
    // dup the fd before reply is freed
    inhibitor_fd_ = dup(inhibitor_fd_);
    sd_bus_message_unref(reply);
    sd_bus_error_free(&error);
    return (inhibitor_fd_ >= 0);
}

void HeadlessPresenter::release_inhibitor_lock() {
    if (inhibitor_fd_ >= 0) {
        close(inhibitor_fd_);
        inhibitor_fd_ = -1;
    }
    if (bus_) {
        sd_bus_unref(bus_);
        bus_ = nullptr;
    }
}
```

### Pattern 8: USB Reconnect Recovery

**What:** On `rs2::camera_disconnected_error`, stop and fully destroy the pipeline object. Wait, then recreate from scratch. Start a new recording file on reconnect.
**When to use:** Capture thread exception handler. Must recreate `rs2::pipeline` — calling `stop()` alone is insufficient.

```cpp
// Source: https://github.com/IntelRealSense/librealsense/issues/11881
// The destroy-and-recreate pattern is required; simply calling stop() and start()
// again leaves the pipeline in "already streaming" state.

// In capture thread:
try {
    CapturedFrame frame = camera->poll_frame();
    queue.push(std::move(frame));
} catch (const rs2::camera_disconnected_error&) {
    presenter_->on_camera_disconnect();
    // For headless: retry loop
    while (!shutdown_flag_.load()) {
        std::this_thread::sleep_for(std::chrono::seconds(2));
        try {
            camera.reset();  // Fully destroy old pipeline
            std::this_thread::sleep_for(std::chrono::milliseconds(500));
            camera = std::make_unique<RealSensePipeline>();
            camera->configure_and_start(warmup_frames_);
            // Start new recording file
            open_new_recording_file();
            presenter_->on_camera_reconnect();
            break;
        } catch (const rs2::error&) {
            // Camera not yet available — keep retrying
        }
    }
} catch (const rs2::error& e) {
    // Other RealSense errors — treat as fatal
    shutdown_flag_.store(true);
}
```

### Pattern 9: toml++ Config File

**What:** Load a TOML config file at startup; CLI flags override individual values. Use `value_or()` for all reads to supply defaults.
**When to use:** Before presenter or pipeline initialization.

```cpp
// Source: https://marzer.github.io/tomlplusplus/
#include <toml++/toml.hpp>

struct Config {
    std::string output_dir    = ".";
    int         jpeg_quality  = 90;
    int         zstd_level    = 1;
    int         queue_size    = 4;
    int         warmup_frames = 30;
    uint64_t    disk_min_mb   = 500;    // Stop recording below this threshold
};

Config load_config(const std::string& path) {
    Config cfg;
    try {
        auto tbl = toml::parse_file(path);
        cfg.output_dir   = tbl["output"]["dir"].value_or(cfg.output_dir);
        cfg.jpeg_quality = tbl["compression"]["jpeg_quality"].value_or(cfg.jpeg_quality);
        cfg.zstd_level   = tbl["compression"]["zstd_level"].value_or(cfg.zstd_level);
        cfg.disk_min_mb  = tbl["recording"]["disk_min_mb"].value_or((int64_t)cfg.disk_min_mb);
    } catch (const toml::parse_error&) {
        // Config file missing or malformed — use defaults
    }
    return cfg;
}
```

### Pattern 10: Disk Space Check (std::filesystem)

**What:** Check free bytes on the output directory's filesystem. Stop recording when below threshold.
**When to use:** Called periodically from the watchdog/stats thread (headless) or render loop (GUI overlay).

```cpp
// Source: https://en.cppreference.com/w/cpp/filesystem/space
#include <filesystem>

uint64_t free_mb_on_path(const std::string& path) {
    std::error_code ec;
    auto si = std::filesystem::space(path, ec);
    if (ec) return UINT64_MAX;  // Can't check — don't stop recording
    return si.available / (1024 * 1024);
}
```

### Anti-Patterns to Avoid

- **Uploading textures with glTexImage2D every frame:** Creates a new texture allocation each call. Use `glTexImage2D` once at creation, then `glTexSubImage2D` for per-frame updates.
- **Calling rs2::pipeline::stop() then start() for reconnect:** Leaves internal state in "already streaming". Always destroy the pipeline object (`camera.reset()`) between disconnect and reconnect.
- **Checking keyboard shortcuts without `!io.WantCaptureKeyboard`:** Space key fires inside InputText during session name entry. Always guard global shortcuts with this check.
- **Calling sd_notify READY=1 before the camera is open and recording:** systemd starts dependent units immediately on READY. Send it only after the recording file is open and first frames are flowing.
- **Blocking the GUI render thread for file writes:** The writer thread is separate (Phase 1). Never call FileWriter from the render thread.
- **Using `HandleLidSwitch=ignore` in logind.conf without the D-Bus inhibitor lock:** The logind.conf drop-in affects ALL sessions on the machine. The D-Bus inhibitor approach is process-scoped and releases automatically on process exit — much safer.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| GUI widget layout | Custom immediate-mode renderer | Dear ImGui 1.92.6 | Handles all input, layout, style, DPI scaling |
| std::string in InputText | char[] buffer + strncpy | imgui_stdlib.h `InputText(label, &str)` | Handles arbitrary length, resize callback built-in |
| TOML parsing | Custom key=value parser | toml++ v3.4.0 | TOML has edge cases (multiline strings, unicode, dates) |
| Watchdog keepalive timing | Custom timer calculation | `sd_watchdog_enabled(0, &usec)` returns the interval | Let systemd tell you the right interval |
| Disk space query | statvfs() manually | `std::filesystem::space()` | C++17, type-safe, error_code overload |
| USB reconnect detection | rs2::context device_hub polling | Catch `rs2::camera_disconnected_error` in capture thread | Error thrown synchronously when poll_frame() fails |

**Key insight:** Every "simple" problem in this list has hidden complexity (Unicode, race conditions, timer drift). The libraries exist because the naive implementations fail in production.

---

## Common Pitfalls

### Pitfall 1: GL Context Not Current When Creating Textures
**What goes wrong:** `glGenTextures` / `glTexImage2D` called before `glfwMakeContextCurrent` returns an error or silently produces texture handle 0.
**Why it happens:** GL operations require the context to be current on the calling thread.
**How to avoid:** Create textures only inside `GuiPresenter::start()` after `glfwMakeContextCurrent(window_)` and ImGui backends are initialized.
**Warning signs:** `ImGui::Image()` renders the font atlas instead of your texture (texture 0 is the ImGui font atlas in the OpenGL backend).

### Pitfall 2: Space Key Triggers Both InputText and Recording Toggle
**What goes wrong:** While typing in the session name field, pressing Space starts/stops recording instead of inserting a space character.
**Why it happens:** Global `ImGui::IsKeyPressed` fires regardless of which widget is focused.
**How to avoid:** Always guard: `if (!io.WantCaptureKeyboard) { if (ImGui::IsKeyPressed(ImGuiKey_Space)) ... }`. This is set to true when any text input widget has focus.
**Warning signs:** Recording toggles mid-session-name-entry.

### Pitfall 3: rs2::pipeline Stuck in "Already Streaming" After Reconnect
**What goes wrong:** After USB disconnect + reconnect, calling `pipeline.start(cfg)` throws "Device is already streaming!".
**Why it happens:** Calling `pipeline.stop()` alone doesn't destroy internal device state. The pipeline object must be fully destroyed and recreated.
**How to avoid:** Use `camera.reset()` (unique_ptr) or equivalent full destruction before re-creating. Add a 500ms sleep between stop and recreate.
**Warning signs:** Reconnect attempt always fails with "already streaming" regardless of how long you wait.

### Pitfall 4: sd_notify READY=1 Before Recording is Actually Running
**What goes wrong:** systemd reports service as active before the camera is open, allowing dependent services to start before the recorder is ready.
**Why it happens:** READY=1 is meant to signal that the service is fully operational.
**How to avoid:** Send READY=1 only after `pipeline.configure_and_start()` succeeds and the first recording file is open.
**Warning signs:** Dependent units start, try to interact with the recorder, and fail intermittently at boot.

### Pitfall 5: D-Bus inhibitor fd Lost on File Descriptor Duplication
**What goes wrong:** The fd returned in the sd-bus reply message is owned by the reply object and becomes invalid when the reply is freed.
**Why it happens:** `sd_bus_message_read(..., "h", &fd)` gives a reference to the fd inside the message; unref-ing the message closes it.
**How to avoid:** Call `inhibitor_fd_ = dup(inhibitor_fd_)` immediately after reading it, before calling `sd_bus_message_unref(reply)`.
**Warning signs:** Inhibitor lock appears to take but releases immediately; laptop sleeps on lid close anyway.

### Pitfall 6: Depth Texture Rendering Inverted or Wrong Colors
**What goes wrong:** Near objects appear red, far objects blue (inverted jet colormap), or colors look wrong.
**Why it happens:** Z16 value 0 means "no depth data" (not zero distance). Also, OpenGL's GL_UNPACK_ROW_LENGTH must be 0 or matching the row stride.
**How to avoid:** Skip pixels where `depth[i] == 0` (render as black). Set `glPixelStorei(GL_UNPACK_ROW_LENGTH, 0)` before `glTexSubImage2D`.
**Warning signs:** Black background artifacts, color inversion.

### Pitfall 7: Date-Based Output Directory Not Created Before Writer Opens File
**What goes wrong:** `FileWriter` constructor throws "No such file or directory" because `YYYY/MM/DD` subdirectory doesn't exist yet.
**Why it happens:** `std::ofstream` doesn't create parent directories.
**How to avoid:** Call `std::filesystem::create_directories(date_path)` before constructing `FileWriter`.
**Warning signs:** Headless service fails immediately on first boot of a new day.

---

## Code Examples

### GuiPresenter Init and Cleanup

```cpp
// Source: https://github.com/ocornut/imgui/blob/master/examples/example_glfw_opengl3/main.cpp
bool GuiPresenter::start() {
    glfwSetErrorCallback([](int, const char* desc) {
        fprintf(stderr, "GLFW error: %s\n", desc);
    });
    if (!glfwInit()) return false;

    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 0);
    window_ = glfwCreateWindow(1280, 720, "ego-recorder", nullptr, nullptr);
    if (!window_) { glfwTerminate(); return false; }

    glfwMakeContextCurrent(window_);
    glfwSwapInterval(1);  // vsync

    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGui::StyleColorsDark();
    ImGui_ImplGlfw_InitForOpenGL(window_, true);
    ImGui_ImplOpenGL3_Init("#version 130");

    create_textures();  // glGenTextures — after context is current
    return true;
}

void GuiPresenter::shutdown() {
    ImGui_ImplOpenGL3_Shutdown();
    ImGui_ImplGlfw_Shutdown();
    ImGui::DestroyContext();
    glDeleteTextures(1, &rgb_tex_);
    glDeleteTextures(1, &depth_tex_);
    glfwDestroyWindow(window_);
    glfwTerminate();
}
```

### systemd Unit File

```ini
# deploy/ego-recorder.service
[Unit]
Description=ego-recorder headless RGBD capture
After=local-fs.target

[Service]
Type=notify
User=ego-recorder
Group=ego-recorder
ExecStart=/usr/local/bin/ego-recorder --headless --config /etc/ego-recorder/config.toml
Restart=on-failure
RestartSec=5s
WatchdogSec=30s
NotifyAccess=main

# Prevent OOM kill
OOMScoreAdjust=-100

[Install]
WantedBy=multi-user.target
```

### logind.conf Drop-in

```ini
# deploy/50-ego-recorder-lid.conf
# Install to: /etc/systemd/logind.conf.d/50-ego-recorder-lid.conf
# Note: prefer D-Bus inhibitor lock (process-scoped) over this file (system-wide).
# Use this as a FALLBACK only, or remove it if D-Bus inhibitor is confirmed working.
[Login]
HandleLidSwitch=ignore
HandleLidSwitchExternalPower=ignore
```

### udev Rules

```udev
# deploy/99-ego-recorder.rules
# Install to: /etc/udev/rules.d/99-ego-recorder.rules
# Vendor 8086 = Intel, D435 = 0x0b07, D435i = 0x0b3a (common IDs; full list in librealsense 99-realsense-libusb.rules)

# Grant ego-recorder system user access, set plugdev group, disable autosuspend
SUBSYSTEMS=="usb", ATTRS{idVendor}=="8086", ATTRS{idProduct}=="0b07", \
    MODE:="0660", GROUP:="plugdev", \
    ATTR{power/control}="on"

SUBSYSTEMS=="usb", ATTRS{idVendor}=="8086", ATTRS{idProduct}=="0b3a", \
    MODE:="0660", GROUP:="plugdev", \
    ATTR{power/control}="on"
```

**Note:** The complete vendor/product ID list for all RealSense D400 series variants is maintained at `librealsense/config/99-realsense-libusb.rules` on GitHub. The install script should copy the full upstream rules file and add only the `ATTR{power/control}="on"` lines. Do not replicate the full list manually.

### Install Script Structure

```bash
#!/usr/bin/env bash
# deploy/install.sh
set -euo pipefail

BINARY=/usr/local/bin/ego-recorder
CONF_DIR=/etc/ego-recorder
RUN_DIR=/run/ego-recorder

# 1. Create system user (no home, no login shell)
if ! id ego-recorder &>/dev/null; then
    useradd --system \
            --no-create-home \
            --home-dir /dev/null \
            --shell /usr/sbin/nologin \
            --comment "ego-recorder service account" \
            ego-recorder
fi

# 2. Add to groups for USB device access
usermod -aG plugdev ego-recorder
usermod -aG video ego-recorder

# 3. Install binary
install -m 755 ego-recorder "${BINARY}"

# 4. Install config
mkdir -p "${CONF_DIR}"
install -m 644 config.toml.example "${CONF_DIR}/config.toml"

# 5. Create runtime directory
install -d -m 750 -o ego-recorder -g ego-recorder "${RUN_DIR}"

# 6. Install systemd unit
install -m 644 ego-recorder.service /etc/systemd/system/
systemctl daemon-reload

# 7. Install udev rules
install -m 644 99-ego-recorder.rules /etc/udev/rules.d/
udevadm control --reload-rules && udevadm trigger

# 8. Install logind drop-in (fallback)
mkdir -p /etc/systemd/logind.conf.d
install -m 644 50-ego-recorder-lid.conf /etc/systemd/logind.conf.d/
systemctl restart systemd-logind

# 9. Enable and start service
systemctl enable ego-recorder.service
systemctl start ego-recorder.service

echo "Installed. Status:"
systemctl status ego-recorder.service
```

### Date-Based Directory Creation

```cpp
// Source: std::filesystem (C++17 stdlib)
std::string make_date_path(const std::string& base_dir) {
    std::time_t now = std::time(nullptr);
    std::tm* t = std::localtime(&now);
    char buf[64];
    std::strftime(buf, sizeof(buf), "%Y/%m/%d", t);
    std::string full = base_dir + "/" + buf;
    std::filesystem::create_directories(full);  // No-op if already exists
    return full;
}
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| SDL2 window for ImGui | GLFW+OpenGL3 backend | ~2020 | GLFW is simpler for pure ImGui use; SDL2 better if audio/gamepad needed |
| ImTextureID as void* | `(ImTextureID)(intptr_t)GLuint` | ImGui 1.89 | Strict aliasing safe cast required |
| `int IsKeyPressed(int keycode)` | `IsKeyPressed(ImGuiKey_Space)` | ImGui 1.87 | Strongly-typed ImGuiKey enum; old int indices no longer work |
| plugdev group + udev MODE 666 | uaccess TAG (systemd 258+) | systemd 258 (2025) | systemd now recommends per-session ACLs via TAG+="uaccess" for desktop; group-based approach still works for system services |
| logind.conf global HandleLidSwitch=ignore | D-Bus inhibitor lock (process-scoped) | Long-standing best practice | Inhibitor releases automatically on process exit; logind.conf change is machine-wide permanent |

**Deprecated/outdated:**
- `ImGui_ImplOpenGL3_NewFrame()` before `ImGui_ImplGlfw_NewFrame()`: Order matters — GLFW backend first, then OpenGL backend.
- `glTexImage2D` every frame: Allocates GPU memory every call. Use once at init; `glTexSubImage2D` for updates.

---

## Open Questions

1. **D-Bus inhibitor C API — exact dup() requirement**
   - What we know: The fd returned in the reply message must be `dup()`-ed before freeing the reply, per sd-bus object ownership rules.
   - What's unclear: Whether sd-bus auto-dups the fd on `sd_bus_message_read("h", ...)` on newer libsystemd versions.
   - Recommendation: Always `dup()` immediately after reading; defensive and correct on all versions.

2. **OpenGL version floor on target systems**
   - What we know: GLFW hint `GLFW_CONTEXT_VERSION_MAJOR=3, MINOR=0` with `#version 130` GLSL works on all Intel integrated GPU laptops from ~2012+.
   - What's unclear: Whether the target system has mesa/llvmpipe or hardware GL. If only software rendering, 30fps texture uploads may lag.
   - Recommendation: Use `#version 130` (GL 3.0). If performance is insufficient, fall back to `#version 110` (GL 2.1) by also changing the imgui target to use `imgui_impl_opengl2.cpp`.

3. **RealSense udev product ID completeness**
   - What we know: D435 = 0x0b07, D435i = 0x0b3a are the most common IDs. The full upstream list covers 100+ variants.
   - What's unclear: Whether any D435 hardware variants have different product IDs.
   - Recommendation: Install script should copy the full upstream `99-realsense-libusb.rules` from librealsense and supplement with autosuspend rules, rather than maintaining a partial manual list.

4. **Frame snapshot thread safety between capture thread and GUI render thread**
   - What we know: Phase 1 pipeline uses a bounded queue for the writer thread. The GUI needs a separate live view of the latest frame without blocking the write pipeline.
   - What's unclear: How to share frames with the GUI without adding latency to the writer path.
   - Recommendation: Add a `std::atomic<std::shared_ptr<FrameSnapshot>>` (or equivalent lock-free latest-frame slot) updated by the capture thread. The GUI presenter reads it each render loop. The `FrameSnapshot` holds copies of rgb_data and depth_data (640x480 each, ~1.5MB total — acceptable).

---

## Sources

### Primary (HIGH confidence)
- Dear ImGui GitHub `examples/example_glfw_opengl3/main.cpp` — GLFW+OpenGL3 boilerplate, init/shutdown sequence
- Dear ImGui GitHub `misc/cpp/imgui_stdlib.h` — std::string InputText API
- Dear ImGui GitHub releases page — confirmed v1.92.6 released February 17, 2025
- toml++ official docs (marzer.github.io/tomlplusplus) — v3.4.0, FetchContent CMake snippet, parse_file API
- `freedesktop.org/software/systemd/man/latest/sd_notify.html` — READY=1, WATCHDOG=1, STATUS=, STOPPING=1 strings; sd_notify()/sd_notifyf() signatures
- `freedesktop.org/software/systemd/man/latest/sd_watchdog_enabled.html` — sd_watchdog_enabled() signature and half-interval recommendation
- `systemd.io/INHIBITOR_LOCKS/` — Inhibit() method parameters, fd lifecycle, block vs delay modes
- cppreference.com `std::filesystem::space` — space_info struct, available field, error_code overload
- ImGui wiki `Image-Loading-and-Displaying-Examples` — glGenTextures + glTexImage2D + ImGui::Image() pattern

### Secondary (MEDIUM confidence)
- librealsense GitHub issue #11881 — destroy-and-recreate pattern for USB reconnect recovery (community, verified against known RS2 behavior)
- freedesktop.org logind.conf man page — drop-in directory `/etc/systemd/logind.conf.d/`, `HandleLidSwitch=ignore`, drop-in naming convention
- sd-bus Inhibit() call pattern — derived from systemd inhibit.c source + org.freedesktop.login1 method signature "ssss"->"h"; no single authoritative C example found
- gist hacst/ee12cd91167aa55b19444fc74c91a8e8 — sd_notify watchdog keepalive structure (aligns with official docs)

### Tertiary (LOW confidence)
- udev RealSense product IDs (0x0b07, 0x0b3a) — from search results and community references; authoritative source is librealsense upstream `99-realsense-libusb.rules`, not independently verified for all variants
- useradd --system best practice — multiple consistent community sources; systemd's own recommendation to move away from plugdev toward uaccess TAG noted but not fully verified in current systemd 258 docs

---

## Metadata

**Confidence breakdown:**
- Standard stack (ImGui, GLFW, OpenGL3, toml++): HIGH — versions, CMake integration, APIs all verified from official sources
- systemd sd_notify / watchdog: HIGH — freedesktop official man pages
- D-Bus inhibitor lock (sd-bus C API): MEDIUM — method signature confirmed, exact `dup()` requirement and version behavior not independently verified
- USB reconnect pattern: MEDIUM — confirmed from community issue + aligns with RS2 pipeline object semantics
- udev product IDs: LOW — community-sourced; install script should pull upstream rules file
- Architecture patterns (IPresenter, frame snapshot): HIGH confidence on pattern correctness; exact field names are design decisions for the planner

**Research date:** 2026-02-19
**Valid until:** 2026-03-21 (30 days; toml++ and ImGui are stable; systemd APIs are stable)
