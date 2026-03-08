// egorec_reader -- pybind11 C extension module for reading .egorec v2 files.
//
// Exposes EgorecFile class to Python with:
//   - header()      -> dict with all metadata
//   - frame_count() -> total number of frames
//   - frames()      -> FrameIterator yielding decoded RGB/depth numpy arrays
//
// H.264 decoding is stateful (P-frame dependencies handled via FFmpeg decoder).
// Zdepth decompression returns bit-exact original Z16 values.

#include <pybind11/pybind11.h>
#include <pybind11/numpy.h>
#include <pybind11/stl.h>

#include "storage/binary_format.h"
#include "compression/zdepth_compressor.h"

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/imgutils.h>
#include <libswscale/swscale.h>
}

#include <fstream>
#include <vector>
#include <deque>
#include <string>
#include <cstring>
#include <stdexcept>

namespace py = pybind11;

// Forward declaration
struct FrameIterator;

class EgorecFile {
    friend struct FrameIterator;

    std::ifstream file_;
    FileHeader header_;
    FileFooter footer_;
    bool has_footer_ = false;
    std::vector<IndexEntry> index_;
    uint64_t trailing_data_offset_ = 0;  // byte offset where trailing codec data starts

    // Zdepth decompressor
    std::unique_ptr<ZdepthCompressor> zdepth_;

    // H.264 decoder state
    AVCodecContext* dec_ctx_ = nullptr;
    AVFrame* yuv_frame_ = nullptr;
    AVPacket* pkt_ = nullptr;
    SwsContext* sws_ = nullptr;
    std::deque<std::vector<uint8_t>> decoded_rgb_queue_;  // buffered decoded frames
    std::vector<uint8_t> rgb_buffer_;  // scratch for sws_scale output

    uint64_t current_frame_ = 0;
    uint64_t total_frames_ = 0;
    bool trailing_flushed_ = false;

public:
    explicit EgorecFile(const std::string& path) {
        // 1. Open file in binary read mode
        file_.open(path, std::ios::binary);
        if (!file_.is_open()) {
            throw std::runtime_error("Failed to open file: " + path);
        }

        // 2. Read FileHeader
        file_.read(reinterpret_cast<char*>(&header_), sizeof(FileHeader));
        if (!file_.good()) {
            throw std::runtime_error("Failed to read FileHeader from: " + path);
        }

        // Validate magic starts with "EGOREC"
        if (std::memcmp(header_.magic, "EGOREC", 6) != 0) {
            throw std::runtime_error("Invalid .egorec file: bad magic bytes in " + path);
        }

        // Check v2 only (per locked decision)
        if (header_.magic[6] != 0x02) {
            throw std::runtime_error(
                "V1 .egorec files are not supported by export tools. "
                "Re-record with v2 format.");
        }

        // 3. Read footer: seek to end - sizeof(FileFooter)
        file_.seekg(-static_cast<std::streamoff>(sizeof(FileFooter)), std::ios::end);
        file_.read(reinterpret_cast<char*>(&footer_), sizeof(FileFooter));
        if (file_.good() && footer_.footer_magic == FOOTER_MAGIC) {
            has_footer_ = true;
        }

        // 4. If footer valid, read index table
        if (has_footer_ && footer_.index_entry_count > 0) {
            index_.resize(footer_.index_entry_count);
            file_.seekg(static_cast<std::streamoff>(footer_.index_offset));
            file_.read(reinterpret_cast<char*>(index_.data()),
                       footer_.index_entry_count * sizeof(IndexEntry));
            if (!file_.good()) {
                throw std::runtime_error("Failed to read index table from: " + path);
            }
        }

        // 5. Calculate trailing_data_offset_
        if (!index_.empty()) {
            // Seek to the last indexed frame to read its FrameBlockHeader
            file_.seekg(static_cast<std::streamoff>(index_.back().file_offset));
            FrameBlockHeader fbh;
            file_.read(reinterpret_cast<char*>(&fbh), sizeof(FrameBlockHeader));
            if (file_.good() && fbh.magic == FRAME_MAGIC) {
                trailing_data_offset_ = index_.back().file_offset + fbh.block_size;
            } else {
                // Fallback: no trailing data
                trailing_data_offset_ = footer_.index_offset;
            }
        } else if (has_footer_) {
            trailing_data_offset_ = footer_.index_offset;
        }

        // 6. Set total_frames_
        total_frames_ = has_footer_ ? footer_.total_frames : 0;

        // 7. Initialize ZdepthCompressor with header's depth dimensions
        zdepth_ = std::make_unique<ZdepthCompressor>(
            static_cast<int>(header_.depth_width),
            static_cast<int>(header_.depth_height));

        // 8. Initialize H.264 decoder
        init_decoder();

        // 9. Seek file position back to start of frame data
        file_.seekg(sizeof(FileHeader));
    }

