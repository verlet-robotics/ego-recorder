#pragma once

#include <cstdint>
#include <memory>
#include <string>
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
    /// @param preset  x264 speed preset (default "ultrafast")
    H264Encoder(int width, int height, int fps = 30, int crf = 23,
                const std::string& preset = "ultrafast");
    ~H264Encoder();

    // Non-copyable, non-movable
    H264Encoder(const H264Encoder&) = delete;
    H264Encoder& operator=(const H264Encoder&) = delete;
    H264Encoder(H264Encoder&&) = delete;
    H264Encoder& operator=(H264Encoder&&) = delete;

    /// Encode one RGB24 frame. Returns a copy of the encoded data.
    /// May return empty vector for buffered frames (encoder hasn't produced output yet).
    /// @param rgb24   Pointer to RGB24 pixel data (width * height * 3 bytes)
    /// @param width   Frame width (must match constructor)
    /// @param height  Frame height (must match constructor)
    std::vector<uint8_t> encode(const uint8_t* rgb24, int width, int height);

    /// Flush remaining buffered frames at end of recording.
    /// Returns a copy of the flushed data.
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
    std::string preset_;
};
