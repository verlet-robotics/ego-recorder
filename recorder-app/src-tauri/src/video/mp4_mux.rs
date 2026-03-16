/// Minimal MP4 (ISO BMFF) muxer for a single H.264 video track.
///
/// Produces a moov-first MP4 file from pre-converted AVCC samples,
/// enabling instant browser seeking via HTTP Range requests.

/// One AVCC-formatted video sample (one frame).
pub struct Mp4Sample {
    /// AVCC-formatted data (4-byte BE length prefix per NAL unit).
    pub data: Vec<u8>,
    pub is_keyframe: bool,
}

/// Track configuration for the H.264 video track.
pub struct Mp4TrackConfig {
    pub width: u32,
    pub height: u32,
    /// Timescale in ticks per second (e.g. 30000 for 30fps with delta=1000).
    pub timescale: u32,
    /// Duration of each sample in timescale units (e.g. 1000 for 30fps @ 30000 ts).
    pub sample_delta: u32,
    /// Raw SPS NAL bytes (including NAL type byte).
    pub sps: Vec<u8>,
    /// Raw PPS NAL bytes (including NAL type byte).
    pub pps: Vec<u8>,
}

/// Build a complete MP4 file in memory.
pub fn build_mp4(config: &Mp4TrackConfig, samples: &[Mp4Sample]) -> Vec<u8> {
    let n = samples.len();
    let keyframe_indices: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_keyframe)
        .map(|(i, _)| (i + 1) as u32) // 1-indexed per ISO spec
        .collect();
    let sample_sizes: Vec<u32> = samples.iter().map(|s| s.data.len() as u32).collect();
    let total_mdat_payload: u64 = sample_sizes.iter().map(|&s| s as u64).sum();
    let total_duration = config.sample_delta as u64 * n as u64;
    // Use 64-bit version boxes if duration overflows u32
    let use_v1 = total_duration > u32::MAX as u64;

    // Pre-compute box sizes bottom-up
    let avcc_size = 8 + 11 + config.sps.len() + config.pps.len();
    let avc1_size = 8 + 78 + avcc_size;
    let stsd_size = 8 + 4 + 4 + avc1_size; // fullbox(8) + version/flags(4) + entry_count(4) + avc1
    let stts_size = 8 + 4 + 4 + 8; // fullbox + entry_count(4) + 1 entry(8)
    let stsc_size = 8 + 4 + 4 + 12; // fullbox + entry_count(4) + 1 entry(12)
    let stsz_size = 8 + 4 + 4 + 4 + 4 * n; // fullbox + sample_size(4) + count(4) + entries
    let co64_size = 8 + 4 + 4 + 8; // fullbox + entry_count(4) + 1 entry(8)
    let stss_size = 8 + 4 + 4 + 4 * keyframe_indices.len(); // fullbox + count(4) + entries
    let stbl_size = 8 + stsd_size + stts_size + stsc_size + stsz_size + co64_size + stss_size;
    let dinf_size = 8 + 8 + 4 + 4 + 12; // dinf(8) + dref fullbox(8+4+4) + url(12)
    let vmhd_size = 8 + 4 + 8; // fullbox + graphicsmode(2) + opcolor(6)
    let minf_size = 8 + vmhd_size + dinf_size + stbl_size;
    let hdlr_size = 8 + 4 + 4 + 4 + 12 + 13; // fullbox + pre_defined + handler + reserved + "VideoHandler\0"
    // mdhd: version 0 = 24 bytes payload, version 1 = 36 bytes payload
    let mdhd_size = if use_v1 { 8 + 4 + 8 + 8 + 4 + 8 + 4 } else { 8 + 4 + 4 + 4 + 4 + 4 + 4 };
    let mdia_size = 8 + mdhd_size + hdlr_size + minf_size;
    // tkhd: version 0 = 80 bytes payload, version 1 = 92 bytes payload
    let tkhd_size = if use_v1 { 8 + 4 + 8 + 8 + 4 + 4 + 8 + 8 + 2 + 2 + 2 + 2 + 36 + 4 + 4 } else { 8 + 4 + 4 + 4 + 4 + 4 + 4 + 8 + 2 + 2 + 2 + 2 + 36 + 4 + 4 };
    let trak_size = 8 + tkhd_size + mdia_size;
    // mvhd: version 0 = 96 bytes payload, version 1 = 108 bytes payload
    let mvhd_size = if use_v1 { 8 + 4 + 8 + 8 + 4 + 8 + 4 + 2 + 10 + 36 + 24 + 4 } else { 8 + 4 + 4 + 4 + 4 + 4 + 4 + 2 + 10 + 36 + 24 + 4 };
    let moov_size = 8 + mvhd_size + trak_size;
    let ftyp_size = 24;
    let mdat_header_size: u64 = if total_mdat_payload + 8 > u32::MAX as u64 { 16 } else { 8 };
    let mdat_data_offset: u64 = ftyp_size as u64 + moov_size as u64 + mdat_header_size;

    let total_file_size = ftyp_size as u64 + moov_size as u64 + mdat_header_size + total_mdat_payload;
    let mut buf = Vec::with_capacity(total_file_size as usize);

    // -- ftyp --
    write_box(&mut buf, b"ftyp", |b| {
        b.extend_from_slice(b"isom");       // major_brand
        b.extend_from_slice(&0x200u32.to_be_bytes()); // minor_version
        b.extend_from_slice(b"isom");       // compatible brand
        b.extend_from_slice(b"avc1");       // compatible brand
    });
    debug_assert_eq!(buf.len(), ftyp_size);

    // -- moov --
    write_box(&mut buf, b"moov", |b| {
        // mvhd (version 0 or 1 for 64-bit duration)
        let mvhd_ver: u8 = if use_v1 { 1 } else { 0 };
        write_full_box(b, b"mvhd", mvhd_ver, 0, |b| {
            if use_v1 {
                b.extend_from_slice(&0u64.to_be_bytes()); // creation_time
                b.extend_from_slice(&0u64.to_be_bytes()); // modification_time
                b.extend_from_slice(&config.timescale.to_be_bytes()); // timescale
                b.extend_from_slice(&total_duration.to_be_bytes()); // duration (64-bit)
            } else {
                b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
                b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
                b.extend_from_slice(&config.timescale.to_be_bytes()); // timescale
                b.extend_from_slice(&(total_duration as u32).to_be_bytes()); // duration
            }
            b.extend_from_slice(&0x00010000u32.to_be_bytes()); // rate = 1.0
            b.extend_from_slice(&0x0100u16.to_be_bytes()); // volume = 1.0
            b.extend_from_slice(&[0u8; 10]); // reserved
            write_identity_matrix(b);
            b.extend_from_slice(&[0u8; 24]); // pre_defined
            b.extend_from_slice(&2u32.to_be_bytes()); // next_track_ID
        });

        // trak
        write_box(b, b"trak", |b| {
            // tkhd (version 0 or 1, flags = track_enabled | track_in_movie)
            write_full_box(b, b"tkhd", mvhd_ver, 3, |b| {
                if use_v1 {
                    b.extend_from_slice(&0u64.to_be_bytes()); // creation_time
                    b.extend_from_slice(&0u64.to_be_bytes()); // modification_time
                    b.extend_from_slice(&1u32.to_be_bytes()); // track_ID
                    b.extend_from_slice(&0u32.to_be_bytes()); // reserved
                    b.extend_from_slice(&total_duration.to_be_bytes()); // duration (64-bit)
                } else {
                    b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
                    b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
                    b.extend_from_slice(&1u32.to_be_bytes()); // track_ID
                    b.extend_from_slice(&0u32.to_be_bytes()); // reserved
                    b.extend_from_slice(&(total_duration as u32).to_be_bytes()); // duration
                }
                b.extend_from_slice(&[0u8; 8]); // reserved
                b.extend_from_slice(&0u16.to_be_bytes()); // layer
                b.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
                b.extend_from_slice(&0u16.to_be_bytes()); // volume (0 for video)
                b.extend_from_slice(&0u16.to_be_bytes()); // reserved
                write_identity_matrix(b);
                // Width/height as 16.16 fixed point, clamped to u16 range
                let w = config.width.min(0xFFFF);
                let h = config.height.min(0xFFFF);
                b.extend_from_slice(&(w << 16).to_be_bytes()); // width 16.16
                b.extend_from_slice(&(h << 16).to_be_bytes()); // height 16.16
            });

            // mdia
            write_box(b, b"mdia", |b| {
                // mdhd (version 0 or 1 for 64-bit duration)
                write_full_box(b, b"mdhd", mvhd_ver, 0, |b| {
                    if use_v1 {
                        b.extend_from_slice(&0u64.to_be_bytes()); // creation_time
                        b.extend_from_slice(&0u64.to_be_bytes()); // modification_time
                        b.extend_from_slice(&config.timescale.to_be_bytes()); // timescale
                        b.extend_from_slice(&total_duration.to_be_bytes()); // duration (64-bit)
                    } else {
                        b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
                        b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
                        b.extend_from_slice(&config.timescale.to_be_bytes()); // timescale
                        b.extend_from_slice(&(total_duration as u32).to_be_bytes()); // duration
                    }
                    b.extend_from_slice(&0x55C4u16.to_be_bytes()); // language = 'und'
                    b.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
                });

                // hdlr
                write_full_box(b, b"hdlr", 0, 0, |b| {
                    b.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
                    b.extend_from_slice(b"vide"); // handler_type
                    b.extend_from_slice(&[0u8; 12]); // reserved
                    b.extend_from_slice(b"VideoHandler\0"); // name (null-terminated)
                });

                // minf
                write_box(b, b"minf", |b| {
                    // vmhd (flags=1 required)
                    write_full_box(b, b"vmhd", 0, 1, |b| {
                        b.extend_from_slice(&0u16.to_be_bytes()); // graphicsmode
                        b.extend_from_slice(&[0u8; 6]); // opcolor
                    });

                    // dinf
                    write_box(b, b"dinf", |b| {
                        write_full_box(b, b"dref", 0, 0, |b| {
                            b.extend_from_slice(&1u32.to_be_bytes()); // entry_count
                            // url (self-contained, flags=1)
                            write_full_box(b, b"url ", 0, 1, |_| {});
                        });
                    });

                    // stbl
                    write_box(b, b"stbl", |b| {
                        write_stsd(b, config);
                        write_stts(b, n as u32, config.sample_delta);
                        write_stsc(b, n as u32);
                        write_stsz(b, &sample_sizes);
                        write_co64(b, mdat_data_offset);
                        write_stss(b, &keyframe_indices);
                    });
                });
            });
        });
    });
    debug_assert_eq!(buf.len(), ftyp_size + moov_size);

    // -- mdat --
    if total_mdat_payload + 8 > u32::MAX as u64 {
        // Extended size for >4GB
        buf.extend_from_slice(&1u32.to_be_bytes()); // size=1 signals extended
        buf.extend_from_slice(b"mdat");
        buf.extend_from_slice(&(total_mdat_payload + 16).to_be_bytes());
    } else {
        buf.extend_from_slice(&((total_mdat_payload as u32 + 8).to_be_bytes()));
        buf.extend_from_slice(b"mdat");
    }

    for sample in samples {
        buf.extend_from_slice(&sample.data);
    }

    debug_assert_eq!(buf.len() as u64, total_file_size);
    buf
}

