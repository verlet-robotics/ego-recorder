#ifdef HAVE_DEPTHAI

#include "capture/oakd_pipeline.h"

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <stdexcept>

// ---- NV12 -> RGB24 conversion ----------------------------------------------
// OAK-D outputs NV12 (Y plane + interleaved UV plane). We convert to tightly-
// packed RGB24 to match what downstream code (preview, H.264 encoder) expects.

void OakDPipeline::nv12_to_rgb24(const uint8_t* nv12, int width, int height,
                                  std::vector<uint8_t>& rgb_out) {
    rgb_out.resize(static_cast<size_t>(width) * height * 3);
    const uint8_t* y_plane  = nv12;
    const uint8_t* uv_plane = nv12 + width * height;

    for (int row = 0; row < height; ++row) {
        for (int col = 0; col < width; ++col) {
            int y_idx  = row * width + col;
            int uv_idx = (row / 2) * width + (col & ~1);  // UV is half-height, paired columns

            int Y = static_cast<int>(y_plane[y_idx]);
            int U = static_cast<int>(uv_plane[uv_idx])     - 128;
            int V = static_cast<int>(uv_plane[uv_idx + 1]) - 128;

            int R = Y + ((359 * V) >> 8);
            int G = Y - ((88 * U + 183 * V) >> 8);
            int B = Y + ((454 * U) >> 8);

            size_t out_idx = static_cast<size_t>(y_idx) * 3;
            rgb_out[out_idx + 0] = static_cast<uint8_t>(std::clamp(R, 0, 255));
            rgb_out[out_idx + 1] = static_cast<uint8_t>(std::clamp(G, 0, 255));
            rgb_out[out_idx + 2] = static_cast<uint8_t>(std::clamp(B, 0, 255));
        }
    }
}

// ---- USB speed to string ---------------------------------------------------

std::string OakDPipeline::usb_speed_to_string(dai::UsbSpeed speed) {
    switch (speed) {
    case dai::UsbSpeed::UNKNOWN:       return "unknown";
    case dai::UsbSpeed::LOW:           return "1.0";
    case dai::UsbSpeed::FULL:          return "1.1";
    case dai::UsbSpeed::HIGH:          return "2.0";
    case dai::UsbSpeed::SUPER:         return "3.0";
    case dai::UsbSpeed::SUPER_PLUS:    return "3.1";
    default:                           return "unknown";
    }
}

// ---- configure_and_start ---------------------------------------------------