    ~EgorecFile() {
        cleanup_decoder();
    }

    // Non-copyable, non-movable
    EgorecFile(const EgorecFile&) = delete;
    EgorecFile& operator=(const EgorecFile&) = delete;
    EgorecFile(EgorecFile&&) = delete;
    EgorecFile& operator=(EgorecFile&&) = delete;

    /// Return all FileHeader fields as a Python dict
    py::dict header() const {
        py::dict d;

        d["session_name"] = std::string(header_.session_name);
        d["format_version"] = static_cast<int>(header_.magic[6]);
        d["frame_count"] = total_frames_;
        d["duration_s"] = has_footer_
            ? static_cast<double>(footer_.total_duration_us) / 1e6
            : 0.0;
        d["start_ts_us"] = header_.start_timestamp_us;

        // Depth intrinsics
        d["depth_width"] = header_.depth_width;
        d["depth_height"] = header_.depth_height;
        d["depth_scale"] = header_.depth_scale;
        d["depth_fx"] = header_.depth_fx;
        d["depth_fy"] = header_.depth_fy;
        d["depth_ppx"] = header_.depth_ppx;
        d["depth_ppy"] = header_.depth_ppy;
        d["depth_distortion_model"] = header_.depth_distortion_model;

        py::list depth_dist;
        for (int i = 0; i < 5; i++) {
            depth_dist.append(header_.depth_distortion_coeffs[i]);
        }
        d["depth_distortion"] = depth_dist;

        // Color intrinsics
        d["color_width"] = header_.color_width;
        d["color_height"] = header_.color_height;
        d["color_fx"] = header_.color_fx;
        d["color_fy"] = header_.color_fy;
        d["color_ppx"] = header_.color_ppx;
        d["color_ppy"] = header_.color_ppy;
        d["color_distortion_model"] = header_.color_distortion_model;

        py::list color_dist;
        for (int i = 0; i < 5; i++) {
            color_dist.append(header_.color_distortion_coeffs[i]);
        }
        d["color_distortion"] = color_dist;

        // Extrinsics
        py::list ext_r;
        for (int i = 0; i < 9; i++) {
            ext_r.append(header_.extrinsic_rotation[i]);
        }
        d["extrinsic_R"] = ext_r;

        py::list ext_t;
        for (int i = 0; i < 3; i++) {
            ext_t.append(header_.extrinsic_translation[i]);
        }
        d["extrinsic_t"] = ext_t;

        // Compression settings
        d["rgb_codec"] = static_cast<int>(header_.rgb_codec);
        d["depth_codec"] = static_cast<int>(header_.depth_codec);

        // Flags
        d["has_imu"] = static_cast<bool>(header_.flags & 0x01);

        // Camera info
        d["serial_number"] = std::string(header_.serial_number);
        d["usb_type"] = std::string(header_.usb_type);

        return d;
    }

    uint64_t frame_count() const {
        return total_frames_;
    }

