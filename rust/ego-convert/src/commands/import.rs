//! Import MP4 video + depth PNGs into .egorec format.

use anyhow::{bail, Context, Result};
use egorec::format::{FileHeader, FILE_HEADER_SIZE, FILE_MAGIC};
use egorec::h264::H264Encoder;
use egorec::EgorecWriter;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use zdepth_sys::ZdepthCompressor;

const KEYFRAME_INTERVAL: u64 = 30;

// EgoPAT3D Azure Kinect intrinsics scaled from 3840x2160 → 1280x720
const DEFAULT_FX: f32 = 602.73;
const DEFAULT_FY: f32 = 602.65;
const DEFAULT_PPX: f32 = 647.43;
const DEFAULT_PPY: f32 = 374.61;

pub fn run(
    video: &str,
    depth_dir: &str,
    output: &str,
    width: u32,
    height: u32,
    fps: u32,
    session_name: Option<&str>,
    quiet: bool,
) -> Result<()> {
    let depth_path = Path::new(depth_dir);
    if !depth_path.is_dir() {
        bail!("Depth directory not found: {}", depth_dir);
    }

    // Count available depth frames (1-indexed)
    let max_depth_frames = count_depth_frames(depth_path);
    if max_depth_frames == 0 {
        bail!("No depth PNGs found in {}", depth_dir);
    }

    // Build file header
    let name = session_name.unwrap_or("imported");
    let header = build_header(width, height, name);

    // Create writer
    let out_path = Path::new(output);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer =
        EgorecWriter::create(out_path, &header).context("Failed to create output file")?;

    // Open video with ffmpeg demuxer
    ffmpeg_next::init()?;
    let mut ictx = ffmpeg_next::format::input(video).context("Failed to open video")?;
    let video_stream_index = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .context("No video stream found")?
        .index();

    let stream = ictx.stream(video_stream_index).unwrap();
    let codec_params = stream.parameters();
    let decoder_ctx = ffmpeg_next::codec::Context::from_parameters(codec_params)?;
    let mut decoder = decoder_ctx.decoder().video()?;
    let src_width = decoder.width();
    let src_height = decoder.height();

    // Create scaler: source format → RGB24 at target resolution
    let mut scaler = ffmpeg_next::software::scaling::Context::get(
        decoder.format(),
        src_width,
        src_height,
        ffmpeg_next::format::Pixel::RGB24,
        width,
        height,
        ffmpeg_next::software::scaling::Flags::BILINEAR,
    )?;

    // Create H.264 encoder and zdepth compressor
    let mut h264_encoder = H264Encoder::new(width, height, fps, 23)?;
    let mut zdepth_compressor = ZdepthCompressor::new()?;

    let frame_duration_us = 1_000_000u64 / fps as u64;

    // Progress bar
    let pb = if !quiet {
        let pb = ProgressBar::new(max_depth_frames as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} frames ({eta})")
                .unwrap(),
        );
        Some(pb)
    } else {
        None
    };

    let mut frame_number: u64 = 0;
    let mut decoded_frame = ffmpeg_next::frame::Video::empty();
    let mut rgb_frame = ffmpeg_next::frame::Video::empty();

    // Process packets from video
    let mut packets: Vec<(usize, ffmpeg_next::Packet)> = Vec::new();
    for (stream, packet) in ictx.packets() {
        if stream.index() == video_stream_index {
            packets.push((stream.index(), packet));
        }
    }

    for (_idx, packet) in &packets {
        decoder.send_packet(packet)?;

        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            if frame_number as usize >= max_depth_frames {
                break;
            }

            // Scale decoded frame to target resolution RGB24
            scaler.run(&decoded_frame, &mut rgb_frame)?;

            let rgb_data = extract_rgb_data(&rgb_frame, width as usize, height as usize);

            // Load depth PNG (1-indexed)
            let depth_file = depth_path.join(format!("{}.png", frame_number + 1));
            let depth_u16 = load_and_resize_depth(&depth_file, width, height)?;

            // Encode RGB
            let is_keyframe = frame_number % KEYFRAME_INTERVAL == 0;
            let (rgb_nal, _rgb_keyframe) = h264_encoder
                .encode_frame(&rgb_data)?
                .context("H264 encoder produced no packet (zerolatency should always emit)")?;

            // Compress depth
            let depth_compressed = zdepth_compressor
                .compress(&depth_u16, width as i32, height as i32, is_keyframe)?
                .to_vec();

            let timestamp_us = frame_number * frame_duration_us;
            writer.write_frame(
                timestamp_us,
                frame_number,
                is_keyframe,
                &rgb_nal,
                &depth_compressed,
            )?;

            frame_number += 1;
            if let Some(ref pb) = pb {
                pb.set_position(frame_number);
            }
        }

        if frame_number as usize >= max_depth_frames {
            break;
        }
    }

    // Flush decoder for remaining frames
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        if frame_number as usize >= max_depth_frames {
            break;
        }

        scaler.run(&decoded_frame, &mut rgb_frame)?;
        let rgb_data = extract_rgb_data(&rgb_frame, width as usize, height as usize);

        let depth_file = depth_path.join(format!("{}.png", frame_number + 1));
        let depth_u16 = load_and_resize_depth(&depth_file, width, height)?;

        let is_keyframe = frame_number % KEYFRAME_INTERVAL == 0;
        let (rgb_nal, _rgb_keyframe) = h264_encoder
            .encode_frame(&rgb_data)?
            .context("H264 encoder produced no packet")?;

        let depth_compressed = zdepth_compressor
            .compress(&depth_u16, width as i32, height as i32, is_keyframe)?
            .to_vec();

        let timestamp_us = frame_number * frame_duration_us;
        writer.write_frame(
            timestamp_us,
            frame_number,
            is_keyframe,
            &rgb_nal,
            &depth_compressed,
        )?;

        frame_number += 1;
        if let Some(ref pb) = pb {
            pb.set_position(frame_number);
        }
    }

    // Flush H.264 encoder
    let trailing = h264_encoder.flush()?;
    for (nal_data, is_kf) in trailing {
        if frame_number as usize >= max_depth_frames {
            break;
        }

        let depth_file = depth_path.join(format!("{}.png", frame_number + 1));
        if !depth_file.exists() {
            break;
        }
        let depth_u16 = load_and_resize_depth(&depth_file, width, height)?;
        let depth_compressed = zdepth_compressor
            .compress(&depth_u16, width as i32, height as i32, is_kf)?
            .to_vec();

        let timestamp_us = frame_number * frame_duration_us;
        writer.write_frame(timestamp_us, frame_number, is_kf, &nal_data, &depth_compressed)?;

        frame_number += 1;
        if let Some(ref pb) = pb {
            pb.set_position(frame_number);
        }
    }

    if let Some(ref pb) = pb {
        pb.finish_with_message("done");
    }

    writer.finalize()?;

    if !quiet {
        let file_size = std::fs::metadata(output)?.len();
        eprintln!(
            "Wrote {} frames to {} ({:.1} MB)",
            frame_number,
            output,
            file_size as f64 / (1024.0 * 1024.0)
        );
    }

    Ok(())
}