void OakDPipeline::configure_and_start(int width, int height, int warmup_frames) {
    try {
        // -----------------------------------------------------------------
        // 0. Resolution strategy for wide-angle FOV preservation
        //
        // The OAK-D Wide RGB sensor (IMX378) is 4:3 native (4056x3040).
        // Requesting 16:9 (e.g. 1280x720) with crop mode discards vertical
        // FOV — defeating the purpose of "Wide". We snap to the closest
        // 4:3 resolution that fits the requested width, preserving the full
        // sensor FOV. Stereo mono cameras (OV9282, 1280x800, ~8:5) are
        // requested at half native resolution to stay within the Myriad X
        // processing budget — the StereoDepth node handles alignment and
        // resize to the color resolution internally.
        // -----------------------------------------------------------------
        const int color_w = width;
        // Snap height to 4:3 aspect (rounded to multiple of 8 for codec compat)
        int color_h_43 = (width * 3) / 4;
        color_h_43 = (color_h_43 + 7) & ~7;  // Round up to next multiple of 8
        // Always use 4:3 height to preserve the full wide-angle sensor FOV.
        const int color_h = color_h_43;

        color_width_  = color_w;
        color_height_ = color_h;

        fprintf(stderr, "OAK-D: requesting %dx%d color (4:3 for full FOV, "
                "caller asked %dx%d)\n", color_w, color_h, width, height);

        // Stereo mono input: half native OV9282 resolution (640x400).
        // Full resolution (1280x800) overwhelms the Myriad X stereo block.
        constexpr int STEREO_W = 640;
        constexpr int STEREO_H = 400;

        // -----------------------------------------------------------------
        // 1. Create the device and a pipeline bound to it.
        //    DepthAI v3 requires the device to exist before the pipeline
        //    graph is built; the queues we create from node outputs need a
        //    real device to attach to. We defer startPipeline() until after
        //    the graph is fully wired so firmware upload happens once.
        // -----------------------------------------------------------------
        device_ = std::make_shared<dai::Device>();
        // DepthAI v3 relays device logs through stdout, which corrupts the
        // preview subprocess's tagged-frame protocol with the Tauri app.
        // Raise the device log level so only CRITICAL messages leak through.
        device_->setLogLevel(dai::LogLevel::CRITICAL);
        device_->setMaxReconnectionAttempts(10);
        pipeline_ = std::make_unique<dai::Pipeline>(device_);
        auto& pipeline = *pipeline_;

        // Color camera (CAM_A = center RGB)
        auto camRgb = pipeline.create<dai::node::Camera>()->build(
            dai::CameraBoardSocket::CAM_A);
        // Use LETTERBOX resize to preserve full sensor FOV without distortion.
        // Any black bars from aspect mismatch are minimal since we're requesting
        // close to the native 4:3 aspect.
        auto colorOut = camRgb->requestOutput(
            {color_w, color_h}, dai::ImgFrame::Type::NV12,
            dai::ImgResizeMode::LETTERBOX, 30.0f);

        // Stereo cameras (CAM_B = left, CAM_C = right)
        auto left = pipeline.create<dai::node::Camera>()->build(
            dai::CameraBoardSocket::CAM_B);
        auto right = pipeline.create<dai::node::Camera>()->build(
            dai::CameraBoardSocket::CAM_C);

        // Stereo depth — feed at half-res mono for processing headroom
        auto stereo = pipeline.create<dai::node::StereoDepth>();
        left->requestOutput({STEREO_W, STEREO_H})->link(stereo->left);
        right->requestOutput({STEREO_W, STEREO_H})->link(stereo->right);
        stereo->setDefaultProfilePreset(dai::node::StereoDepth::PresetMode::DEFAULT);
        stereo->setDepthAlign(dai::CameraBoardSocket::CAM_A);  // Align depth to color
        stereo->setOutputSize(color_w, color_h);  // Output at color resolution

        // Sync node: align color + depth by timestamp
        auto sync = pipeline.create<dai::node::Sync>();
        colorOut->link(sync->inputs["color"]);
        stereo->depth.link(sync->inputs["depth"]);

        // Output queues — v3 API: create directly from node outputs
        auto syncQueue = sync->out.createOutputQueue();

        // IMU (optional — not all OAK-D models have it)
        std::shared_ptr<dai::node::IMU> imu;
        std::shared_ptr<dai::MessageQueue> imuQueue;
        try {
            imu = pipeline.create<dai::node::IMU>();
            imu->enableIMUSensor(dai::IMUSensor::ACCELEROMETER_RAW, 480);
            imu->enableIMUSensor(dai::IMUSensor::GYROSCOPE_RAW, 400);
            imu->setBatchReportThreshold(5);
            imu->setMaxBatchReports(20);
            imuQueue = imu->out.createOutputQueue();
        } catch (...) {
            imu.reset();
            imuQueue.reset();
            fprintf(stderr, "OAK-D: IMU not available on this model\n");
        }

        // -----------------------------------------------------------------
        // 2. Boot the pipeline on the device (firmware upload ~2-3 seconds).
        //    pipeline.start() is the v3 canonical entry point when the
        //    Pipeline was constructed with an explicit Device; it wires up
        //    the host-side queues and kicks off firmware execution.
        // -----------------------------------------------------------------
        pipeline.start();

        // -----------------------------------------------------------------
        // 3. Store output queues
        // -----------------------------------------------------------------
        sync_queue_ = syncQueue;

        if (imu && imuQueue) {
            imu_queue_ = imuQueue;
            has_imu_ = true;
            fprintf(stderr, "OAK-D: IMU detected (BNO086)\n");
        }

        // -----------------------------------------------------------------
        // 4. Device info
        // -----------------------------------------------------------------
        serial_number_ = device_->getDeviceId();
        usb_type_      = usb_speed_to_string(device_->getUsbSpeed());

        if (!usb_type_.empty() && usb_type_[0] == '2') {
            fprintf(stderr,
                "USB_WARNING: OAK-D on USB 2.0 port. Use USB 3.0 for reliable operation.\n");
        }

        // -----------------------------------------------------------------
        // 5. Try to enable IR projector/flood light (OAK-D Pro only)
        // -----------------------------------------------------------------
        try {
            device_->setIrLaserDotProjectorIntensity(0.8f);
            device_->setIrFloodLightIntensity(0.5f);
            fprintf(stderr, "OAK-D Pro: IR projector + flood light enabled\n");
        } catch (...) {
            // Non-Pro model -- no IR projector, this is expected
        }

        // -----------------------------------------------------------------
        // 6. Read calibration
        // -----------------------------------------------------------------
        auto calib = device_->readCalibration();

        // Color intrinsics — use actual output resolution (color_w x color_h),
        // not the caller's original width/height, since we adjusted for 4:3 FOV.
        {
            auto M = calib.getCameraIntrinsics(dai::CameraBoardSocket::CAM_A, color_w, color_h);
            auto dist = calib.getDistortionCoefficients(dai::CameraBoardSocket::CAM_A);

            color_intrinsics_.width  = color_w;
            color_intrinsics_.height = color_h;
            color_intrinsics_.fx     = M[0][0];
            color_intrinsics_.fy     = M[1][1];
            color_intrinsics_.ppx    = M[0][2];
            color_intrinsics_.ppy    = M[1][2];
            // Map to RS2_DISTORTION_BROWN_CONRADY (4) for .egorec compatibility
            color_intrinsics_.distortion_model = 4;
            for (int i = 0; i < 5 && i < static_cast<int>(dist.size()); ++i) {
                color_intrinsics_.distortion_coeffs[i] = dist[i];
            }
        }

        // Depth intrinsics (aligned to color, so same socket + resolution)
        // Since depth is aligned to CAM_A via setDepthAlign(), use CAM_A
        // intrinsics at the actual color output resolution.
        {
            auto M = calib.getCameraIntrinsics(dai::CameraBoardSocket::CAM_A, color_w, color_h);
            auto dist = calib.getDistortionCoefficients(dai::CameraBoardSocket::CAM_A);

            depth_intrinsics_.width  = color_w;
            depth_intrinsics_.height = color_h;
            depth_intrinsics_.fx     = M[0][0];
            depth_intrinsics_.fy     = M[1][1];
            depth_intrinsics_.ppx    = M[0][2];
            depth_intrinsics_.ppy    = M[1][2];
            depth_intrinsics_.distortion_model = 4;
            for (int i = 0; i < 5 && i < static_cast<int>(dist.size()); ++i) {
                depth_intrinsics_.distortion_coeffs[i] = dist[i];
            }
        }

        // Extrinsics (depth/left stereo -> color)
        // CRITICAL: DepthAI returns translation in CENTIMETERS, not meters!
        {
            auto ex = calib.getCameraExtrinsics(
                dai::CameraBoardSocket::CAM_B, dai::CameraBoardSocket::CAM_A);

            // ex is a 4x4 transformation matrix (or 3x4)
            // Rotation is the upper-left 3x3, translation is the right column
            for (int r = 0; r < 3; ++r) {
                for (int c = 0; c < 3; ++c) {
                    depth_to_color_extrinsics_.rotation[r * 3 + c] = ex[r][c];
                }
                // Translation in centimeters -> convert to meters
                depth_to_color_extrinsics_.translation[r] = ex[r][3] * 0.01f;
            }
        }

        fprintf(stderr, "OAK-D: calibration loaded (color: %dx%d, fx=%.1f fy=%.1f)\n",
                color_intrinsics_.width, color_intrinsics_.height,
                color_intrinsics_.fx, color_intrinsics_.fy);

        // -----------------------------------------------------------------
        // 7. Warmup -- drop first N frames for auto-exposure stabilization
        // -----------------------------------------------------------------
        fprintf(stderr, "Warming up camera (%d frames)...\n", warmup_frames);
        {
            auto warmup_start = std::chrono::steady_clock::now();
            // OAK-D needs a longer deadline than RealSense: firmware upload
            // takes 2-3 seconds, then each warmup frame poll has a 2s timeout.
            constexpr auto warmup_deadline = std::chrono::seconds(20);

            for (int i = 0; i < warmup_frames; ++i) {
                auto elapsed = std::chrono::steady_clock::now() - warmup_start;
                if (elapsed >= warmup_deadline) {
                    fprintf(stderr, "Warmup deadline reached after %d/%d frames\n",
                            i, warmup_frames);
                    break;
                }

                bool warmup_timed_out = false;
                auto msg = sync_queue_->get<dai::MessageGroup>(
                    std::chrono::milliseconds(2000), warmup_timed_out);
                if (warmup_timed_out || !msg) {
                    fprintf(stderr, "Warmup frame %d/%d timed out\n", i + 1, warmup_frames);
                }
            }
        }
        fprintf(stderr, "Camera ready.\n");

        // -----------------------------------------------------------------
        // 8. Initialize frame counter
        // -----------------------------------------------------------------
        frame_counter_ = 0;

    } catch (const std::exception& e) {
        throw std::runtime_error(
            std::string("OAK-D: failed to configure and start: ") + e.what());
    }
}

