#pragma once

#include <cstdint>
#include <memory>
#include <vector>

/// RAII wrapper around FFmpeg libavcodec for real-time H.264 encoding.
///
/// Converts RGB24 input to YUV420P via libswscale, then encodes using
/// libx264 backend at CRF 23 with preset "fast" and no B-frames.
/// Non-copyable, non-movable -- owns codec context and frame buffers.
class H264Encoder {
public:
    /// @param width   Frame width in pixels (must be even)
    /// @param height  Frame height in pixels (must be even)
    /// @param fps     Target framerate (default 30)
    /// @param crf     Constant Rate Factor 0-51 (default 23)
    H264Encoder(int width, int height, int fps = 30, int crf = 23);
    ~H264Encoder();

    // Non-copyable, non-movable
    H264Encoder(const H264Encoder&) = delete;
    H264Encoder& operator=(const H264Encoder&) = delete;
    H264Encoder(H264Encoder&&) = delete;
    H264Encoder& operator=(H264Encoder&&) = delete;

    /// Encode one RGB24 frame. Returns compressed H.264 NAL units.
    /// May return empty vector for buffered frames.
    /// @param rgb24   Pointer to RGB24 pixel data (width * height * 3 bytes)
    /// @param width   Frame width (must match constructor)
    /// @param height  Frame height (must match constructor)
    std::vector<uint8_t> encode(const uint8_t* rgb24, int width, int height);

    /// Flush remaining buffered frames at end of recording.
    /// @returns Concatenated NAL units for all remaining frames
    std::vector<uint8_t> flush();

    /// Reset encoder state for a new recording session.
    /// Must be called when starting a new file (after reconnect etc).
    void reset();

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
    int width_;
    int height_;
    int fps_;
    int crf_;
};
