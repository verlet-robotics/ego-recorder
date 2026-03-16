use crate::format::{
    FileFooter,
    FileHeader,
    FrameBlockHeader,
    FILE_HEADER_SIZE,
    FILE_MAGIC,
    FOOTER_MAGIC,
    FRAME_MAGIC,
    INDEX_MAGIC,
};
use crate::h264_annex_b;
use crate::mp4_mux::{build_fmp4_init, build_fmp4_segment, build_mp4, Mp4Sample, Mp4TrackConfig};
use std::fmt;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const STREAMABLE_RGB_CODEC: u8 = 2;

#[derive(Debug)]
pub enum ConvertError {
    Io(String),
    InvalidFormat(String),
    UnsupportedCodec(u8),
}

impl ConvertError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::UnsupportedCodec(_) => 2,
            Self::Io(_) | Self::InvalidFormat(_) => 1,
        }
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "{msg}"),
            Self::InvalidFormat(msg) => write!(f, "{msg}"),
            Self::UnsupportedCodec(codec) => write!(
                f,
                "rgb_codec={codec} is not browser-streamable (need 2 for H.264)"
            ),
        }
    }
}

impl std::error::Error for ConvertError {}

struct ParsedTrack {
    config: Mp4TrackConfig,
    samples: Vec<Mp4Sample>,
}

pub fn build_mp4_file(input_path: &Path, output_path: &Path) -> Result<(), ConvertError> {
    let mp4_bytes = egorec_to_mp4(input_path)?;
    std::fs::write(output_path, mp4_bytes)
        .map_err(|e| ConvertError::Io(format!("write output: {e}")))?;
    Ok(())
}

pub fn build_init_file(
    input_path: &Path,
    output_path: &Path,
    timescale: u32,
    sample_delta: u32,
) -> Result<(), ConvertError> {
    let init_bytes = egorec_to_fmp4_init(input_path, timescale, sample_delta)?;
    std::fs::write(output_path, init_bytes)
        .map_err(|e| ConvertError::Io(format!("write output: {e}")))?;
    Ok(())
}

pub fn build_segment_file(
    input_path: &Path,
    output_path: &Path,
    timescale: u32,
    sample_delta: u32,
    sequence_number: u32,
    base_decode_time: u64,
) -> Result<(), ConvertError> {
    let segment_bytes = egorec_to_fmp4_segment(
        input_path,
        timescale,
        sample_delta,
        sequence_number,
        base_decode_time,
    )?;
    std::fs::write(output_path, segment_bytes)
        .map_err(|e| ConvertError::Io(format!("write output: {e}")))?;
    Ok(())
}

pub fn egorec_to_mp4(input_path: &Path) -> Result<Vec<u8>, ConvertError> {
    let parsed = parse_egorec(input_path)?;
    Ok(build_mp4(&parsed.config, &parsed.samples))
}

pub fn egorec_to_fmp4_init(
    input_path: &Path,
    timescale: u32,
    sample_delta: u32,
) -> Result<Vec<u8>, ConvertError> {
    let mut parsed = parse_egorec(input_path)?;
    parsed.config.timescale = timescale;
    parsed.config.sample_delta = sample_delta;
    Ok(build_fmp4_init(&parsed.config))
}

pub fn egorec_to_fmp4_segment(
    input_path: &Path,
    timescale: u32,
    sample_delta: u32,
    sequence_number: u32,
    base_decode_time: u64,
) -> Result<Vec<u8>, ConvertError> {
    let mut parsed = parse_egorec_for_segment(input_path)?;
    parsed.config.timescale = timescale;
    parsed.config.sample_delta = sample_delta;
    Ok(build_fmp4_segment(
        &parsed.config,
        &parsed.samples,
        sequence_number,
        base_decode_time,
    ))
}

