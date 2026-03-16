// ego-recorder: synchronized RGBD capture from Intel RealSense D435/D435i.
//
// Supports two modes selected via --headless flag:
//
//   GUI mode (default):
//     Opens a Dear ImGui window with live RGB+depth preview, recording controls,
//     keyboard shortcuts (Space=record, Tab=view cycle, Esc=quit), and a stats
//     overlay. Recording starts only when the user clicks "Start Recording".
//
//   Headless mode (--headless):
//     Designed for unattended systemd service operation. Waits for a RealSense
//     camera to appear (hotplug), plays a 3-second audio countdown, then starts
//     recording with an auto-generated timestamp session name. Integrates with
//     systemd sd_notify (READY, WATCHDOG, STATUS, STOPPING) and the D-Bus
//     inhibitor lock to block lid-close during recording.
//
// Three-thread pipeline (both modes):
//   - Capture thread: polls camera at 30fps, enqueues CapturedFrames.
//     In GUI mode, also updates the GuiPresenter frame buffer for live preview.
//   - Writer thread:  dequeues frames, compresses (H.264 + Zdepth), writes to .egorec.
//     Only runs when recording is active.
//   - Main thread:    presenter tick loop (ImGui render or headless watchdog),
//     stats reporting, disconnect/reconnect recovery.
//
// USB disconnect recovery:
//   - GUI mode:     On disconnect, shows "Camera Disconnected" banner and a
//     "Reconnect" button. When clicked, on_reconnect_requested callback fires:
//     pipeline is destroyed, 500ms sleep, recreated, recording resumes.
//   - Headless mode: Auto-retry every 2 seconds until camera is available again.
//     Plays 3-second audio countdown before resuming recording.
//     New recording file opened after each successful reconnect.
//
// Config + CLI merge:
//   Config file loaded first (--config), then any explicitly-provided CLI flags
//   override. cxxopts count() detects explicit vs default-only.
//
// The FileHeader is assembled in main.cpp (not pipeline.h or storage/) --
// preserves the Phase 1 pipeline-storage decoupling decision.

#include <csignal>
#include <cxxopts.hpp>

#include "capture/pipeline.h"
#include "capture/frame_types.h"
#include "threading/bounded_queue.h"
#include "compression/jpeg_compressor.h"
#include "compression/zstd_compressor.h"
#include "compression/zdepth_compressor.h"
#include "compression/h264_encoder.h"
#include "storage/binary_format.h"
#include "storage/file_writer.h"
#include "utils/signal_handler.h"
#include "utils/stats.h"
#include "config/config.h"
#include "presenter/ipresenter.h"

#ifdef HAVE_GUI
#include "presenter/gui_presenter.h"
#endif

#include "presenter/headless_presenter.h"
#include "presenter/preview_presenter.h"
#include "utils/audio_alert.h"
#include "dataset/dataset_manifest.h"
#include "dataset/dataset_commands.h"

#include <atomic>
#include <cerrno>
#include <chrono>
#include <cinttypes>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

#include <unistd.h>

#include <librealsense2/rs.hpp>

// ---- Helpers ---------------------------------------------------------------

/// Frame gap threshold for soft episode split in headless mode.
/// If recording is active and no frames arrive for this long (but less than
/// the hard 1s watchdog), split into a new episode. This catches brief
/// USB disconnects where librealsense reconnects internally.
static constexpr int64_t EPISODE_SPLIT_MS = 500;

/// Generate an indexed session name: "{dataset}_{NNN}"
/// Scans the output directory tree for existing .egorec files matching the
/// pattern to find the next available index.
static std::string make_session_name(const std::string& output_dir) {
    // Dataset name = last path component of the base output dir
    // e.g. /var/lib/ego-recorder/pick → "pick"
    std::filesystem::path base(output_dir);
    // Strip trailing slashes
    while (base.has_filename() && base.filename() == ".") {
        base = base.parent_path();
    }
    std::string dataset = base.filename().string();
    if (dataset.empty()) dataset = "capture";

    // Scan for existing {dataset}_NNN.egorec files to find the max index
    int max_idx = -1;
    std::string prefix = dataset + "_";
    std::error_code ec;
    for (auto& entry : std::filesystem::recursive_directory_iterator(output_dir, ec)) {
        if (!entry.is_regular_file()) continue;
        auto fname = entry.path().stem().string();  // without .egorec
        if (fname.size() > prefix.size() && fname.substr(0, prefix.size()) == prefix) {
            auto suffix = fname.substr(prefix.size());
            try {
                int idx = std::stoi(suffix);
                if (idx > max_idx) max_idx = idx;
            } catch (...) {}
        }
    }

    char buf[32];
    std::snprintf(buf, sizeof(buf), "%s_%03d", dataset.c_str(), max_idx + 1);
    return std::string(buf);
}

/// Generate date-based output directory for headless mode:
/// {output_dir}/{YYYY}/{MM}/{DD}/
static std::string make_date_dir(const std::string& output_dir) {
    std::time_t now = std::time(nullptr);
    std::tm* tm_info = std::localtime(&now);
    char date_path[32];
    std::strftime(date_path, sizeof(date_path), "%Y/%m/%d/", tm_info);

    std::string path = output_dir;
    if (!path.empty() && path.back() != '/') {
        path += '/';
    }
    path += date_path;
    return path;
}

/// Generate output filepath.
///
/// When add_timestamp_suffix is true (GUI mode):
///   {dir}/{session}_{YYYYMMDD_HHMMSS}.egorec
///
/// When add_timestamp_suffix is false (headless mode):
///   {dir}/{session}.egorec
///
/// In headless mode, make_session_name() produces indexed names like
/// "pick_003", so add_timestamp_suffix is false to avoid appending a
/// redundant timestamp.
static std::string make_output_path(const std::string& output_dir,
                                    const std::string& session_name,
                                    bool add_timestamp_suffix = true) {
    std::string path = output_dir;
    if (!path.empty() && path.back() != '/') {
        path += '/';
    }
    path += session_name;

    if (add_timestamp_suffix) {
        std::time_t now = std::time(nullptr);
        std::tm* tm_info = std::localtime(&now);
        char ts[32];
        std::strftime(ts, sizeof(ts), "%Y%m%d_%H%M%S", tm_info);
        path += '_';
        path += ts;
    }

    path += ".egorec";
    return path;
}

/// Microseconds since Unix epoch from system_clock.
static uint64_t now_us() {
    using namespace std::chrono;
    return static_cast<uint64_t>(
        duration_cast<microseconds>(system_clock::now().time_since_epoch()).count());
}

