// PreviewPresenter implementation -- binary frame streaming to stdout,
// JSON command parsing from stdin.
//
// See preview_presenter.h for the full protocol specification.
//
// Architecture:
//   - update_frame() runs on capture thread: fast memcpy into double buffer (~1ms)
//   - preview_thread_loop() runs on dedicated thread: downscale RGB + colorize depth + encode + atomic fwrite
//   - stdin_reader_loop() uses poll() to detect parent death and check shutdown every 200ms

#include "presenter/preview_presenter.h"
#include "utils/depth_colorizer.h"

#include <nlohmann/json.hpp>

#include <cstdio>
#include <cstring>
#include <iostream>
#include <string>
#include <csignal>

#include <poll.h>
#include <unistd.h>

using json = nlohmann::json;

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

PreviewPresenter::PreviewPresenter(
    const Config& config,
    const std::string& serial,
    const std::string& usb,
    bool has_imu,
    int width,
    int height
)
    : config_(config)
    , serial_(serial)
    , usb_(usb)
    , has_imu_(has_imu)
    , width_(width)
    , height_(height)
    , half_w_(width / 2)
    , half_h_(height / 2)
{
    const size_t full_pixels = static_cast<size_t>(width * height);
    // Double buffers for capture thread -> preview thread handoff
    rgb_buf_[0].resize(full_pixels * 3, 0);
    rgb_buf_[1].resize(full_pixels * 3, 0);
    depth_buf_[0].resize(full_pixels * 2, 0);
    depth_buf_[1].resize(full_pixels * 2, 0);

    // Half-res RGB working buffer
    const size_t half_pixels = static_cast<size_t>(half_w_ * half_h_);
    half_rgb_.resize(half_pixels * 3, 0);

    // Full-res depth colorization buffer
    colorized_buf_.resize(full_pixels * 3, 0);
}

// ---------------------------------------------------------------------------
// start()
// ---------------------------------------------------------------------------

bool PreviewPresenter::start()
{
    // Write device info JSON line to stdout
    json info;
    info["serial"] = serial_;
    info["usb"] = usb_;
    info["hasImu"] = has_imu_;
    info["width"] = width_;
    info["height"] = height_;

    std::string info_line = info.dump() + "\n";
    {
        std::lock_guard<std::mutex> lk(write_mutex_);
        size_t written = fwrite(info_line.data(), 1, info_line.size(), stdout);
        fflush(stdout);
        if (written != info_line.size()) {
            shutdown_.store(true, std::memory_order_release);
            return false;
        }
    }

    fprintf(stderr, "[preview] Full=%dx%d  Half=%dx%d\n", width_, height_, half_w_, half_h_);

    stdin_thread_ = std::thread([this]() { stdin_reader_loop(); });
    preview_thread_ = std::thread([this]() { preview_thread_loop(); });

    return true;
}

// ---------------------------------------------------------------------------
// tick()
// ---------------------------------------------------------------------------

bool PreviewPresenter::tick()
{
    std::this_thread::sleep_for(std::chrono::milliseconds(100));

    if (stat_is_recording_.load(std::memory_order_relaxed)) {
        if (++tick_counter_ % 20 == 0) {
            int rec_s = static_cast<int>(stat_rec_elapsed_);
            fprintf(stderr, "REC %02d:%02d | Frames: %llu written, %llu dropped | FPS: %.1f cap / %.1f write | Size: %.1f MB\n",
                    rec_s / 60, rec_s % 60,
                    (unsigned long long)stat_written_.load(std::memory_order_relaxed),
                    (unsigned long long)stat_dropped_.load(std::memory_order_relaxed),
                    stat_capture_fps_, stat_write_fps_,
                    static_cast<double>(stat_bytes_.load(std::memory_order_relaxed)) / 1e6);
        }
    }

    return !shutdown_.load(std::memory_order_acquire);
}

// ---------------------------------------------------------------------------
// shutdown()
// ---------------------------------------------------------------------------

void PreviewPresenter::shutdown()
{
    shutdown_.store(true, std::memory_order_release);

    if (preview_thread_.joinable()) {
        preview_thread_.join();
    }

    if (stdin_thread_.joinable()) {
        stdin_thread_.join();
    }
}

