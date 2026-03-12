/// EgorecReader: reads .egorec v2 files and yields decoded frames.
/// Matches the logic in src/python/egorec_reader.cpp.
use crate::format::*;
use crate::h264::H264Decoder;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use thiserror::Error;
use zdepth_sys::ZdepthDecompressor;

#[derive(Debug, Error)]
pub enum EgorecError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid .egorec file: bad magic bytes")]
    BadMagic,
    #[error("V1 .egorec files are not supported by export tools")]
    UnsupportedVersion,
    #[error("invalid frame magic at frame {0}")]
    BadFrameMagic(u64),
    #[error("failed to read footer")]
    BadFooter,
    #[error("H.264 decoder error: {0}")]
    H264(#[from] crate::h264::H264Error),
    #[error("zdepth error: {0}")]
    Zdepth(#[from] zdepth_sys::ZdepthError),
}

/// A decoded frame from an .egorec file.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// RGB24 data, row-major, size = color_width * color_height * 3
    pub rgb: Vec<u8>,
    /// Depth Z16 data, row-major, size = depth_width * depth_height
    pub depth: Vec<u16>,
    /// Hardware timestamp in microseconds
    pub timestamp_us: u64,
    /// Relative timestamp in seconds from recording start
    pub timestamp_relative_s: f64,
    /// Sequential frame number
    pub frame_number: u64,
}

pub struct EgorecReader {
    file: BufReader<File>,
    header: FileHeader,
    footer: Option<FileFooter>,
    index: Vec<IndexEntry>,
    trailing_data_offset: u64,
    total_frames: u64,
    duration_s: f64,
}

impl EgorecReader {
    /// Open an .egorec v2 file.
    pub fn open(path: &str) -> Result<Self, EgorecError> {
        let mut file = BufReader::new(File::open(path)?);

        // Read FileHeader
        let header = FileHeader::read_from(&mut file)?;

        // Validate magic
        if header.magic[..6] != FILE_MAGIC[..6] {
            return Err(EgorecError::BadMagic);
        }
        if header.magic[6] != 0x02 {
            return Err(EgorecError::UnsupportedVersion);
        }

        // Read footer from end of file
        let file_len = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::End(-(FileFooter::SIZE as i64)))?;
        let footer_result = FileFooter::read_from(&mut file);
        let footer = match footer_result {
            Ok(f) if f.footer_magic == FOOTER_MAGIC => Some(f),
            _ => None,
        };

        // Read index table if footer is valid
        let mut index = Vec::new();
        if let Some(ref f) = footer {
            if f.index_entry_count > 0 {
                file.seek(SeekFrom::Start(f.index_offset))?;
                for _ in 0..f.index_entry_count {
                    index.push(IndexEntry::read_from(&mut file)?);
                }
            }
        }

        // Calculate trailing_data_offset
        let trailing_data_offset = if !index.is_empty() {
            // Read the last indexed frame's FrameBlockHeader to get block_size
            let last = &index[index.len() - 1];
            file.seek(SeekFrom::Start(last.file_offset))?;
            let fbh = FrameBlockHeader::read_from(&mut file)?;
            if fbh.magic == FRAME_MAGIC {
                last.file_offset + fbh.block_size as u64
            } else {
                footer.as_ref().map_or(file_len, |f| f.index_offset)
            }
        } else {
            footer.as_ref().map_or(file_len, |f| f.index_offset)
        };

        let total_frames = footer.as_ref().map_or(0, |f| f.total_frames);
        let duration_s = footer
            .as_ref()
            .map_or(0.0, |f| f.total_duration_us as f64 / 1e6);

        // Seek back to start of frame data
        file.seek(SeekFrom::Start(FILE_HEADER_SIZE as u64))?;

        Ok(Self {
            file,
            header,
            footer,
            index,
            trailing_data_offset,
            total_frames,
            duration_s,
        })
    }

    pub fn header(&self) -> &FileHeader {
        &self.header
    }

    pub fn frame_count(&self) -> u64 {
        self.total_frames
    }

    pub fn duration_s(&self) -> f64 {
        self.duration_s
    }

    /// Create a frame iterator that yields all decoded frames.
    pub fn frames(mut self) -> Result<FrameIterator, EgorecError> {
        // Seek to start of frame data
        self.file
            .seek(SeekFrom::Start(FILE_HEADER_SIZE as u64))?;

        let h264_decoder = H264Decoder::new(self.header.color_width, self.header.color_height)?;
        let zdepth = ZdepthDecompressor::new()?;

        Ok(FrameIterator {
            reader: self,
            h264_decoder,
            zdepth,
            current_frame: 0,
            trailing_flushed: false,
        })
    }
}

pub struct FrameIterator {
    reader: EgorecReader,
    h264_decoder: H264Decoder,
    zdepth: ZdepthDecompressor,
    current_frame: u64,
    trailing_flushed: bool,
}

fn relative_timestamp_s(frame_timestamp_us: u64, start_timestamp_us: u64) -> f64 {
    if start_timestamp_us > 0 {
        frame_timestamp_us.saturating_sub(start_timestamp_us) as f64 / 1e6
    } else {
        0.0
    }
}

