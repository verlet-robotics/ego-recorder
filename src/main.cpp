// ego-recorder: synchronized RGBD capture from Intel RealSense D435/D435i.
//
// Three-thread pipeline:
//   - Capture thread: polls camera at 30fps, enqueues CapturedFrames
//   - Writer thread:  dequeues frames, compresses (JPEG + ZSTD), writes to .egorec
//   - Main thread:    signal handling (via setup_signal_handling), stats reporting
//
// The FileHeader is assembled here (main.cpp) where both pipeline.h and
// binary_format.h are available, keeping those two modules decoupled.

#include <cxxopts.hpp>

#include "capture/pipeline.h"
#include "capture/frame_types.h"
#include "threading/bounded_queue.h"
#include "compression/jpeg_compressor.h"
#include "compression/zstd_compressor.h"
#include "storage/binary_format.h"
#include "storage/file_writer.h"
#include "utils/signal_handler.h"
#include "utils/stats.h"

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include <librealsense2/rs.hpp>

// ---- Helpers ---------------------------------------------------------------

/// Generate output filename: {dir}/{session}_{YYYYMMDD_HHMMSS}.egorec
static std::string make_output_path(const std::string& output_dir,
                                    const std::string& session_name) {
    std::time_t now = std::time(nullptr);
    std::tm* tm_info = std::localtime(&now);
    char ts[32];
    std::strftime(ts, sizeof(ts), "%Y%m%d_%H%M%S", tm_info);

    std::string path = output_dir;
    // Ensure trailing slash
    if (!path.empty() && path.back() != '/') {
        path += '/';
    }
    path += session_name;
    path += '_';
    path += ts;
    path += ".egorec";
    return path;
}

/// Microseconds since Unix epoch from system_clock.
static uint64_t now_us() {
    using namespace std::chrono;
    return static_cast<uint64_t>(
        duration_cast<microseconds>(system_clock::now().time_since_epoch()).count());
}

// ---- main ------------------------------------------------------------------

