#pragma once

// ICameraPipeline -- Strategy interface for camera capture backends.
//
// Defines the lifecycle contract shared by RealSensePipeline (Intel D435/D435i)
// and OakDPipeline (Luxonis OAK-D).  main.cpp programs against this interface;
// the concrete backend handles device initialization, stream configuration,
// intrinsics/extrinsics extraction, IMU detection, and frame polling.
//
// Follows the same pattern as IPresenter (see presenter/ipresenter.h).
//
// Lifecycle sequence:
//   1. configure_and_start()  -- once, blocks during warmup
//   2. poll_frame()           -- in capture loop; returns nullopt on timeout
//   3. stop()                 -- once, on shutdown or disconnect recovery
//
// Device disconnect is detected via is_device_lost() (polled from capture thread)
// or by poll_frame() throwing std::runtime_error.

#include "capture/camera_types.h"
#include "capture/frame_types.h"

#include <optional>
#include <string>

class ICameraPipeline {
public:
    virtual ~ICameraPipeline() = default;

    /// Configure streams and start the camera pipeline.
    /// Blocks during warmup (dropping first warmup_frames frames).
    virtual void configure_and_start(int width = 1280, int height = 720,
                                     int warmup_frames = 30) = 0;

    /// Stop the pipeline and release the device.
    virtual void stop() = 0;

    /// Wait up to timeout_ms for the next synchronized frameset.
    /// Returns CapturedFrame on success, std::nullopt on timeout.
    /// Throws std::runtime_error on unrecoverable device errors.
    virtual std::optional<CapturedFrame> poll_frame(unsigned int timeout_ms = 500) = 0;

    /// Returns true if the device was physically disconnected.
    virtual bool is_device_lost() const = 0;

    /// Returns true if IMU streams are enabled (e.g. D435i, OAK-D with BNO055).
    virtual bool has_imu() const = 0;

    /// Camera serial number string.
    virtual std::string serial_number() const = 0;

    /// USB connection type descriptor (e.g. "3.2", "2.1").
    virtual std::string usb_type() const = 0;

    /// Depth scale: multiply raw Z16 value by this to get meters.
    virtual float depth_scale() const = 0;

    /// Depth stream intrinsics.
    virtual CameraIntrinsics depth_intrinsics() const = 0;

    /// Color stream intrinsics.
    virtual CameraIntrinsics color_intrinsics() const = 0;

    /// Extrinsics from depth to color coordinate space.
    virtual CameraExtrinsics depth_to_color_extrinsics() const = 0;

    /// Set laser/IR projector power (0.0 = off, 1.0 = max).
    /// Default no-op for cameras without active IR.
    virtual void set_laser_power(float) {}

    /// Read ASIC/SoC temperature in degrees Celsius.
    /// Returns -1.0 if not supported.
    virtual float asic_temperature() const { return -1.0f; }

    /// Perform a hardware reset on the device.
    /// Default no-op; RealSense implements via rs2 hardware_reset().
    virtual void hardware_reset() {}
};