    /// Read and decode a single frame at the given sequential index
    py::dict read_frame(uint64_t frame_idx) {
        // Handle end-of-file: flush trailing codec data and decoder
        if (frame_idx >= total_frames_) {
            if (!trailing_flushed_) {
                flush_trailing_and_decoder();
                trailing_flushed_ = true;
            }
            if (!decoded_rgb_queue_.empty()) {
                // Return a frame using buffered RGB with a synthetic depth
                auto rgb_data = std::move(decoded_rgb_queue_.front());
                decoded_rgb_queue_.pop_front();

                // Create zero-filled depth for trailing flush frames
                size_t depth_pixels = header_.depth_width * header_.depth_height;
                std::vector<uint16_t> depth_data(depth_pixels, 0);

                return make_frame_dict(
                    rgb_data, depth_data,
                    0, 0.0, frame_idx);
            }
            throw py::stop_iteration();
        }

        // 1. Check decoded_rgb_queue_ for buffered frames
        std::vector<uint8_t> rgb_result;
        bool have_rgb_from_queue = false;
        if (!decoded_rgb_queue_.empty()) {
            rgb_result = std::move(decoded_rgb_queue_.front());
            decoded_rgb_queue_.pop_front();
            have_rgb_from_queue = true;
        }

        // 2. Seek to frame position (if we have an index)
        if (!index_.empty() && frame_idx < index_.size()) {
            file_.seekg(static_cast<std::streamoff>(index_[frame_idx].file_offset));
        }
        // Otherwise, file position should already be at the next frame

        // 3. Read FrameBlockHeader
        FrameBlockHeader fbh;
        file_.read(reinterpret_cast<char*>(&fbh), sizeof(FrameBlockHeader));
        if (!file_.good()) {
            throw std::runtime_error("Failed to read FrameBlockHeader at frame " +
                                     std::to_string(frame_idx));
        }
        if (fbh.magic != FRAME_MAGIC) {
            throw std::runtime_error("Invalid frame magic at frame " +
                                     std::to_string(frame_idx));
        }

        // 4. Read H.264 data
        std::vector<uint8_t> h264_data(fbh.rgb_compressed_size);
        if (fbh.rgb_compressed_size > 0) {
            file_.read(reinterpret_cast<char*>(h264_data.data()),
                       fbh.rgb_compressed_size);
        }

        // 5. Read Zdepth data
        std::vector<uint8_t> zdepth_data(fbh.depth_compressed_size);
        if (fbh.depth_compressed_size > 0) {
            file_.read(reinterpret_cast<char*>(zdepth_data.data()),
                       fbh.depth_compressed_size);
        }

        // 6. Skip IMU samples
        if (fbh.imu_sample_count > 0) {
            file_.seekg(fbh.imu_sample_count * sizeof(IMUSampleWire), std::ios::cur);
        }

        // H.264 decode
        if (!have_rgb_from_queue) {
            decode_h264_packet(h264_data.data(), h264_data.size());

            if (!decoded_rgb_queue_.empty()) {
                rgb_result = std::move(decoded_rgb_queue_.front());
                decoded_rgb_queue_.pop_front();
            } else {
                // Decoder hasn't output a frame yet (buffering).
                // Return a zero RGB frame as placeholder -- this shouldn't happen
                // in normal operation but handles edge cases.
                size_t rgb_size = header_.color_width * header_.color_height * 3;
                rgb_result.resize(rgb_size, 0);
            }
        } else {
            // We already have an RGB frame from the queue. Still need to feed
            // the H.264 data to the decoder to maintain state.
            if (fbh.rgb_compressed_size > 0) {
                decode_h264_packet(h264_data.data(), h264_data.size());
            }
        }

        // Zdepth decompress
        std::vector<uint16_t> depth_result;
        if (fbh.depth_compressed_size > 0) {
            depth_result = zdepth_->decompress(zdepth_data.data(), zdepth_data.size());
        } else {
            size_t depth_pixels = header_.depth_width * header_.depth_height;
            depth_result.resize(depth_pixels, 0);
        }

        // Compute relative timestamp
        double timestamp_relative_s = 0.0;
        if (header_.start_timestamp_us > 0) {
            timestamp_relative_s = static_cast<double>(
                fbh.timestamp_us - header_.start_timestamp_us) / 1e6;
        }

        return make_frame_dict(
            rgb_result, depth_result,
            fbh.timestamp_us, timestamp_relative_s, fbh.frame_number);
    }

