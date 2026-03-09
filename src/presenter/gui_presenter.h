#pragma once

#ifdef HAVE_GUI

// GuiPresenter -- Dear ImGui + GLFW + OpenGL3 interactive GUI mode.
//
// Displays live RGB and depth camera preview with recording controls,
// keyboard shortcuts, and a semi-transparent stats overlay.
//
// Lifecycle:
//   1. Construct with Config + four callback functions
//   2. Call start() -- creates GLFW window and initializes ImGui
//   3. Call tick() in the main loop -- renders one frame; returns false to quit
//   4. Call shutdown() -- cleans up GPU resources and destroys window
//
// Thread safety:
//   update_frame() is called from the capture thread while tick() runs on the
//   main thread.  A std::mutex protects the shared frame buffer (lock held only
//   for a memcpy -- typically <1 ms).

#include "presenter/ipresenter.h"
#include "config/config.h"

#include <GLFW/glfw3.h>

#include <functional>
#include <mutex>
#include <string>
#include <vector>
#include <cstdint>

class GuiPresenter : public IPresenter {
public:
    /// Construct a GuiPresenter.
    ///
    /// @param config                 Application configuration (output dir, session name, etc.)
    /// @param on_start_recording     Called when user clicks Start or presses Space (not recording)
    /// @param on_stop_recording      Called when user clicks Stop or presses Space (recording)
    /// @param on_session_name_changed Called with the new session name string whenever it changes
    /// @param on_reconnect_requested  Called when user clicks the Reconnect button during disconnect
    GuiPresenter(
        const Config& config,
        std::function<void()>              on_start_recording,
        std::function<void()>              on_stop_recording,
        std::function<void(const std::string&)> on_session_name_changed,
        std::function<void()>              on_reconnect_requested
    );

    ~GuiPresenter() override = default;

    // IPresenter lifecycle
    bool start()    override;
    bool tick()     override;
    void shutdown() override;

    // IPresenter camera events
    void on_camera_disconnect() override;
    void on_camera_reconnect()  override;

    // IPresenter stats push
    void update_stats(const Stats& stats) override;

    /// Called from capture thread: provide latest RGB + depth frame for display.
    ///
    /// Locks the shared frame buffer mutex, copies the pixel data, and returns.
    /// @param rgb_data    Raw RGB24 pixels (width * height * 3 bytes)
    /// @param depth_data  Raw Z16 depth pixels (width * height * 2 bytes)
    /// @param width       Frame width in pixels (typically 640)
    /// @param height      Frame height in pixels (typically 480)
    /// @param depth_scale Depth units (metres per count, e.g. 0.001 for 1 mm/count)
    void update_frame(
        const uint8_t*  rgb_data,
        const uint16_t* depth_data,
        int             width,
        int             height,
        float           depth_scale
    );

    /// Set the dataset name to display above session name input.
    void set_dataset_name(const std::string& name) { dataset_name_ = name; }

    /// Returns true while recording is active.
    bool        is_recording()  const { return recording_; }

    /// Returns the current session name entered by the user.
    std::string session_name()  const { return session_name_; }

private:
    // ---- View mode ----
    enum class ViewMode { RGB_ONLY, DEPTH_ONLY, SIDE_BY_SIDE };

    // ---- Depth colorization helper ----
    /// Convert Z16 depth pixels to RGB using turbo colormap with histogram equalization.
    /// Auto-ranges to 2nd-98th percentile of non-zero depth values per frame.
    void colorize_depth(
        const uint16_t* depth,
        uint8_t*        out_rgb,
        int             width,
        int             height
    );

    // ---- Config / callbacks ----
    const Config& config_;

    std::function<void()>                  on_start_recording_;
    std::function<void()>                  on_stop_recording_;
    std::function<void(const std::string&)> on_session_name_changed_;
    std::function<void()>                  on_reconnect_requested_;

    // ---- GLFW / OpenGL ----
    GLFWwindow* window_    = nullptr;
    unsigned int rgb_tex_  = 0;  ///< GL texture handle for RGB frame
    unsigned int depth_tex_= 0;  ///< GL texture handle for jet-colorized depth

    // ---- Shared frame buffer (capture thread writes, render thread reads) ----
    std::mutex              frame_mutex_;
    int                     frame_width_  = 1280;
    int                     frame_height_ = 720;
    float                   depth_scale_  = 0.001f;
    std::vector<uint8_t>    rgb_buf_;        ///< Latest RGB24 data (shared)
    std::vector<uint8_t>    depth_buf_;      ///< Latest Z16 data as bytes (shared)
    std::vector<uint8_t>    rgb_local_;      ///< Thread-local copy for rendering
    std::vector<uint8_t>    depth_local_;    ///< Thread-local copy for rendering
    std::vector<uint8_t>    jet_buf_;        ///< Jet-colorized depth RGB (local)
    bool                    frame_ready_ = false;

    // ---- GUI state ----
    std::string dataset_name_;
    std::string session_name_;
    bool        recording_    = false;
    bool        disconnected_ = false;
    ViewMode    view_mode_    = ViewMode::SIDE_BY_SIDE;

    // ---- Countdown before recording ----
    static constexpr int kCountdownSeconds = 3;
    bool   countdown_active_ = false;
    double countdown_start_  = 0.0;   // glfwGetTime() when countdown began

    // ---- Cached stats for overlay ----
    uint64_t stat_captured_      = 0;
    uint64_t stat_written_       = 0;
    uint64_t stat_dropped_       = 0;
    uint64_t stat_bytes_         = 0;
    double   stat_capture_fps_   = 0.0;
    double   stat_write_fps_     = 0.0;
    double   stat_elapsed_       = 0.0;
    double   stat_rec_elapsed_   = 0.0;
    bool     stat_is_recording_  = false;
};

#endif // HAVE_GUI