fn build_header(width: u32, height: u32, session_name: &str) -> FileHeader {
    let mut session_bytes = [0u8; 128];
    let name_bytes = session_name.as_bytes();
    let copy_len = name_bytes.len().min(127);
    session_bytes[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // Identity extrinsic rotation (3x3)
    let extrinsic_rotation = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

    FileHeader {
        magic: FILE_MAGIC,
        header_size: FILE_HEADER_SIZE as u32,
        flags: 0, // no IMU
        serial_number: [0u8; 32],
        depth_scale: 0.001,
        depth_width: width,
        depth_height: height,
        depth_fx: DEFAULT_FX,
        depth_fy: DEFAULT_FY,
        depth_ppx: DEFAULT_PPX,
        depth_ppy: DEFAULT_PPY,
        depth_distortion_model: 0,
        depth_distortion_coeffs: [0.0; 5],
        color_width: width,
        color_height: height,
        color_fx: DEFAULT_FX,
        color_fy: DEFAULT_FY,
        color_ppx: DEFAULT_PPX,
        color_ppy: DEFAULT_PPY,
        color_distortion_model: 0,
        color_distortion_coeffs: [0.0; 5],
        extrinsic_rotation,
        extrinsic_translation: [0.0; 3],
        session_name: session_bytes,
        start_timestamp_us: 0,
        usb_type: [0u8; 8],
        rgb_codec: 2,
        depth_codec: 2,
        rgb_quality: 23,
        zstd_level: 0,
        reserved: [0u8; 128],
    }
}

fn count_depth_frames(dir: &Path) -> usize {
    let mut count = 0;
    loop {
        let path = dir.join(format!("{}.png", count + 1));
        if path.exists() {
            count += 1;
        } else {
            break;
        }
    }
    count
}

fn extract_rgb_data(
    frame: &ffmpeg_next::frame::Video,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let stride = frame.stride(0);
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let src_start = y * stride;
        let dst_start = y * width * 3;
        rgb[dst_start..dst_start + width * 3]
            .copy_from_slice(&frame.data(0)[src_start..src_start + width * 3]);
    }
    rgb
}

fn load_and_resize_depth(path: &Path, target_w: u32, target_h: u32) -> Result<Vec<u16>> {
    let img = image::open(path)
        .with_context(|| format!("Failed to load depth PNG: {}", path.display()))?;
    let gray16 = img.into_luma16();
    let (src_w, src_h) = gray16.dimensions();

    if src_w == target_w && src_h == target_h {
        return Ok(gray16.into_raw());
    }

    // Nearest-neighbor resize to preserve depth values
    let mut out = vec![0u16; (target_w * target_h) as usize];
    for y in 0..target_h {
        let sy = (y as f64 * src_h as f64 / target_h as f64) as u32;
        let sy = sy.min(src_h - 1);
        for x in 0..target_w {
            let sx = (x as f64 * src_w as f64 / target_w as f64) as u32;
            let sx = sx.min(src_w - 1);
            out[(y * target_w + x) as usize] = gray16.get_pixel(sx, sy).0[0];
        }
    }
    Ok(out)
}
