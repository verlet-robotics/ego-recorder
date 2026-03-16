#include "capture/pipeline.h"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <optional>
#include <stdexcept>

void RealSensePipeline::configure_and_start(int width, int height, int warmup_frames)
{
    // -------------------------------------------------------------------------
    // 1. Configure RGB and depth streams at requested resolution
    // -------------------------------------------------------------------------
    rs2::config cfg;
    cfg.enable_stream(RS2_STREAM_COLOR, width, height, RS2_FORMAT_RGB8, 30);
    cfg.enable_stream(RS2_STREAM_DEPTH, width, height, RS2_FORMAT_Z16,  30);

    fprintf(stderr, "Requesting %dx%d @ 30fps (RGB + depth)\n", width, height);

    // -------------------------------------------------------------------------
    // 2. Attempt IMU streams (D435i detection)
    //
    // Try to start with accel + gyro. If it throws (D435 without IMU),
    // fall back to RGB+depth only.
    // -------------------------------------------------------------------------
    cfg.enable_stream(RS2_STREAM_ACCEL, RS2_FORMAT_MOTION_XYZ32F);
    cfg.enable_stream(RS2_STREAM_GYRO,  RS2_FORMAT_MOTION_XYZ32F);

    try {
        profile_ = pipe_.start(cfg);
        has_imu_ = true;
        fprintf(stderr, "IMU detected (D435i mode)\n");
    } catch (const rs2::error&) {
        // IMU not available -- retry with RGB+depth only
        rs2::config cfg_no_imu;
        cfg_no_imu.enable_stream(RS2_STREAM_COLOR, width, height, RS2_FORMAT_RGB8, 30);
        cfg_no_imu.enable_stream(RS2_STREAM_DEPTH, width, height, RS2_FORMAT_Z16,  30);
        profile_ = pipe_.start(cfg_no_imu);
        has_imu_ = false;
        fprintf(stderr, "No IMU detected (D435 mode)\n");
    }

    // -------------------------------------------------------------------------
    // 3. Extract device info + register hotplug callback
    // -------------------------------------------------------------------------
    device_ = profile_.get_device();
    serial_number_ = device_.get_info(RS2_CAMERA_INFO_SERIAL_NUMBER);
    usb_type_      = device_.get_info(RS2_CAMERA_INFO_USB_TYPE_DESCRIPTOR);

    // Register device-changed callback for instant disconnect detection.
    // Fires on a librealsense background thread within milliseconds of USB
    // unplug -- much faster than waiting for wait_for_frames() to time out.
    device_lost_.store(false, std::memory_order_release);
    ctx_.set_devices_changed_callback(
        [this](rs2::event_information& info) {
            if (info.was_removed(device_)) {
                device_lost_.store(true, std::memory_order_release);
            }
        });

    if (!usb_type_.empty() && usb_type_[0] == '2') {
        // Structured sentinel parsed by recorder-app stderr reader
        fprintf(stderr,
            "USB_WARNING: Camera on USB 2.0 port. Use USB 3.0 for reliable operation.\n");
    }

    // -------------------------------------------------------------------------
    // 4. Disable auto-exposure priority
    //    Setting to 0 prevents the sensor from reducing frame rate to maintain
    //    exposure target, ensuring a constant 30fps.
    // -------------------------------------------------------------------------
    auto color_sensor = profile_.get_device().first<rs2::color_sensor>();
    color_sensor.set_option(RS2_OPTION_AUTO_EXPOSURE_PRIORITY, 0.0f);

    // -------------------------------------------------------------------------
    // 5. Enable global time synchronization
    //    Aligns hardware timestamps across sensors to a common clock.
    // -------------------------------------------------------------------------
    auto depth_sensor = profile_.get_device().first<rs2::depth_sensor>();
    if (depth_sensor.supports(RS2_OPTION_GLOBAL_TIME_ENABLED)) {
        depth_sensor.set_option(RS2_OPTION_GLOBAL_TIME_ENABLED, 1.0f);
    }

    // -------------------------------------------------------------------------
    // 6. Extract depth scale
    //    Multiply raw Z16 value by depth_scale_ to get depth in meters.
    // -------------------------------------------------------------------------
    depth_scale_ = depth_sensor.get_depth_scale();

    // -------------------------------------------------------------------------
    // 7. Extract intrinsics and extrinsics
    // -------------------------------------------------------------------------
    auto depth_stream = profile_.get_stream(RS2_STREAM_DEPTH).as<rs2::video_stream_profile>();
    auto color_stream = profile_.get_stream(RS2_STREAM_COLOR).as<rs2::video_stream_profile>();

    depth_intrinsics_          = depth_stream.get_intrinsics();
    color_intrinsics_          = color_stream.get_intrinsics();
    depth_to_color_extrinsics_ = depth_stream.get_extrinsics_to(color_stream);

    // -------------------------------------------------------------------------
    // 8. Warmup -- drop first N frames for auto-exposure stabilization
    //    Uses per-frame timeout (2s) and total deadline (10s) to avoid hanging
    //    if the camera is slow to start producing frames.
    // -------------------------------------------------------------------------
    fprintf(stderr, "Warming up camera (%d frames)...\n", warmup_frames);
    {
        auto warmup_start = std::chrono::steady_clock::now();
        constexpr auto warmup_deadline = std::chrono::seconds(10);
        constexpr unsigned int per_frame_timeout_ms = 2000;

        for (int i = 0; i < warmup_frames; ++i) {
            auto elapsed = std::chrono::steady_clock::now() - warmup_start;
            if (elapsed >= warmup_deadline) {
                fprintf(stderr, "Warmup deadline reached after %d/%d frames\n",
                        i, warmup_frames);
                break;
            }

            rs2::frameset fs;
            if (!pipe_.try_wait_for_frames(&fs, per_frame_timeout_ms)) {
                fprintf(stderr, "Warmup frame %d/%d timed out\n", i + 1, warmup_frames);
            }
        }
    }
    fprintf(stderr, "Camera ready.\n");

    // -------------------------------------------------------------------------
    // 9. Initialize frame counter
    // -------------------------------------------------------------------------
    frame_counter_ = 0;
}