int main(int argc, char* argv[]) {
    // ---- CLI Parsing -------------------------------------------------------
    cxxopts::Options options("ego-recorder",
        "Record synchronized RGBD data from Intel RealSense D435/D435i to .egorec files");

    options.add_options()
        ("o,output",       "Output directory",
         cxxopts::value<std::string>()->default_value("."))
        ("s,session-name", "Session name (used in output filename)",
         cxxopts::value<std::string>()->default_value("capture"))
        ("d,duration",     "Max recording duration in seconds (0 = unlimited)",
         cxxopts::value<int>()->default_value("0"))
        ("q,quality",      "JPEG quality 1-100",
         cxxopts::value<int>()->default_value("90"))
        ("z,zstd-level",   "ZSTD compression level 1-22",
         cxxopts::value<int>()->default_value("1"))
        ("queue-size",     "Bounded queue size (2-16)",
         cxxopts::value<int>()->default_value("4"))
        ("warmup",         "Camera warmup frames to skip",
         cxxopts::value<int>()->default_value("30"))
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

    const std::string output_dir   = args["output"].as<std::string>();
    const std::string session_name = args["session-name"].as<std::string>();
    const int         max_duration = args["duration"].as<int>();
    const int         jpeg_quality = args["quality"].as<int>();
    const int         zstd_level   = args["zstd-level"].as<int>();
    const int         queue_size   = args["queue-size"].as<int>();
    const int         warmup_frames = args["warmup"].as<int>();

    // Validate ranges
    if (jpeg_quality < 1 || jpeg_quality > 100) {
        fprintf(stderr, "Error: --quality must be 1-100 (got %d)\n", jpeg_quality);
        return 1;
    }
    if (zstd_level < 1 || zstd_level > 22) {
        fprintf(stderr, "Error: --zstd-level must be 1-22 (got %d)\n", zstd_level);
        return 1;
    }
    if (queue_size < 2 || queue_size > 16) {
        fprintf(stderr, "Error: --queue-size must be 2-16 (got %d)\n", queue_size);
        return 1;
    }

    const std::string output_filepath = make_output_path(output_dir, session_name);

    // ---- Signal handling (MUST be first, before any thread creation) -------
    std::atomic<bool> shutdown_flag{false};
    setup_signal_handling(shutdown_flag);

    // ---- Outer try/catch for RealSense and std exceptions ------------------
    FileWriter* writer_ptr = nullptr;
    try {
        // ---- Camera initialization -----------------------------------------
        RealSensePipeline camera;
        camera.configure_and_start(warmup_frames);

        fprintf(stderr, "Camera: %s (USB %s)\n",
                camera.serial_number().c_str(),
                camera.usb_type().c_str());
        fprintf(stderr, "IMU: %s\n",
                camera.has_imu() ? "detected" : "not detected");
        fprintf(stderr, "Recording to: %s\n", output_filepath.c_str());
        fprintf(stderr, "Press Ctrl+C to stop recording.\n\n");

        // ---- File writer ---------------------------------------------------
        FileWriter writer(output_filepath);
        writer_ptr = &writer;

        // ---- Assemble FileHeader from pipeline getters ---------------------
        FileHeader header;
        std::memset(&header, 0, sizeof(header));
        std::memcpy(header.magic, FILE_MAGIC, sizeof(FILE_MAGIC));
        header.header_size = sizeof(FileHeader);
        header.flags = camera.has_imu() ? 0x01u : 0x00u;  // bit 0: has_imu

        // Serial number
        std::strncpy(header.serial_number,
                     camera.serial_number().c_str(),
                     sizeof(header.serial_number) - 1);

        // Depth scale
        header.depth_scale = camera.depth_scale();

        // Depth intrinsics
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

        // Color intrinsics
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

        // Extrinsics (depth to color)
        auto ex = camera.depth_to_color_extrinsics();
        static_assert(sizeof(header.extrinsic_rotation) == sizeof(ex.rotation),
                      "extrinsic rotation array size mismatch");
        static_assert(sizeof(header.extrinsic_translation) == sizeof(ex.translation),
                      "extrinsic translation array size mismatch");
        std::memcpy(header.extrinsic_rotation, ex.rotation,
                    sizeof(header.extrinsic_rotation));
        std::memcpy(header.extrinsic_translation, ex.translation,
                    sizeof(header.extrinsic_translation));

        // USB type
        std::strncpy(header.usb_type,
                     camera.usb_type().c_str(),
                     sizeof(header.usb_type) - 1);

        // Session metadata
        std::strncpy(header.session_name,
                     session_name.c_str(),
                     sizeof(header.session_name) - 1);
        header.start_timestamp_us = now_us();

        // Compression settings
        header.rgb_codec   = 1;  // JPEG
        header.depth_codec = 1;  // ZSTD
        header.rgb_quality = static_cast<uint8_t>(jpeg_quality);
        header.zstd_level  = static_cast<uint8_t>(zstd_level);

        writer.write_header(header);

        // ---- Thread infrastructure -----------------------------------------
        BoundedQueue<CapturedFrame> queue(static_cast<size_t>(queue_size));
        Stats stats;

        // 640x480 RGB24 = 921,600 bytes; Z16 = 614,400 bytes
        JpegCompressor  jpeg(640, 480, jpeg_quality);
        ZstdCompressor  zstd(640 * 480 * 2, zstd_level);

        // ---- Capture thread ------------------------------------------------
        std::thread capture_thread([&]() {
            while (!shutdown_flag.load(std::memory_order_acquire)) {
                try {
                    CapturedFrame frame = camera.poll_frame();
                    stats.frame_captured();
                    queue.push(std::move(frame));

                    // Duration limit check
                    if (max_duration > 0 &&
                        stats.elapsed_seconds() >= static_cast<double>(max_duration)) {
                        shutdown_flag.store(true, std::memory_order_release);
                    }
                } catch (const rs2::error& e) {
                    fprintf(stderr, "\nRealSense error in capture thread: %s\n", e.what());
                    shutdown_flag.store(true, std::memory_order_release);
                }
            }
            queue.close();  // Signal writer to drain and exit
        });

        // ---- Writer thread -------------------------------------------------
        std::thread writer_thread([&]() {
            while (true) {
                auto maybe_frame = queue.pop();
                if (!maybe_frame) break;  // Queue closed and empty

                auto& frame = *maybe_frame;

                // Compress RGB
                auto [jpeg_data, jpeg_size] = jpeg.compress(
                    frame.rgb_data.data(), 640, 480);

                // Compress depth
                auto [zstd_data, zstd_size] = zstd.compress(
                    frame.depth_data.data(), frame.depth_data.size());

                // Convert IMU samples to wire format
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

                // Write frame to file
                writer.write_frame(
                    jpeg_data, jpeg_size,
                    zstd_data, zstd_size,
                    frame.timestamp_us,
                    frame.frame_number,
                    imu_wire);

                stats.frame_written();
                stats.bytes_written(jpeg_size + zstd_size);
            }
        });

        // ---- Stats reporting (main thread) ---------------------------------
        while (!shutdown_flag.load(std::memory_order_acquire)) {
            std::this_thread::sleep_for(std::chrono::seconds(2));
            fprintf(stderr, "\r%s", stats.summary().c_str());
            fflush(stderr);
        }

        // ---- Shutdown sequence ---------------------------------------------
        capture_thread.join();

        // Writer drains remaining queue items then exits
        writer_thread.join();

        // Update dropped count from queue
        stats.frames_dropped(queue.dropped());

        // Finalize file (write index table + footer)
        writer.finalize();
        writer_ptr = nullptr;

        // Stop camera
        camera.stop();

        // Print final summary
        fprintf(stderr, "\n\nRecording complete.\n%s\n", stats.summary().c_str());
        fprintf(stderr, "Output: %s\n", output_filepath.c_str());

        return 0;

    } catch (const rs2::error& e) {
        fprintf(stderr, "\nRealSense error: %s\n", e.what());
        // Best-effort finalize if file was opened
        if (writer_ptr && !writer_ptr->is_finalized()) {
            try { writer_ptr->finalize(); } catch (...) {}
        }
        return 1;
    } catch (const std::exception& e) {
        fprintf(stderr, "\nError: %s\n", e.what());
        if (writer_ptr && !writer_ptr->is_finalized()) {
            try { writer_ptr->finalize(); } catch (...) {}
        }
        return 1;
    }
}
