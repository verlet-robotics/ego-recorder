#include "capture/camera_factory.h"

#include <algorithm>
#include <stdexcept>

#ifdef HAVE_REALSENSE
#include "capture/pipeline.h"
#endif

#ifdef HAVE_DEPTHAI
#include "capture/oakd_pipeline.h"
#endif

CameraType parse_camera_type(const std::string& s) {
    // Normalize to lowercase for comparison
    std::string lower = s;
    std::transform(lower.begin(), lower.end(), lower.begin(),
                   [](unsigned char c) { return std::tolower(c); });

    if (lower == "realsense" || lower == "rs" || lower == "d435") {
        return CameraType::RealSense;
    }
    if (lower == "oakd" || lower == "oak-d" || lower == "depthai" || lower == "oak") {
        return CameraType::OakD;
    }
    throw std::runtime_error(
        "Unknown camera type '" + s + "'. Supported: realsense, oakd");
}

std::unique_ptr<ICameraPipeline> create_camera(CameraType type) {
    switch (type) {
    case CameraType::RealSense:
#ifdef HAVE_REALSENSE
        return std::make_unique<RealSensePipeline>();
#else
        throw std::runtime_error(
            "RealSense support not compiled in (build with -DWITH_REALSENSE=ON)");
#endif

    case CameraType::OakD:
#ifdef HAVE_DEPTHAI
        return std::make_unique<OakDPipeline>();
#else
        throw std::runtime_error(
            "DepthAI/OAK-D support not compiled in (build with -DWITH_DEPTHAI=ON)");
#endif
    }

    // Unreachable, but silence compiler warning
    throw std::runtime_error("Invalid camera type");
}

void reset_all_cameras(CameraType type) {
    switch (type) {
    case CameraType::RealSense:
#ifdef HAVE_REALSENSE
        RealSensePipeline::hardware_reset_all();
#endif
        break;

    case CameraType::OakD:
        // OAK-D handles reconnection internally via DepthAI auto-reconnect.
        // No equivalent "reset all" operation needed.
        break;
    }
}