void RealSensePipeline::stop()
{
    try {
        pipe_.stop();
    } catch (const rs2::error&) {
        // Device may already be gone (USB unplug) -- stop() can throw
    }
}

std::optional<CapturedFrame> RealSensePipeline::poll_frame(unsigned int timeout_ms)
{
    // Wait with a short timeout so the caller can check is_device_lost()
    // promptly after a USB unplug, rather than blocking for 15 seconds.
    rs2::frameset frames;
    if (!pipe_.try_wait_for_frames(&frames, timeout_ms)) {
        return std::nullopt;  // No frame within timeout -- let caller re-check
    }

    // Extract color and depth frames from the synchronized set.
    rs2::video_frame color = frames.get_color_frame();
    rs2::depth_frame depth = frames.get_depth_frame();

    CapturedFrame cf;

    // Timestamp: SDK returns milliseconds as double; convert to microseconds.
    cf.timestamp_us = static_cast<uint64_t>(color.get_timestamp() * 1000.0);
    cf.frame_number = frame_counter_++;

    // Copy RGB data into the CapturedFrame vector.
    // IMPORTANT: color.get_data() is only valid while `frames` is alive.
    // Strip stride padding (if any) so rgb_data is tightly packed width*3
    // per row, matching what downstream code (preview, encoder) expects.
    {
        const auto* ptr = static_cast<const uint8_t*>(color.get_data());
        const int stride = color.get_stride_in_bytes();
        const int w = color.get_width();
        const int h = color.get_height();
        const int row_bytes = w * 3;  // RGB8 = 3 bytes/pixel
        if (stride == row_bytes) {
            cf.rgb_data.assign(ptr, ptr + row_bytes * h);
        } else {
            cf.rgb_data.resize(static_cast<size_t>(row_bytes) * h);
            for (int y = 0; y < h; ++y) {
                std::memcpy(cf.rgb_data.data() + y * row_bytes,
                            ptr + y * stride, row_bytes);
            }
        }
    }

    // Copy depth data into the CapturedFrame vector.
    // Same stride-stripping as RGB for consistency.
    {
        const auto* ptr = static_cast<const uint8_t*>(depth.get_data());
        const int stride = depth.get_stride_in_bytes();
        const int w = depth.get_width();
        const int h = depth.get_height();
        const int row_bytes = w * 2;  // Z16 = 2 bytes/pixel
        if (stride == row_bytes) {
            cf.depth_data.assign(ptr, ptr + row_bytes * h);
        } else {
            cf.depth_data.resize(static_cast<size_t>(row_bytes) * h);
            for (int y = 0; y < h; ++y) {
                std::memcpy(cf.depth_data.data() + y * row_bytes,
                            ptr + y * stride, row_bytes);
            }
        }
    }

    // Collect IMU samples if IMU is enabled (D435i mode).
    // IMU frames arrive at higher rate (200-400 Hz) than video (30 fps),
    // so there may be 0 or more motion frames bundled in the frameset.
    if (has_imu_) {
        frames.foreach_rs([&](const rs2::frame& f) {
            if (f.get_profile().stream_type() == RS2_STREAM_ACCEL ||
                f.get_profile().stream_type() == RS2_STREAM_GYRO)
            {
                rs2::motion_frame mf = f.as<rs2::motion_frame>();
                rs2_vector motion_data = mf.get_motion_data();
                IMUSample sample{};
                sample.timestamp_us = static_cast<uint64_t>(mf.get_timestamp() * 1000.0);
                if (f.get_profile().stream_type() == RS2_STREAM_ACCEL) {
                    sample.accel[0] = motion_data.x;
                    sample.accel[1] = motion_data.y;
                    sample.accel[2] = motion_data.z;
                } else {
                    sample.gyro[0] = motion_data.x;
                    sample.gyro[1] = motion_data.y;
                    sample.gyro[2] = motion_data.z;
                }
                cf.imu_samples.push_back(std::move(sample));
            }
        });
    }

    return cf;
}
