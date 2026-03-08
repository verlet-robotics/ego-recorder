#include "wrapper.h"
#include "vendor/include/zdepth.hpp"
#include <vector>

struct ZdepthDecompressorC {
    zdepth::DepthCompressor decompressor;
    std::vector<uint16_t> depth_out;
};

extern "C" {

ZdepthDecompressorC* zdepth_decompressor_new(void) {
    return new ZdepthDecompressorC();
}

void zdepth_decompressor_free(ZdepthDecompressorC* d) {
    delete d;
}

int zdepth_decompressor_decompress(
    ZdepthDecompressorC* d,
    const uint8_t* compressed_data,
    size_t compressed_size,
    int* out_width,
    int* out_height,
    const uint16_t** out_data,
    size_t* out_count)
{
    if (!d || !compressed_data || compressed_size == 0) {
        return -1;
    }

    std::vector<uint8_t> compressed_vec(compressed_data, compressed_data + compressed_size);

    int width = 0, height = 0;
    zdepth::DepthResult result = d->decompressor.Decompress(
        compressed_vec, width, height, d->depth_out);

    if (result != zdepth::DepthResult::Success) {
        return static_cast<int>(result) + 1;
    }

    *out_width = width;
    *out_height = height;
    *out_data = d->depth_out.data();
    *out_count = d->depth_out.size();
    return 0;
}

} // extern "C"