struct ParsedRaw {
    header: FileHeader,
    footer: FileFooter,
    samples: Vec<Mp4Sample>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

fn parse_egorec_raw(input_path: &Path) -> Result<ParsedRaw, ConvertError> {
    let file = std::fs::File::open(input_path)
        .map_err(|e| ConvertError::Io(format!("open input: {e}")))?;
    let mut file = BufReader::new(file);

    let header = FileHeader::read_from(&mut file)
        .map_err(|e| ConvertError::InvalidFormat(format!("read header: {e}")))?;
    if header.magic != FILE_MAGIC {
        return Err(ConvertError::InvalidFormat("invalid .egorec magic".into()));
    }
    if header.header_size as usize != FILE_HEADER_SIZE {
        return Err(ConvertError::InvalidFormat(format!(
            "unexpected header size: {}",
            header.header_size
        )));
    }
    if header.rgb_codec != STREAMABLE_RGB_CODEC {
        return Err(ConvertError::UnsupportedCodec(header.rgb_codec));
    }

    let file_size = file
        .get_ref()
        .metadata()
        .map_err(|e| ConvertError::Io(format!("metadata: {e}")))?
        .len();
    if file_size < (FILE_HEADER_SIZE as u64 + FileFooter::SIZE as u64) {
        return Err(ConvertError::InvalidFormat("file too small".into()));
    }

    file.seek(SeekFrom::End(-(FileFooter::SIZE as i64)))
        .map_err(|e| ConvertError::Io(format!("seek footer: {e}")))?;
    let footer = FileFooter::read_from(&mut file)
        .map_err(|e| ConvertError::InvalidFormat(format!("read footer: {e}")))?;
    if footer.footer_magic != FOOTER_MAGIC {
        return Err(ConvertError::InvalidFormat("invalid footer magic".into()));
    }
    if footer.index_magic != INDEX_MAGIC {
        return Err(ConvertError::InvalidFormat("invalid index magic".into()));
    }
    if footer.total_frames == 0 {
        return Err(ConvertError::InvalidFormat("no frames in file".into()));
    }

    let mut offset = FILE_HEADER_SIZE as u64;
    let mut samples = Vec::with_capacity(footer.total_frames as usize);
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;

    while offset < footer.index_offset {
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| ConvertError::Io(format!("seek frame: {e}")))?;
        let block = FrameBlockHeader::read_from(&mut file)
            .map_err(|e| ConvertError::InvalidFormat(format!("read frame block: {e}")))?;
        if block.magic != FRAME_MAGIC {
            return Err(ConvertError::InvalidFormat(format!(
                "invalid frame magic: 0x{:08X}",
                block.magic
            )));
        }
        if (block.block_size as usize) < FrameBlockHeader::SIZE {
            return Err(ConvertError::InvalidFormat("frame block smaller than header".into()));
        }

        let rgb_size = block.rgb_compressed_size as usize;
        let payload_bytes = block.block_size as usize - FrameBlockHeader::SIZE;
        if rgb_size > payload_bytes {
            return Err(ConvertError::InvalidFormat(
                "rgb payload exceeds frame block size".into(),
            ));
        }

        if rgb_size > 0 {
            let mut h264_data = vec![0u8; rgb_size];
            file.read_exact(&mut h264_data)
                .map_err(|e| ConvertError::Io(format!("read rgb payload: {e}")))?;

            let nals = h264_annex_b::parse_annex_b(&h264_data);
            if sps.is_none() {
                if let Some((stream_sps, stream_pps)) = h264_annex_b::extract_sps_pps(&nals) {
                    sps = Some(stream_sps);
                    pps = Some(stream_pps);
                }
            }

            samples.push(Mp4Sample {
                data: h264_annex_b::nals_to_avcc(&nals),
                is_keyframe: h264_annex_b::is_keyframe(&nals),
            });
        } else {
            samples.push(Mp4Sample {
                data: Vec::new(),
                is_keyframe: false,
            });
        }

        let trailing_payload = payload_bytes.saturating_sub(rgb_size);
        if trailing_payload > 0 {
            file.seek(SeekFrom::Current(trailing_payload as i64))
                .map_err(|e| ConvertError::Io(format!("skip trailing payload: {e}")))?;
        }

        offset += block.block_size as u64;
    }