// ---------------------------------------------------------------------------
// Camera events
// ---------------------------------------------------------------------------

void PreviewPresenter::on_camera_disconnect()
{
    fprintf(stderr, "\nDISCONNECTED\n");
    fflush(stderr);
}

void PreviewPresenter::on_camera_reconnect()
{
    fprintf(stderr, "RECONNECTED\n");
    fflush(stderr);
}

// ---------------------------------------------------------------------------
// update_stats()
// ---------------------------------------------------------------------------

void PreviewPresenter::update_stats(const Stats& stats)
{
    stat_written_.store(stats.written(), std::memory_order_relaxed);
    stat_dropped_.store(stats.dropped(), std::memory_order_relaxed);
    stat_bytes_.store(stats.total_bytes(), std::memory_order_relaxed);
    stat_capture_fps_ = stats.capture_fps();
    stat_write_fps_ = stats.write_fps();
    stat_rec_elapsed_ = stats.recording_elapsed_seconds();
    stat_is_recording_.store(stats.is_recording(), std::memory_order_relaxed);
}

// ---------------------------------------------------------------------------
// update_frame() -- called from capture thread, fast memcpy only
// ---------------------------------------------------------------------------

void PreviewPresenter::update_frame(
    const uint8_t*  rgb_data,
    const uint16_t* depth_data,
    int             width,
    int             height,
    float           /*depth_scale*/
)
{
    uint64_t count = frame_counter_.fetch_add(1, std::memory_order_relaxed);
    if (count % 6 != 0) return;  // ~5fps at 30fps capture

    if (shutdown_.load(std::memory_order_relaxed)) return;

    // Acquire the current active index so we write to the OTHER buffer.
    // The preview thread reads `active_buf_` with acquire, so our release
    // store below creates a happens-before: the preview thread will only
    // see the new index after both memcpys have completed.
    int wr = 1 - active_buf_.load(std::memory_order_acquire);
    std::memcpy(rgb_buf_[wr].data(), rgb_data,
                static_cast<size_t>(width) * height * 3);
    std::memcpy(depth_buf_[wr].data(), reinterpret_cast<const uint8_t*>(depth_data),
                static_cast<size_t>(width) * height * 2);

    // Release ensures both memcpys are visible before the index flip.
    active_buf_.store(wr, std::memory_order_release);
    frame_ready_.store(true, std::memory_order_release);
}

// ---------------------------------------------------------------------------
// preview_thread_loop()
// ---------------------------------------------------------------------------

void PreviewPresenter::preview_thread_loop()
{
    // RGB: half-res, low quality (smooth natural image compresses well)
    JpegCompressor rgb_jpeg(half_w_, half_h_, 30);
    // Depth: full-res, higher quality (sparse colored dots on black need it)
    JpegCompressor depth_jpeg(width_, height_, 65);

    while (!shutdown_.load(std::memory_order_acquire)) {
        if (!frame_ready_.load(std::memory_order_acquire)) {
            std::this_thread::sleep_for(std::chrono::milliseconds(50));
            continue;
        }
        frame_ready_.store(false, std::memory_order_relaxed);

        int rd = active_buf_.load(std::memory_order_acquire);
        const uint8_t* rgb_src = rgb_buf_[rd].data();
        const uint8_t* depth_src = depth_buf_[rd].data();

        // Nearest-neighbor downscale RGB to half resolution
        for (int y = 0; y < half_h_; ++y) {
            int src_y = y * 2;
            for (int x = 0; x < half_w_; ++x) {
                int src_x = x * 2;
                int src_idx = src_y * width_ + src_x;
                int dst_idx = y * half_w_ + x;
                half_rgb_[dst_idx * 3 + 0] = rgb_src[src_idx * 3 + 0];
                half_rgb_[dst_idx * 3 + 1] = rgb_src[src_idx * 3 + 1];
                half_rgb_[dst_idx * 3 + 2] = rgb_src[src_idx * 3 + 2];
            }
        }

        // JPEG encode RGB
        auto [rgb_ptr, rgb_size] = rgb_jpeg.compress(half_rgb_.data(), half_w_, half_h_);

        // Assemble write buffer
        bool send_depth = depth_enabled_.load(std::memory_order_acquire);

        write_buf_.clear();

        // RGB frame: 'R' + u32_le(size) + jpeg_bytes
        write_buf_.push_back('R');
        uint32_t rgb_size_le = static_cast<uint32_t>(rgb_size);
        write_buf_.insert(write_buf_.end(),
                          reinterpret_cast<const uint8_t*>(&rgb_size_le),
                          reinterpret_cast<const uint8_t*>(&rgb_size_le) + 4);
        write_buf_.insert(write_buf_.end(), rgb_ptr, rgb_ptr + rgb_size);

        // Depth frame (only when enabled): colorize at full res, encode, append
        if (send_depth) {
            const uint16_t* depth_u16 = reinterpret_cast<const uint16_t*>(depth_src);
            colorize_depth(depth_u16, colorized_buf_.data(), width_, height_);
            auto [depth_ptr, depth_size] = depth_jpeg.compress(
                colorized_buf_.data(), width_, height_);

            write_buf_.push_back('D');
            uint32_t depth_size_le = static_cast<uint32_t>(depth_size);
            write_buf_.insert(write_buf_.end(),
                              reinterpret_cast<const uint8_t*>(&depth_size_le),
                              reinterpret_cast<const uint8_t*>(&depth_size_le) + 4);
            write_buf_.insert(write_buf_.end(), depth_ptr, depth_ptr + depth_size);
        }

        // Atomic write
        {
            std::lock_guard<std::mutex> lk(write_mutex_);
            if (fwrite(write_buf_.data(), 1, write_buf_.size(), stdout) != write_buf_.size()) {
                shutdown_.store(true, std::memory_order_release);
                return;
            }
            fflush(stdout);
        }
    }
}

