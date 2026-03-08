#pragma once

// .egorec binary file format -- wire format constants and packed structs.
//
// All multi-byte values are little-endian (host byte order on x86_64).
// All structs use #pragma pack(push, 1) to eliminate compiler padding.
//
// File layout:
//   [FileHeader]
//   [FrameBlockHeader][rgb_data][depth_data][imu_samples...]  (repeated per frame)
//   [IndexEntry * N]
//   [FileFooter]

#include <cstdint>
#include <cassert>

#pragma pack(push, 1)

// ---- Magic constants -------------------------------------------------------

/// 8-byte file magic: ASCII "EGOREC" + version 2.0
static constexpr uint8_t FILE_MAGIC[8] = {'E','G','O','R','E','C', 0x02, 0x00};

/// Frame block boundary marker: 'FRME' (0x46524D45 as specified in format spec)
static constexpr uint32_t FRAME_MAGIC = 0x46524D45u;

/// Index table start marker: 'INDX' (0x58444E49 as specified in format spec)
static constexpr uint32_t INDEX_MAGIC = 0x58444E49u;

/// Footer marker: 'DONE' (0x454E4F44 as specified in format spec)
static constexpr uint32_t FOOTER_MAGIC = 0x454E4F44u;

// ---- FileHeader (padded to ~512 bytes) ------------------------------------

/// File header written at byte offset 0. Contains camera calibration,
/// session metadata, and compression settings.
struct FileHeader {
    // -- Identity ---
    uint8_t  magic[8];          ///< FILE_MAGIC (8 bytes)
    uint32_t header_size;       ///< sizeof(FileHeader) -- forward compat guard
    uint32_t flags;             ///< bit 0: has_imu, bit 1: has_index

    // -- Camera serial ---
    char     serial_number[32]; ///< Null-terminated camera serial string

    // -- Depth intrinsics ---
    float    depth_scale;       ///< Z16 units to meters (typically 0.001)
    uint32_t depth_width;       ///< Depth frame width in pixels
    uint32_t depth_height;      ///< Depth frame height in pixels
    float    depth_fx;          ///< Depth focal length X
    float    depth_fy;          ///< Depth focal length Y
    float    depth_ppx;         ///< Depth principal point X
    float    depth_ppy;         ///< Depth principal point Y
    uint32_t depth_distortion_model;       ///< rs2_distortion enum value
    float    depth_distortion_coeffs[5];   ///< k1, k2, p1, p2, k3

    // -- Color intrinsics ---
    uint32_t color_width;       ///< Color frame width in pixels
    uint32_t color_height;      ///< Color frame height in pixels
    float    color_fx;          ///< Color focal length X
    float    color_fy;          ///< Color focal length Y
    float    color_ppx;         ///< Color principal point X
    float    color_ppy;         ///< Color principal point Y
    uint32_t color_distortion_model;       ///< rs2_distortion enum value
    float    color_distortion_coeffs[5];   ///< k1, k2, p1, p2, k3

    // -- Extrinsics (depth to color) ---
    float    extrinsic_rotation[9];    ///< 3x3 rotation matrix, row-major
    float    extrinsic_translation[3]; ///< Translation vector in meters

    // -- Session metadata ---
    char     session_name[128];        ///< Null-terminated session label
    uint64_t start_timestamp_us;       ///< Unix epoch microseconds at recording start
    char     usb_type[8];              ///< USB type string, e.g. "USB 3.2"

    // -- Compression settings (extensible codec IDs) ---
    // Codec IDs are extensible enum values. New codecs add new values
    // without requiring a format version bump. Readers must check these
    // fields per-stream rather than assuming a single global codec.
    uint8_t  rgb_codec;    ///< RGB codec ID: 0=raw, 1=JPEG, 2=H264
    uint8_t  depth_codec;  ///< Depth codec ID: 0=raw, 1=ZSTD, 2=Zdepth
    uint8_t  rgb_quality;  ///< JPEG quality 0-100 (codec=1), or CRF (codec=2)
    uint8_t  zstd_level;   ///< ZSTD compression level (codec=1), reserved (codec=2)

    // -- Future use ---
    uint8_t  reserved[128]; ///< Zero-filled, reserved for future fields
};

// ---- FrameBlockHeader -----------------------------------------------------

/// Header preceding each frame's compressed data in the file.
struct FrameBlockHeader {
    uint32_t magic;                ///< FRAME_MAGIC (0x454D5246)
    uint32_t block_size;           ///< Total block size including this header
    uint64_t timestamp_us;         ///< Frame hardware timestamp, microseconds
    uint64_t frame_number;         ///< Sequential 0-based frame index
    uint32_t rgb_compressed_size;  ///< Compressed RGB data size in bytes
    uint32_t depth_compressed_size;///< Compressed depth data size in bytes
    uint16_t imu_sample_count;     ///< Number of IMUSampleWire items following depth
    uint16_t flags;                ///< Reserved, set to 0
};

// ---- IMUSampleWire (32 bytes) ---------------------------------------------

/// Wire format for a single IMU measurement (both accel and gyro).
struct IMUSampleWire {
    uint64_t timestamp_us; ///< IMU hardware timestamp, microseconds
    float    accel_x;      ///< Accelerometer X (m/s^2)
    float    accel_y;      ///< Accelerometer Y (m/s^2)
    float    accel_z;      ///< Accelerometer Z (m/s^2)
    float    gyro_x;       ///< Gyroscope X (rad/s)
    float    gyro_y;       ///< Gyroscope Y (rad/s)
    float    gyro_z;       ///< Gyroscope Z (rad/s)
};

// ---- IndexEntry (24 bytes) ------------------------------------------------

/// One entry in the seek index table written before the footer.
struct IndexEntry {
    uint64_t timestamp_us; ///< Frame timestamp, microseconds
    uint64_t file_offset;  ///< Byte offset of the FrameBlockHeader for this frame
    uint64_t frame_number; ///< Sequential frame number
};

// ---- FileFooter -----------------------------------------------------------

/// Written at the end of the file. Allows readers to locate the index table.
struct FileFooter {
    uint32_t index_magic;       ///< INDEX_MAGIC (0x58444E49)
    uint64_t index_offset;      ///< Byte offset where the index table starts
    uint32_t index_entry_count; ///< Number of IndexEntry items in the index table
    uint64_t total_frames;      ///< Total frames written
    uint64_t total_duration_us; ///< last_timestamp - first_timestamp (microseconds)
    uint32_t footer_magic;      ///< FOOTER_MAGIC (0x454E4F44)
};

#pragma pack(pop)

// ---- Compile-time size assertions -----------------------------------------
// These catch struct packing issues at build time.

static_assert(sizeof(IMUSampleWire) == 32,
    "IMUSampleWire must be 32 bytes (8 + 6*4)");

static_assert(sizeof(IndexEntry) == 24,
    "IndexEntry must be 24 bytes (3 * uint64_t)");

static_assert(sizeof(FrameBlockHeader) == 36,
    "FrameBlockHeader must be 36 bytes (4+4+8+8+4+4+2+2)");

static_assert(sizeof(FileFooter) == 36,
    "FileFooter must be 36 bytes (4+8+4+8+8+4)");
