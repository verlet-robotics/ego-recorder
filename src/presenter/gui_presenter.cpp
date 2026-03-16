#ifdef HAVE_GUI

// GuiPresenter implementation -- Dear ImGui + GLFW + OpenGL3.
//
// See gui_presenter.h for the full API contract.

#include "presenter/gui_presenter.h"

#include <imgui.h>
#include <imgui_internal.h>
#include <imgui_impl_glfw.h>
#include <imgui_impl_opengl3.h>
#include <imgui_stdlib.h>

#include <GLFW/glfw3.h>

#include "utils/audio_alert.h"
#include "utils/depth_colorizer.h"

#include <cmath>
#include <cstdio>
#include <algorithm>
#include <cstring>
#include <thread>

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

GuiPresenter::GuiPresenter(
    const Config& config,
    std::function<void()>                   on_start_recording,
    std::function<void()>                   on_stop_recording,
    std::function<void(const std::string&)> on_session_name_changed,
    std::function<void()>                   on_reconnect_requested
)
    : config_(config)
    , on_start_recording_(std::move(on_start_recording))
    , on_stop_recording_(std::move(on_stop_recording))
    , on_session_name_changed_(std::move(on_session_name_changed))
    , on_reconnect_requested_(std::move(on_reconnect_requested))
    , session_name_(config.session_name)
{
    const int w = config.frame_width;
    const int h = config.frame_height;

    rgb_buf_.resize(static_cast<size_t>(w * h * 3), 0);
    depth_buf_.resize(static_cast<size_t>(w * h * 2), 0);
    rgb_local_.resize(static_cast<size_t>(w * h * 3), 0);
    depth_local_.resize(static_cast<size_t>(w * h * 2), 0);
    jet_buf_.resize(static_cast<size_t>(w * h * 3), 0);

    frame_width_  = w;
    frame_height_ = h;
}

// ---------------------------------------------------------------------------
// start()
// ---------------------------------------------------------------------------

bool GuiPresenter::start()
{
    // 1. GLFW error callback
    glfwSetErrorCallback([](int code, const char* desc) {
        std::fprintf(stderr, "[GLFW error %d] %s\n", code, desc);
    });

    // 2. Initialize GLFW
    if (!glfwInit()) {
        std::fprintf(stderr, "GuiPresenter: glfwInit() failed\n");
        return false;
    }

    // 3. OpenGL 3.0 core hints
    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 0);

    // 4. Create window
    window_ = glfwCreateWindow(1280, 720, "ego-recorder", nullptr, nullptr);
    if (!window_) {
        std::fprintf(stderr, "GuiPresenter: glfwCreateWindow() failed\n");
        glfwTerminate();
        return false;
    }

    // 5. Make context current
    glfwMakeContextCurrent(window_);

    // 6. Enable vsync
    glfwSwapInterval(1);

    // 7. ImGui context
    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGui::StyleColorsDark();

    // 8. Platform / renderer backends
    ImGui_ImplGlfw_InitForOpenGL(window_, true);
    ImGui_ImplOpenGL3_Init("#version 130");

    // 9+10. Create GPU textures (MUST be after glfwMakeContextCurrent)
    const int w = frame_width_;
    const int h = frame_height_;

    // RGB texture
    glGenTextures(1, &rgb_tex_);
    glBindTexture(GL_TEXTURE_2D, rgb_tex_);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, w, h, 0, GL_RGB, GL_UNSIGNED_BYTE, nullptr);

    // Depth (jet-colorized) texture
    glGenTextures(1, &depth_tex_);
    glBindTexture(GL_TEXTURE_2D, depth_tex_);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, w, h, 0, GL_RGB, GL_UNSIGNED_BYTE, nullptr);

    return true;
}

// ---------------------------------------------------------------------------
// tick()  -- one render frame
// ---------------------------------------------------------------------------

