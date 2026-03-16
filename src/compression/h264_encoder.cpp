#include "compression/h264_encoder.h"

#include <cassert>
#include <stdexcept>
#include <string>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/opt.h>
#include <libavutil/imgutils.h>
#include <libswscale/swscale.h>
}

/// Pimpl to isolate FFmpeg includes from the header.
struct H264Encoder::Impl {
    const AVCodec* codec{nullptr};
    AVCodecContext* ctx{nullptr};
    AVFrame* yuv_frame{nullptr};
    AVPacket* pkt{nullptr};
    SwsContext* sws{nullptr};
    int64_t frame_counter{0};
    std::vector<uint8_t> out_buf;  ///< Reusable output buffer (avoids alloc per frame)

    /// Initialize all FFmpeg resources for encoding.
    void init(int width, int height, int fps, int crf, const std::string& preset) {
        // Find the libx264 encoder
        codec = avcodec_find_encoder_by_name("libx264");
        if (!codec) {
            throw std::runtime_error(
                "H264Encoder: libx264 encoder not found. "
                "Ensure FFmpeg was compiled with --enable-libx264.");
        }

        // Allocate codec context
        ctx = avcodec_alloc_context3(codec);
        if (!ctx) {
            throw std::runtime_error("H264Encoder: failed to allocate AVCodecContext");
        }

        // Configure encoder
        ctx->width = width;
        ctx->height = height;
        ctx->time_base = AVRational{1, fps};
        ctx->framerate = AVRational{fps, 1};
        ctx->pix_fmt = AV_PIX_FMT_YUV420P;
        ctx->gop_size = fps;       // One keyframe per second
        ctx->max_b_frames = 0;     // No B-frames for real-time encoding

        // Set CRF, preset, and tune via private options.
        // "zerolatency" disables the lookahead buffer so that each encode()
        // call produces exactly one output packet.  Without this, x264
        // buffers ~46 frames before emitting output, causing the first N
        // frame blocks to have 0-byte RGB data and misaligning RGB with
        // depth in the written file.
        //
        // "ultrafast" is critical for maintaining 30fps at 1280x720 on
        // laptop CPUs. "fast" uses ~7-25ms/frame depending on CPU, which
        // leaves no headroom for Zdepth compression + I/O within the 33ms
        // budget. "ultrafast" cuts encoding time by ~2.5x with slightly
        // larger files but identical visual quality at the same CRF —
        // an acceptable trade-off since VLMs downsample to 224x224 anyway.
        av_opt_set(ctx->priv_data, "preset", preset.c_str(), 0);
        av_opt_set(ctx->priv_data, "tune", "zerolatency", 0);
        av_opt_set(ctx->priv_data, "crf", std::to_string(crf).c_str(), 0);

        // Open the codec
        int ret = avcodec_open2(ctx, codec, nullptr);
        if (ret < 0) {
            avcodec_free_context(&ctx);
            throw std::runtime_error(
                "H264Encoder: avcodec_open2 failed (error " +
                std::to_string(ret) + ")");
        }

        // Allocate YUV420P frame
        yuv_frame = av_frame_alloc();
        if (!yuv_frame) {
            avcodec_free_context(&ctx);
            throw std::runtime_error("H264Encoder: failed to allocate AVFrame");
        }
        yuv_frame->format = AV_PIX_FMT_YUV420P;
        yuv_frame->width = width;
        yuv_frame->height = height;
        ret = av_frame_get_buffer(yuv_frame, 0);
        if (ret < 0) {
            av_frame_free(&yuv_frame);
            avcodec_free_context(&ctx);
            throw std::runtime_error(
                "H264Encoder: av_frame_get_buffer failed (error " +
                std::to_string(ret) + ")");
        }

        // Allocate packet
        pkt = av_packet_alloc();
        if (!pkt) {
            av_frame_free(&yuv_frame);
            avcodec_free_context(&ctx);
            throw std::runtime_error("H264Encoder: failed to allocate AVPacket");
        }

        // Create RGB24 -> YUV420P scaler
        sws = sws_getContext(
            width, height, AV_PIX_FMT_RGB24,
            width, height, AV_PIX_FMT_YUV420P,
            SWS_FAST_BILINEAR, nullptr, nullptr, nullptr);
        if (!sws) {
            av_packet_free(&pkt);
            av_frame_free(&yuv_frame);
            avcodec_free_context(&ctx);
            throw std::runtime_error("H264Encoder: sws_getContext failed");
        }

        frame_counter = 0;
    }

