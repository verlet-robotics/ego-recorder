#pragma once

// PreviewPresenter -- IPresenter for Tauri preview subprocess mode.
//
// Streams JPEG-encoded RGB + colorized depth frames to stdout as a binary
// protocol. Reads JSON commands from stdin to start/stop recording.
//
// Binary stdout protocol:
//   First line: JSON device info terminated by '\n'
//   Then repeating frames at ~5fps:
//     'R' u32_le(jpeg_size) <jpeg_bytes>   <- RGB JPEG (always)
//     'D' u32_le(jpeg_size) <jpeg_bytes>   <- colorized depth JPEG (when enabled)
//   Depth frames stop after recording ends (set_depth_enabled(false)).
//
// Text stdin protocol (line-oriented JSON):
//   {"cmd":"record","output_dir":"/path","session":"rec_001","crf":23,"warmup":30}
//   {"cmd":"stop"}
//
// stderr: stats lines (same format as headless mode) + DISCONNECTED/RECONNECTED sentinels.

#include "presenter/ipresenter.h"
#include "config/config.h"
#include "compression/jpeg_compressor.h"

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

/// JSON-parsed record command from stdin.
struct RecordCmd {
    std::string output_dir;
    std::string session;
    int crf = 23;
    int warmup = 30;
    std::string preset = "ultrafast";
};

class PreviewPresenter : public IPresenter {
public:
    PreviewPresenter(const Config& config,
                     const std::string& serial,
                     const std::string& usb,
                     bool has_imu,
                     int width,
                     int height);

    ~PreviewPresenter() override = default;

    // IPresenter lifecycle
    bool start()    override;
    bool tick()     override;
    void shutdown() override;

    // IPresenter camera events
    void on_camera_disconnect() override;
    void on_camera_reconnect()  override;

    // IPresenter stats push
    void update_stats(const Stats& stats) override;

    /// Called from capture thread: provide latest RGB + depth frame for preview.
    /// Fast memcpy only (~1ms) -- encoding happens on dedicated preview thread.
    /// Throttles to every 6th call (~5fps at 30fps capture).
    void update_frame(
        const uint8_t*  rgb_data,
        const uint16_t* depth_data,
        int             width,
        int             height,
        float           depth_scale
    );

    /// Enable/disable depth in the preview stream.
    /// Call set_depth_enabled(false) after recording stops.
    void set_depth_enabled(bool enabled) {
        depth_enabled_.store(enabled, std::memory_order_release);
    }

    bool has_pending_record() const { return pending_record_.load(std::memory_order_acquire); }
    bool consume_record_cmd(RecordCmd& out);
    bool consume_pending_stop();
    bool should_shutdown() const { return shutdown_.load(std::memory_order_acquire); }

private:
    void stdin_reader_loop();
    void preview_thread_loop();

    // ---- Config ----
    const Config& config_;
    std::string serial_;
    std::string usb_;
    bool has_imu_;
    int width_;
    int height_;

    // ---- Preview frame output ----
    std::mutex write_mutex_;
    std::atomic<uint64_t> frame_counter_{0};

    // Double buffer: capture thread writes, preview thread reads
    std::vector<uint8_t>  rgb_buf_[2];
    std::vector<uint8_t>  depth_buf_[2];
    std::atomic<int>      active_buf_{0};
    std::atomic<bool>     frame_ready_{false};

    // Working buffers
    static constexpr int kHalfWidth  = 640;
    static constexpr int kHalfHeight = 360;
    std::vector<uint8_t>  half_rgb_;        // 640x360x3 (downscaled RGB)
    std::vector<uint8_t>  colorized_buf_;   // full-res depth colorized (width*height*3)
    std::vector<uint8_t>  write_buf_;       // assembled frame(s) for atomic write

    // Depth streaming control
    std::atomic<bool> depth_enabled_{true};

    // ---- Threads ----
    std::thread stdin_thread_;
    std::thread preview_thread_;
    std::atomic<bool> shutdown_{false};

    // Pending commands
    std::mutex cmd_mutex_;
    std::atomic<bool> pending_record_{false};
    RecordCmd pending_record_cmd_;
    std::atomic<bool> pending_stop_{false};

    // ---- Stats ----
    std::atomic<uint64_t> stat_written_{0};
    std::atomic<uint64_t> stat_dropped_{0};
    std::atomic<uint64_t> stat_bytes_{0};
    double stat_capture_fps_ = 0.0;
    double stat_write_fps_ = 0.0;
    double stat_rec_elapsed_ = 0.0;
    std::atomic<bool> stat_is_recording_{false};
    uint64_t tick_counter_ = 0;
};
