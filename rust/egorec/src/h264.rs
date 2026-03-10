/// H.264 encoder and decoder using ffmpeg-next.
use std::collections::VecDeque;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum H264Error {
    #[error("H.264 decoder not found")]
    DecoderNotFound,
    #[error("H.264 encoder not found")]
    EncoderNotFound,
    #[error("ffmpeg error: {0}")]
    Ffmpeg(#[from] ffmpeg_next::Error),
}

/// H.264 encoder producing raw NAL units (no container).
/// Uses zerolatency tune for 1:1 frame-to-packet output.
pub struct H264Encoder {
    encoder: ffmpeg_next::codec::encoder::Video,
    scaler: ffmpeg_next::software::scaling::Context,
    pts: i64,
    width: u32,
    height: u32,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32, crf: u32) -> Result<Self, H264Error> {
        ffmpeg_next::init().map_err(H264Error::Ffmpeg)?;

        let codec = ffmpeg_next::codec::encoder::find(ffmpeg_next::codec::Id::H264)
            .ok_or(H264Error::EncoderNotFound)?;

        let context = ffmpeg_next::codec::Context::new_with_codec(codec);
        let mut encoder_params = context.encoder().video().map_err(H264Error::Ffmpeg)?;
        encoder_params.set_width(width);
        encoder_params.set_height(height);
        encoder_params.set_format(ffmpeg_next::format::Pixel::YUV420P);
        encoder_params.set_time_base(ffmpeg_next::Rational::new(1, fps as i32));
        encoder_params.set_frame_rate(Some(ffmpeg_next::Rational::new(fps as i32, 1)));
        encoder_params.set_max_b_frames(0);

        // Set x264 options for raw NAL output
        let mut opts = ffmpeg_next::Dictionary::new();
        opts.set("preset", "medium");
        opts.set("tune", "zerolatency");
        opts.set("crf", &crf.to_string());

        let encoder = encoder_params
            .open_with(opts)
            .map_err(H264Error::Ffmpeg)?;

        let scaler = ffmpeg_next::software::scaling::Context::get(
            ffmpeg_next::format::Pixel::RGB24,
            width,
            height,
            ffmpeg_next::format::Pixel::YUV420P,
            width,
            height,
            ffmpeg_next::software::scaling::Flags::BILINEAR,
        )
        .map_err(H264Error::Ffmpeg)?;

        Ok(Self {
            encoder,
            scaler,
            pts: 0,
            width,
            height,
        })
    }

    /// Encode one RGB24 frame. Returns (nal_data, is_keyframe) if a packet is produced.
    pub fn encode_frame(&mut self, rgb: &[u8]) -> Result<Option<(Vec<u8>, bool)>, H264Error> {
        let w = self.width as usize;
        let h = self.height as usize;

        // Build RGB24 input frame
        let mut src_frame = ffmpeg_next::frame::Video::new(
            ffmpeg_next::format::Pixel::RGB24,
            self.width,
            self.height,
        );
        let stride = src_frame.stride(0);
        for y in 0..h {
            let src_start = y * w * 3;
            let dst_start = y * stride;
            src_frame.data_mut(0)[dst_start..dst_start + w * 3]
                .copy_from_slice(&rgb[src_start..src_start + w * 3]);
        }

        // Convert RGB24 → YUV420P
        let mut yuv_frame = ffmpeg_next::frame::Video::empty();
        self.scaler
            .run(&src_frame, &mut yuv_frame)
            .map_err(H264Error::Ffmpeg)?;
        yuv_frame.set_pts(Some(self.pts));
        self.pts += 1;

        // Encode
        self.encoder
            .send_frame(&yuv_frame)
            .map_err(H264Error::Ffmpeg)?;

        self.receive_packet()
    }

    /// Flush encoder and return all remaining packets.
    pub fn flush(&mut self) -> Result<Vec<(Vec<u8>, bool)>, H264Error> {
        self.encoder.send_eof().map_err(H264Error::Ffmpeg)?;
        let mut packets = Vec::new();
        while let Some(pkt) = self.receive_packet()? {
            packets.push(pkt);
        }
        Ok(packets)
    }

    fn receive_packet(&mut self) -> Result<Option<(Vec<u8>, bool)>, H264Error> {
        let mut packet = ffmpeg_next::Packet::empty();
        match self.encoder.receive_packet(&mut packet) {
            Ok(()) => {
                let is_keyframe = packet.is_key();
                let data = packet.data().unwrap_or(&[]).to_vec();
                Ok(Some((data, is_keyframe)))
            }
            Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {
                Ok(None)
            }
            Err(e) if e == ffmpeg_next::Error::Eof => Ok(None),
            Err(e) => Err(H264Error::Ffmpeg(e)),
        }
    }
}

