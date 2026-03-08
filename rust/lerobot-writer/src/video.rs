/// MP4 video writer using ffmpeg-next.
/// Encodes RGB24 frames to H.264 in an MP4 container.
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("ffmpeg error: {0}")]
    Ffmpeg(#[from] ffmpeg_next::Error),
    #[error("encoder not found")]
    EncoderNotFound,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Mp4Writer {
    octx: ffmpeg_next::format::context::Output,
    encoder: ffmpeg_next::codec::encoder::Video,
    scaler: ffmpeg_next::software::scaling::Context,
    stream_index: usize,
    pts: i64,
    width: u32,
    height: u32,
    time_base: ffmpeg_next::Rational,
}

impl Mp4Writer {
    /// Create a new MP4 writer.
    pub fn new(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, VideoError> {
        ffmpeg_next::init().map_err(VideoError::Ffmpeg)?;

        let mut octx = ffmpeg_next::format::output(path).map_err(VideoError::Ffmpeg)?;

        let codec = ffmpeg_next::codec::encoder::find(ffmpeg_next::codec::Id::H264)
            .ok_or(VideoError::EncoderNotFound)?;

        let mut stream = octx.add_stream(codec).map_err(VideoError::Ffmpeg)?;
        let stream_index = stream.index();

        let mut encoder = ffmpeg_next::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(VideoError::Ffmpeg)?;

        let time_base = ffmpeg_next::Rational::new(1, fps as i32);
        encoder.set_width(width);
        encoder.set_height(height);
        encoder.set_format(ffmpeg_next::format::Pixel::YUV420P);
        encoder.set_time_base(time_base);
        encoder.set_frame_rate(Some(ffmpeg_next::Rational::new(fps as i32, 1)));

        // Set CRF via private options
        let mut opts = ffmpeg_next::Dictionary::new();
        opts.set("crf", "23");
        opts.set("preset", "medium");

        let encoder = encoder.open_with(opts).map_err(VideoError::Ffmpeg)?;

        stream.set_parameters(&encoder);

        octx.write_header().map_err(VideoError::Ffmpeg)?;

        let scaler = ffmpeg_next::software::scaling::Context::get(
            ffmpeg_next::format::Pixel::RGB24,
            width,
            height,
            ffmpeg_next::format::Pixel::YUV420P,
            width,
            height,
            ffmpeg_next::software::scaling::Flags::BILINEAR,
        )
        .map_err(VideoError::Ffmpeg)?;

        Ok(Self {
            octx,
            encoder,
            scaler,
            stream_index,
            pts: 0,
            width,
            height,
            time_base,
        })
    }

    /// Add an RGB24 frame to the video.
    pub fn add_frame(&mut self, rgb: &[u8]) -> Result<(), VideoError> {
        let mut rgb_frame = ffmpeg_next::frame::Video::new(
            ffmpeg_next::format::Pixel::RGB24,
            self.width,
            self.height,
        );

        // Copy RGB data into frame, respecting stride
        let stride = rgb_frame.stride(0);
        let w3 = self.width as usize * 3;
        for y in 0..self.height as usize {
            let src_start = y * w3;
            let dst_start = y * stride;
            rgb_frame.data_mut(0)[dst_start..dst_start + w3]
                .copy_from_slice(&rgb[src_start..src_start + w3]);
        }

        let mut yuv_frame = ffmpeg_next::frame::Video::empty();
        self.scaler
            .run(&rgb_frame, &mut yuv_frame)
            .map_err(VideoError::Ffmpeg)?;

        yuv_frame.set_pts(Some(self.pts));
        self.pts += 1;

        self.encoder
            .send_frame(&yuv_frame)
            .map_err(VideoError::Ffmpeg)?;
        self.receive_and_write_packets()?;

        Ok(())
    }

    /// Finalize the video: flush encoder and write trailer.
    pub fn finish(mut self) -> Result<(), VideoError> {
        self.encoder.send_eof().map_err(VideoError::Ffmpeg)?;
        self.receive_and_write_packets()?;
        self.octx.write_trailer().map_err(VideoError::Ffmpeg)?;
        Ok(())
    }

    fn receive_and_write_packets(&mut self) -> Result<(), VideoError> {
        let mut packet = ffmpeg_next::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream_index);
            packet.rescale_ts(self.time_base, self.octx.stream(self.stream_index).unwrap().time_base());
            packet
                .write_interleaved(&mut self.octx)
                .map_err(VideoError::Ffmpeg)?;
        }
        Ok(())
    }
}
