#pragma once

#include <librealsense2/rs.hpp>
#include <atomic>
#include <optional>
#include <string>
#include <cstdint>

#include "capture/frame_types.h"

/// RealSense camera pipeline wrapper.
///
/// Handles device initialization, stream configuration, intrinsics/extrinsics
/// extraction, IMU detection, auto-exposure warmup, and frame polling.
///
/// Registers an rs2::context device-changed callback for instant USB hotplug
/// disconnect detection. Check is_device_lost() rather than waiting for
/// poll_frame() to throw after a 15-second timeout.
///
/// Usage:
///   RealSensePipeline pipeline;
///   pipeline.configure_and_start();          // blocks for warmup (~1 sec)
///   while (running) {
///       if (pipeline.is_device_lost()) { /* handle disconnect */ break; }
///       CapturedFrame frame = pipeline.poll_frame();  // blocks until next frame
///       // process frame ...
///   }
///   pipeline.stop();
class RealSensePipeline {
public:
    /// Default constructor -- does nothing. Call configure_and_start() to initialize.
    RealSensePipeline() = default;

    /// Non-copyable, non-movable (owns rs2::pipeline which is non-copyable).
    RealSensePipeline(const RealSensePipeline&) = delete;
    RealSensePipeline& operator=(const RealSensePipeline&) = delete;
    RealSensePipeline(RealSensePipeline&&) = delete;
    RealSensePipeline& operator=(RealSensePipeline&&) = delete;

    ~RealSensePipeline() = default;

    /// Configure and start the RealSense pipeline.
    ///
    /// Sequence:
    ///   1. Configure RGB (640x480 @ 30fps RGB8) and depth (640x480 @ 30fps Z16) streams
    ///   2. Attempt IMU streams (D435i) -- falls back gracefully if not available (D435)
    ///   3. Extract device info: serial number, USB type
    ///   4. Disable auto-exposure priority to maintain constant 30fps
    ///   5. Enable global time synchronization
    ///   6. Extract depth scale
    ///   7. Extract intrinsics and extrinsics
    ///   8. Drop first warmup_frames frames for auto-exposure stabilization
    ///   9. Initialize frame counter to 0
    ///
    /// @param warmup_frames  Number of frames to discard during warmup (default: 30)
    void configure_and_start(int warmup_frames = 30);

    /// Stop the pipeline and release the device.
    void stop();

    /// Wait up to \p timeout_ms for the next frameset, copy data into a
    /// CapturedFrame, and return it. Returns std::nullopt on timeout (no frame
    /// available), allowing the caller to check is_device_lost() promptly.
    ///
    /// @param timeout_ms  Max wait time in milliseconds (default: 500)
    /// @return CapturedFrame if a frame arrived, std::nullopt on timeout.
    std::optional<CapturedFrame> poll_frame(unsigned int timeout_ms = 500);

    // ---- Accessors for camera metadata (used by main.cpp to assemble FileHeader) ----

    /// Returns true if the device was removed (USB unplug detected via hotplug callback).
    /// Fires within milliseconds of physical disconnect -- no need to wait for poll_frame()
    /// to throw. Check this before each poll_frame() call.
    bool is_device_lost() const { return device_lost_.load(std::memory_order_acquire); }

    /// Returns true if IMU streams were successfully enabled (D435i mode).
    bool has_imu() const { return has_imu_; }

    /// Camera serial number string (e.g., "012345678901").
    std::string serial_number() const { return serial_number_; }

    /// USB connection type descriptor (e.g., "3.2", "2.1").
    std::string usb_type() const { return usb_type_; }

    /// Depth scale: multiply Z16 raw value by this to get meters.
    float depth_scale() const { return depth_scale_; }

    /// Depth stream intrinsics (focal lengths, principal point, distortion).
    rs2_intrinsics depth_intrinsics() const { return depth_intrinsics_; }

    /// Color stream intrinsics.
    rs2_intrinsics color_intrinsics() const { return color_intrinsics_; }

    /// Extrinsics from depth frame coordinate space to color frame coordinate space.
    rs2_extrinsics depth_to_color_extrinsics() const { return depth_to_color_extrinsics_; }

private:
    rs2::context          ctx_;
    rs2::pipeline         pipe_{ctx_};
    rs2::pipeline_profile profile_;
    rs2::device           device_;              ///< Active device (for hotplug check)
    std::atomic<bool>     device_lost_{false};  ///< Set by hotplug callback

    rs2_intrinsics   depth_intrinsics_           {};
    rs2_intrinsics   color_intrinsics_            {};
    rs2_extrinsics   depth_to_color_extrinsics_  {};

    float            depth_scale_    {0.0f};
    std::string      serial_number_;
    std::string      usb_type_;
    bool             has_imu_        {false};
    uint64_t         frame_counter_  {0};
};