pub struct H264Decoder {
    decoder: ffmpeg_next::codec::decoder::Video,
    scaler: Option<ffmpeg_next::software::scaling::Context>,
    color_width: u32,
    color_height: u32,
    queue: VecDeque<Vec<u8>>,
    initialized_scaler: bool,
}

impl H264Decoder {
    pub fn new(color_width: u32, color_height: u32) -> Result<Self, H264Error> {
        ffmpeg_next::init().map_err(H264Error::Ffmpeg)?;

        // Suppress H.264 frame num change warnings from ffmpeg (AV_LOG_FATAL = 8)
        unsafe {
            ffmpeg_next::ffi::av_log_set_level(8);
        }

        let codec = ffmpeg_next::codec::decoder::find(ffmpeg_next::codec::Id::H264)
            .ok_or(H264Error::DecoderNotFound)?;

        let context = ffmpeg_next::codec::Context::new_with_codec(codec);
        let decoder = context.decoder().video().map_err(H264Error::Ffmpeg)?;

        Ok(Self {
            decoder,
            scaler: None,
            color_width,
            color_height,
            queue: VecDeque::new(),
            initialized_scaler: false,
        })
    }

    /// Feed H.264 compressed data to the decoder, drain output frames to queue.
    pub fn decode_packet(&mut self, data: &[u8]) -> Result<(), H264Error> {
        if data.is_empty() {
            self.drain_decoder()?;
            return Ok(());
        }

        let packet = ffmpeg_next::Packet::copy(data);
        // send_packet may return EAGAIN which is non-fatal
        match self.decoder.send_packet(&packet) {
            Ok(()) => {}
            Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {}
            Err(_) => {
                // Non-fatal: some packets may be incomplete at start
                return Ok(());
            }
        }

        self.drain_decoder()?;
        Ok(())
    }

    /// Flush the decoder: send EOF and drain remaining frames.
    pub fn flush(&mut self) -> Result<(), H264Error> {
        self.decoder.send_eof().ok();
        self.drain_decoder()?;
        Ok(())
    }

    /// Pop a decoded RGB24 frame from the queue, if available.
    pub fn pop_frame(&mut self) -> Option<Vec<u8>> {
        self.queue.pop_front()
    }

    /// Check if there are buffered frames in the queue.
    pub fn has_frames(&self) -> bool {
        !self.queue.is_empty()
    }

    fn ensure_scaler(&mut self) -> Result<(), H264Error> {
        if !self.initialized_scaler {
            self.scaler = Some(
                ffmpeg_next::software::scaling::Context::get(
                    ffmpeg_next::format::Pixel::YUV420P,
                    self.color_width,
                    self.color_height,
                    ffmpeg_next::format::Pixel::RGB24,
                    self.color_width,
                    self.color_height,
                    ffmpeg_next::software::scaling::Flags::BILINEAR,
                )
                .map_err(H264Error::Ffmpeg)?,
            );
            self.initialized_scaler = true;
        }
        Ok(())
    }

    fn drain_decoder(&mut self) -> Result<(), H264Error> {
        let mut decoded_frame = ffmpeg_next::frame::Video::empty();
        while self.decoder.receive_frame(&mut decoded_frame).is_ok() {
            self.ensure_scaler()?;

            let mut rgb_frame = ffmpeg_next::frame::Video::empty();
            self.scaler
                .as_mut()
                .unwrap()
                .run(&decoded_frame, &mut rgb_frame)
                .map_err(H264Error::Ffmpeg)?;

            // Copy RGB24 data from frame planes
            let w = self.color_width as usize;
            let h = self.color_height as usize;
            let mut rgb_data = vec![0u8; w * h * 3];
            let stride = rgb_frame.stride(0);
            for y in 0..h {
                let src_start = y * stride;
                let dst_start = y * w * 3;
                rgb_data[dst_start..dst_start + w * 3]
                    .copy_from_slice(&rgb_frame.data(0)[src_start..src_start + w * 3]);
            }

            self.queue.push_back(rgb_data);
        }
        Ok(())
    }
}
