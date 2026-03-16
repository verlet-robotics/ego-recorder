/// Minimal MP4/fMP4 muxer for a single H.264 video track.

const TRACK_ID: u32 = 1;
const NON_SYNC_SAMPLE_FLAGS: u32 = 0x0101_0000;
const SYNC_SAMPLE_FLAGS: u32 = 0x0200_0000;

#[derive(Clone)]
pub struct Mp4Sample {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
}

#[derive(Clone)]
pub struct Mp4TrackConfig {
    pub width: u32,
    pub height: u32,
    pub timescale: u32,
    pub sample_delta: u32,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

struct FullSampleTables<'a> {
    sample_sizes: &'a [u32],
    keyframe_indices: &'a [u32],
    chunk_offset: u64,
}

pub fn build_mp4(config: &Mp4TrackConfig, samples: &[Mp4Sample]) -> Vec<u8> {
    let sample_sizes: Vec<u32> = samples.iter().map(|s| s.data.len() as u32).collect();
    let keyframe_indices: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_keyframe)
        .map(|(i, _)| (i + 1) as u32)
        .collect();
    let total_duration = config.sample_delta as u64 * samples.len() as u64;
    let total_mdat_payload: u64 = sample_sizes.iter().map(|&size| size as u64).sum();

    let mut out = Vec::new();
    write_ftyp(&mut out);

    let placeholder_moov = build_moov(
        config,
        total_duration,
        Some(FullSampleTables {
            sample_sizes: &sample_sizes,
            keyframe_indices: &keyframe_indices,
            chunk_offset: 0,
        }),
        false,
    );
    let mdat_header_size = if total_mdat_payload + 8 > u32::MAX as u64 {
        16
    } else {
        8
    };
    let chunk_offset = out.len() as u64 + placeholder_moov.len() as u64 + mdat_header_size as u64;
    let moov = build_moov(
        config,
        total_duration,
        Some(FullSampleTables {
            sample_sizes: &sample_sizes,
            keyframe_indices: &keyframe_indices,
            chunk_offset,
        }),
        false,
    );
    out.extend_from_slice(&moov);
    write_mdat_header(&mut out, total_mdat_payload);
    for sample in samples {
        out.extend_from_slice(&sample.data);
    }
    out
}

pub fn build_fmp4_init(config: &Mp4TrackConfig) -> Vec<u8> {
    let mut out = Vec::new();
    write_ftyp(&mut out);
    out.extend_from_slice(&build_moov(config, 0, None, true));
    out
}

pub fn build_fmp4_segment(
    config: &Mp4TrackConfig,
    samples: &[Mp4Sample],
    sequence_number: u32,
    base_decode_time: u64,
) -> Vec<u8> {
    let total_mdat_payload: u64 = samples.iter().map(|sample| sample.data.len() as u64).sum();
    let mut moof = Vec::new();
    let mut data_offset_pos = None;

    write_box(&mut moof, b"moof", |b| {
        write_full_box(b, b"mfhd", 0, 0, |b| {
            b.extend_from_slice(&sequence_number.to_be_bytes());
        });

        write_box(b, b"traf", |b| {
            write_full_box(b, b"tfhd", 0, 0x020000, |b| {
                b.extend_from_slice(&TRACK_ID.to_be_bytes());
            });

            write_full_box(b, b"tfdt", 1, 0, |b| {
                b.extend_from_slice(&base_decode_time.to_be_bytes());
            });

            write_full_box(b, b"trun", 0, 0x000701, |b| {
                b.extend_from_slice(&(samples.len() as u32).to_be_bytes());
                let offset_pos = b.len();
                b.extend_from_slice(&0i32.to_be_bytes());
                data_offset_pos = Some(offset_pos);

                for sample in samples {
                    b.extend_from_slice(&config.sample_delta.to_be_bytes());
                    b.extend_from_slice(&(sample.data.len() as u32).to_be_bytes());
                    b.extend_from_slice(&sample_flags(sample).to_be_bytes());
                }
            });
        });
    });

    let data_offset = (moof.len() + 8) as i32;
    if let Some(offset_pos) = data_offset_pos {
        moof[offset_pos..offset_pos + 4].copy_from_slice(&data_offset.to_be_bytes());
    }

    let mut out = Vec::with_capacity(moof.len() + 8 + total_mdat_payload as usize);
    out.extend_from_slice(&moof);
    write_mdat_header(&mut out, total_mdat_payload);
    for sample in samples {
        out.extend_from_slice(&sample.data);
    }
    out
}

