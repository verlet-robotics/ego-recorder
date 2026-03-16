#pragma once

#include <cstdint>

/// Turbo colormap LUT (Google, 2019) -- 256 entries, perceptually uniform.
/// Source: https://gist.github.com/mikhailov-work/0d177465a8151eb6ede1768d51d476c7
/// License: Apache 2.0
extern const uint8_t kTurboLUT[256][3];

/// Convert Z16 depth pixels to RGB using turbo colormap with histogram equalization.
/// Auto-ranges to 2nd-98th percentile of non-zero depth values per frame.
///
/// @param depth    Z16 raw depth values (width * height)
/// @param out_rgb  Output RGB24 buffer (width * height * 3 bytes, must be pre-allocated)
/// @param width    Frame width in pixels
/// @param height   Frame height in pixels
void colorize_depth(
    const uint16_t* depth,
    uint8_t*        out_rgb,
    int             width,
    int             height
);
