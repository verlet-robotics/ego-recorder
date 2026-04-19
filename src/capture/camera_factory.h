#pragma once

// Camera pipeline factory -- creates the appropriate ICameraPipeline
// implementation based on a CameraType enum.
//
// Available backends are determined at compile time:
//   -DHAVE_REALSENSE  -> RealSensePipeline  (Intel D435/D435i)
//   -DHAVE_DEPTHAI    -> OakDPipeline        (Luxonis OAK-D)
//
// Throws std::runtime_error if the requested backend was not compiled in.

#include "capture/icamera_pipeline.h"
#include <memory>
#include <string>

/// Supported camera backends.
enum class CameraType {
    RealSense,
    OakD
};

/// Parse a camera type string ("realsense" or "oakd") into a CameraType enum.
/// Throws std::runtime_error on unrecognized input.
CameraType parse_camera_type(const std::string& s);

/// Create a camera pipeline for the given backend.
/// Throws std::runtime_error if the backend was not compiled in.
std::unique_ptr<ICameraPipeline> create_camera(CameraType type);

/// Hardware-reset all connected devices for the given backend.
/// For RealSense: creates a fresh rs2::context and resets every enumerated device
/// (clears stale USB/firmware state that causes silent hangs after long sessions).
/// For OAK-D: no-op (DepthAI handles reconnection internally).
void reset_all_cameras(CameraType type);