    /// Clean up all FFmpeg resources.
    void cleanup() {
        if (sws) {
            sws_freeContext(sws);
            sws = nullptr;
        }
        if (yuv_frame) {
            av_frame_free(&yuv_frame);
        }
        if (pkt) {
            av_packet_free(&pkt);
        }
        if (ctx) {
            avcodec_free_context(&ctx);
        }
        codec = nullptr;
        frame_counter = 0;
    }

    /// Receive all available packets from the encoder into out_buf.
    void receive_packets() {
        out_buf.clear();
        while (true) {
            int ret = avcodec_receive_packet(ctx, pkt);
            if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
                break;
            }
            if (ret < 0) {
                throw std::runtime_error(
                    "H264Encoder: avcodec_receive_packet failed (error " +
                    std::to_string(ret) + ")");
            }
            out_buf.insert(out_buf.end(), pkt->data, pkt->data + pkt->size);
            av_packet_unref(pkt);
        }
    }
};

H264Encoder::H264Encoder(int width, int height, int fps, int crf,
                         const std::string& preset)
    : impl_(std::make_unique<Impl>())
    , width_(width)
    , height_(height)
    , fps_(fps)
    , crf_(crf)
    , preset_(preset)
{
    // YUV420P requires even dimensions
    if (width % 2 != 0 || height % 2 != 0) {
        throw std::runtime_error(
            "H264Encoder: width (" + std::to_string(width) +
            ") and height (" + std::to_string(height) +
            ") must be even (YUV420P requirement)");
    }

    impl_->init(width, height, fps, crf, preset);
}

H264Encoder::~H264Encoder() {
    impl_->cleanup();
}

std::vector<uint8_t> H264Encoder::encode(
    const uint8_t* rgb24, int width, int height)
{
    if (width != width_ || height != height_) {
        throw std::runtime_error(
            "H264Encoder::encode: dimension mismatch (expected " +
            std::to_string(width_) + "x" + std::to_string(height_) +
            ", got " + std::to_string(width) + "x" + std::to_string(height) + ")");
    }
    if (rgb24 == nullptr) {
        throw std::runtime_error("H264Encoder::encode: null RGB24 pointer");
    }

    // Make the frame writable (required before writing to data planes)
    int ret = av_frame_make_writable(impl_->yuv_frame);
    if (ret < 0) {
        throw std::runtime_error(
            "H264Encoder: av_frame_make_writable failed (error " +
            std::to_string(ret) + ")");
    }

    // Convert RGB24 -> YUV420P
    const uint8_t* src_data[1] = {rgb24};
    int src_linesize[1] = {width * 3};
    sws_scale(impl_->sws,
              src_data, src_linesize, 0, height,
              impl_->yuv_frame->data, impl_->yuv_frame->linesize);

    // Set PTS (monotonically increasing)
    impl_->yuv_frame->pts = impl_->frame_counter++;

    // Send frame to encoder
    ret = avcodec_send_frame(impl_->ctx, impl_->yuv_frame);
    if (ret < 0) {
        throw std::runtime_error(
            "H264Encoder: avcodec_send_frame failed (error " +
            std::to_string(ret) + ")");
    }

    // Receive all available encoded packets into reusable buffer
    impl_->receive_packets();
    return impl_->out_buf;
}

std::vector<uint8_t> H264Encoder::flush() {
    // Send null frame to signal end of stream
    int ret = avcodec_send_frame(impl_->ctx, nullptr);
    if (ret < 0 && ret != AVERROR_EOF) {
        throw std::runtime_error(
            "H264Encoder: flush avcodec_send_frame(null) failed (error " +
            std::to_string(ret) + ")");
    }

    // Receive all remaining buffered packets into reusable buffer
    impl_->receive_packets();
    return impl_->out_buf;
}

void H264Encoder::reset() {
    // Close and reinitialize the codec with same parameters
    impl_->cleanup();
    impl_->init(width_, height_, fps_, crf_, preset_);
}