// ---- stop ------------------------------------------------------------------

void OakDPipeline::stop() {
    try {
        if (device_) {
            device_->close();
        }
    } catch (...) {
        // Device may already be gone (USB unplug)
    }
    sync_queue_.reset();
    imu_queue_.reset();
    pipeline_.reset();
    device_.reset();
}

// ---- poll_frame ------------------------------------------------------------

std::optional<CapturedFrame> OakDPipeline::poll_frame(unsigned int timeout_ms) {
    try {
        // Get synchronized color + depth frame group
        bool timed_out = false;
        auto msg_group = sync_queue_->get<dai::MessageGroup>(
            std::chrono::milliseconds(timeout_ms), timed_out);
        if (timed_out || !msg_group) {
            return std::nullopt;  // Timeout
        }

        // Both color and depth must be present for a valid frame.
        // If the Sync node delivers a partial group (e.g., stereo dropped a
        // frame due to insufficient texture), skip it rather than sending
        // empty vectors to downstream compressors which would crash.
        auto colorFrame = msg_group->get<dai::ImgFrame>("color");
        auto depthFrame = msg_group->get<dai::ImgFrame>("depth");
        if (!colorFrame || !depthFrame) {
            return std::nullopt;  // Incomplete sync group — treat as timeout
        }

        CapturedFrame cf;

        // ---- Color frame (NV12 -> RGB24) ----
        {
            auto ts = colorFrame->getTimestampDevice();
            cf.timestamp_us = static_cast<uint64_t>(
                std::chrono::duration_cast<std::chrono::microseconds>(
                    ts.time_since_epoch()).count());

            const auto* nv12_data = colorFrame->getData().data();
            nv12_to_rgb24(nv12_data, color_width_, color_height_, cf.rgb_data);
        }

        cf.frame_number = frame_counter_++;

        // ---- Depth frame (RAW16 / uint16_t, values in millimeters) ----
        {
            const auto* ptr = depthFrame->getData().data();
            // Read actual frame dimensions rather than assuming they match color.
            // They should match (setDepthAlign + setOutputSize) but this is safer.
            int dw = depthFrame->getWidth();
            int dh = depthFrame->getHeight();
            size_t depth_bytes = static_cast<size_t>(dw) * dh * 2;  // Z16 = 2 bytes/pixel
            cf.depth_data.assign(ptr, ptr + depth_bytes);
        }

        // ---- IMU samples ----
        if (has_imu_ && imu_queue_) {
            // Drain all available IMU packets
            while (auto imu_data = imu_queue_->tryGet<dai::IMUData>()) {
                for (const auto& packet : imu_data->packets) {
                    IMUSample sample{};

                    // Accelerometer
                    auto& accel = packet.acceleroMeter;
                    auto accel_ts = accel.getTimestampDevice();
                    sample.timestamp_us = static_cast<uint64_t>(
                        std::chrono::duration_cast<std::chrono::microseconds>(
                            accel_ts.time_since_epoch()).count());
                    sample.accel[0] = accel.x;
                    sample.accel[1] = accel.y;
                    sample.accel[2] = accel.z;

                    // Gyroscope
                    auto& gyro = packet.gyroscope;
                    sample.gyro[0] = gyro.x;
                    sample.gyro[1] = gyro.y;
                    sample.gyro[2] = gyro.z;

                    cf.imu_samples.push_back(std::move(sample));
                }
            }
        }

        return cf;

    } catch (const std::exception& e) {
        throw std::runtime_error(
            std::string("OAK-D: frame polling failed: ") + e.what());
    }
}

// ---- is_device_lost --------------------------------------------------------
//
// DepthAI v3's Device::isClosed() is explicitly documented as thread-unsafe
// and "may return outdated incorrect values" — calling it from the capture
// thread produced spurious hotplug-unplug events. Instead, treat the device
// as lost only when we've actively closed + released it. Transient USB drops
// surface as poll_frame() errors and the recorder recovers through its
// existing reconnect path.

bool OakDPipeline::is_device_lost() const {
    return !device_;
}

#endif // HAVE_DEPTHAI