    /// Create a FrameIterator for this file (resets to beginning)
    FrameIterator frames_iterator();

private:
    void init_decoder() {
        const AVCodec* codec = avcodec_find_decoder(AV_CODEC_ID_H264);
        if (!codec) {
            throw std::runtime_error("H.264 decoder not found in FFmpeg");
        }

        dec_ctx_ = avcodec_alloc_context3(codec);
        if (!dec_ctx_) {
            throw std::runtime_error("Failed to allocate H.264 decoder context");
        }

        if (avcodec_open2(dec_ctx_, codec, nullptr) < 0) {
            avcodec_free_context(&dec_ctx_);
            throw std::runtime_error("Failed to open H.264 decoder");
        }

        yuv_frame_ = av_frame_alloc();
        if (!yuv_frame_) {
            avcodec_free_context(&dec_ctx_);
            throw std::runtime_error("Failed to allocate AVFrame");
        }

        pkt_ = av_packet_alloc();
        if (!pkt_) {
            av_frame_free(&yuv_frame_);
            avcodec_free_context(&dec_ctx_);
            throw std::runtime_error("Failed to allocate AVPacket");
        }

        // SwsContext for YUV420P -> RGB24
        sws_ = sws_getContext(
            static_cast<int>(header_.color_width),
            static_cast<int>(header_.color_height),
            AV_PIX_FMT_YUV420P,
            static_cast<int>(header_.color_width),
            static_cast<int>(header_.color_height),
            AV_PIX_FMT_RGB24,
            SWS_BILINEAR,
            nullptr, nullptr, nullptr);
        if (!sws_) {
            av_packet_free(&pkt_);
            av_frame_free(&yuv_frame_);
            avcodec_free_context(&dec_ctx_);
            throw std::runtime_error("Failed to create SwsContext");
        }

        // Allocate RGB scratch buffer
        rgb_buffer_.resize(header_.color_width * header_.color_height * 3);
    }

    void cleanup_decoder() {
        if (sws_) { sws_freeContext(sws_); sws_ = nullptr; }
        if (yuv_frame_) { av_frame_free(&yuv_frame_); yuv_frame_ = nullptr; }
        if (pkt_) { av_packet_free(&pkt_); pkt_ = nullptr; }
        if (dec_ctx_) { avcodec_free_context(&dec_ctx_); dec_ctx_ = nullptr; }
    }

    void reset_decoder() {
        cleanup_decoder();
        init_decoder();
    }

    /// Feed H.264 data to decoder and drain all output frames into queue
    void decode_h264_packet(const uint8_t* data, size_t size) {
        if (size == 0) {
            // Try to receive any buffered frames
            drain_decoder();
            return;
        }

        pkt_->data = const_cast<uint8_t*>(data);
        pkt_->size = static_cast<int>(size);

        int ret = avcodec_send_packet(dec_ctx_, pkt_);
        if (ret < 0 && ret != AVERROR(EAGAIN)) {
            // Non-fatal: some packets may be incomplete at start
            return;
        }

        drain_decoder();
    }