bool GuiPresenter::tick()
{
    // 1. Should the window close?
    if (glfwWindowShouldClose(window_)) {
        return false;
    }

    // 2. Poll OS events
    glfwPollEvents();

    // 3. Begin new ImGui frame
    ImGui_ImplOpenGL3_NewFrame();
    ImGui_ImplGlfw_NewFrame();
    ImGui::NewFrame();

    // 4. Copy latest frame data under lock, then release before any GL work
    {
        std::lock_guard<std::mutex> lk(frame_mutex_);
        if (frame_ready_) {
            rgb_local_   = rgb_buf_;
            depth_local_ = depth_buf_;
        }
    }

    // 5. Upload RGB texture
    glBindTexture(GL_TEXTURE_2D, rgb_tex_);
    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
    glTexSubImage2D(GL_TEXTURE_2D, 0,
                    0, 0, frame_width_, frame_height_,
                    GL_RGB, GL_UNSIGNED_BYTE,
                    rgb_local_.data());

    // 6. Apply turbo colormap (histogram-equalized) to depth, then upload
    {
        const uint16_t* d16 = reinterpret_cast<const uint16_t*>(depth_local_.data());
        colorize_depth(d16, jet_buf_.data(), frame_width_, frame_height_);
    }
    glBindTexture(GL_TEXTURE_2D, depth_tex_);
    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
    glTexSubImage2D(GL_TEXTURE_2D, 0,
                    0, 0, frame_width_, frame_height_,
                    GL_RGB, GL_UNSIGNED_BYTE,
                    jet_buf_.data());

    // ------------------------------------------------------------------
    // 7. Camera preview window -- fills the entire OS window
    // ------------------------------------------------------------------
    int fb_w = 0, fb_h = 0;
    glfwGetFramebufferSize(window_, &fb_w, &fb_h);

    // Reserve bottom strip for controls panel (~120 px logical)
    const float controls_h = 120.0f;
    const float preview_h  = static_cast<float>(fb_h) - controls_h;

    // Set the preview window to fill the top portion
    ImGui::SetNextWindowPos(ImVec2(0.0f, 0.0f), ImGuiCond_Always);
    ImGui::SetNextWindowSize(ImVec2(static_cast<float>(fb_w), preview_h), ImGuiCond_Always);
    ImGui::SetNextWindowBgAlpha(0.0f);

    const ImGuiWindowFlags preview_flags =
        ImGuiWindowFlags_NoDecoration |
        ImGuiWindowFlags_NoInputs     |
        ImGuiWindowFlags_NoMove       |
        ImGuiWindowFlags_NoScrollbar  |
        ImGuiWindowFlags_NoBringToFrontOnFocus;

    ImGui::Begin("Preview", nullptr, preview_flags);
    {
        const ImVec2 avail = ImGui::GetContentRegionAvail();
        const float  aspect = (frame_height_ > 0)
                              ? static_cast<float>(frame_width_) / static_cast<float>(frame_height_)
                              : (4.0f / 3.0f);

        auto fit_size = [&](float max_w, float max_h) -> ImVec2 {
            float w = max_w;
            float h = w / aspect;
            if (h > max_h) { h = max_h; w = h * aspect; }
            return ImVec2(w, h);
        };

        const ImTextureID rgb_id   = (ImTextureID)(intptr_t)rgb_tex_;
        const ImTextureID depth_id = (ImTextureID)(intptr_t)depth_tex_;

        if (view_mode_ == ViewMode::RGB_ONLY) {
            ImVec2 sz = fit_size(avail.x, avail.y);
            ImGui::Image(rgb_id, sz);
        } else if (view_mode_ == ViewMode::DEPTH_ONLY) {
            ImVec2 sz = fit_size(avail.x, avail.y);
            ImGui::Image(depth_id, sz);
        } else {
            // SIDE_BY_SIDE: each panel gets half the width
            const float half_w = avail.x * 0.5f - 4.0f; // 4px gutter
            ImVec2 sz = fit_size(half_w, avail.y);
            ImGui::Image(rgb_id, sz);
            ImGui::SameLine(0.0f, 8.0f);
            ImGui::Image(depth_id, sz);
        }
    }
    ImGui::End();

    // ------------------------------------------------------------------
    // 8. Controls panel -- pinned at bottom
    // ------------------------------------------------------------------
    ImGui::SetNextWindowPos(ImVec2(0.0f, preview_h), ImGuiCond_Always);
    ImGui::SetNextWindowSize(ImVec2(static_cast<float>(fb_w), controls_h), ImGuiCond_Always);

    const ImGuiWindowFlags ctrl_flags =
        ImGuiWindowFlags_NoDecoration |
        ImGuiWindowFlags_NoMove       |
        ImGuiWindowFlags_NoScrollbar;

    ImGui::Begin("Controls", nullptr, ctrl_flags);
    {
        // Dataset label (shown if recording to a dataset directory)
        if (!dataset_name_.empty()) {
            ImGui::TextColored(ImVec4(0.4f, 0.8f, 1.0f, 1.0f),
                "Dataset: %s", dataset_name_.c_str());
            if (episode_count_ > 0) {
                ImGui::SameLine();
                ImGui::TextColored(ImVec4(0.4f, 1.0f, 0.4f, 1.0f),
                    "  Episode %d", episode_count_);
            }
        } else if (episode_count_ > 0) {
            ImGui::TextColored(ImVec4(0.4f, 1.0f, 0.4f, 1.0f),
                "Episode %d", episode_count_);
        }

        // Session name input
        ImGui::SetNextItemWidth(300.0f);
        if (ImGui::InputText("Session Name", &session_name_)) {
            if (on_session_name_changed_) {
                on_session_name_changed_(session_name_);
            }
        }
        ImGui::SameLine();

        // Start / Stop button (disabled when session name is empty)
        const bool name_empty = session_name_.empty();
        if (name_empty) ImGui::BeginDisabled();

        const char* btn_label = recording_
            ? "Stop Recording (Space/Esc)"
            : (countdown_active_ ? "Cancel Countdown (Esc)" : "Start Recording (Space)");

        if (ImGui::Button(btn_label)) {
            if (recording_) {
                recording_ = false;
                if (on_stop_recording_) on_stop_recording_();
            } else if (countdown_active_) {
                countdown_active_    = false;
                countdown_last_beep_ = -1;
            } else {
                countdown_active_    = true;
                countdown_start_     = glfwGetTime();
                countdown_last_beep_ = -1;
            }
        }
        if (name_empty) ImGui::EndDisabled();

        // Camera disconnect banner + reconnect button
        if (disconnected_) {
            ImGui::PushStyleColor(ImGuiCol_Text, ImVec4(1.0f, 0.3f, 0.3f, 1.0f));
            ImGui::Text("  Camera Disconnected");
            ImGui::PopStyleColor();
            ImGui::SameLine();
            if (ImGui::Button("Reconnect")) {
                if (on_reconnect_requested_) on_reconnect_requested_();
            }
        }
    }
    ImGui::End();

    // ------------------------------------------------------------------
    // 9. Stats overlay -- semi-transparent, top-right corner
    // ------------------------------------------------------------------
    {
        const float overlay_margin = 10.0f;
        ImGui::SetNextWindowPos(
            ImVec2(static_cast<float>(fb_w) - overlay_margin, overlay_margin),
            ImGuiCond_Always,
            ImVec2(1.0f, 0.0f)  // pivot: right edge, top
        );
        ImGui::SetNextWindowBgAlpha(0.5f);

        const ImGuiWindowFlags overlay_flags =
            ImGuiWindowFlags_NoDecoration         |
            ImGuiWindowFlags_AlwaysAutoResize      |
            ImGuiWindowFlags_NoFocusOnAppearing    |
            ImGuiWindowFlags_NoNav                 |
            ImGuiWindowFlags_NoMove;

        ImGui::Begin("Stats", nullptr, overlay_flags);

        // Camera section (always visible)
        ImGui::Text("Camera FPS: %.1f", stat_capture_fps_);
        ImGui::Separator();

        if (stat_is_recording_) {
            // Active recording section
            const int rec_s = static_cast<int>(stat_rec_elapsed_);
            ImGui::TextColored(ImVec4(1.0f, 0.3f, 0.3f, 1.0f),
                "REC %02d:%02d", rec_s / 60, rec_s % 60);

            ImGui::Text("Frames:    %llu", (unsigned long long)stat_written_);
            ImGui::Text("Dropped:   %llu", (unsigned long long)stat_dropped_);
            ImGui::Text("Write FPS: %.1f", stat_write_fps_);

            const double bytes_d = static_cast<double>(stat_bytes_);
            if (bytes_d >= 1e9) {
                ImGui::Text("File size: %.2f GB", bytes_d / 1e9);
            } else {
                ImGui::Text("File size: %.1f MB", bytes_d / 1e6);
            }
        } else if (stat_written_ > 0) {
            // Idle with previous recording data
            const int rec_s = static_cast<int>(stat_rec_elapsed_);
            ImGui::Text("Last rec:  %llu frames", (unsigned long long)stat_written_);
            ImGui::Text("Duration:  %02d:%02d", rec_s / 60, rec_s % 60);

            const double bytes_d = static_cast<double>(stat_bytes_);
            if (bytes_d >= 1e9) {
                ImGui::Text("File size: %.2f GB", bytes_d / 1e9);
            } else {
                ImGui::Text("File size: %.1f MB", bytes_d / 1e6);
            }
        } else {
            ImGui::Text("Ready to record");
        }

        ImGui::End();
    }

    // ------------------------------------------------------------------
    // 9a. Recording viewfinder overlay -- top-left, visible during recording
    // ------------------------------------------------------------------
    if (stat_is_recording_) {
        const float overlay_margin = 10.0f;
        ImGui::SetNextWindowPos(
            ImVec2(overlay_margin, overlay_margin),
            ImGuiCond_Always,
            ImVec2(0.0f, 0.0f)  // pivot: left edge, top
        );
        ImGui::SetNextWindowBgAlpha(0.4f);

        const ImGuiWindowFlags rec_flags =
            ImGuiWindowFlags_NoDecoration         |
            ImGuiWindowFlags_AlwaysAutoResize      |
            ImGuiWindowFlags_NoFocusOnAppearing    |
            ImGuiWindowFlags_NoNav                 |
            ImGuiWindowFlags_NoMove                |
            ImGuiWindowFlags_NoInputs;

        ImGui::Begin("RecIndicator", nullptr, rec_flags);
        {
            // Blinking red dot + REC label
            const bool blink_on = std::fmod(glfwGetTime(), 1.6) < 0.8;
            ImVec2 cursor = ImGui::GetCursorScreenPos();

            if (blink_on) {
                ImGui::GetWindowDrawList()->AddCircleFilled(
                    ImVec2(cursor.x + 8.0f, cursor.y + 8.0f),
                    6.0f,
                    IM_COL32(255, 50, 50, 255)
                );
            }
            ImGui::Dummy(ImVec2(18.0f, 16.0f));
            ImGui::SameLine();
            ImGui::TextColored(ImVec4(1.0f, 0.2f, 0.2f, 1.0f), "REC");
            ImGui::SameLine();

            // Timer MM:SS in larger font
            const int rec_s = static_cast<int>(stat_rec_elapsed_);
            ImGui::SetWindowFontScale(2.0f);
            ImGui::Text("%02d:%02d", rec_s / 60, rec_s % 60);
            ImGui::SetWindowFontScale(1.0f);

            // Frame count
            ImGui::Text("Frames: %llu", (unsigned long long)stat_written_);

            // Episode count (if any completed)
            if (episode_count_ > 0) {
                ImGui::Text("Episode %d", episode_count_ + 1);
            }
        }
        ImGui::End();
    }

    // ------------------------------------------------------------------
    // 9b. Countdown overlay -- large centered number over the preview
    // ------------------------------------------------------------------
    if (countdown_active_) {
        const double elapsed = glfwGetTime() - countdown_start_;
        const int remaining  = kCountdownSeconds - static_cast<int>(elapsed);

        if (remaining <= 0) {
            // Countdown finished -- start recording
            countdown_active_    = false;
            countdown_last_beep_ = -1;
            recording_ = true;
            // "Go" beep: higher pitch, longer duration
            std::thread([]{ play_beep(1200, 300); }).detach();
            if (on_start_recording_) on_start_recording_();
        } else {
            // Play beep for each new countdown second (fire-and-forget thread)
            if (remaining != countdown_last_beep_) {
                countdown_last_beep_ = remaining;
                std::thread([]{ play_beep(800, 150); }).detach();
            }
            // Draw centered countdown number
            char count_text[16];
            std::snprintf(count_text, sizeof(count_text), "%d", remaining);

            // Semi-transparent full-screen backdrop over the preview area
            ImGui::SetNextWindowPos(ImVec2(0.0f, 0.0f), ImGuiCond_Always);
            ImGui::SetNextWindowSize(ImVec2(static_cast<float>(fb_w), preview_h), ImGuiCond_Always);
            ImGui::SetNextWindowBgAlpha(0.3f);

            const ImGuiWindowFlags cd_flags =
                ImGuiWindowFlags_NoDecoration      |
                ImGuiWindowFlags_NoInputs          |
                ImGuiWindowFlags_NoMove            |
                ImGuiWindowFlags_NoScrollbar       |
                ImGuiWindowFlags_NoBringToFrontOnFocus;

            ImGui::Begin("Countdown", nullptr, cd_flags);
            {
                // Scale font for the big number
                const float font_scale = 8.0f;
                ImGui::SetWindowFontScale(font_scale);

                const ImVec2 text_size = ImGui::CalcTextSize(count_text);
                const ImVec2 avail_cd  = ImGui::GetContentRegionAvail();

                // Center the number
                ImGui::SetCursorPos(ImVec2(
                    (avail_cd.x - text_size.x) * 0.5f,
                    (avail_cd.y - text_size.y) * 0.5f
                ));

                ImGui::TextColored(ImVec4(1.0f, 1.0f, 1.0f, 0.9f), "%s", count_text);
                ImGui::SetWindowFontScale(1.0f);
            }
            ImGui::End();
        }
    }

    // ------------------------------------------------------------------
    // 10. Keyboard shortcuts
    // ------------------------------------------------------------------
    // Space is a global recording toggle -- it must work even when the
    // Session Name InputText has keyboard focus.  We check it outside the
    // WantCaptureKeyboard guard so the user can press Space at any time.
    // If the InputText was active we clear it to avoid typing a stray ' '.
    {
        ImGuiIO& io = ImGui::GetIO();

        if (ImGui::IsKeyPressed(ImGuiKey_Space)) {
            if (!session_name_.empty()) {
                if (io.WantCaptureKeyboard) {
                    ImGui::ClearActiveID();
                }
                if (recording_) {
                    recording_ = false;
                    if (on_stop_recording_) on_stop_recording_();
                } else if (countdown_active_) {
                    countdown_active_    = false;
                    countdown_last_beep_ = -1;
                } else {
                    countdown_active_    = true;
                    countdown_start_     = glfwGetTime();
                    countdown_last_beep_ = -1;
                }
            }
        }

        if (!io.WantCaptureKeyboard) {
            if (ImGui::IsKeyPressed(ImGuiKey_Tab)) {
                switch (view_mode_) {
                    case ViewMode::RGB_ONLY:    view_mode_ = ViewMode::DEPTH_ONLY;   break;
                    case ViewMode::DEPTH_ONLY:  view_mode_ = ViewMode::SIDE_BY_SIDE; break;
                    case ViewMode::SIDE_BY_SIDE: view_mode_ = ViewMode::RGB_ONLY;    break;
                }
            }
            if (ImGui::IsKeyPressed(ImGuiKey_Escape)) {
                if (countdown_active_) {
                    countdown_active_    = false;
                    countdown_last_beep_ = -1;
                } else if (recording_) {
                    recording_ = false;
                    if (on_stop_recording_) on_stop_recording_();
                } else {
                    ImGui::Render();
                    int fw2 = 0, fh2 = 0;
                    glfwGetFramebufferSize(window_, &fw2, &fh2);
                    glViewport(0, 0, fw2, fh2);
                    glClear(GL_COLOR_BUFFER_BIT);
                    ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
                    glfwSwapBuffers(window_);
                    return false;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 11. Render
    // ------------------------------------------------------------------
    ImGui::Render();
    {
        int fw = 0, fh = 0;
        glfwGetFramebufferSize(window_, &fw, &fh);
        glViewport(0, 0, fw, fh);
        glClear(GL_COLOR_BUFFER_BIT);
        ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
    }
    glfwSwapBuffers(window_);

    return true;
}

// ---------------------------------------------------------------------------
// shutdown()
// ---------------------------------------------------------------------------

void GuiPresenter::shutdown()
{
    ImGui_ImplOpenGL3_Shutdown();
    ImGui_ImplGlfw_Shutdown();
    ImGui::DestroyContext();

    if (depth_tex_) { glDeleteTextures(1, &depth_tex_); depth_tex_ = 0; }
    if (rgb_tex_)   { glDeleteTextures(1, &rgb_tex_);   rgb_tex_   = 0; }

    if (window_) {
        glfwDestroyWindow(window_);
        window_ = nullptr;
    }
    glfwTerminate();
}

// ---------------------------------------------------------------------------
// IPresenter camera events
// ---------------------------------------------------------------------------

void GuiPresenter::on_camera_disconnect()
{
    disconnected_ = true;
}

void GuiPresenter::on_camera_reconnect()
{
    disconnected_ = false;
}

// ---------------------------------------------------------------------------
// update_stats()  -- main thread / other thread pushes stats
// ---------------------------------------------------------------------------

void GuiPresenter::update_stats(const Stats& stats)
{
    stat_captured_      = stats.captured();
    stat_written_       = stats.written();
    stat_dropped_       = stats.dropped();
    stat_bytes_         = stats.total_bytes();
    stat_capture_fps_   = stats.capture_fps();
    stat_write_fps_     = stats.write_fps();
    stat_elapsed_       = stats.elapsed_seconds();
    stat_rec_elapsed_   = stats.recording_elapsed_seconds();
    stat_is_recording_  = stats.is_recording();
}

// ---------------------------------------------------------------------------
// update_frame()  -- called from capture thread
// ---------------------------------------------------------------------------

void GuiPresenter::update_frame(
    const uint8_t*  rgb_data,
    const uint16_t* depth_data,
    int             width,
    int             height,
    float           depth_scale
)
{
    const size_t rgb_bytes   = static_cast<size_t>(width * height * 3);
    const size_t depth_bytes = static_cast<size_t>(width * height * 2);

    std::lock_guard<std::mutex> lk(frame_mutex_);

    frame_width_  = width;
    frame_height_ = height;
    depth_scale_  = depth_scale;

    if (rgb_buf_.size() != rgb_bytes) {
        rgb_buf_.resize(rgb_bytes);
    }
    if (depth_buf_.size() != depth_bytes) {
        depth_buf_.resize(depth_bytes);
        depth_local_.resize(depth_bytes, 0);
    }

    std::memcpy(rgb_buf_.data(), rgb_data, rgb_bytes);
    std::memcpy(depth_buf_.data(), depth_data, depth_bytes);
    frame_ready_ = true;
}

// GuiPresenter::colorize_depth delegates to shared utility in depth_colorizer.cpp
// (the kTurboLUT and algorithm now live in utils/depth_colorizer.h/.cpp)
void GuiPresenter::colorize_depth(
    const uint16_t* depth,
    uint8_t*        out_rgb,
    int             width,
    int             height
)
{
    ::colorize_depth(depth, out_rgb, width, height);
}

#endif // HAVE_GUI