// -- Box writing helpers --

fn write_box(buf: &mut Vec<u8>, box_type: &[u8; 4], content_fn: impl FnOnce(&mut Vec<u8>)) {
    let size_pos = buf.len();
    buf.extend_from_slice(&0u32.to_be_bytes()); // placeholder
    buf.extend_from_slice(box_type);
    content_fn(buf);
    let box_size = (buf.len() - size_pos) as u32;
    buf[size_pos..size_pos + 4].copy_from_slice(&box_size.to_be_bytes());
}

fn write_full_box(
    buf: &mut Vec<u8>,
    box_type: &[u8; 4],
    version: u8,
    flags: u32,
    content_fn: impl FnOnce(&mut Vec<u8>),
) {
    write_box(buf, box_type, |b| {
        let vf = ((version as u32) << 24) | (flags & 0x00FFFFFF);
        b.extend_from_slice(&vf.to_be_bytes());
        content_fn(b);
    });
}

fn write_identity_matrix(buf: &mut Vec<u8>) {
    // Unity matrix in 16.16 / 2.30 fixed point (ISO 14496-12 sec 6.2.2)
    for &val in &[
        0x00010000u32, 0, 0,
        0, 0x00010000, 0,
        0, 0, 0x40000000,
    ] {
        buf.extend_from_slice(&val.to_be_bytes());
    }
}

