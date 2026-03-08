#ifdef HAVE_GUI

// GuiPresenter implementation -- Dear ImGui + GLFW + OpenGL3.
//
// See gui_presenter.h for the full API contract.

#include "presenter/gui_presenter.h"

#include <imgui.h>
#include <imgui_impl_glfw.h>
#include <imgui_impl_opengl3.h>
#include <imgui_stdlib.h>

#include <GLFW/glfw3.h>

#include <cmath>
#include <cstdio>
#include <algorithm>
#include <cstring>

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
    const int w = 640;
    const int h = 480;

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
            : "Start Recording (Space)";

        if (ImGui::Button(btn_label)) {
            if (recording_) {
                recording_ = false;
                if (on_stop_recording_) on_stop_recording_();
            } else {
                recording_ = true;
                if (on_start_recording_) on_start_recording_();
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
    // 10. Keyboard shortcuts (only when ImGui is not eating keyboard)
    // ------------------------------------------------------------------
    {
        ImGuiIO& io = ImGui::GetIO();
        if (!io.WantCaptureKeyboard) {
            if (ImGui::IsKeyPressed(ImGuiKey_Space)) {
                if (!session_name_.empty()) {
                    if (recording_) {
                        recording_ = false;
                        if (on_stop_recording_) on_stop_recording_();
                    } else {
                        recording_ = true;
                        if (on_start_recording_) on_start_recording_();
                    }
                }
            }
            if (ImGui::IsKeyPressed(ImGuiKey_Tab)) {
                // Cycle: RGB_ONLY -> DEPTH_ONLY -> SIDE_BY_SIDE -> RGB_ONLY
                switch (view_mode_) {
                    case ViewMode::RGB_ONLY:    view_mode_ = ViewMode::DEPTH_ONLY;   break;
                    case ViewMode::DEPTH_ONLY:  view_mode_ = ViewMode::SIDE_BY_SIDE; break;
                    case ViewMode::SIDE_BY_SIDE: view_mode_ = ViewMode::RGB_ONLY;    break;
                }
            }
            if (ImGui::IsKeyPressed(ImGuiKey_Escape)) {
                if (recording_) {
                    // Cancel running recording
                    recording_ = false;
                    if (on_stop_recording_) on_stop_recording_();
                } else {
                    // No recording active -- quit
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

// ---------------------------------------------------------------------------
// Turbo colormap LUT (Google, 2019) -- 256 entries, perceptually uniform.
// Source: https://gist.github.com/mikhailov-work/0d177465a8151eb6ede1768d51d476c7
// License: Apache 2.0
// ---------------------------------------------------------------------------
static const uint8_t kTurboLUT[256][3] = {
    {48,18,59},{50,21,67},{51,24,74},{52,27,81},{53,30,88},{54,33,95},{55,36,102},{56,39,109},
    {57,42,115},{58,45,121},{59,47,128},{60,50,134},{61,53,139},{62,56,145},{63,58,150},{63,61,156},
    {64,64,161},{65,66,166},{65,69,171},{66,72,176},{66,74,180},{67,77,185},{68,79,189},{68,82,193},
    {69,84,197},{69,87,201},{70,89,205},{70,92,209},{71,94,212},{71,97,216},{71,99,219},{72,101,223},
    {72,104,226},{72,106,229},{72,108,232},{72,111,235},{72,113,237},{72,115,240},{72,118,242},{72,120,244},
    {72,122,246},{71,125,248},{71,127,250},{71,129,251},{71,131,253},{70,134,254},{70,136,255},{70,138,255},
    {69,140,255},{69,142,255},{68,145,255},{68,147,255},{67,149,255},{66,151,255},{66,153,255},{65,155,254},
    {64,157,254},{63,159,253},{63,161,252},{62,163,252},{61,165,251},{60,167,250},{59,169,249},{58,170,248},
    {57,172,247},{56,174,246},{55,176,244},{54,178,243},{53,179,241},{52,181,240},{51,183,238},{50,184,236},
    {49,186,234},{48,188,233},{47,189,231},{46,191,229},{45,192,227},{44,194,225},{43,195,223},{42,197,221},
    {42,198,218},{41,200,216},{40,201,214},{39,203,212},{38,204,209},{37,206,207},{36,207,205},{35,208,202},
    {34,210,200},{33,211,197},{33,213,195},{32,214,192},{31,215,190},{31,217,187},{30,218,185},{29,219,182},
    {29,221,180},{28,222,177},{28,223,174},{27,225,172},{27,226,169},{26,227,166},{26,228,164},{25,230,161},
    {25,231,158},{24,232,155},{24,233,153},{24,234,150},{23,236,147},{23,237,144},{23,238,142},{23,239,139},
    {23,240,136},{23,241,133},{24,242,131},{24,243,128},{24,243,125},{25,244,122},{25,245,120},{26,246,117},
    {27,247,114},{27,248,111},{28,248,108},{29,249,106},{30,250,103},{31,250,100},{32,251,97},{33,252,95},
    {35,252,92},{36,253,89},{37,253,87},{39,254,84},{40,254,81},{42,254,79},{43,255,76},{45,255,73},
    {47,255,71},{49,255,68},{51,255,66},{53,255,63},{55,255,61},{57,255,58},{60,255,56},{62,255,54},
    {64,255,51},{67,254,49},{69,254,47},{72,254,45},{74,254,43},{77,253,41},{79,253,39},{82,253,37},
    {84,252,36},{87,252,34},{89,251,33},{92,251,31},{94,250,30},{97,250,28},{99,249,27},{102,249,26},
    {105,248,25},{107,248,24},{110,247,23},{112,246,22},{115,246,21},{118,245,21},{120,245,20},{123,244,20},
    {125,243,19},{128,243,19},{131,242,19},{133,241,19},{136,241,18},{138,240,18},{141,239,18},{144,239,18},
    {146,238,19},{149,237,19},{151,237,19},{154,236,20},{156,235,20},{159,235,21},{161,234,21},{164,233,22},
    {166,233,23},{169,232,23},{171,231,24},{174,230,25},{176,230,26},{179,229,27},{181,228,28},{183,227,29},
    {186,227,30},{188,226,31},{191,225,33},{193,224,34},{195,223,35},{198,223,37},{200,222,38},{202,221,40},
    {205,220,41},{207,219,43},{209,219,44},{212,218,46},{214,217,48},{216,216,49},{218,216,51},{221,215,53},
    {223,214,55},{225,213,57},{227,213,59},{229,212,61},{231,211,63},{233,211,65},{235,210,67},{237,209,69},
    {239,209,71},{240,208,74},{242,207,76},{244,207,78},{245,206,80},{247,206,83},{248,205,85},{250,204,87},
    {251,204,90},{252,203,92},{254,203,95},{255,202,97},{255,201,100},{255,201,102},{255,200,105},{255,200,108},
    {255,199,110},{255,198,113},{255,198,116},{255,197,118},{255,197,121},{255,196,124},{255,195,127},{255,195,129},
    {255,194,132},{255,194,135},{255,193,138},{255,192,141},{255,192,143},{255,191,146},{255,190,149},{255,190,152},
    {254,189,154},{254,189,157},{254,188,160},{253,187,163},{253,187,165},{252,186,168},{252,185,171},{251,185,174},
};

// colorize_depth()  -- histogram-equalized turbo colormap
// ---------------------------------------------------------------------------
// Two-pass: (1) build histogram, find 2nd/98th percentile range,
//           (2) normalize each pixel to [0,1] within that range and look up turbo LUT.
// This matches the approach used by librealsense's rs2::colorizer.

void GuiPresenter::colorize_depth(
    const uint16_t* depth,
    uint8_t*        out_rgb,
    int             width,
    int             height
)
{
    const int n = width * height;

    // Pass 1: histogram of non-zero depth values
    uint32_t hist[65536] = {};
    int valid_count = 0;
    for (int i = 0; i < n; ++i) {
        if (depth[i] != 0) {
            hist[depth[i]]++;
            valid_count++;
        }
    }

    // If no valid depth, fill black and return
    if (valid_count == 0) {
        std::memset(out_rgb, 0, n * 3);
        return;
    }

    // Find 2nd and 98th percentile raw values
    const int low_cut  = valid_count * 2  / 100;
    const int high_cut = valid_count * 98 / 100;
    uint16_t d_min = 1, d_max = 65535;
    int cum = 0;
    bool found_min = false;
    for (int v = 1; v < 65536; ++v) {
        cum += hist[v];
        if (!found_min && cum >= low_cut) {
            d_min = static_cast<uint16_t>(v);
            found_min = true;
        }
        if (cum >= high_cut) {
            d_max = static_cast<uint16_t>(v);
            break;
        }
    }

    // Avoid division by zero
    if (d_max <= d_min) d_max = d_min + 1;

    const float inv_range = 255.0f / static_cast<float>(d_max - d_min);

    // Pass 2: colorize with turbo LUT
    for (int i = 0; i < n; ++i) {
        const uint16_t raw = depth[i];
        if (raw == 0) {
            out_rgb[i * 3 + 0] = 0;
            out_rgb[i * 3 + 1] = 0;
            out_rgb[i * 3 + 2] = 0;
            continue;
        }

        // Normalize to [0, 255] LUT index
        int idx;
        if (raw <= d_min) idx = 0;
        else if (raw >= d_max) idx = 255;
        else idx = static_cast<int>(static_cast<float>(raw - d_min) * inv_range + 0.5f);

        out_rgb[i * 3 + 0] = kTurboLUT[idx][0];
        out_rgb[i * 3 + 1] = kTurboLUT[idx][1];
        out_rgb[i * 3 + 2] = kTurboLUT[idx][2];
    }
}

#endif // HAVE_GUI