    /// Drain all available frames from the decoder into decoded_rgb_queue_
    void drain_decoder() {
        while (true) {
            int ret = avcodec_receive_frame(dec_ctx_, yuv_frame_);
            if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
                break;
            }
            if (ret < 0) {
                break;
            }

            // Convert YUV420P -> RGB24
            uint8_t* dst_data[1] = { rgb_buffer_.data() };
            int dst_linesize[1] = { static_cast<int>(header_.color_width * 3) };

            sws_scale(sws_,
                      yuv_frame_->data, yuv_frame_->linesize,
                      0, static_cast<int>(header_.color_height),
                      dst_data, dst_linesize);

            // Copy into queue
            decoded_rgb_queue_.emplace_back(rgb_buffer_.begin(), rgb_buffer_.end());
        }
    }

    /// Flush trailing H.264 data and drain the decoder
    void flush_trailing_and_decoder() {
        // 1. Read and feed trailing codec flush data
        if (has_footer_ && trailing_data_offset_ < footer_.index_offset) {
            size_t trailing_size = footer_.index_offset - trailing_data_offset_;
            if (trailing_size > 0) {
                std::vector<uint8_t> trailing_buf(trailing_size);
                file_.seekg(static_cast<std::streamoff>(trailing_data_offset_));
                file_.read(reinterpret_cast<char*>(trailing_buf.data()),
                           static_cast<std::streamoff>(trailing_size));
                if (file_.good()) {
                    decode_h264_packet(trailing_buf.data(), trailing_buf.size());
                }
            }
        }

        // 2. Flush decoder with null packet
        avcodec_send_packet(dec_ctx_, nullptr);
        drain_decoder();
    }

    /// Build a Python dict for a single frame
    py::dict make_frame_dict(
        const std::vector<uint8_t>& rgb_data,
        const std::vector<uint16_t>& depth_data,
        uint64_t timestamp_us,
        double timestamp_relative_s,
        uint64_t frame_number)
    {
        py::dict frame;
        frame["timestamp_us"] = timestamp_us;
        frame["timestamp_relative_s"] = timestamp_relative_s;
        frame["frame_number"] = frame_number;

        // Create RGB numpy array: shape (H, W, 3), dtype uint8
        auto rgb_array = py::array_t<uint8_t>(
            {static_cast<py::ssize_t>(header_.color_height),
             static_cast<py::ssize_t>(header_.color_width),
             static_cast<py::ssize_t>(3)});
        auto rgb_buf = rgb_array.mutable_unchecked<3>();
        std::memcpy(rgb_array.mutable_data(), rgb_data.data(),
                    std::min(rgb_data.size(),
                             static_cast<size_t>(header_.color_width * header_.color_height * 3)));
        frame["rgb"] = rgb_array;

        // Create depth numpy array: shape (H, W), dtype uint16
        auto depth_array = py::array_t<uint16_t>(
            {static_cast<py::ssize_t>(header_.depth_height),
             static_cast<py::ssize_t>(header_.depth_width)});
        std::memcpy(depth_array.mutable_data(), depth_data.data(),
                    std::min(depth_data.size() * sizeof(uint16_t),
                             static_cast<size_t>(header_.depth_width * header_.depth_height * sizeof(uint16_t))));
        frame["depth"] = depth_array;

        return frame;
    }
};

struct FrameIterator {
    EgorecFile* file;
    uint64_t current;

    py::dict next() {
        // Check if there are still frames or buffered decoded data
        if (current >= file->total_frames_ && file->decoded_rgb_queue_.empty()) {
            // Try flushing trailing data first
            if (!file->trailing_flushed_) {
                file->flush_trailing_and_decoder();
                file->trailing_flushed_ = true;
                if (!file->decoded_rgb_queue_.empty()) {
                    return file->read_frame(current++);
                }
            }
            throw py::stop_iteration();
        }
        return file->read_frame(current++);
    }
};

FrameIterator EgorecFile::frames_iterator() {
    // Reset to beginning of frame data
    file_.seekg(sizeof(FileHeader));
    current_frame_ = 0;
    decoded_rgb_queue_.clear();
    trailing_flushed_ = false;

    // Reset decoder state for fresh iteration
    reset_decoder();

    return FrameIterator{this, 0};
}

PYBIND11_MODULE(egorec_reader, m) {
    m.doc() = "C++ reader for .egorec v2 files";

    py::class_<FrameIterator>(m, "FrameIterator")
        .def("__iter__", [](FrameIterator& self) -> FrameIterator& { return self; })
        .def("__next__", &FrameIterator::next);

    py::class_<EgorecFile>(m, "EgorecFile")
        .def(py::init<const std::string&>())
        .def("header", &EgorecFile::header)
        .def("frame_count", &EgorecFile::frame_count)
        .def("frames", &EgorecFile::frames_iterator,
             py::keep_alive<0, 1>());  // iterator refs file
}