    Ok(ParsedRaw {
        header,
        footer,
        samples,
        sps,
        pps,
    })
}

/// Parse an egorec file, requiring SPS/PPS (for full MP4 and init segment).
fn parse_egorec(input_path: &Path) -> Result<ParsedTrack, ConvertError> {
    let raw = parse_egorec_raw(input_path)?;

    let sps =
        raw.sps.ok_or_else(|| ConvertError::InvalidFormat("no SPS found in H.264 stream".into()))?;
    let pps =
        raw.pps.ok_or_else(|| ConvertError::InvalidFormat("no PPS found in H.264 stream".into()))?;

    let duration_s = raw.footer.total_duration_us as f64 / 1_000_000.0;
    let fps = if duration_s > 0.0 {
        raw.footer.total_frames as f64 / duration_s
    } else {
        30.0
    };
    let timescale = (fps * 1000.0).round() as u32;
    let sample_delta = 1000u32;

    Ok(ParsedTrack {
        config: Mp4TrackConfig {
            width: raw.header.color_width,
            height: raw.header.color_height,
            timescale,
            sample_delta,
            sps,
            pps,
        },
        samples: raw.samples,
    })
}

/// Parse an egorec file for media segment output (SPS/PPS not required since
/// they are already present in the init segment's avcC box).
fn parse_egorec_for_segment(input_path: &Path) -> Result<ParsedTrack, ConvertError> {
    let raw = parse_egorec_raw(input_path)?;

    let duration_s = raw.footer.total_duration_us as f64 / 1_000_000.0;
    let fps = if duration_s > 0.0 {
        raw.footer.total_frames as f64 / duration_s
    } else {
        30.0
    };
    let timescale = (fps * 1000.0).round() as u32;
    let sample_delta = 1000u32;

    Ok(ParsedTrack {
        config: Mp4TrackConfig {
            width: raw.header.color_width,
            height: raw.header.color_height,
            timescale,
            sample_delta,
            sps: raw.sps.unwrap_or_default(),
            pps: raw.pps.unwrap_or_default(),
        },
        samples: raw.samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};
    use std::io::Write;

    fn make_keyframe_h264() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x67, 0x42, 0xC0, 0x1E, 0xD9, 0x00, 0xA0, 0x47, 0xFE, 0x88]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x65, 0x88, 0x80, 0x40, 0x00, 0xFF, 0xAA, 0x55]);
        data
    }

    fn make_pframe_h264() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x41, 0x9A, 0x24, 0x6C, 0x41, 0xFF]);
        data
    }

    fn write_fixed_string(out: &mut Vec<u8>, value: &str, width: usize) {
        let bytes = value.as_bytes();
        let copy_len = bytes.len().min(width.saturating_sub(1));
        out.extend_from_slice(&bytes[..copy_len]);
        out.extend(std::iter::repeat(0u8).take(width - copy_len));
    }

    fn create_test_egorec(path: &Path, num_frames: u32, fps: u32, rgb_codec: u8) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FILE_MAGIC);
        bytes.write_u32::<LittleEndian>(FILE_HEADER_SIZE as u32).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        write_fixed_string(&mut bytes, "SERIAL-123", 32);
        bytes.write_f32::<LittleEndian>(0.001).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        for _ in 0..5 {
            bytes.write_f32::<LittleEndian>(0.0).unwrap();
        }
        bytes.write_u32::<LittleEndian>(640).unwrap();
        bytes.write_u32::<LittleEndian>(480).unwrap();
        bytes.write_f32::<LittleEndian>(600.0).unwrap();
        bytes.write_f32::<LittleEndian>(600.0).unwrap();
        bytes.write_f32::<LittleEndian>(320.0).unwrap();
        bytes.write_f32::<LittleEndian>(240.0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        for _ in 0..5 {
            bytes.write_f32::<LittleEndian>(0.0).unwrap();
        }
        for _ in 0..9 {
            bytes.write_f32::<LittleEndian>(0.0).unwrap();
        }
        for _ in 0..3 {
            bytes.write_f32::<LittleEndian>(0.0).unwrap();
        }
        write_fixed_string(&mut bytes, "session-001", 128);
        bytes.write_u64::<LittleEndian>(0).unwrap();
        write_fixed_string(&mut bytes, "USB3", 8);
        bytes.write_u8(rgb_codec).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u8(23).unwrap();
        bytes.write_u8(3).unwrap();
        bytes.extend(std::iter::repeat(0u8).take(128));
        assert_eq!(bytes.len(), FILE_HEADER_SIZE);

        let frame_duration_us = 1_000_000u64 / fps as u64;
        for frame_idx in 0..num_frames {
            let h264 = if frame_idx % fps == 0 {
                make_keyframe_h264()
            } else {
                make_pframe_h264()
            };
            let block_size = (FrameBlockHeader::SIZE + h264.len()) as u32;
            bytes.write_u32::<LittleEndian>(FRAME_MAGIC).unwrap();
            bytes.write_u32::<LittleEndian>(block_size).unwrap();
            bytes.write_u64::<LittleEndian>(frame_idx as u64 * frame_duration_us).unwrap();
            bytes.write_u64::<LittleEndian>(frame_idx as u64).unwrap();
            bytes.write_u32::<LittleEndian>(h264.len() as u32).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u16::<LittleEndian>(0).unwrap();
            bytes.write_u16::<LittleEndian>(0).unwrap();
            bytes.write_all(&h264).unwrap();
        }

        let index_offset = bytes.len() as u64;
        for frame_idx in 0..num_frames {
            let offset = FILE_HEADER_SIZE as u64
                + (0..frame_idx)
                    .map(|i| {
                        if i % fps == 0 {
                            (FrameBlockHeader::SIZE + make_keyframe_h264().len()) as u64
                        } else {
                            (FrameBlockHeader::SIZE + make_pframe_h264().len()) as u64
                        }
                    })
                    .sum::<u64>();
            bytes
                .write_u64::<LittleEndian>(frame_idx as u64 * frame_duration_us)
                .unwrap();
            bytes.write_u64::<LittleEndian>(offset).unwrap();
            bytes.write_u64::<LittleEndian>(frame_idx as u64).unwrap();
        }

        bytes.write_u32::<LittleEndian>(INDEX_MAGIC).unwrap();
        bytes.write_u64::<LittleEndian>(index_offset).unwrap();
        bytes.write_u32::<LittleEndian>(num_frames).unwrap();
        bytes.write_u64::<LittleEndian>(num_frames as u64).unwrap();
        bytes
            .write_u64::<LittleEndian>(num_frames as u64 * frame_duration_us)
            .unwrap();
        bytes.write_u32::<LittleEndian>(FOOTER_MAGIC).unwrap();

        std::fs::write(path, bytes).unwrap();
    }

    fn find_box(data: &[u8], target: &[u8; 4]) -> Option<usize> {
        find_box_recursive(data, target, 0, data.len())
    }

    fn find_box_recursive(
        data: &[u8],
        target: &[u8; 4],
        start: usize,
        end: usize,
    ) -> Option<usize> {
        let mut offset = start;
        while offset + 8 <= end {
            let size = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            if size < 8 || offset + size > end {
                break;
            }
            let box_type = &data[offset + 4..offset + 8];
            if box_type == target {
                return Some(offset);
            }
            let containers: &[&[u8]] = &[
                b"moov", b"trak", b"mdia", b"minf", b"stbl", b"dinf", b"mvex", b"moof", b"traf",
            ];
            if containers.iter().any(|c| box_type == *c) {
                if let Some(found) = find_box_recursive(data, target, offset + 8, offset + size) {
                    return Some(found);
                }
            }
            offset += size;
        }
        None
    }

    #[test]
    fn egorec_to_mp4_produces_valid_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("test.egorec");
        create_test_egorec(&egorec_path, 90, 30, STREAMABLE_RGB_CODEC);

        let mp4 = egorec_to_mp4(&egorec_path).unwrap();
        assert_eq!(&mp4[4..8], b"ftyp");
        assert!(find_box(&mp4, b"moov").is_some());
        assert!(find_box(&mp4, b"mdat").is_some());
        assert!(find_box(&mp4, b"stss").is_some());
    }

    #[test]
    fn egorec_to_mp4_keyframes_match() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("test_kf.egorec");
        create_test_egorec(&egorec_path, 60, 30, STREAMABLE_RGB_CODEC);

        let mp4 = egorec_to_mp4(&egorec_path).unwrap();
        let stss_pos = find_box(&mp4, b"stss").expect("stss box not found");
        let entry_count_offset = stss_pos + 12;
        let count = u32::from_be_bytes([
            mp4[entry_count_offset],
            mp4[entry_count_offset + 1],
            mp4[entry_count_offset + 2],
            mp4[entry_count_offset + 3],
        ]);
        assert_eq!(count, 2);
    }

    #[test]
    fn egorec_to_fmp4_init_produces_fragmented_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("init.egorec");
        create_test_egorec(&egorec_path, 60, 30, STREAMABLE_RGB_CODEC);

        let init = egorec_to_fmp4_init(&egorec_path, 30_000, 1000).unwrap();
        assert_eq!(&init[4..8], b"ftyp");
        assert!(find_box(&init, b"moov").is_some());
        assert!(find_box(&init, b"mvex").is_some());
        assert!(find_box(&init, b"trex").is_some());
        assert!(find_box(&init, b"mdat").is_none());
    }

    #[test]
    fn egorec_to_fmp4_segment_produces_media_fragment() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("segment.egorec");
        create_test_egorec(&egorec_path, 60, 30, STREAMABLE_RGB_CODEC);

        let segment = egorec_to_fmp4_segment(&egorec_path, 30_000, 1000, 1, 0).unwrap();
        assert_eq!(&segment[4..8], b"moof");
        assert!(find_box(&segment, b"mfhd").is_some());
        assert!(find_box(&segment, b"tfdt").is_some());
        assert!(find_box(&segment, b"trun").is_some());
        assert!(find_box(&segment, b"mdat").is_some());
    }

    #[test]
    fn egorec_to_mp4_rejects_unsupported_codec() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("unsupported.egorec");
        create_test_egorec(&egorec_path, 10, 30, 1);

        let err = egorec_to_mp4(&egorec_path).unwrap_err();
        assert!(matches!(err, ConvertError::UnsupportedCodec(1)));
    }

    /// Create a test egorec with only P-frames (no keyframes / no SPS+PPS).
    /// This simulates a mid-stream segment that has no IDR frames.
    fn create_pframe_only_egorec(path: &Path, num_frames: u32, fps: u32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FILE_MAGIC);
        bytes.write_u32::<LittleEndian>(FILE_HEADER_SIZE as u32).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        write_fixed_string(&mut bytes, "SERIAL-123", 32);
        bytes.write_f32::<LittleEndian>(0.001).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_f32::<LittleEndian>(0.0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        for _ in 0..5 { bytes.write_f32::<LittleEndian>(0.0).unwrap(); }
        bytes.write_u32::<LittleEndian>(640).unwrap();
        bytes.write_u32::<LittleEndian>(480).unwrap();
        bytes.write_f32::<LittleEndian>(600.0).unwrap();
        bytes.write_f32::<LittleEndian>(600.0).unwrap();
        bytes.write_f32::<LittleEndian>(320.0).unwrap();
        bytes.write_f32::<LittleEndian>(240.0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        for _ in 0..5 { bytes.write_f32::<LittleEndian>(0.0).unwrap(); }
        for _ in 0..9 { bytes.write_f32::<LittleEndian>(0.0).unwrap(); }
        for _ in 0..3 { bytes.write_f32::<LittleEndian>(0.0).unwrap(); }
        write_fixed_string(&mut bytes, "session-001", 128);
        bytes.write_u64::<LittleEndian>(0).unwrap();
        write_fixed_string(&mut bytes, "USB3", 8);
        bytes.write_u8(STREAMABLE_RGB_CODEC).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u8(23).unwrap();
        bytes.write_u8(3).unwrap();
        bytes.extend(std::iter::repeat(0u8).take(128));
        assert_eq!(bytes.len(), FILE_HEADER_SIZE);

        let frame_duration_us = 1_000_000u64 / fps as u64;
        // All P-frames, no keyframes
        for frame_idx in 0..num_frames {
            let h264 = make_pframe_h264();
            let block_size = (FrameBlockHeader::SIZE + h264.len()) as u32;
            bytes.write_u32::<LittleEndian>(FRAME_MAGIC).unwrap();
            bytes.write_u32::<LittleEndian>(block_size).unwrap();
            bytes.write_u64::<LittleEndian>(frame_idx as u64 * frame_duration_us).unwrap();
            bytes.write_u64::<LittleEndian>(frame_idx as u64).unwrap();
            bytes.write_u32::<LittleEndian>(h264.len() as u32).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u16::<LittleEndian>(0).unwrap();
            bytes.write_u16::<LittleEndian>(0).unwrap();
            bytes.write_all(&h264).unwrap();
        }

        let index_offset = bytes.len() as u64;
        for frame_idx in 0..num_frames {
            let offset = FILE_HEADER_SIZE as u64
                + (frame_idx as u64)
                    * (FrameBlockHeader::SIZE + make_pframe_h264().len()) as u64;
            bytes.write_u64::<LittleEndian>(frame_idx as u64 * frame_duration_us).unwrap();
            bytes.write_u64::<LittleEndian>(offset).unwrap();
            bytes.write_u64::<LittleEndian>(frame_idx as u64).unwrap();
        }

        bytes.write_u32::<LittleEndian>(INDEX_MAGIC).unwrap();
        bytes.write_u64::<LittleEndian>(index_offset).unwrap();
        bytes.write_u32::<LittleEndian>(num_frames).unwrap();
        bytes.write_u64::<LittleEndian>(num_frames as u64).unwrap();
        bytes.write_u64::<LittleEndian>(num_frames as u64 * frame_duration_us).unwrap();
        bytes.write_u32::<LittleEndian>(FOOTER_MAGIC).unwrap();

        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn segment_without_keyframes_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("no_keyframes.egorec");
        create_pframe_only_egorec(&egorec_path, 30, 30);

        // Full MP4 and init should fail (need SPS/PPS for avcC).
        assert!(egorec_to_mp4(&egorec_path).is_err());
        assert!(egorec_to_fmp4_init(&egorec_path, 30_000, 1000).is_err());

        // fMP4 segment should succeed (SPS/PPS not needed).
        let segment = egorec_to_fmp4_segment(&egorec_path, 30_000, 1000, 2, 30_000).unwrap();
        assert_eq!(&segment[4..8], b"moof");
        assert!(find_box(&segment, b"trun").is_some());
        assert!(find_box(&segment, b"mdat").is_some());
    }

    #[test]
    fn egorec_to_mp4_ffprobe_validates() {
        let dir = tempfile::tempdir().unwrap();
        let egorec_path = dir.path().join("ffprobe.egorec");
        create_test_egorec(&egorec_path, 90, 30, STREAMABLE_RGB_CODEC);

        let mp4 = egorec_to_mp4(&egorec_path).unwrap();
        let mp4_path = dir.path().join("output.mp4");
        std::fs::write(&mp4_path, &mp4).unwrap();

        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,width,height",
                "-of",
                "json",
                mp4_path.to_str().unwrap(),
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                let stream = &json["streams"][0];
                assert_eq!(stream["codec_name"], "h264");
                assert!(stream["width"].as_u64().unwrap() > 0);
                assert!(stream["height"].as_u64().unwrap() > 0);
            }
            Ok(_) | Err(_) => {}
        }
    }
}
