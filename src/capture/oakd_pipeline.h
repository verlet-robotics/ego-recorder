#pragma once

#ifdef HAVE_DEPTHAI

// OakDPipeline -- ICameraPipeline implementation for Luxonis OAK-D cameras.
//
// Uses the DepthAI SDK to capture synchronized RGB + stereo depth + IMU data
// from OAK-D, OAK-D Wide, and OAK-D Pro Wide cameras.
//
// Key differences from RealSensePipeline:
//   - Uses DepthAI v3 pipeline graph (Camera -> StereoDepth -> Sync nodes)
//   - Color output is NV12, converted to RGB24 in poll_frame()
//   - IMU is BNO086 (accel + gyro), separate queue from sync
//   - Calibration: intrinsics from 3x3 matrix, extrinsics translation in cm (converted to m)
//   - Disconnect detection via device_->isClosed()
//   - Pro Wide: optional IR dot projector + flood light (non-Pro models silently skip)
//
// FOV preservation (Wide models):
//   RGB sensor (IMX378) is 4:3 native, stereo sensors (OV9282) are 8:5 native.
//   We request color at 4:3 aspect (e.g. 1280x960) to preserve the full wide-angle
//   FOV. Stereo input is requested at half native mono resolution (640x400) to
//   stay within the Myriad X's processing budget. Depth is aligned to color and
//   output at the same resolution as color.

#include "capture/icamera_pipeline.h"

#include <depthai/depthai.hpp>

#include <atomic>
#include <memory>
#include <string>

class OakDPipeline : public ICameraPipeline {
public:
    OakDPipeline() = default;
    ~OakDPipeline() override = default;

    // Non-copyable, non-movable
    OakDPipeline(const OakDPipeline&) = delete;
    OakDPipeline& operator=(const OakDPipeline&) = delete;

    void configure_and_start(int width = 1280, int height = 720,
                             int warmup_frames = 30) override;
    void stop() override;
    std::optional<CapturedFrame> poll_frame(unsigned int timeout_ms = 500) override;

    bool is_device_lost() const override;
    bool has_imu() const override { return has_imu_; }
    std::string serial_number() const override { return serial_number_; }
    std::string usb_type() const override { return usb_type_; }
    float depth_scale() const override { return 0.001f; } // OAK-D depth is in mm
    CameraIntrinsics depth_intrinsics() const override { return depth_intrinsics_; }
    CameraIntrinsics color_intrinsics() const override { return color_intrinsics_; }
    CameraExtrinsics depth_to_color_extrinsics() const override { return depth_to_color_extrinsics_; }

private:
    /// Convert NV12 frame data to tightly-packed RGB24.
    static void nv12_to_rgb24(const uint8_t* nv12, int width, int height,
                              std::vector<uint8_t>& rgb_out);

    /// Map DepthAI USB speed enum to a human-readable string.
    static std::string usb_speed_to_string(dai::UsbSpeed speed);

    std::shared_ptr<dai::Device>       device_;
    // Pipeline owns the node graph + host-side queues. Must outlive the
    // queues below — v3 closes the queues when the Pipeline destructs.
    std::unique_ptr<dai::Pipeline>     pipeline_;
    std::shared_ptr<dai::MessageQueue> sync_queue_;
    std::shared_ptr<dai::MessageQueue> imu_queue_;

    CameraIntrinsics  depth_intrinsics_{};
    CameraIntrinsics  color_intrinsics_{};
    CameraExtrinsics  depth_to_color_extrinsics_{};

    std::string serial_number_;
    std::string usb_type_;
    bool        has_imu_       = false;
    uint64_t    frame_counter_ = 0;
    int         color_width_   = 0;
    int         color_height_  = 0;
};

#endif // HAVE_DEPTHAI
