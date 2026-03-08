#ifndef ZDEPTH_C_H
#define ZDEPTH_C_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ZdepthDecompressorC ZdepthDecompressorC;

ZdepthDecompressorC* zdepth_decompressor_new(void);
void zdepth_decompressor_free(ZdepthDecompressorC* d);

/// Decompress a Zdepth-compressed buffer.
/// Returns 0 on success, non-zero on error.
/// On success, *out_width and *out_height are set, and *out_data points
/// to an internal buffer of (*out_width * *out_height) uint16_t values.
/// The buffer is valid until the next call to zdepth_decompressor_decompress
/// or zdepth_decompressor_free.
int zdepth_decompressor_decompress(
    ZdepthDecompressorC* d,
    const uint8_t* compressed_data,
    size_t compressed_size,
    int* out_width,
    int* out_height,
    const uint16_t** out_data,
    size_t* out_count);

#ifdef __cplusplus
}
#endif

#endif /* ZDEPTH_C_H */
