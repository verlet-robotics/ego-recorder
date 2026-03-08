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
//     Designed for unattended systemd service operation. Starts recording
//     immediately with an auto-generated timestamp session name. Integrates with
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
//     New recording file opened after each successful reconnect.
//
// Config + CLI merge:
//   Config file loaded first (--config), then any explicitly-provided CLI flags
//   override. cxxopts count() detects explicit vs default-only.
//
// The FileHeader is assembled in main.cpp (not pipeline.h or storage/) --
// preserves the Phase 1 pipeline-storage decoupling decision.

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

/// Generate a timestamp-based session name: "capture_YYYYMMDD_HHMMSS"
static std::string make_session_name() {
    std::time_t now = std::time(nullptr);
    std::tm* tm_info = std::localtime(&now);
    char ts[32];
    std::strftime(ts, sizeof(ts), "%Y%m%d_%H%M%S", tm_info);
    return std::string("capture_") + ts;
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
/// In headless mode, make_session_name() already embeds the timestamp inside
/// the session name (e.g. "capture_20260219_090520"), so appending a second
/// timestamp would produce a double-timestamp filename.
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

// ---- main ------------------------------------------------------------------

int main(int argc, char* argv[]) {
    // ---- Subcommand dispatch (before cxxopts parsing) ----------------------
    if (argc >= 2) {
        std::string_view cmd = argv[1];

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

        // ---- export subcommand (dispatches to Python scripts) ----
        // Per locked decision: `ego-recorder export rlds` and `ego-recorder export lerobot`
        if (cmd == "export") {
            if (argc < 3) {
                fprintf(stderr, "Usage: ego-recorder export <rlds|lerobot> [options] <file.egorec> [...]\n");
                return 1;
            }
            std::string_view format = argv[2];

            // Locate the Python script relative to the binary.
            // Try: (1) ../python/ relative to binary, (2) ./python/ from cwd
            std::string binary_path = std::filesystem::canonical(
                std::filesystem::path(argv[0])).parent_path().string();
            std::string script;

            if (format == "rlds") {
                script = "export_rlds.py";
            } else if (format == "lerobot") {
                script = "export_lerobot.py";
            } else {
                fprintf(stderr, "Error: unknown export format '%.*s'\n",
                        static_cast<int>(format.size()), format.data());
                fprintf(stderr, "Supported formats: rlds, lerobot\n");
                return 1;
            }

            // Search for the script in likely locations
            std::vector<std::string> search_paths = {
                binary_path + "/../python/" + script,
                binary_path + "/python/" + script,
                "python/" + script,
            };

            std::string script_path;
            for (const auto& p : search_paths) {
                if (std::filesystem::exists(p)) {
                    script_path = std::filesystem::canonical(p).string();
                    break;
                }
            }

            if (script_path.empty()) {
                fprintf(stderr, "Error: could not find %s\n", script.c_str());
                fprintf(stderr, "Looked in:\n");
                for (const auto& p : search_paths) {
                    fprintf(stderr, "  %s\n", p.c_str());
                }
                fprintf(stderr, "\nYou can also run directly:\n");
                fprintf(stderr, "  PYTHONPATH=build python python/%s [options] <files>\n",
                        script.c_str());
                return 1;
            }

            // Build argv for Python: python3 script_path [remaining args...]
            // Skip argv[0] (ego-recorder) and argv[1] (export) and argv[2] (format)
            std::vector<std::string> py_args = {"python3", script_path};
            for (int i = 3; i < argc; ++i) {
                py_args.push_back(argv[i]);
            }

            // Build C-style argv for execvp
            std::vector<char*> c_args;
            for (auto& a : py_args) {
                c_args.push_back(a.data());
            }
            c_args.push_back(nullptr);

            // Set PYTHONPATH to include the build directory (for egorec_reader.so)
            std::string pythonpath = binary_path;
            const char* existing = std::getenv("PYTHONPATH");
            if (existing && existing[0] != '\0') {
                pythonpath += ":";
                pythonpath += existing;
            }
            setenv("PYTHONPATH", pythonpath.c_str(), 1);

            execvp("python3", c_args.data());

            // execvp only returns on error
            fprintf(stderr, "Error: failed to exec python3: %s\n", strerror(errno));
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
        ("queue-size",     "Bounded queue size (2-16)",
         cxxopts::value<int>()->default_value("0"))  // 0 = use config value
        ("warmup",         "Camera warmup frames to skip",
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
    if (args.count("queue-size") && args["queue-size"].as<int>() != 0) {
        config.queue_size = args["queue-size"].as<int>();
    }
    if (args.count("warmup") && args["warmup"].as<int>() != 0) {
        config.warmup_frames = args["warmup"].as<int>();
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

    if (config.headless) {
        // Auto-generate timestamp-based session name if not explicitly provided
        if (session_name.empty() || session_name == "capture") {
            // "capture" is the default from Config; treat as unset in headless mode
            if (args.count("session-name") == 0 ||
                args["session-name"].as<std::string>().empty()) {
                session_name = make_session_name();
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
    std::string                    current_session_name = session_name;

    // ---- Outer try/catch ---------------------------------------------------
    try {
        // ---- Camera initialization -----------------------------------------
        auto camera = std::make_unique<RealSensePipeline>();
        camera->configure_and_start(config.warmup_frames);

        fprintf(stderr, "Camera: %s (USB %s)\n",
                camera->serial_number().c_str(),
                camera->usb_type().c_str());
        fprintf(stderr, "IMU: %s\n",
                camera->has_imu() ? "detected" : "not detected");
        fprintf(stderr, "Mode: %s\n",
                config.headless ? "headless" : "GUI");

        // ---- Compression infrastructure ------------------------------------
        ZdepthCompressor zdepth_comp(640, 480);
        H264Encoder h264(640, 480, 30, config.h264_crf);

        // ---- Helper lambdas ------------------------------------------------

        /// Open a new FileWriter and start the writer thread.
        ///
        /// add_timestamp_suffix controls whether a second timestamp is appended
        /// to the session name in the output filename. Set to false for headless
        /// mode where the session name already contains a timestamp from
        /// make_session_name(), to avoid double-timestamp filenames like
        /// "capture_20260219_090520_20260219_090522.egorec".
        auto start_recording = [&](const std::string& sname, const std::string& out_dir,
                                   bool add_timestamp_suffix = true) {
            const std::string filepath = make_output_path(out_dir, sname, add_timestamp_suffix);
            fprintf(stderr, "Recording to: %s\n", filepath.c_str());

            // Create new queue and writer
            queue  = std::make_unique<BoundedQueue<CapturedFrame>>(
                         static_cast<size_t>(config.queue_size));
            writer = std::make_unique<FileWriter>(filepath);

            // Assemble and write FileHeader
            FileHeader header = make_file_header(*camera, sname,
                                                 config.h264_crf);
            writer->write_header(header);

            recording_active.store(true, std::memory_order_release);
            stats.recording_started();

            // Start writer thread
            writer_thread = std::thread([&]() {
                while (true) {
                    auto maybe_frame = queue->pop();
                    if (!maybe_frame) break;

                    auto& frame = *maybe_frame;

                    // H.264 encode RGB (returns pointer to internal buffer)
                    auto [h264_data, h264_size] = h264.encode(frame.rgb_data.data(), 640, 480);

                    // Zdepth compress depth -- keyframe every 30 frames (GOP=30)
                    bool keyframe = (frame.frame_number % 30 == 0);
                    auto [zdepth_data, zdepth_size] = zdepth_comp.compress(
                        reinterpret_cast<const uint16_t*>(frame.depth_data.data()),
                        640, 480, keyframe);

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
                        h264_data, h264_size,
                        zdepth_data, zdepth_size,
                        frame.timestamp_us,
                        frame.frame_number,
                        imu_wire);

                    stats.frame_written();
                    stats.bytes_written(h264_size + zdepth_size);
                }
            });
        };

        /// Stop recording: flush H.264, finalize file, join writer thread.
        auto stop_recording = [&]() {
            if (!recording_active.load()) return;

            recording_active.store(false, std::memory_order_release);
            stats.recording_stopped();

            if (queue) {
                queue->close();
            }
            if (writer_thread.joinable()) {
                writer_thread.join();
            }

            // Flush H.264 encoder AFTER writer thread exits (no more encode() calls)
            // but BEFORE finalize() (file still open for writing)
            if (writer && !writer->is_finalized()) {
                auto [flush_data, flush_size] = h264.flush();
                if (flush_size > 0) {
                    // Write trailing H.264 NAL units without creating an IndexEntry.
                    // The reader recovers these by reading bytes between the last
                    // indexed frame block's end and footer.index_offset.
                    writer->write_trailing_codec_data(flush_data, flush_size);
                }
                writer->finalize();
            }
            writer.reset();

            // Reset H.264 encoder for potential next recording session
            h264.reset();

            // Update dropped count
            if (queue) {
                stats.frames_dropped(queue->dropped());
                queue.reset();
            }

            fprintf(stderr, "\n%s\n", stats.summary().c_str());
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
                stop_recording();
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
                        camera->configure_and_start(config.warmup_frames);
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

            presenter = std::make_unique<GuiPresenter>(
                config,
                on_start_recording,
                on_stop_recording,
                on_session_name_changed,
                on_reconnect_requested
            );
#endif
        }

        // ---- Start presenter -----------------------------------------------
        if (!presenter->start()) {
            fprintf(stderr, "Error: presenter failed to start.\n");
            camera->stop();
            return 1;
        }

        // ---- For headless mode: start recording immediately ----------------
        if (config.headless) {
            // Pass false: session_name already contains a timestamp from
            // make_session_name(), so no second timestamp should be appended.
            start_recording(session_name, output_dir, /*add_timestamp_suffix=*/false);
            fprintf(stderr, "Press Ctrl+C to stop recording.\n\n");
        }

        // ---- Capture thread ------------------------------------------------
        // Always runs to feed live preview frames (GUI mode) or record frames
        // (headless mode). In GUI mode, frames are pushed to GuiPresenter for
        // preview even before recording starts.
        std::thread capture_thread([&]() {
            while (!shutdown_flag.load(std::memory_order_acquire)) {
                // If camera is disconnected (GUI mode), wait for reconnect callback
                if (camera_disconnected.load(std::memory_order_acquire)) {
                    std::this_thread::sleep_for(std::chrono::milliseconds(100));
                    continue;
                }

                // Camera may be null during reconnect sequence
                if (!camera) {
                    std::this_thread::sleep_for(std::chrono::milliseconds(100));
                    continue;
                }

                try {
                    CapturedFrame frame = camera->poll_frame();
                    stats.frame_captured();

                    // Feed frame to GUI presenter for live preview
#ifdef HAVE_GUI
                    if (!config.headless) {
                        auto* gui = static_cast<GuiPresenter*>(presenter.get());
                        gui->update_frame(
                            frame.rgb_data.data(),
                            reinterpret_cast<const uint16_t*>(frame.depth_data.data()),
                            640, 480,
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
                } catch (const rs2::camera_disconnected_error& e) {
                    fprintf(stderr, "\n[capture] Camera disconnected: %s\n", e.what());

                    if (config.headless) {
                        // Headless: notify presenter, then auto-retry every 2s
                        presenter->on_camera_disconnect();
                        camera_disconnected.store(true, std::memory_order_release);

                        // Finalize current recording file (new file opened after reconnect)
                        stop_recording();

                        // Auto-retry loop: destroy + sleep + recreate pipeline
                        bool reconnected = false;
                        while (!shutdown_flag.load() && !reconnected) {
                            camera.reset();
                            std::this_thread::sleep_for(std::chrono::milliseconds(500));

                            try {
                                camera = std::make_unique<RealSensePipeline>();
                                camera->configure_and_start(config.warmup_frames);
                                reconnected = true;
                            } catch (const rs2::error&) {
                                camera.reset();
                                // Wait remainder of 2s retry interval
                                std::this_thread::sleep_for(std::chrono::milliseconds(1500));
                            }
                        }

                        if (reconnected && !shutdown_flag.load()) {
                            camera_disconnected.store(false, std::memory_order_release);
                            presenter->on_camera_reconnect();
                            // Open new recording file after reconnect.
                            // Generate a fresh session name with the current
                            // timestamp so each reconnect produces a distinct file.
                            session_name = make_session_name();
                            start_recording(session_name, output_dir,
                                            /*add_timestamp_suffix=*/false);
                        }
                    } else {
                        // GUI mode: notify presenter (shows banner + Reconnect button)
                        // Do NOT auto-retry -- user triggers reconnect via button
                        camera_disconnected.store(true, std::memory_order_release);
                        presenter->on_camera_disconnect();
                        // Camera pipeline is left alive; reconnect lambda destroys it
                    }
                } catch (const rs2::error& e) {
                    fprintf(stderr, "\n[capture] RealSense error: %s\n", e.what());
                    shutdown_flag.store(true, std::memory_order_release);
                }
            }

            // Signal writer to drain and exit
            if (queue) {
                queue->close();
            }
        });

        // ---- Main loop: presenter tick + stats reporting -------------------
        while (!shutdown_flag.load(std::memory_order_acquire)) {
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
            }
        }

        // ---- Shutdown sequence ---------------------------------------------
        capture_thread.join();

        stop_recording();

        // Final stats
        fprintf(stderr, "\n\nRecording complete.\n%s\n", stats.summary().c_str());

        // Shutdown presenter (sends STOPPING=1 for headless, destroys ImGui for GUI)
        presenter->shutdown();

        // Stop camera
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
