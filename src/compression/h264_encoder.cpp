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

    /// Initialize all FFmpeg resources for encoding.
    void init(int width, int height, int fps, int crf) {
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

        // Set CRF and preset via private options
        av_opt_set(ctx->priv_data, "preset", "fast", 0);
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

    /// Receive all available packets from the encoder, appending NAL data.
    std::vector<uint8_t> receive_packets() {
        std::vector<uint8_t> out;
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
            out.insert(out.end(), pkt->data, pkt->data + pkt->size);
            av_packet_unref(pkt);
        }
        return out;
    }
};

H264Encoder::H264Encoder(int width, int height, int fps, int crf)
    : impl_(std::make_unique<Impl>())
    , width_(width)
    , height_(height)
    , fps_(fps)
    , crf_(crf)
{
    // YUV420P requires even dimensions
    if (width % 2 != 0 || height % 2 != 0) {
        throw std::runtime_error(
            "H264Encoder: width (" + std::to_string(width) +
            ") and height (" + std::to_string(height) +
            ") must be even (YUV420P requirement)");
    }

    impl_->init(width, height, fps, crf);
}

H264Encoder::~H264Encoder() {
    impl_->cleanup();
}

std::vector<uint8_t> H264Encoder::encode(
    const uint8_t* rgb24, int width, int height)
{
    assert(width == width_ && "Width mismatch");
    assert(height == height_ && "Height mismatch");
    assert(rgb24 != nullptr && "Null RGB24 pointer");

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

    // Receive all available encoded packets
    return impl_->receive_packets();
}

std::vector<uint8_t> H264Encoder::flush() {
    // Send null frame to signal end of stream
    int ret = avcodec_send_frame(impl_->ctx, nullptr);
    if (ret < 0 && ret != AVERROR_EOF) {
        throw std::runtime_error(
            "H264Encoder: flush avcodec_send_frame(null) failed (error " +
            std::to_string(ret) + ")");
    }

    // Receive all remaining buffered packets
    return impl_->receive_packets();
}

void H264Encoder::reset() {
    // Close and reinitialize the codec with same parameters
    impl_->cleanup();
    impl_->init(width_, height_, fps_, crf_);
}
