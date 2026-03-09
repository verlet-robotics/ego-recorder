#pragma once

// FileWriter -- sequential .egorec file writer.
//
// Usage pattern:
//   FileWriter writer("recording.egorec");
//   writer.write_header(header);
//   for (each frame) {
//       writer.write_frame(rgb, rgb_size, depth, depth_size, ts, fn, imu);
//   }
//   writer.finalize();  // writes index table + footer, flushes, closes file

#include "storage/binary_format.h"

#include <atomic>
#include <string>
#include <vector>
#include <fstream>
#include <cstdint>
#include <cstddef>

class FileWriter {
public:
    /// Open \p filepath for binary writing. Throws std::runtime_error on failure.
    explicit FileWriter(const std::string& filepath);

    // Non-copyable, non-movable (owns an open file handle).
    FileWriter(const FileWriter&) = delete;
    FileWriter& operator=(const FileWriter&) = delete;
    FileWriter(FileWriter&&) = delete;
    FileWriter& operator=(FileWriter&&) = delete;

    /// Destructor: attempts finalize() if not already finalized (best-effort).
    ~FileWriter();

    /// Write the file header at position 0.
    /// Must be called exactly once before any write_frame() calls.
    void write_header(const FileHeader& header);

    /// Append one compressed frame block to the file.
    /// @param rgb_compressed   Pointer to compressed RGB data (JPEG or raw)
    /// @param rgb_size         Size of compressed RGB data in bytes
    /// @param depth_compressed Pointer to compressed depth data (ZSTD or raw)
    /// @param depth_size       Size of compressed depth data in bytes
    /// @param timestamp_us     Frame hardware timestamp in microseconds
    /// @param frame_number     Sequential 0-based frame index
    /// @param imu_samples      IMU samples accumulated since the last frame
    /// Returns false on write error (caller should stop recording).
    bool write_frame(const uint8_t* rgb_compressed,   size_t rgb_size,
                     const uint8_t* depth_compressed, size_t depth_size,
                     uint64_t timestamp_us,
                     uint64_t frame_number,
                     const std::vector<IMUSampleWire>& imu_samples);

    /// Write raw codec flush data (e.g., H.264 trailing NAL units) after the
    /// last frame block. Does NOT create a FrameBlockHeader or IndexEntry.
    /// Must be called after the last write_frame() and before finalize().
    /// The reader must read bytes between the last indexed frame's end and
    /// index_offset to recover trailing codec data.
    void write_trailing_codec_data(const uint8_t* data, size_t size);

    /// Finalize the file: write index table + footer, flush, close.
    /// Safe to call multiple times (no-op after first call).
    void finalize();

    /// Returns true if finalize() has been called successfully.
    bool is_finalized() const { return finalized_; }

    /// Returns true if any write error has occurred (disk full, I/O error, etc.).
    /// Thread-safe: can be polled from a different thread than the one calling write_frame().
    bool has_write_error() const { return write_error_.load(std::memory_order_acquire); }

private:
    // Write buffer size: 256 KB reduces syscall overhead on sequential writes.
    static constexpr size_t WRITE_BUFFER_SIZE = 256 * 1024;

    std::ofstream file_;
    std::string   filepath_;                     ///< Stored for fdatasync after close
    char          write_buf_[WRITE_BUFFER_SIZE]; ///< Buffer for rdbuf()->pubsetbuf()

    std::vector<IndexEntry> index_;   ///< In-memory index, one entry per frame

    uint64_t first_timestamp_{0};     ///< Timestamp of first frame (for duration)
    uint64_t last_timestamp_{0};      ///< Timestamp of most recent frame

    bool header_written_{false};
    bool finalized_{false};
    std::atomic<bool> write_error_{false};  ///< Set on first I/O error; thread-safe

    /// Helper: write \p len bytes from \p data; checks file_.good() after write.
    /// Returns false on write error (caller decides whether to abort).
    bool raw_write(const void* data, size_t len);
};