fn build_moov(
    config: &Mp4TrackConfig,
    total_duration: u64,
    sample_tables: Option<FullSampleTables<'_>>,
    fragmented: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    write_box(&mut out, b"moov", |b| {
        write_mvhd(b, config.timescale, total_duration);

        write_box(b, b"trak", |b| {
            write_tkhd(b, total_duration, config.width, config.height);

            write_box(b, b"mdia", |b| {
                write_mdhd(b, config.timescale, total_duration);
                write_hdlr(b);

                write_box(b, b"minf", |b| {
                    write_vmhd(b);
                    write_dinf(b);

                    write_box(b, b"stbl", |b| {
                        write_stsd(b, config);
                        match sample_tables {
                            Some(FullSampleTables {
                                sample_sizes,
                                keyframe_indices,
                                chunk_offset,
                            }) => {
                                write_stts(b, sample_sizes.len() as u32, config.sample_delta);
                                write_stsc(b, sample_sizes.len() as u32);
                                write_stsz(b, sample_sizes);
                                write_co64(b, chunk_offset);
                                if !keyframe_indices.is_empty() {
                                    write_stss(b, keyframe_indices);
                                }
                            }
                            None => {
                                write_empty_stts(b);
                                write_empty_stsc(b);
                                write_empty_stsz(b);
                                write_empty_stco(b);
                            }
                        }
                    });
                });
            });
        });

        if fragmented {
            write_box(b, b"mvex", |b| {
                write_full_box(b, b"trex", 0, 0, |b| {
                    b.extend_from_slice(&TRACK_ID.to_be_bytes());
                    b.extend_from_slice(&1u32.to_be_bytes());
                    b.extend_from_slice(&config.sample_delta.to_be_bytes());
                    b.extend_from_slice(&0u32.to_be_bytes());
                    b.extend_from_slice(&NON_SYNC_SAMPLE_FLAGS.to_be_bytes());
                });
            });
        }
    });
    out
}

fn write_mdat_header(buf: &mut Vec<u8>, payload_size: u64) {
    if payload_size + 8 > u32::MAX as u64 {
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.extend_from_slice(b"mdat");
        buf.extend_from_slice(&(payload_size + 16).to_be_bytes());
    } else {
        buf.extend_from_slice(&((payload_size as u32 + 8).to_be_bytes()));
        buf.extend_from_slice(b"mdat");
    }
}

fn sample_flags(sample: &Mp4Sample) -> u32 {
    if sample.is_keyframe {
        SYNC_SAMPLE_FLAGS
    } else {
        NON_SYNC_SAMPLE_FLAGS
    }
}

fn write_ftyp(buf: &mut Vec<u8>) {
    write_box(buf, b"ftyp", |b| {
        b.extend_from_slice(b"isom");
        b.extend_from_slice(&0x200u32.to_be_bytes());
        for brand in [b"isom", b"iso6", b"avc1", b"mp41"] {
            b.extend_from_slice(brand);
        }
    });
}

fn write_mvhd(buf: &mut Vec<u8>, timescale: u32, duration: u64) {
    write_full_box(buf, b"mvhd", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&timescale.to_be_bytes());
        b.extend_from_slice(&(duration.min(u32::MAX as u64) as u32).to_be_bytes());
        b.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        b.extend_from_slice(&0x0100u16.to_be_bytes());
        b.extend_from_slice(&[0u8; 10]);
        write_identity_matrix(b);
        b.extend_from_slice(&[0u8; 24]);
        b.extend_from_slice(&2u32.to_be_bytes());
    });
}

fn write_tkhd(buf: &mut Vec<u8>, duration: u64, width: u32, height: u32) {
    write_full_box(buf, b"tkhd", 0, 3, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&TRACK_ID.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&(duration.min(u32::MAX as u64) as u32).to_be_bytes());
        b.extend_from_slice(&[0u8; 8]);
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        write_identity_matrix(b);
        b.extend_from_slice(&(width << 16).to_be_bytes());
        b.extend_from_slice(&(height << 16).to_be_bytes());
    });
}

fn write_mdhd(buf: &mut Vec<u8>, timescale: u32, duration: u64) {
    write_full_box(buf, b"mdhd", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&timescale.to_be_bytes());
        b.extend_from_slice(&(duration.min(u32::MAX as u64) as u32).to_be_bytes());
        b.extend_from_slice(&0x55C4u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
    });
}

fn write_hdlr(buf: &mut Vec<u8>) {
    write_full_box(buf, b"hdlr", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"vide");
        b.extend_from_slice(&[0u8; 12]);
        b.extend_from_slice(b"VideoHandler\0");
    });
}

