#pragma once

#include <librealsense2/rs.hpp>
#include <atomic>
#include <optional>
#include <string>
#include <cstdint>

#include "capture/icamera_pipeline.h"
#include "capture/frame_types.h"

/// RealSense camera pipeline wrapper.
///
/// Implements the ICameraPipeline interface for Intel RealSense D435/D435i.
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
class RealSensePipeline : public ICameraPipeline {
public:
    /// Default constructor -- does nothing. Call configure_and_start() to initialize.
    RealSensePipeline() = default;

    /// Non-copyable, non-movable (owns rs2::pipeline which is non-copyable).
    RealSensePipeline(const RealSensePipeline&) = delete;
    RealSensePipeline& operator=(const RealSensePipeline&) = delete;
    RealSensePipeline(RealSensePipeline&&) = delete;
    RealSensePipeline& operator=(RealSensePipeline&&) = delete;

    ~RealSensePipeline() override = default;

    /// Configure and start the RealSense pipeline.
    ///
    /// Sequence:
    ///   1. Configure RGB and depth streams at the specified resolution @ 30fps
    ///   2. Attempt IMU streams (D435i) -- falls back gracefully if not available (D435)
    ///   3. Extract device info: serial number, USB type
    ///   4. Disable auto-exposure priority to maintain constant 30fps
    ///   5. Enable global time synchronization
    ///   6. Extract depth scale
    ///   7. Extract intrinsics and extrinsics
    ///   8. Drop first warmup_frames frames for auto-exposure stabilization
    ///   9. Initialize frame counter to 0
    ///
    /// @param width          Frame width  (default: 1280, must be supported by D435/D435i)
    /// @param height         Frame height (default: 720)
    /// @param warmup_frames  Number of frames to discard during warmup (default: 30)
    void configure_and_start(int width = 1280, int height = 720,
                             int warmup_frames = 30) override;

    /// Stop the pipeline and release the device.
    void stop() override;

    /// Wait up to \p timeout_ms for the next frameset, copy data into a
    /// CapturedFrame, and return it. Returns std::nullopt on timeout (no frame
    /// available), allowing the caller to check is_device_lost() promptly.
    ///
    /// @param timeout_ms  Max wait time in milliseconds (default: 500)
    /// @return CapturedFrame if a frame arrived, std::nullopt on timeout.
    std::optional<CapturedFrame> poll_frame(unsigned int timeout_ms = 500) override;

    // ---- ICameraPipeline accessors (override) ----

    bool is_device_lost() const override {
        return device_lost_.load(std::memory_order_acquire);
    }

    bool has_imu() const override { return has_imu_; }
    std::string serial_number() const override { return serial_number_; }
    std::string usb_type() const override { return usb_type_; }
    float depth_scale() const override { return depth_scale_; }

    CameraIntrinsics depth_intrinsics() const override;
    CameraIntrinsics color_intrinsics() const override;
    CameraExtrinsics depth_to_color_extrinsics() const override;

    // ---- RealSense-specific methods ----

    /// Set laser power (0.0 = off, 1.0 = max). Only works on D435 with active IR.
    void set_laser_power(float power) override;

    /// Read ASIC temperature in degrees Celsius.
    float asic_temperature() const override;

    /// Hardware-reset the active device.
    void hardware_reset() override;

    /// Hardware-reset ALL connected RealSense devices (static, not on interface).
    static void hardware_reset_all();

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
