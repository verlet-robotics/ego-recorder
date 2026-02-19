#pragma once

#include <cstdint>
#include <vector>

/// A single IMU measurement sample (accelerometer + gyroscope).
/// Size: 32 bytes (8 + 12 + 12).
struct IMUSample {
    uint64_t timestamp_us;  ///< Hardware timestamp, microseconds
    float    accel[3];      ///< Accelerometer: x, y, z (m/s^2)
    float    gyro[3];       ///< Gyroscope: x, y, z (rad/s)
};

/// One captured frame bundle: RGB + depth + accumulated IMU samples.
/// Move-only to avoid accidental copies of large pixel buffers.
struct CapturedFrame {
    uint64_t              timestamp_us;   ///< Global time, microseconds
    uint64_t              frame_number;   ///< Sequential counter (0-based)
    std::vector<uint8_t>  rgb_data;       ///< Raw RGB24 pixels: 640*480*3 = 921,600 bytes
    std::vector<uint8_t>  depth_data;     ///< Raw Z16 pixels:  640*480*2 = 614,400 bytes
    std::vector<IMUSample> imu_samples;   ///< IMU samples since last frame (empty if D435)

    // Move-only semantics
    CapturedFrame() = default;
    CapturedFrame(const CapturedFrame&) = delete;
    CapturedFrame& operator=(const CapturedFrame&) = delete;
    CapturedFrame(CapturedFrame&&) = default;
    CapturedFrame& operator=(CapturedFrame&&) = default;
};
