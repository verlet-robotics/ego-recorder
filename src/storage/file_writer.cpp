// FileWriter -- implementation of .egorec sequential frame writer.

#include "storage/file_writer.h"

#include <cinttypes>
#include <cstdio>
#include <cstring>
#include <stdexcept>

#include <fcntl.h>   // open, O_WRONLY
#include <unistd.h>  // fdatasync, close

// ---------------------------------------------------------------------------
// Construction / destruction
// ---------------------------------------------------------------------------

FileWriter::FileWriter(const std::string& filepath)
    : filepath_(filepath) {
    file_.open(filepath, std::ios::binary | std::ios::trunc);
    if (!file_.is_open()) {
        throw std::runtime_error("FileWriter: failed to open file for writing: " + filepath);
    }
    // Set a large write buffer to reduce syscall overhead on sequential writes.
    file_.rdbuf()->pubsetbuf(write_buf_, WRITE_BUFFER_SIZE);
}

FileWriter::~FileWriter() {
    if (!finalized_) {
        // Best-effort finalize on destruction (e.g., exception unwind or forgotten call).
        // Errors are silently swallowed here because throwing from a destructor is UB.
        try {
            finalize();
        } catch (...) {
            // Swallow -- destructor must not throw.
        }
    }
}

// ---------------------------------------------------------------------------
// write_header
// ---------------------------------------------------------------------------

void FileWriter::write_header(const FileHeader& header) {
    if (header_written_) {
        std::fprintf(stderr, "FileWriter: write_header called more than once -- ignored\n");
        return;
    }
    if (!raw_write(&header, sizeof(header))) {
        std::fprintf(stderr, "FileWriter: write_header failed (I/O error)\n");
        write_error_.store(true, std::memory_order_release);
        return;
    }
    header_written_ = true;
}

// ---------------------------------------------------------------------------
// write_frame
// ---------------------------------------------------------------------------

bool FileWriter::write_frame(const uint8_t* rgb_compressed,   size_t rgb_size,
                             const uint8_t* depth_compressed, size_t depth_size,
                             uint64_t timestamp_us,
                             uint64_t frame_number,
                             const std::vector<IMUSampleWire>& imu_samples) {
    if (finalized_) {
        std::fprintf(stderr, "FileWriter: write_frame called after finalize -- ignored\n");
        return false;
    }

    // Record byte offset of this frame block for the index.
    const uint64_t frame_offset = static_cast<uint64_t>(file_.tellp());

    // Build the frame block header.
    FrameBlockHeader fbh{};
    fbh.magic                 = FRAME_MAGIC;
    fbh.timestamp_us          = timestamp_us;
    fbh.frame_number          = frame_number;
    fbh.rgb_compressed_size   = static_cast<uint32_t>(rgb_size);
    fbh.depth_compressed_size = static_cast<uint32_t>(depth_size);
    fbh.imu_sample_count      = static_cast<uint16_t>(imu_samples.size());
    fbh.flags                 = 0;
    fbh.block_size            = static_cast<uint32_t>(
        sizeof(FrameBlockHeader)
        + rgb_size
        + depth_size
        + imu_samples.size() * sizeof(IMUSampleWire));

    bool ok = true;
    ok &= raw_write(&fbh, sizeof(fbh));
    if (rgb_size > 0) {
        ok &= raw_write(rgb_compressed, rgb_size);
    }
    if (depth_size > 0) {
        ok &= raw_write(depth_compressed, depth_size);
    }
    for (const auto& imu : imu_samples) {
        ok &= raw_write(&imu, sizeof(imu));
    }

    if (!ok) {
        std::fprintf(stderr, "FileWriter: write error during frame %" PRIu64 "\n",
                     frame_number);
        write_error_.store(true, std::memory_order_release);
        // Do not append an index entry for a partially-written frame.
        return false;
    }

    // Update index and timestamps.
    index_.push_back(IndexEntry{timestamp_us, frame_offset, frame_number});

    if (index_.size() == 1) {
        first_timestamp_ = timestamp_us;
    }
    last_timestamp_ = timestamp_us;
    return true;
}

// ---------------------------------------------------------------------------
// write_trailing_codec_data
// ---------------------------------------------------------------------------

void FileWriter::write_trailing_codec_data(const uint8_t* data, size_t size) {
    if (finalized_) {
        std::fprintf(stderr, "FileWriter: write_trailing_codec_data called after finalize -- ignored\n");
        return;
    }
    if (size > 0 && data != nullptr) {
        if (!raw_write(data, size)) {
            std::fprintf(stderr, "FileWriter: write_trailing_codec_data failed (I/O error)\n");
            write_error_.store(true, std::memory_order_release);
        }
    }
}

// ---------------------------------------------------------------------------
// finalize
// ---------------------------------------------------------------------------

void FileWriter::finalize() {
    if (finalized_) {
        return;
    }
    finalized_ = true;  // Set early so destructor won't re-enter on exception.

    if (!file_.is_open()) {
        return;
    }

    // Record where the index table starts.
    const uint64_t index_offset = static_cast<uint64_t>(file_.tellp());

    // Write all index entries sequentially.
    for (const auto& entry : index_) {
        if (!raw_write(&entry, sizeof(entry))) {
            std::fprintf(stderr, "FileWriter: error writing index entry\n");
        }
    }

    // Build and write the footer.
    FileFooter footer{};
    footer.index_magic       = INDEX_MAGIC;
    footer.index_offset      = index_offset;
    footer.index_entry_count = static_cast<uint32_t>(index_.size());
    footer.total_frames      = static_cast<uint64_t>(index_.size());
    footer.total_duration_us = (last_timestamp_ >= first_timestamp_)
                               ? (last_timestamp_ - first_timestamp_)
                               : 0;
    footer.footer_magic      = FOOTER_MAGIC;

    if (!raw_write(&footer, sizeof(footer))) {
        std::fprintf(stderr, "FileWriter: error writing footer\n");
    }

    file_.flush();
    file_.close();

    // Ensure data (especially the index table and footer written last) reaches
    // persistent storage.  Re-open write-only just for the sync -- fdatasync()
    // requires a writable fd per POSIX. O_WRONLY without O_TRUNC is safe:
    // no data is modified, we only flush dirty pages.
    int fd = ::open(filepath_.c_str(), O_WRONLY);
    if (fd >= 0) {
        ::fdatasync(fd);
        ::close(fd);
    }
}

// ---------------------------------------------------------------------------
// raw_write
// ---------------------------------------------------------------------------

bool FileWriter::raw_write(const void* data, size_t len) {
    file_.write(static_cast<const char*>(data), static_cast<std::streamsize>(len));
    if (!file_.good()) {
        std::fprintf(stderr, "FileWriter: raw_write failed (stream error)\n");
        return false;
    }
    return true;
}