// ---------------------------------------------------------------------------
// consume_record_cmd / consume_pending_stop
// ---------------------------------------------------------------------------

bool PreviewPresenter::consume_record_cmd(RecordCmd& out)
{
    if (!pending_record_.load(std::memory_order_acquire)) return false;
    std::lock_guard<std::mutex> lk(cmd_mutex_);
    out = pending_record_cmd_;
    pending_record_.store(false, std::memory_order_release);
    return true;
}

bool PreviewPresenter::consume_pending_stop()
{
    return pending_stop_.exchange(false, std::memory_order_acq_rel);
}

// ---------------------------------------------------------------------------
// stdin_reader_loop() -- poll()-based, detects parent death via POLLHUP
// ---------------------------------------------------------------------------

void PreviewPresenter::stdin_reader_loop()
{
    struct pollfd pfd = { STDIN_FILENO, POLLIN, 0 };
    std::string line;

    while (!shutdown_.load(std::memory_order_acquire)) {
        int ret = poll(&pfd, 1, 200);
        if (ret < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (ret == 0) continue;

        if (pfd.revents & (POLLHUP | POLLERR)) break;
        if (!(pfd.revents & POLLIN)) continue;

        if (!std::getline(std::cin, line)) break;
        if (shutdown_.load(std::memory_order_acquire)) break;

        while (!line.empty() && (line.back() == '\r' || line.back() == '\n' || line.back() == ' '))
            line.pop_back();
        if (line.empty()) continue;

        try {
            auto j = json::parse(line);
            std::string cmd = j.value("cmd", "");

            if (cmd == "record") {
                std::lock_guard<std::mutex> lk(cmd_mutex_);
                pending_record_cmd_.output_dir = j.value("output_dir", config_.output_dir);
                pending_record_cmd_.session = j.value("session", "");
                pending_record_cmd_.crf = j.value("crf", config_.h264_crf);
                pending_record_cmd_.warmup = j.value("warmup", config_.warmup_frames);
                pending_record_cmd_.preset = j.value("preset", config_.h264_preset);
                pending_record_.store(true, std::memory_order_release);
            } else if (cmd == "stop") {
                pending_stop_.store(true, std::memory_order_release);
            }
        } catch (const json::exception& e) {
            // Log but don't break — malformed JSON shouldn't kill the command channel
            fprintf(stderr, "[preview] Invalid JSON on stdin: %s (line: '%s')\n",
                    e.what(), line.c_str());
            continue;
        }
    }

    shutdown_.store(true, std::memory_order_release);
}