// -- Sample table boxes --

fn write_stsd(buf: &mut Vec<u8>, config: &Mp4TrackConfig) {
    write_full_box(buf, b"stsd", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes()); // entry_count

        // avc1 box (VisualSampleEntry)
        write_box(b, b"avc1", |b| {
            b.extend_from_slice(&[0u8; 6]); // reserved
            b.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
            b.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
            b.extend_from_slice(&0u16.to_be_bytes()); // reserved
            b.extend_from_slice(&[0u8; 12]); // pre_defined
            b.extend_from_slice(&(config.width as u16).to_be_bytes());
            b.extend_from_slice(&(config.height as u16).to_be_bytes());
            b.extend_from_slice(&0x00480000u32.to_be_bytes()); // horizresolution 72 dpi
            b.extend_from_slice(&0x00480000u32.to_be_bytes()); // vertresolution 72 dpi
            b.extend_from_slice(&0u32.to_be_bytes()); // reserved
            b.extend_from_slice(&1u16.to_be_bytes()); // frame_count
            b.extend_from_slice(&[0u8; 32]); // compressorname
            b.extend_from_slice(&0x0018u16.to_be_bytes()); // depth = 24
            b.extend_from_slice(&0xFFFFu16.to_be_bytes()); // pre_defined = -1

            // avcC (AVCDecoderConfigurationRecord)
            write_box(b, b"avcC", |b| {
                b.push(1); // configurationVersion
                b.push(config.sps.get(1).copied().unwrap_or(0x42)); // AVCProfileIndication
                b.push(config.sps.get(2).copied().unwrap_or(0x00)); // profile_compatibility
                b.push(config.sps.get(3).copied().unwrap_or(0x1E)); // AVCLevelIndication
                b.push(0xFF); // lengthSizeMinusOne = 3, top 6 bits reserved = 1
                b.push(0xE1); // numSPS = 1, top 3 bits reserved = 1
                // SPS/PPS lengths must fit in u16 per ISO 14496-15
                let sps_len = config.sps.len().min(u16::MAX as usize) as u16;
                let pps_len = config.pps.len().min(u16::MAX as usize) as u16;
                b.extend_from_slice(&sps_len.to_be_bytes());
                b.extend_from_slice(&config.sps[..sps_len as usize]);
                b.push(1); // numPPS = 1
                b.extend_from_slice(&pps_len.to_be_bytes());
                b.extend_from_slice(&config.pps[..pps_len as usize]);
            });
        });
    });
}