/// Assemble a FileHeader from pipeline metadata + config.
/// The header is fully populated -- caller just needs to call write_header().
static FileHeader make_file_header(const RealSensePipeline& camera,
                                   const std::string& session_name,
                                   int crf) {
    FileHeader header;
    std::memset(&header, 0, sizeof(header));
    std::memcpy(header.magic, FILE_MAGIC, sizeof(FILE_MAGIC));
    header.header_size = sizeof(FileHeader);
    header.flags = camera.has_imu() ? 0x01u : 0x00u;

    std::strncpy(header.serial_number,
                 camera.serial_number().c_str(),
                 sizeof(header.serial_number) - 1);

    header.depth_scale = camera.depth_scale();

    auto di = camera.depth_intrinsics();
    header.depth_width  = static_cast<uint32_t>(di.width);
    header.depth_height = static_cast<uint32_t>(di.height);
    header.depth_fx     = di.fx;
    header.depth_fy     = di.fy;
    header.depth_ppx    = di.ppx;
    header.depth_ppy    = di.ppy;
    header.depth_distortion_model = static_cast<uint32_t>(di.model);
    static_assert(sizeof(header.depth_distortion_coeffs) == sizeof(di.coeffs),
                  "depth distortion coefficients array size mismatch");
    std::memcpy(header.depth_distortion_coeffs, di.coeffs,
                sizeof(header.depth_distortion_coeffs));

    auto ci = camera.color_intrinsics();
    header.color_width  = static_cast<uint32_t>(ci.width);
    header.color_height = static_cast<uint32_t>(ci.height);
    header.color_fx     = ci.fx;
    header.color_fy     = ci.fy;
    header.color_ppx    = ci.ppx;
    header.color_ppy    = ci.ppy;
    header.color_distortion_model = static_cast<uint32_t>(ci.model);
    static_assert(sizeof(header.color_distortion_coeffs) == sizeof(ci.coeffs),
                  "color distortion coefficients array size mismatch");
    std::memcpy(header.color_distortion_coeffs, ci.coeffs,
                sizeof(header.color_distortion_coeffs));

    auto ex = camera.depth_to_color_extrinsics();
    static_assert(sizeof(header.extrinsic_rotation) == sizeof(ex.rotation),
                  "extrinsic rotation array size mismatch");
    static_assert(sizeof(header.extrinsic_translation) == sizeof(ex.translation),
                  "extrinsic translation array size mismatch");
    std::memcpy(header.extrinsic_rotation, ex.rotation,
                sizeof(header.extrinsic_rotation));
    std::memcpy(header.extrinsic_translation, ex.translation,
                sizeof(header.extrinsic_translation));

    std::strncpy(header.usb_type,
                 camera.usb_type().c_str(),
                 sizeof(header.usb_type) - 1);

    std::strncpy(header.session_name,
                 session_name.c_str(),
                 sizeof(header.session_name) - 1);
    header.start_timestamp_us = now_us();

    header.rgb_codec   = 2;  // H264
    header.depth_codec = 2;  // Zdepth
    header.rgb_quality = static_cast<uint8_t>(crf);
    header.zstd_level  = 0;  // reserved for Zdepth

    return header;
}

// ---- preview subcommand ----------------------------------------------------
// Unified preview + recording subprocess for the Tauri desktop app.
// Streams JPEG-encoded RGB + colorized depth to stdout, accepts JSON commands
// on stdin to start/stop recording. Camera stays open the entire time.