impl Iterator for FrameIterator {
    type Item = Result<DecodedFrame, EgorecError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_frame() {
            Ok(Some(frame)) => Some(Ok(frame)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl FrameIterator {
    /// Access the file header.
    pub fn header(&self) -> &FileHeader {
        &self.reader.header
    }

    /// Total number of frames.
    pub fn frame_count(&self) -> u64 {
        self.reader.total_frames
    }

    fn next_frame(&mut self) -> Result<Option<DecodedFrame>, EgorecError> {
        let total = self.reader.total_frames;

        // Past the last indexed frame: handle trailing flush
        if self.current_frame >= total {
            if !self.trailing_flushed {
                self.flush_trailing_and_decoder()?;
                self.trailing_flushed = true;
            }
            if self.h264_decoder.has_frames() {
                let rgb = self.h264_decoder.pop_frame().unwrap();
                let depth_pixels =
                    (self.reader.header.depth_width * self.reader.header.depth_height) as usize;
                let depth = vec![0u16; depth_pixels];
                let frame_num = self.current_frame;
                self.current_frame += 1;
                return Ok(Some(DecodedFrame {
                    rgb,
                    depth,
                    timestamp_us: 0,
                    timestamp_relative_s: 0.0,
                    frame_number: frame_num,
                }));
            }
            return Ok(None);
        }

        // Check if we have a buffered RGB frame from the decoder
        let have_rgb_from_queue = self.h264_decoder.has_frames();
        let rgb_from_queue = if have_rgb_from_queue {
            self.h264_decoder.pop_frame()
        } else {
            None
        };

        // Seek to frame position if we have an index
        if !self.reader.index.is_empty() && (self.current_frame as usize) < self.reader.index.len()
        {
            let offset = self.reader.index[self.current_frame as usize].file_offset;
            self.reader.file.seek(SeekFrom::Start(offset))?;
        }

        // Read FrameBlockHeader
        let fbh = FrameBlockHeader::read_from(&mut self.reader.file)?;
        if fbh.magic != FRAME_MAGIC {
            return Err(EgorecError::BadFrameMagic(self.current_frame));
        }

        // Read H.264 data
        let mut h264_data = vec![0u8; fbh.rgb_compressed_size as usize];
        if fbh.rgb_compressed_size > 0 {
            self.reader.file.read_exact(&mut h264_data)?;
        }

        // Read Zdepth data
        let mut zdepth_data = vec![0u8; fbh.depth_compressed_size as usize];
        if fbh.depth_compressed_size > 0 {
            self.reader.file.read_exact(&mut zdepth_data)?;
        }

        // Skip IMU samples
        if fbh.imu_sample_count > 0 {
            let skip = fbh.imu_sample_count as u64 * IMUSampleWire::SIZE as u64;
            self.reader.file.seek(SeekFrom::Current(skip as i64))?;
        }

        // H.264 decode
        let rgb = if let Some(rgb) = rgb_from_queue {
            // Still feed H.264 data to maintain decoder state
            if fbh.rgb_compressed_size > 0 {
                self.h264_decoder.decode_packet(&h264_data)?;
            }
            rgb
        } else {
            self.h264_decoder.decode_packet(&h264_data)?;
            if self.h264_decoder.has_frames() {
                self.h264_decoder.pop_frame().unwrap()
            } else {
                // Decoder hasn't output a frame yet (buffering)
                let rgb_size =
                    (self.reader.header.color_width * self.reader.header.color_height * 3) as usize;
                vec![0u8; rgb_size]
            }
        };

        // Zdepth decompress (gracefully handle MissingPFrame for partial recordings)
        let depth = if fbh.depth_compressed_size > 0 {
            match self.zdepth.decompress(&zdepth_data) {
                Ok((_w, _h, d)) => d,
                Err(_) => {
                    // MissingPFrame or other error: return zero depth
                    // This happens with partial recordings that start mid-stream
                    let pixels = (self.reader.header.depth_width
                        * self.reader.header.depth_height) as usize;
                    vec![0u16; pixels]
                }
            }
        } else {
            let pixels =
                (self.reader.header.depth_width * self.reader.header.depth_height) as usize;
            vec![0u16; pixels]
        };

        // Compute relative timestamp
        let timestamp_relative_s =
            relative_timestamp_s(fbh.timestamp_us, self.reader.header.start_timestamp_us);

        self.current_frame += 1;

        Ok(Some(DecodedFrame {
            rgb,
            depth,
            timestamp_us: fbh.timestamp_us,
            timestamp_relative_s,
            frame_number: fbh.frame_number,
        }))
    }

    /// Flush trailing H.264 data between last frame and index, then drain decoder.
    fn flush_trailing_and_decoder(&mut self) -> Result<(), EgorecError> {
        // Only process trailing H.264 data if the file has RGB (codec != 0)
        if self.reader.header.rgb_codec != 0 {
            if let Some(ref footer) = self.reader.footer {
                if self.reader.trailing_data_offset < footer.index_offset {
                    let trailing_size =
                        (footer.index_offset - self.reader.trailing_data_offset) as usize;
                    if trailing_size > 0 {
                        self.reader
                            .file
                            .seek(SeekFrom::Start(self.reader.trailing_data_offset))?;
                        let mut trailing_buf = vec![0u8; trailing_size];
                        self.reader.file.read_exact(&mut trailing_buf)?;
                        self.h264_decoder.decode_packet(&trailing_buf)?;
                    }
                }
            }

            // Flush decoder with EOF
            self.h264_decoder.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::relative_timestamp_s;

    #[test]
    fn relative_timestamp_uses_delta_when_frame_follows_start() {
        assert_eq!(relative_timestamp_s(2_500_000, 1_000_000), 1.5);
    }

    #[test]
    fn relative_timestamp_saturates_when_frame_precedes_start() {
        assert_eq!(relative_timestamp_s(900_000, 1_000_000), 0.0);
    }
}