/// stts: one entry with constant sample_delta.
fn write_stts(buf: &mut Vec<u8>, sample_count: u32, sample_delta: u32) {
    write_full_box(buf, b"stts", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes()); // entry_count
        b.extend_from_slice(&sample_count.to_be_bytes());
        b.extend_from_slice(&sample_delta.to_be_bytes());
    });
}

/// stsc: all samples in a single chunk.
fn write_stsc(buf: &mut Vec<u8>, sample_count: u32) {
    write_full_box(buf, b"stsc", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes()); // entry_count
        b.extend_from_slice(&1u32.to_be_bytes()); // first_chunk
        b.extend_from_slice(&sample_count.to_be_bytes()); // samples_per_chunk
        b.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index
    });
}

/// stsz: per-sample sizes (variable size mode).
fn write_stsz(buf: &mut Vec<u8>, sizes: &[u32]) {
    write_full_box(buf, b"stsz", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0 (variable)
        b.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        for &sz in sizes {
            b.extend_from_slice(&sz.to_be_bytes());
        }
    });
}

/// co64: single chunk offset (64-bit).
fn write_co64(buf: &mut Vec<u8>, chunk_offset: u64) {
    write_full_box(buf, b"co64", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes()); // entry_count
        b.extend_from_slice(&chunk_offset.to_be_bytes());
    });
}