static int run_preview(int argc, char* argv[]) {
    // Ignore SIGPIPE so broken pipe from parent doesn't kill us instantly
    signal(SIGPIPE, SIG_IGN);

    // Parse optional flags (same as regular mode, but only a subset matter)
    Config config = load_config("");

    // Accept optional --width, --height, --warmup, --crf, --queue-size, --config
    for (int i = 2; i < argc; ++i) {
        std::string arg = argv[i];
        auto next = [&]() -> std::string {
            return (i + 1 < argc) ? argv[++i] : "";
        };
        if (arg == "--config")     { config = load_config(next()); }
        else if (arg == "--width")  { config.frame_width = std::stoi(next()); }
        else if (arg == "--height") { config.frame_height = std::stoi(next()); }
        else if (arg == "--warmup") { config.warmup_frames = std::stoi(next()); }
        else if (arg == "--crf")    { config.h264_crf = std::stoi(next()); }
        else if (arg == "--preset") { config.h264_preset = next(); }
        else if (arg == "--queue-size") { config.queue_size = std::stoi(next()); }
    }

    const int fw = config.frame_width;
    const int fh = config.frame_height;

    // Signal handling
    std::atomic<bool> shutdown_flag{false};
    setup_signal_handling(shutdown_flag);

    // Camera initialization (poll until available)
    std::unique_ptr<RealSensePipeline> camera;
    fprintf(stderr, "[preview] Waiting for RealSense camera...\n");
    while (!shutdown_flag.load(std::memory_order_acquire)) {
        try {
            camera = std::make_unique<RealSensePipeline>();
            camera->configure_and_start(fw, fh, config.warmup_frames);
            break;
        } catch (const rs2::error&) {
            camera.reset();
            for (int ms = 0; ms < 2000 && !shutdown_flag.load(); ms += 100) {
                std::this_thread::sleep_for(std::chrono::milliseconds(100));
            }
        }
    }
    if (shutdown_flag.load() || !camera) return 0;

    fprintf(stderr, "[preview] Camera: %s (USB %s)\n",
            camera->serial_number().c_str(), camera->usb_type().c_str());

    // Compression infrastructure
    ZdepthCompressor zdepth_comp(fw, fh);
    H264Encoder h264(fw, fh, 30, config.h264_crf, config.h264_preset);

    // Shared recording state
    std::unique_ptr<FileWriter> writer;
    std::unique_ptr<BoundedQueue<CapturedFrame>> queue;
    std::thread writer_thread;
    Stats stats;
    std::atomic<bool> recording_active{false};
    std::atomic<bool> camera_disconnected{false};
    std::atomic<int64_t> last_capture_ms{0};
    std::string last_recording_path_;
    int episode_count = 0;

    // Create preview presenter
    auto preview = std::make_unique<PreviewPresenter>(
        config,
        camera->serial_number(),
        camera->usb_type(),
        camera->has_imu(),
        fw, fh
    );

    // Start recording lambda (same pattern as main recording flow)
    auto start_recording = [&](const std::string& sname, const std::string& out_dir,
                               int crf_override) {
        // Use provided CRF or fallback to config
        int crf = (crf_override > 0) ? crf_override : config.h264_crf;

        // Ensure output directory exists (matches headless/GUI mode behavior)
        {
            std::error_code ec;
            std::filesystem::create_directories(out_dir, ec);
            if (ec) {
                fprintf(stderr, "[preview] ERROR: cannot create output dir '%s': %s\n",
                        out_dir.c_str(), ec.message().c_str());
                return;
            }
        }

        const std::string filepath = make_output_path(out_dir, sname, true);
        last_recording_path_ = filepath;
        fprintf(stderr, "[preview] Recording to: %s\n", filepath.c_str());

        queue = std::make_unique<BoundedQueue<CapturedFrame>>(
                    static_cast<size_t>(config.queue_size));
        writer = std::make_unique<FileWriter>(filepath);

        FileHeader header = make_file_header(*camera, sname, crf);
        writer->write_header(header);

        last_capture_ms.store(0, std::memory_order_release);
        recording_active.store(true, std::memory_order_release);
        stats.recording_started();

        writer_thread = std::thread([&]() {
            try {
                while (true) {
                    auto maybe_frame = queue->pop();
                    if (!maybe_frame) break;

                    auto& frame = *maybe_frame;
                    auto h264_buf = h264.encode(frame.rgb_data.data(), fw, fh);

                    bool keyframe = (frame.frame_number % 30 == 0);
                    auto [zdepth_data, zdepth_size] = zdepth_comp.compress(
                        reinterpret_cast<const uint16_t*>(frame.depth_data.data()),
                        fw, fh, keyframe);

                    std::vector<IMUSampleWire> imu_wire;
                    imu_wire.reserve(frame.imu_samples.size());
                    for (const auto& imu : frame.imu_samples) {
                        IMUSampleWire w;
                        w.timestamp_us = imu.timestamp_us;
                        w.accel_x = imu.accel[0]; w.accel_y = imu.accel[1]; w.accel_z = imu.accel[2];
                        w.gyro_x = imu.gyro[0]; w.gyro_y = imu.gyro[1]; w.gyro_z = imu.gyro[2];
                        imu_wire.push_back(w);
                    }

                    writer->write_frame(h264_buf.data(), h264_buf.size(), zdepth_data, zdepth_size,
                                        frame.timestamp_us, frame.frame_number, imu_wire);
                    stats.frame_written();
                    stats.bytes_written(h264_buf.size() + zdepth_size);
                }
            } catch (const std::exception& e) {
                fprintf(stderr, "[preview] Writer thread error: %s\n", e.what());
            }
        });
    };

    // Stop recording lambda
    auto stop_recording = [&]() -> bool {
        if (!recording_active.exchange(false)) return false;
        stats.recording_stopped();

        if (queue) queue->close();
        if (writer_thread.joinable()) writer_thread.join();

        bool has_frames = stats.captured() > 0;

        if (writer && !writer->is_finalized()) {
            auto flush_buf = h264.flush();
            if (!flush_buf.empty()) {
                writer->write_trailing_codec_data(flush_buf.data(), flush_buf.size());
            }
            writer->finalize();
        }
        writer.reset();

        if (has_frames && !last_recording_path_.empty()) {
            register_episode(config.output_dir, last_recording_path_);
        } else if (!has_frames && !last_recording_path_.empty()) {
            std::error_code ec;
            std::filesystem::remove(last_recording_path_, ec);
        }
        last_recording_path_.clear();
        h264.reset();

        if (queue) {
            stats.frames_dropped(queue->dropped());
            queue.reset();
        }

        fprintf(stderr, "\n%s\n", stats.summary().c_str());
        fprintf(stderr, "Recording complete.\n");
        return has_frames;
    };

    // Start presenter
    if (!preview->start()) {
        fprintf(stderr, "[preview] Failed to start preview presenter\n");
        camera->stop();
        return 1;
    }

    // Capture thread
    std::thread capture_thread([&]() {
        while (!shutdown_flag.load(std::memory_order_acquire) && !preview->should_shutdown()) {
            if (camera_disconnected.load(std::memory_order_acquire)) {
                std::this_thread::sleep_for(std::chrono::milliseconds(100));
                continue;
            }

            if (!camera) {
                std::this_thread::sleep_for(std::chrono::milliseconds(100));
                continue;
            }

            if (camera->is_device_lost()) {
                fprintf(stderr, "\n[preview] Camera unplugged\n");
                camera_disconnected.store(true, std::memory_order_release);
                preview->on_camera_disconnect();
                if (recording_active.load(std::memory_order_acquire)) {
                    stop_recording();
                }
                continue;
            }

            try {
                auto maybe_frame = camera->poll_frame();
                if (!maybe_frame) continue;

                CapturedFrame& frame = *maybe_frame;
                stats.frame_captured();
                last_capture_ms.store(
                    std::chrono::duration_cast<std::chrono::milliseconds>(
                        std::chrono::steady_clock::now().time_since_epoch()).count(),
                    std::memory_order_release);

                // Feed to preview presenter (handles throttling internally)
                preview->update_frame(
                    frame.rgb_data.data(),
                    reinterpret_cast<const uint16_t*>(frame.depth_data.data()),
                    fw, fh, camera->depth_scale()
                );

                // Push to writer queue if recording
                if (recording_active.load(std::memory_order_acquire) && queue) {
                    queue->push(std::move(frame));
                }
            } catch (const rs2::error& e) {
                fprintf(stderr, "\n[preview] RealSense error: %s\n", e.what());
                camera_disconnected.store(true, std::memory_order_release);
                preview->on_camera_disconnect();
                if (recording_active.load(std::memory_order_acquire)) {
                    stop_recording();
                }
            }
        }
    });

    // Main loop: presenter tick + command processing
    while (!shutdown_flag.load(std::memory_order_acquire) && !preview->should_shutdown()) {
        // Sync live queue drop count into stats for real-time reporting
        if (queue) {
            stats.set_dropped(queue->dropped());
        }

        preview->update_stats(stats);

        if (!preview->tick()) break;

        // Check for pending record command from stdin
        RecordCmd rec_cmd;
        if (preview->consume_record_cmd(rec_cmd)) {
            if (!recording_active.load()) {
                try {
                    start_recording(rec_cmd.session, rec_cmd.output_dir, rec_cmd.crf);
                } catch (const std::exception& e) {
                    fprintf(stderr, "[preview] ERROR: failed to start recording: %s\n", e.what());
                    fprintf(stderr, "Recording complete.\n");
                }
            }
        }

        // Check for pending stop command from stdin
        if (preview->consume_pending_stop()) {
            if (recording_active.load()) {
                bool had_frames = stop_recording();
                if (had_frames) episode_count++;
            }
        }

        // Handle camera disconnect recovery
        if (camera_disconnected.load(std::memory_order_acquire)) {
            try {
                camera = std::make_unique<RealSensePipeline>();
                camera->configure_and_start(fw, fh, config.warmup_frames);
                camera_disconnected.store(false, std::memory_order_release);
                preview->on_camera_reconnect();
                fprintf(stderr, "[preview] Camera reconnected.\n");
            } catch (const rs2::error&) {
                camera.reset();
                // Retry next tick
            }
        }
    }

    // Shutdown
    shutdown_flag.store(true, std::memory_order_release);

    if (capture_thread.joinable()) capture_thread.join();
    stop_recording();

    preview->shutdown();

    if (camera) camera->stop();

    return 0;
}

// ---- main ------------------------------------------------------------------

