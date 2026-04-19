#pragma once

#include <cstdint>

/// Camera-agnostic intrinsics -- replaces rs2_intrinsics at interface boundaries.
///
/// All fields match the .egorec FileHeader layout (width, height, focal lengths,
/// principal point, distortion model + coefficients), so conversion to the wire
/// format is a simple field copy.
struct CameraIntrinsics {
    int      width;
    int      height;
    float    fx, fy;           ///< Focal lengths (pixels)
    float    ppx, ppy;         ///< Principal point (pixels)
    uint32_t distortion_model; ///< Maps to rs2_distortion enum for .egorec compat
    float    distortion_coeffs[5]; ///< k1, k2, p1, p2, k3
};

/// Camera-agnostic extrinsics -- replaces rs2_extrinsics at interface boundaries.
///
/// Rotation is a 3x3 row-major matrix; translation is in meters.
struct CameraExtrinsics {
    float rotation[9];     ///< 3x3 row-major rotation matrix
    float translation[3];  ///< Translation in meters
};