/// stss: sync sample (keyframe) table.
fn write_stss(buf: &mut Vec<u8>, keyframe_indices: &[u32]) {
    write_full_box(buf, b"stss", 0, 0, |b| {
        b.extend_from_slice(&(keyframe_indices.len() as u32).to_be_bytes());
        for &idx in keyframe_indices {
            b.extend_from_slice(&idx.to_be_bytes());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> Mp4TrackConfig {
        Mp4TrackConfig {
            width: 640,
            height: 480,
            timescale: 30000,
            sample_delta: 1000,
            sps: vec![0x67, 0x42, 0x00, 0x1E, 0xAB],
            pps: vec![0x68, 0xCE, 0x38, 0x80],
        }
    }

    fn make_test_samples() -> Vec<Mp4Sample> {
        vec![
            Mp4Sample { data: vec![0x00, 0x00, 0x00, 0x05, 0x65, 0xAA, 0xBB, 0xCC, 0xDD], is_keyframe: true },
            Mp4Sample { data: vec![0x00, 0x00, 0x00, 0x03, 0x41, 0x9A, 0x24], is_keyframe: false },
            Mp4Sample { data: vec![0x00, 0x00, 0x00, 0x03, 0x41, 0x9A, 0x25], is_keyframe: false },
        ]
    }

    #[test]
    fn mp4_starts_with_ftyp() {
        let config = make_test_config();
        let samples = make_test_samples();
        let mp4 = build_mp4(&config, &samples);

        assert!(mp4.len() > 8);
        assert_eq!(&mp4[4..8], b"ftyp");
    }

    #[test]
    fn mp4_has_moov_after_ftyp() {
        let config = make_test_config();
        let samples = make_test_samples();
        let mp4 = build_mp4(&config, &samples);

        let ftyp_size = u32::from_be_bytes([mp4[0], mp4[1], mp4[2], mp4[3]]) as usize;
        assert_eq!(&mp4[ftyp_size + 4..ftyp_size + 8], b"moov");
    }

    #[test]
    fn mp4_has_mdat_after_moov() {
        let config = make_test_config();
        let samples = make_test_samples();
        let mp4 = build_mp4(&config, &samples);

        let ftyp_size = u32::from_be_bytes([mp4[0], mp4[1], mp4[2], mp4[3]]) as usize;
        let moov_size = u32::from_be_bytes([
            mp4[ftyp_size], mp4[ftyp_size + 1], mp4[ftyp_size + 2], mp4[ftyp_size + 3],
        ]) as usize;
        let mdat_offset = ftyp_size + moov_size;
        assert_eq!(&mp4[mdat_offset + 4..mdat_offset + 8], b"mdat");
    }

    #[test]
    fn mp4_mdat_contains_sample_data() {
        let config = make_test_config();
        let samples = make_test_samples();
        let total_sample_bytes: usize = samples.iter().map(|s| s.data.len()).sum();
        let mp4 = build_mp4(&config, &samples);

        let ftyp_size = u32::from_be_bytes([mp4[0], mp4[1], mp4[2], mp4[3]]) as usize;
        let moov_size = u32::from_be_bytes([
            mp4[ftyp_size], mp4[ftyp_size + 1], mp4[ftyp_size + 2], mp4[ftyp_size + 3],
        ]) as usize;
        let mdat_offset = ftyp_size + moov_size;
        let mdat_size = u32::from_be_bytes([
            mp4[mdat_offset], mp4[mdat_offset + 1], mp4[mdat_offset + 2], mp4[mdat_offset + 3],
        ]) as usize;

        assert_eq!(mdat_size, 8 + total_sample_bytes);
        assert_eq!(mp4.len(), mdat_offset + mdat_size);
    }

    #[test]
    fn box_sizes_are_self_consistent() {
        let config = make_test_config();
        let samples = make_test_samples();
        let mp4 = build_mp4(&config, &samples);

        // Walk top-level boxes and verify sizes sum to total
        let mut offset = 0;
        let mut box_count = 0;
        while offset < mp4.len() {
            let size = u32::from_be_bytes([
                mp4[offset], mp4[offset + 1], mp4[offset + 2], mp4[offset + 3],
            ]) as usize;
            assert!(size >= 8, "Box at offset {} has invalid size {}", offset, size);
            assert!(offset + size <= mp4.len(), "Box at offset {} overflows file", offset);
            offset += size;
            box_count += 1;
        }
        assert_eq!(offset, mp4.len());
        assert_eq!(box_count, 3); // ftyp + moov + mdat
    }

    #[test]
    fn empty_samples_produces_valid_mp4() {
        let config = make_test_config();
        let mp4 = build_mp4(&config, &[]);
        assert_eq!(&mp4[4..8], b"ftyp");
        assert!(mp4.len() > 24);
    }
}