int main(int argc, char* argv[]) {
    // ---- Subcommand dispatch (before cxxopts parsing) ----------------------
    if (argc >= 2) {
        std::string_view cmd = argv[1];

        // ---- preview subcommand (Tauri desktop app integration) ----
        if (cmd == "preview") {
            return run_preview(argc, argv);
        }

        // ---- info subcommand (pure C++, no Python dependency) ----
        if (cmd == "info") {
            if (argc < 3) {
                fprintf(stderr, "Usage: ego-recorder info <file.egorec> [...]\n");
                return 1;
            }
            for (int i = 2; i < argc; ++i) {
                const char* path = argv[i];
                std::ifstream in(path, std::ios::binary);
                if (!in.is_open()) {
                    fprintf(stderr, "Error: cannot open '%s'\n", path);
                    continue;
                }
                FileHeader hdr;
                in.read(reinterpret_cast<char*>(&hdr), sizeof(hdr));
                if (!in.good() || std::memcmp(hdr.magic, "EGOREC", 6) != 0) {
                    fprintf(stderr, "Error: '%s' is not a valid .egorec file\n", path);
                    continue;
                }

                // Read footer for frame count and duration
                in.seekg(-static_cast<int>(sizeof(FileFooter)), std::ios::end);
                FileFooter footer;
                in.read(reinterpret_cast<char*>(&footer), sizeof(footer));
                bool has_footer = in.good() && footer.footer_magic == FOOTER_MAGIC;

                printf("File: %s\n", path);
                printf("  Format version: %d.%d\n", hdr.magic[6], hdr.magic[7]);
                printf("  Session: %s\n", hdr.session_name);
                printf("  Serial: %s\n", hdr.serial_number);
                printf("  USB: %s\n", hdr.usb_type);
                printf("  Resolution: %ux%u (depth), %ux%u (color)\n",
                       hdr.depth_width, hdr.depth_height,
                       hdr.color_width, hdr.color_height);
                printf("  Depth scale: %.6f\n", hdr.depth_scale);
                printf("  RGB codec: %s (%d)\n",
                       hdr.rgb_codec == 0 ? "raw" : hdr.rgb_codec == 1 ? "JPEG" : hdr.rgb_codec == 2 ? "H264" : "unknown",
                       hdr.rgb_codec);
                printf("  Depth codec: %s (%d)\n",
                       hdr.depth_codec == 0 ? "raw" : hdr.depth_codec == 1 ? "ZSTD" : hdr.depth_codec == 2 ? "Zdepth" : "unknown",
                       hdr.depth_codec);
                printf("  RGB quality/CRF: %d\n", hdr.rgb_quality);
                printf("  Depth intrinsics: fx=%.2f fy=%.2f ppx=%.2f ppy=%.2f\n",
                       hdr.depth_fx, hdr.depth_fy, hdr.depth_ppx, hdr.depth_ppy);
                printf("  Color intrinsics: fx=%.2f fy=%.2f ppx=%.2f ppy=%.2f\n",
                       hdr.color_fx, hdr.color_fy, hdr.color_ppx, hdr.color_ppy);
                printf("  IMU: %s\n", (hdr.flags & 0x01) ? "yes" : "no");
                if (has_footer) {
                    printf("  Frames: %" PRIu64 "\n", footer.total_frames);
                    printf("  Duration: %.2f s\n", footer.total_duration_us / 1e6);
                }
                printf("\n");
            }
            return 0;
        }

        // ---- dataset subcommand ----
        if (cmd == "dataset") {
            if (argc < 3) {
                fprintf(stderr, "Usage: ego-recorder dataset <init|info|add|remove> [args...]\n");
                return 1;
            }
            std::string_view subcmd = argv[2];
            // Pass remaining args after "dataset <subcmd>"
            int sub_argc = argc - 3;
            char** sub_argv = argv + 3;

            if (subcmd == "init")   return cmd_dataset_init(sub_argc, sub_argv);
            if (subcmd == "info")   return cmd_dataset_info(sub_argc, sub_argv);
            if (subcmd == "add")    return cmd_dataset_add(sub_argc, sub_argv);
            if (subcmd == "remove") return cmd_dataset_remove(sub_argc, sub_argv);

            fprintf(stderr, "Error: unknown dataset subcommand '%.*s'\n",
                    static_cast<int>(subcmd.size()), subcmd.data());
            fprintf(stderr, "Available: init, info, add, remove\n");
            return 1;
        }

        // ---- export subcommand (dispatches to Rust ego-convert binary) ----
        // Per locked decision: `ego-recorder export rlds` and `ego-recorder export lerobot`
        if (cmd == "export") {
            if (argc < 3) {
                fprintf(stderr, "Usage: ego-recorder export <rlds|lerobot> [options] <file.egorec> [...]\n");
                return 1;
            }
            std::string_view format = argv[2];

            if (format != "rlds" && format != "lerobot") {
                fprintf(stderr, "Error: unknown export format '%.*s'\n",
                        static_cast<int>(format.size()), format.data());
                fprintf(stderr, "Supported formats: rlds, lerobot\n");
                return 1;
            }

            // Locate the ego-convert binary relative to this binary.
            std::string binary_path = std::filesystem::canonical(
                std::filesystem::path(argv[0])).parent_path().string();

            std::vector<std::string> search_paths = {
                binary_path + "/ego-convert",
                binary_path + "/../rust/target/release/ego-convert",
                "rust/target/release/ego-convert",
            };

            std::string ego_convert_path;
            for (const auto& p : search_paths) {
                if (std::filesystem::exists(p)) {
                    ego_convert_path = std::filesystem::canonical(p).string();
                    break;
                }
            }

            if (ego_convert_path.empty()) {
                fprintf(stderr, "Error: could not find ego-convert binary\n");
                fprintf(stderr, "Looked in:\n");
                for (const auto& p : search_paths) {
                    fprintf(stderr, "  %s\n", p.c_str());
                }
                fprintf(stderr, "\nBuild with: cd rust && cargo build --release\n");
                fprintf(stderr, "Or run directly: rust/target/release/ego-convert %.*s [options] <files>\n",
                        static_cast<int>(format.size()), format.data());
                return 1;
            }

            // Build argv for ego-convert: ego-convert <format> [remaining args...]
            // Skip argv[0] (ego-recorder) and argv[1] (export)
            //
            // If a positional arg is a directory with dataset.json, expand it
            // to the resolved .egorec file paths and add dataset metadata flags.
            std::vector<std::string> conv_args = {ego_convert_path, std::string(format)};
            for (int i = 3; i < argc; ++i) {
                std::string arg = argv[i];
                // Check if this arg is a dataset directory (not a flag)
                if (!arg.empty() && arg[0] != '-' &&
                    std::filesystem::is_directory(arg) && has_manifest(arg)) {
                    DatasetManifest ds_manifest;
                    if (load_manifest(arg, ds_manifest)) {
                        // Add dataset metadata flags
                        if (!ds_manifest.name.empty()) {
                            conv_args.push_back("--dataset-name");
                            conv_args.push_back(ds_manifest.name);
                        }
                        if (!ds_manifest.description.empty()) {
                            conv_args.push_back("--dataset-description");
                            conv_args.push_back(ds_manifest.description);
                        }
                        if (!ds_manifest.tags.empty()) {
                            std::string tags_joined;
                            for (size_t t = 0; t < ds_manifest.tags.size(); ++t) {
                                if (t > 0) tags_joined += ',';
                                tags_joined += ds_manifest.tags[t];
                            }
                            conv_args.push_back("--dataset-tags");
                            conv_args.push_back(tags_joined);
                        }
                        // Resolve episode paths
                        std::filesystem::path ds_dir = std::filesystem::absolute(arg);
                        for (const auto& ep : ds_manifest.episodes) {
                            conv_args.push_back(
                                (ds_dir / ep.filename).string());
                        }
                    }
                } else {
                    conv_args.push_back(arg);
                }
            }

            // Build C-style argv for execvp
            std::vector<char*> c_args;
            for (auto& a : conv_args) {
                c_args.push_back(a.data());
            }
            c_args.push_back(nullptr);

            execvp(ego_convert_path.c_str(), c_args.data());

            // execvp only returns on error
            fprintf(stderr, "Error: failed to exec ego-convert: %s\n", strerror(errno));
            return 1;
        }
    }

    // ---- CLI Parsing -------------------------------------------------------
    cxxopts::Options options("ego-recorder",
        "Record synchronized RGBD data from Intel RealSense D435/D435i to .egorec files");

    options.add_options()
        ("headless",       "Run in headless/systemd mode (no GUI, auto-record)",
         cxxopts::value<bool>()->default_value("false"))
        ("config",         "Path to TOML configuration file",
         cxxopts::value<std::string>()->default_value(""))
        ("o,output",       "Output directory",
         cxxopts::value<std::string>()->default_value("."))
        ("s,session-name", "Session name (used in output filename; headless auto-generates if not set)",
         cxxopts::value<std::string>()->default_value(""))
        ("d,duration",     "Max recording duration in seconds (0 = unlimited)",
         cxxopts::value<int>()->default_value("0"))
        ("crf",            "H.264 CRF quality 0-51 (default 23)",
         cxxopts::value<int>()->default_value("0"))  // 0 = use config value
        ("preset",         "H.264 encoder speed preset (ultrafast/superfast/veryfast/fast)",
         cxxopts::value<std::string>()->default_value(""))
        ("queue-size",     "Bounded queue size (2-16)",
         cxxopts::value<int>()->default_value("0"))  // 0 = use config value
        ("warmup",         "Camera warmup frames to skip",
         cxxopts::value<int>()->default_value("0"))  // 0 = use config value
        ("width",          "Capture width  (default 1280, must be multiple of 8)",
         cxxopts::value<int>()->default_value("0"))  // 0 = use config value
        ("height",         "Capture height (default 720, must be multiple of 8)",
         cxxopts::value<int>()->default_value("0"))  // 0 = use config value
        ("h,help",         "Print usage");

    cxxopts::ParseResult args;
    try {
        args = options.parse(argc, argv);
    } catch (const cxxopts::exceptions::exception& e) {
        fprintf(stderr, "Error parsing options: %s\n", e.what());
        fprintf(stderr, "%s\n", options.help().c_str());
        return 1;
    }

    if (args.count("help")) {
        fprintf(stdout, "%s\n", options.help().c_str());
        return 0;
    }

    // ---- Config loading + CLI override -------------------------------------
    // Load config file first (provides defaults), then CLI flags override
    // any values that were explicitly provided by the user.
    const std::string config_path = args["config"].as<std::string>();
    Config config = load_config(config_path);

    // CLI overrides: only apply when user explicitly provided the flag
    // (cxxopts count() > 0 means flag was on the command line)
    if (args.count("headless")) {
        config.headless = args["headless"].as<bool>();
    }
    if (args.count("output")) {
        config.output_dir = args["output"].as<std::string>();
    }
    if (args.count("session-name")) {
        const std::string sn = args["session-name"].as<std::string>();
        if (!sn.empty()) {
            config.session_name = sn;
        }
    }
    if (args.count("duration")) {
        // duration is not in Config struct -- handled separately
    }
    if (args.count("crf") && args["crf"].as<int>() != 0) {
        config.h264_crf = args["crf"].as<int>();
    }
    if (args.count("preset")) {
        auto p = args["preset"].as<std::string>();
        if (!p.empty()) config.h264_preset = p;
    }
    if (args.count("queue-size") && args["queue-size"].as<int>() != 0) {
        config.queue_size = args["queue-size"].as<int>();
    }
    if (args.count("warmup") && args["warmup"].as<int>() != 0) {
        config.warmup_frames = args["warmup"].as<int>();
    }
    if (args.count("width") && args["width"].as<int>() != 0) {
        config.frame_width = args["width"].as<int>();
    }
    if (args.count("height") && args["height"].as<int>() != 0) {
        config.frame_height = args["height"].as<int>();
    }

    // Duration: CLI flag only (not in config struct)
    const int max_duration = args["duration"].as<int>();

    // Validate ranges (post-merge)
    if (config.h264_crf < 0 || config.h264_crf > 51) {
        fprintf(stderr, "Error: CRF must be 0-51 (got %d)\n", config.h264_crf);
        return 1;
    }
    if (config.queue_size < 2 || config.queue_size > 16) {
        fprintf(stderr, "Error: queue-size must be 2-16 (got %d)\n", config.queue_size);
        return 1;
    }
    if (config.frame_width % 8 != 0 || config.frame_height % 8 != 0) {
        fprintf(stderr, "Error: width (%d) and height (%d) must be multiples of 8\n",
                config.frame_width, config.frame_height);
        return 1;
    }

    // ---- GUI availability check -------------------------------------------
    if (!config.headless) {
#ifndef HAVE_GUI
        fprintf(stderr, "Error: GUI not available (built without WITH_GUI).\n");
        fprintf(stderr, "       Rebuild with -DWITH_GUI=ON or run with --headless.\n");
        return 1;
#endif
    }

    // ---- Headless mode: session name + output directory setup --------------
    std::string session_name = config.session_name;
    std::string output_dir   = config.output_dir;
    bool headless_auto_session = false;

    if (config.headless) {
        // Auto-generate timestamp-based session name if not explicitly provided
        if (session_name.empty() || session_name == "capture") {
            // "capture" is the default from Config; treat as unset in headless mode
            if (args.count("session-name") == 0 ||
                args["session-name"].as<std::string>().empty()) {
                headless_auto_session = true;
                session_name = make_session_name(config.output_dir);
            }
        }
        // Create date-based output directory: {output_dir}/{YYYY}/{MM}/{DD}/
        output_dir = make_date_dir(config.output_dir);
        std::error_code ec;
        std::filesystem::create_directories(output_dir, ec);
        if (ec) {
            fprintf(stderr, "Warning: could not create output directory '%s': %s\n",
                    output_dir.c_str(), ec.message().c_str());
            // Fall back to config.output_dir
            output_dir = config.output_dir;
        }
    }

    // ---- Signal handling (MUST be before any thread creation) -------------
    std::atomic<bool> shutdown_flag{false};
    setup_signal_handling(shutdown_flag);

    // ---- Shared recording state --------------------------------------------
    // These variables are accessed from the main thread and GUI callbacks.
    // The GUI callbacks are invoked from tick() which runs on the main thread,
    // so no additional locking is needed for these.
    std::unique_ptr<FileWriter>    writer;
    std::unique_ptr<BoundedQueue<CapturedFrame>> queue;
    std::thread                    writer_thread;
    Stats                          stats;
    std::atomic<bool>              recording_active{false};
    std::atomic<bool>              camera_disconnected{false};
    std::atomic<int64_t>           last_capture_ms{0};  // steady_clock ms, 0 = no frame yet
    std::atomic<int>               capture_generation{0};
    std::string                    current_session_name = session_name;
    std::string                    last_recording_path_;
    int                            episode_count = 0;

    // Zombie storage: old capture threads/cameras that are stuck inside
    // librealsense's 15-second internal timeout. Kept alive until shutdown.
    std::vector<std::thread>                         zombie_threads;
    std::vector<std::unique_ptr<RealSensePipeline>>  zombie_cameras;

    // ---- Outer try/catch ---------------------------------------------------
    try {
        // ---- Camera initialization -----------------------------------------
        std::unique_ptr<RealSensePipeline> camera;

        if (config.headless) {
            // Headless: poll for camera availability (supports hotplug --
            // the camera may not be connected when the service starts).
            fprintf(stderr, "[headless] Waiting for RealSense camera...\n");
            while (!shutdown_flag.load(std::memory_order_acquire)) {
                try {
                    camera = std::make_unique<RealSensePipeline>();
                    camera->configure_and_start(config.frame_width, config.frame_height,
                                            config.warmup_frames);
                    break;
                } catch (const rs2::error&) {
                    camera.reset();
                    for (int ms = 0; ms < 2000 && !shutdown_flag.load(); ms += 100) {
                        std::this_thread::sleep_for(std::chrono::milliseconds(100));
                    }
                }
            }
            if (shutdown_flag.load()) return 0;
        } else {
            camera = std::make_unique<RealSensePipeline>();
            camera->configure_and_start(config.frame_width, config.frame_height,
                                        config.warmup_frames);
        }

        const int fw = config.frame_width;
        const int fh = config.frame_height;

        fprintf(stderr, "Camera: %s (USB %s)\n",
                camera->serial_number().c_str(),
                camera->usb_type().c_str());
        fprintf(stderr, "Resolution: %dx%d @ 30fps\n", fw, fh);
        fprintf(stderr, "IMU: %s\n",
                camera->has_imu() ? "detected" : "not detected");
        fprintf(stderr, "Mode: %s\n",
                config.headless ? "headless" : "GUI");

        // ---- Compression infrastructure ------------------------------------
        ZdepthCompressor zdepth_comp(fw, fh);
        H264Encoder h264(fw, fh, 30, config.h264_crf, config.h264_preset);

        // ---- Helper lambdas ------------------------------------------------

        /// Open a new FileWriter and start the writer thread.
        ///
        /// add_timestamp_suffix controls whether a timestamp is appended
        /// to the session name in the output filename. Set to false for headless
        /// mode where the session name is already unique (e.g. "pick_003").
        auto start_recording = [&](const std::string& sname, const std::string& out_dir,
                                   bool add_timestamp_suffix = true) {
            const std::string filepath = make_output_path(out_dir, sname, add_timestamp_suffix);
            last_recording_path_ = filepath;
            fprintf(stderr, "Recording to: %s\n", filepath.c_str());

            // Create new queue and writer
            queue  = std::make_unique<BoundedQueue<CapturedFrame>>(
                         static_cast<size_t>(config.queue_size));
            writer = std::make_unique<FileWriter>(filepath);

            // Assemble and write FileHeader
            FileHeader header = make_file_header(*camera, sname,
                                                 config.h264_crf);
            writer->write_header(header);

            last_capture_ms.store(0, std::memory_order_release);
            recording_active.store(true, std::memory_order_release);
            stats.recording_started();

            // Start writer thread
            writer_thread = std::thread([&]() {
                try {
                    while (true) {
                        auto maybe_frame = queue->pop();
                        if (!maybe_frame) break;

                        auto& frame = *maybe_frame;

                        // H.264 encode RGB (returns owned buffer)
                        auto h264_buf = h264.encode(frame.rgb_data.data(), fw, fh);

                        // Zdepth compress depth -- keyframe every 30 frames (GOP=30)
                        bool keyframe = (frame.frame_number % 30 == 0);
                        auto [zdepth_data, zdepth_size] = zdepth_comp.compress(
                            reinterpret_cast<const uint16_t*>(frame.depth_data.data()),
                            fw, fh, keyframe);

                        std::vector<IMUSampleWire> imu_wire;
                        imu_wire.reserve(frame.imu_samples.size());
                        for (const auto& imu : frame.imu_samples) {
                            IMUSampleWire w;
                            w.timestamp_us = imu.timestamp_us;
                            w.accel_x = imu.accel[0];
                            w.accel_y = imu.accel[1];
                            w.accel_z = imu.accel[2];
                            w.gyro_x  = imu.gyro[0];
                            w.gyro_y  = imu.gyro[1];
                            w.gyro_z  = imu.gyro[2];
                            imu_wire.push_back(w);
                        }

                        writer->write_frame(
                            h264_buf.data(), h264_buf.size(),
                            zdepth_data, zdepth_size,
                            frame.timestamp_us,
                            frame.frame_number,
                            imu_wire);

                        stats.frame_written();
                        stats.bytes_written(h264_buf.size() + zdepth_size);
                    }
                } catch (const std::exception& e) {
                    fprintf(stderr, "[writer] Thread error: %s\n", e.what());
                }
            });
        };

        /// Stop recording: flush H.264, finalize file, join writer thread.
        /// Stop recording. Returns true if a non-empty episode was saved,
        /// false if recording was inactive or the episode had zero frames
        /// (in which case the empty file is deleted).
        auto stop_recording = [&]() -> bool {
            // Atomic exchange: only one caller proceeds, all others return.
            // Prevents TOCTOU race when watchdog (main thread) and
            // handle_disconnect (capture thread) both call stop_recording.
            if (!recording_active.exchange(false)) return false;
            stats.recording_stopped();

            if (queue) {
                queue->close();
            }
            if (writer_thread.joinable()) {
                writer_thread.join();
            }

            bool has_frames = stats.captured() > 0;

            // Flush H.264 encoder AFTER writer thread exits (no more encode() calls)
            // but BEFORE finalize() (file still open for writing)
            if (writer && !writer->is_finalized()) {
                auto flush_buf = h264.flush();
                if (!flush_buf.empty()) {
                    // Write trailing H.264 NAL units without creating an IndexEntry.
                    // The reader recovers these by reading bytes between the last
                    // indexed frame block's end and footer.index_offset.
                    writer->write_trailing_codec_data(flush_buf.data(), flush_buf.size());
                }
                writer->finalize();
            }
            writer.reset();

            if (has_frames && !last_recording_path_.empty()) {
                // Auto-register episode in dataset manifest (no-op if no dataset.json)
                register_episode(config.output_dir, last_recording_path_);
            } else if (!has_frames && !last_recording_path_.empty()) {
                // Discard empty episode file (e.g., soft split when camera
                // never came back). The file is tiny (header + footer only).
                std::error_code ec;
                std::filesystem::remove(last_recording_path_, ec);
            }
            last_recording_path_.clear();

            // Reset H.264 encoder for potential next recording session
            h264.reset();

            // Update dropped count
            if (queue) {
                stats.frames_dropped(queue->dropped());
                queue.reset();
            }

            fprintf(stderr, "\n%s\n", stats.summary().c_str());
            return has_frames;
        };

        // ---- Create presenter ----------------------------------------------
        std::unique_ptr<IPresenter> presenter;

        if (config.headless) {
            // HeadlessPresenter: disk-full triggers clean shutdown
            auto on_request_shutdown = [&]() {
                shutdown_flag.store(true, std::memory_order_release);
            };
            presenter = std::make_unique<HeadlessPresenter>(config, on_request_shutdown);
        } else {
#ifdef HAVE_GUI
            // GUI callbacks -- all invoked from main thread (presenter->tick())
            auto on_start_recording = [&]() {
                if (recording_active.load()) return;
                start_recording(current_session_name, config.output_dir);
            };

            auto on_stop_recording = [&]() {
                bool had_frames = stop_recording();
                if (had_frames) {
                    episode_count++;
                    static_cast<GuiPresenter*>(presenter.get())
                        ->set_episode_count(episode_count);
                }
            };

            auto on_session_name_changed = [&](const std::string& new_name) {
                current_session_name = new_name;
            };

            // on_reconnect_requested: destroy current pipeline, sleep 500ms,
            // recreate, configure_and_start, call on_camera_reconnect,
            // start new recording file if was recording before disconnect.
            auto on_reconnect_requested = [&]() {
                fprintf(stderr, "[gui] Reconnect requested -- recreating camera pipeline...\n");

                const bool was_recording = recording_active.load();

                // Stop recording temporarily (finalize current file)
                if (was_recording) {
                    stop_recording();
                }

                // Destroy current pipeline object
                camera.reset();
                std::this_thread::sleep_for(std::chrono::milliseconds(500));

                // Recreate pipeline -- retry until camera comes back
                bool reconnected = false;
                for (int attempt = 0; attempt < 30 && !shutdown_flag.load(); ++attempt) {
                    try {
                        camera = std::make_unique<RealSensePipeline>();
                        camera->configure_and_start(config.frame_width, config.frame_height,
                                            config.warmup_frames);
                        reconnected = true;
                        break;
                    } catch (const rs2::error& e) {
                        fprintf(stderr, "[gui] Reconnect attempt %d failed: %s\n",
                                attempt + 1, e.what());
                        camera.reset();
                        std::this_thread::sleep_for(std::chrono::milliseconds(2000));
                    }
                }

                if (!reconnected) {
                    fprintf(stderr, "[gui] Camera reconnect failed after retries.\n");
                    shutdown_flag.store(true, std::memory_order_release);
                    return;
                }

                camera_disconnected.store(false, std::memory_order_release);
                presenter->on_camera_reconnect();

                // Open new recording file if was recording before disconnect
                if (was_recording && !current_session_name.empty()) {
                    start_recording(current_session_name, config.output_dir);
                }

                fprintf(stderr, "[gui] Camera reconnected successfully.\n");
            };

            auto gui = std::make_unique<GuiPresenter>(
                config,
                on_start_recording,
                on_stop_recording,
                on_session_name_changed,
                on_reconnect_requested
            );

            // Show dataset name in GUI if output dir has a manifest
            if (has_manifest(config.output_dir)) {
                DatasetManifest ds_manifest;
                if (load_manifest(config.output_dir, ds_manifest)) {
                    gui->set_dataset_name(ds_manifest.name);
                }
            }

            presenter = std::move(gui);
#endif
        }

        // ---- Headless countdown before signaling ready ---------------------
        if (config.headless) {
            fprintf(stderr, "[headless] Camera detected -- starting countdown.\n");
            play_countdown(shutdown_flag);
            if (shutdown_flag.load()) {
                if (camera) camera->stop();
                return 0;
            }
        }

        // ---- Start presenter -----------------------------------------------
        if (!presenter->start()) {
            fprintf(stderr, "Error: presenter failed to start.\n");
            camera->stop();
            return 1;
        }

        // ---- For headless mode: start recording after countdown ------------
        if (config.headless) {
            // Regenerate session name and output dir with fresh timestamps
            // (originals were set before camera wait + countdown delay).
            if (headless_auto_session) {
                session_name = make_session_name(config.output_dir);
            }
            output_dir = make_date_dir(config.output_dir);
            {
                std::error_code ec;
                std::filesystem::create_directories(output_dir, ec);
                if (ec) { output_dir = config.output_dir; }
            }
            start_recording(session_name, output_dir, /*add_timestamp_suffix=*/false);
            fprintf(stderr, "Press Ctrl+C to stop recording.\n\n");
        }

        // ---- Disconnect handler (shared by both error catch paths) ----------
        // Returns true if the capture loop should continue, false to break.
        //
        // Headless: saves episode, sets camera_disconnected, returns false.
        //   The capture thread exits; the main thread handles reconnection
        //   and spawns a new capture thread. This avoids blocking on
        //   librealsense's 15-second internal timeout.
        //
        // GUI: sets camera_disconnected, returns true (capture thread sleeps
        //   until the user clicks Reconnect on the main thread).
        auto handle_disconnect = [&]() -> bool {
            if (config.headless) {
                if (!camera_disconnected.exchange(true, std::memory_order_acq_rel)) {
                    presenter->on_camera_disconnect();
                }
                if (recording_active.load(std::memory_order_acquire)) {
                    if (stop_recording()) {
                        std::thread([]{ play_speech("Episode saved"); }).detach();
                        fprintf(stderr, "[headless] Episode saved.\n");
                    } else {
                        fprintf(stderr, "[headless] Empty episode discarded.\n");
                    }
                }
                return false;  // exit capture loop; main thread reconnects
            } else {
                // GUI mode: show disconnect banner, user triggers reconnect
                camera_disconnected.store(true, std::memory_order_release);
                presenter->on_camera_disconnect();
                return true;
            }
        };

        // ---- Capture thread factory -----------------------------------------
        // Creates a capture thread bound to a generation number. When the
        // generation is superseded (main thread spawned a replacement), the
        // old thread exits at its next opportunity -- even if it was stuck
        // inside librealsense's 15-second internal reconnect timeout.
        auto spawn_capture_thread = [&](int gen) -> std::thread {
            return std::thread([&, gen]() {
                while (!shutdown_flag.load(std::memory_order_acquire)) {
                    // Exit if this thread has been superseded
                    if (capture_generation.load(std::memory_order_acquire) != gen) break;

                    // If camera is disconnected:
                    //   Headless: exit loop, main thread handles reconnect
                    //   GUI:      sleep until main thread reconnects via button
                    if (camera_disconnected.load(std::memory_order_acquire)) {
                        if (config.headless) {
                            break;
                        } else {
                            std::this_thread::sleep_for(std::chrono::milliseconds(100));
                            continue;
                        }
                    }

                    // Camera may be null during reconnect sequence
                    if (!camera) {
                        std::this_thread::sleep_for(std::chrono::milliseconds(100));
                        continue;
                    }

                    // Proactive disconnect detection via rs2 hotplug callback
                    if (camera->is_device_lost()) {
                        fprintf(stderr, "\n[capture] Camera unplugged (hotplug event)\n");
                        if (!handle_disconnect()) break;
                        continue;
                    }

                    try {
                        auto maybe_frame = camera->poll_frame();

                        // After unblocking from poll_frame, check if this
                        // thread has been superseded or the camera disconnected.
                        // This is the earliest safe exit before accessing camera
                        // again (main thread may have moved it to zombie storage).
                        if (capture_generation.load(std::memory_order_acquire) != gen) break;
                        if (camera_disconnected.load(std::memory_order_acquire)) break;

                        if (!maybe_frame) continue;

                        CapturedFrame& frame = *maybe_frame;
                        stats.frame_captured();
                        last_capture_ms.store(
                            std::chrono::duration_cast<std::chrono::milliseconds>(
                                std::chrono::steady_clock::now().time_since_epoch()).count(),
                            std::memory_order_release);

                        // Feed frame to GUI presenter for live preview
#ifdef HAVE_GUI
                        if (!config.headless) {
                            auto* gui = static_cast<GuiPresenter*>(presenter.get());
                            gui->update_frame(
                                frame.rgb_data.data(),
                                reinterpret_cast<const uint16_t*>(frame.depth_data.data()),
                                fw, fh,
                                camera->depth_scale()
                            );
                        }
#endif

                        // Push to writer queue if recording
                        if (recording_active.load(std::memory_order_acquire) && queue) {
                            queue->push(std::move(frame));
                        }

                        // Duration limit check
                        if (max_duration > 0 &&
                            stats.elapsed_seconds() >= static_cast<double>(max_duration)) {
                            shutdown_flag.store(true, std::memory_order_release);
                        }
                    } catch (const rs2::error& e) {
                        if (capture_generation.load(std::memory_order_acquire) != gen) break;
                        fprintf(stderr, "\n[capture] RealSense error: %s\n", e.what());
                        if (!handle_disconnect()) break;
                    }
                }

                // Signal writer to drain -- but only if we're still the
                // active generation (a superseded zombie must not close
                // the new recording's queue).
                if (capture_generation.load(std::memory_order_acquire) == gen && queue) {
                    queue->close();
                }
            });
        };

        std::thread capture_thread = spawn_capture_thread(0);

        // ---- Main loop: presenter tick + stats reporting -------------------
        while (!shutdown_flag.load(std::memory_order_acquire)) {
            // Sync live queue drop count into stats so the presenter sees
            // real-time drops, not just the post-recording total.
            if (queue) {
                stats.set_dropped(queue->dropped());
            }

            // Push latest stats to presenter
            presenter->update_stats(stats);

            // Tick the presenter:
            //   GUI:      renders one ImGui frame, polls GLFW events
            //   Headless: pings watchdog, checks disk space (sleeps 100ms)
            if (!presenter->tick()) {
                // Presenter wants to quit (window closed, disk full, etc.)
                shutdown_flag.store(true, std::memory_order_release);
                break;
            }

            // For headless mode, also print stats to stderr periodically.
            // GuiPresenter renders stats to the overlay -- no stderr needed.
            if (config.headless) {
                static int stat_counter = 0;
                if (++stat_counter % 20 == 0) {  // Every ~2s (20 * 100ms tick)
                    fprintf(stderr, "\r%s", stats.summary().c_str());
                    fflush(stderr);
                }

                // Soft episode split: if recording but no frames for >500ms
                // (and <1s), the camera was briefly disconnected. Split into
                // a new episode without tearing down the capture pipeline.
                if (recording_active.load(std::memory_order_acquire)
                    && !camera_disconnected.load(std::memory_order_acquire))
                {
                    int64_t last_ms = last_capture_ms.load(std::memory_order_acquire);
                    int64_t now_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                        std::chrono::steady_clock::now().time_since_epoch()).count();

                    if (last_ms > 0 && (now_ms - last_ms) > EPISODE_SPLIT_MS
                        && (now_ms - last_ms) <= 1000
                        && stats.captured() > 0)
                    {
                        fprintf(stderr, "\n[headless] Frame gap >%lldms -- soft episode split\n",
                                (long long)EPISODE_SPLIT_MS);
                        stop_recording();
                        std::thread([]{ play_speech("Episode saved"); }).detach();

                        session_name = make_session_name(config.output_dir);
                        start_recording(session_name, output_dir,
                                        /*add_timestamp_suffix=*/false);
                        last_capture_ms.store(now_ms, std::memory_order_release);
                    }
                }

                // Frame-stall watchdog: if recording but no frames for 1 second,
                // camera is likely disconnected. Save episode immediately without
                // waiting for librealsense's 15-second internal reconnect timeout.
                if (recording_active.load() && !camera_disconnected.load()) {
                    int64_t last_ms = last_capture_ms.load(std::memory_order_acquire);
                    int64_t now_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                        std::chrono::steady_clock::now().time_since_epoch()).count();
                    if (last_ms > 0 && (now_ms - last_ms) > 1000) {
                        bool saved = stop_recording();
                        if (saved) {
                            fprintf(stderr, "\n[headless] No frames for 1s -- saving episode\n");
                            std::thread([]{ play_speech("Episode saved"); }).detach();
                        } else {
                            fprintf(stderr, "\n[headless] No frames after split -- entering disconnect recovery\n");
                        }
                        camera_disconnected.store(true, std::memory_order_release);
                        presenter->on_camera_disconnect();
                    }
                }

                // Main-thread reconnect: when camera_disconnected is set
                // (by watchdog above or by capture thread's handle_disconnect),
                // retire the old capture thread and poll for a new camera.
                // The old thread may still be stuck inside librealsense --
                // it's moved to zombie storage and will exit when it unblocks.
                if (camera_disconnected.load(std::memory_order_acquire)
                    && !shutdown_flag.load()) {
                    // Retire old capture thread once (joinable == false after move).
                    // Stop the old pipeline first to release the USB device --
                    // otherwise librealsense's internal reconnect holds the
                    // device for 15 seconds, blocking the new pipeline.
                    if (capture_thread.joinable()) {
                        if (camera) camera->stop();
                        zombie_cameras.push_back(std::move(camera));
                        zombie_threads.push_back(std::move(capture_thread));
                        fprintf(stderr, "[headless] Waiting for camera...\n");
                    }

                    // Try to create a new camera
                    try {
                        camera = std::make_unique<RealSensePipeline>();
                        camera->configure_and_start(config.frame_width, config.frame_height,
                                            config.warmup_frames);

                        // Spawn new capture thread (old zombie exits on gen check)
                        int new_gen = capture_generation.fetch_add(1,
                            std::memory_order_acq_rel) + 1;
                        capture_thread = spawn_capture_thread(new_gen);
                        camera_disconnected.store(false, std::memory_order_release);

                        play_speech("Camera ready");
                        play_countdown(shutdown_flag);
                        if (shutdown_flag.load()) continue;

                        presenter->on_camera_reconnect();
                        session_name = make_session_name(config.output_dir);
                        output_dir = make_date_dir(config.output_dir);
                        {
                            std::error_code ec;
                            std::filesystem::create_directories(output_dir, ec);
                            if (ec) { output_dir = config.output_dir; }
                        }
                        start_recording(session_name, output_dir,
                                        /*add_timestamp_suffix=*/false);
                    } catch (const rs2::error&) {
                        // Camera not available yet, retry next tick (~100ms)
                        camera.reset();
                    }
                }
            }
        }

        // ---- Shutdown sequence ---------------------------------------------
        if (capture_thread.joinable()) {
            capture_thread.join();
        }

        stop_recording();

        // Join zombie capture threads from previous disconnect cycles
        for (auto& t : zombie_threads) {
            if (t.joinable()) t.join();
        }
        zombie_threads.clear();
        zombie_cameras.clear();

        // Final stats
        fprintf(stderr, "\n\nRecording complete.\n%s\n", stats.summary().c_str());

        // Shutdown presenter (sends STOPPING=1 for headless, destroys ImGui for GUI)
        presenter->shutdown();

        // Stop camera (stop() is safe even if device was unplugged)
        if (camera) {
            camera->stop();
        }

        return 0;

    } catch (const rs2::error& e) {
        fprintf(stderr, "\nRealSense error: %s\n", e.what());
        return 1;
    } catch (const std::exception& e) {
        fprintf(stderr, "\nError: %s\n", e.what());
        return 1;
    }
}