fn write_vmhd(buf: &mut Vec<u8>) {
    write_full_box(buf, b"vmhd", 0, 1, |b| {
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&[0u8; 6]);
    });
}

fn write_dinf(buf: &mut Vec<u8>) {
    write_box(buf, b"dinf", |b| {
        write_full_box(b, b"dref", 0, 0, |b| {
            b.extend_from_slice(&1u32.to_be_bytes());
            write_full_box(b, b"url ", 0, 1, |_| {});
        });
    });
}

fn write_stsd(buf: &mut Vec<u8>, config: &Mp4TrackConfig) {
    write_full_box(buf, b"stsd", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes());

        write_box(b, b"avc1", |b| {
            b.extend_from_slice(&[0u8; 6]);
            b.extend_from_slice(&1u16.to_be_bytes());
            b.extend_from_slice(&0u16.to_be_bytes());
            b.extend_from_slice(&0u16.to_be_bytes());
            b.extend_from_slice(&[0u8; 12]);
            b.extend_from_slice(&(config.width as u16).to_be_bytes());
            b.extend_from_slice(&(config.height as u16).to_be_bytes());
            b.extend_from_slice(&0x0048_0000u32.to_be_bytes());
            b.extend_from_slice(&0x0048_0000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&1u16.to_be_bytes());
            b.extend_from_slice(&[0u8; 32]);
            b.extend_from_slice(&0x0018u16.to_be_bytes());
            b.extend_from_slice(&0xFFFFu16.to_be_bytes());

            write_box(b, b"avcC", |b| {
                b.push(1);
                b.push(config.sps.get(1).copied().unwrap_or(0x42));
                b.push(config.sps.get(2).copied().unwrap_or(0x00));
                b.push(config.sps.get(3).copied().unwrap_or(0x1E));
                b.push(0xFF);
                b.push(0xE1);
                b.extend_from_slice(&(config.sps.len() as u16).to_be_bytes());
                b.extend_from_slice(&config.sps);
                b.push(1);
                b.extend_from_slice(&(config.pps.len() as u16).to_be_bytes());
                b.extend_from_slice(&config.pps);
            });
        });
    });
}

fn write_stts(buf: &mut Vec<u8>, sample_count: u32, sample_delta: u32) {
    write_full_box(buf, b"stts", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&sample_count.to_be_bytes());
        b.extend_from_slice(&sample_delta.to_be_bytes());
    });
}

fn write_empty_stts(buf: &mut Vec<u8>) {
    write_full_box(buf, b"stts", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
    });
}

fn write_stsc(buf: &mut Vec<u8>, sample_count: u32) {
    write_full_box(buf, b"stsc", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&sample_count.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
    });
}

fn write_empty_stsc(buf: &mut Vec<u8>) {
    write_full_box(buf, b"stsc", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
    });
}

fn write_stsz(buf: &mut Vec<u8>, sample_sizes: &[u32]) {
    write_full_box(buf, b"stsz", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        for size in sample_sizes {
            b.extend_from_slice(&size.to_be_bytes());
        }
    });
}

fn write_empty_stsz(buf: &mut Vec<u8>) {
    write_full_box(buf, b"stsz", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
    });
}

fn write_co64(buf: &mut Vec<u8>, offset: u64) {
    write_full_box(buf, b"co64", 0, 0, |b| {
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&offset.to_be_bytes());
    });
}

fn write_empty_stco(buf: &mut Vec<u8>) {
    write_full_box(buf, b"stco", 0, 0, |b| {
        b.extend_from_slice(&0u32.to_be_bytes());
    });
}

fn write_stss(buf: &mut Vec<u8>, keyframe_indices: &[u32]) {
    write_full_box(buf, b"stss", 0, 0, |b| {
        b.extend_from_slice(&(keyframe_indices.len() as u32).to_be_bytes());
        for index in keyframe_indices {
            b.extend_from_slice(&index.to_be_bytes());
        }
    });
}

fn write_box(buf: &mut Vec<u8>, box_type: &[u8; 4], content_fn: impl FnOnce(&mut Vec<u8>)) {
    let size_pos = buf.len();
    buf.extend_from_slice(&0u32.to_be_bytes());
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
        let vf = ((version as u32) << 24) | (flags & 0x00FF_FFFF);
        b.extend_from_slice(&vf.to_be_bytes());
        content_fn(b);
    });
}

fn write_identity_matrix(buf: &mut Vec<u8>) {
    for &val in &[
        0x0001_0000u32,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
    ] {
        buf.extend_from_slice(&val.to_be_bytes());
    }
}
