use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

// ── Turbo colormap ───────────────────────────────────────────────────────────

fn turbo_colormap(t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);

    let r = (34.61
        + t * (1172.33 + t * (-10793.56 + t * (33300.12 + t * (-38394.49 + t * 14825.05)))))
        .clamp(0.0, 255.0);
    let g = (23.31
        + t * (557.33 + t * (1225.33 + t * (-8574.70 + t * (12168.53 + t * (-5765.44))))))
        .clamp(0.0, 255.0);
    let b = (27.2
        + t * (3211.1 + t * (-15327.97 + t * (27814.0 + t * (-22569.18 + t * 6838.66)))))
        .clamp(0.0, 255.0);

    (r as u8, g as u8, b as u8)
}

fn depth_to_rgb(depth: &[u16], width: u32, height: u32, max_depth_mm: u16) -> Vec<u8> {
    let npixels = (width * height) as usize;
    let mut rgb = vec![0u8; npixels * 3];
    let inv_max = if max_depth_mm > 0 {
        1.0 / max_depth_mm as f32
    } else {
        0.0
    };

    for i in 0..npixels {
        let d = if i < depth.len() { depth[i] } else { 0 };
        if d == 0 {
            rgb[i * 3] = 30;
            rgb[i * 3 + 1] = 30;
            rgb[i * 3 + 2] = 30;
        } else {
            let t = (d.min(max_depth_mm) as f32) * inv_max;
            let (r, g, b) = turbo_colormap(t);
            rgb[i * 3] = r;
            rgb[i * 3 + 1] = g;
            rgb[i * 3 + 2] = b;
        }
    }

    rgb
}

// ── Inline MP4 writer (ffmpeg-next, no lerobot-writer/arrow/parquet) ─────────

struct Mp4Writer {
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
    fn new(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self> {
        ffmpeg_next::init()?;

        let mut octx = ffmpeg_next::format::output(path)?;

        let codec = ffmpeg_next::codec::encoder::find(ffmpeg_next::codec::Id::H264)
            .ok_or_else(|| anyhow::anyhow!("H.264 encoder not found"))?;

        let mut stream = octx.add_stream(codec)?;
        let stream_index = stream.index();

        let mut encoder = ffmpeg_next::codec::Context::new_with_codec(codec)
            .encoder()
            .video()?;

        let time_base = ffmpeg_next::Rational::new(1, fps as i32);
        encoder.set_width(width);
        encoder.set_height(height);
        encoder.set_format(ffmpeg_next::format::Pixel::YUV420P);
        encoder.set_time_base(time_base);
        encoder.set_frame_rate(Some(ffmpeg_next::Rational::new(fps as i32, 1)));

        let mut opts = ffmpeg_next::Dictionary::new();
        opts.set("crf", "23");
        opts.set("preset", "medium");

        let encoder = encoder.open_with(opts)?;
        stream.set_parameters(&encoder);
        octx.write_header()?;

        let scaler = ffmpeg_next::software::scaling::Context::get(
            ffmpeg_next::format::Pixel::RGB24,
            width,
            height,
            ffmpeg_next::format::Pixel::YUV420P,
            width,
            height,
            ffmpeg_next::software::scaling::Flags::BILINEAR,
        )?;

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

    fn add_frame(&mut self, rgb: &[u8]) -> Result<()> {
        let mut rgb_frame = ffmpeg_next::frame::Video::new(
            ffmpeg_next::format::Pixel::RGB24,
            self.width,
            self.height,
        );

        let stride = rgb_frame.stride(0);
        let w3 = self.width as usize * 3;
        for y in 0..self.height as usize {
            let src_start = y * w3;
            let dst_start = y * stride;
            rgb_frame.data_mut(0)[dst_start..dst_start + w3]
                .copy_from_slice(&rgb[src_start..src_start + w3]);
        }

        let mut yuv_frame = ffmpeg_next::frame::Video::empty();
        self.scaler.run(&rgb_frame, &mut yuv_frame)?;

        yuv_frame.set_pts(Some(self.pts));
        self.pts += 1;

        self.encoder.send_frame(&yuv_frame)?;
        self.receive_and_write_packets()?;

        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.encoder.send_eof()?;
        self.receive_and_write_packets()?;
        self.octx.write_trailer()?;
        Ok(())
    }

    fn receive_and_write_packets(&mut self) -> Result<()> {
        let mut packet = ffmpeg_next::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream_index);
            packet.rescale_ts(
                self.time_base,
                self.octx.stream(self.stream_index).unwrap().time_base(),
            );
            packet.write_interleaved(&mut self.octx)?;
        }
        Ok(())
    }
}

// ── Progress ─────────────────────────────────────────────────────────────────

struct Progress {
    bar: indicatif::ProgressBar,
    bytes: u64,
    start: std::time::Instant,
}

impl Progress {
    fn new(total: u64, prefix: &str) -> Self {
        let bar = indicatif::ProgressBar::new(total);
        bar.set_style(
            indicatif::ProgressStyle::with_template(
                "{prefix} [{bar:40.cyan/blue}] {pos}/{len} frames ({msg})",
            )
            .unwrap()
            .progress_chars("=> "),
        );
        bar.set_prefix(prefix.to_string());
        Self { bar, bytes: 0, start: std::time::Instant::now() }
    }

    fn update(&mut self, frame_bytes: u64) {
        self.bytes += frame_bytes;
        self.bar.inc(1);
        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bar.set_message(format!("{:.1} MB/s", (self.bytes as f64 / 1e6) / elapsed));
        }
    }

    fn finish(&self) {
        self.bar.finish();
    }
}

// ── Command ──────────────────────────────────────────────────────────────────

pub fn run(files: &[String], output: Option<&str>, quiet: bool) -> Result<()> {
    for f in files {
        if !Path::new(f).exists() {
            bail!("file not found: {}", f);
        }
    }

    for f in files {
        let reader = egorec::EgorecReader::open(f)?;
        let header = reader.header().clone();
        let total_frames = reader.frame_count();
        let duration_s = reader.duration_s();

        let color_w = header.color_width;
        let color_h = header.color_height;
        let depth_w = header.depth_width;
        let depth_h = header.depth_height;

        let fps = if duration_s > 0.0 && total_frames > 0 {
            (total_frames as f64 / duration_s).round() as u32
        } else {
            30
        };

        let stem = Path::new(f)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let filename = Path::new(f)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let out_dir = match output {
            Some(o) => PathBuf::from(o),
            None => Path::new(f)
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf(),
        };

        std::fs::create_dir_all(&out_dir)?;

        let mp4_path = out_dir.join(format!("{}.mp4", stem));
        let depth_mp4_path = out_dir.join(format!("{}.depth.mp4", stem));

        if !quiet {
            eprintln!("Converting {} -> {}", f, mp4_path.display());
            eprintln!(
                "  Color: {}x{}, Depth: {}x{}, FPS: {}, Frames: {}",
                color_w, color_h, depth_w, depth_h, fps, total_frames
            );
        }

        let mut rgb_writer = Mp4Writer::new(&mp4_path, color_w, color_h, fps)?;
        let mut depth_writer = Mp4Writer::new(&depth_mp4_path, depth_w, depth_h, fps)?;

        let frames = reader.frames()?;

        let mut progress = if !quiet {
            Some(Progress::new(total_frames, &filename))
        } else {
            None
        };

        let max_depth_mm: u16 = 2000;

        for frame_result in frames {
            let frame = frame_result?;

            rgb_writer.add_frame(&frame.rgb)?;

            let depth_rgb = depth_to_rgb(&frame.depth, depth_w, depth_h, max_depth_mm);
            depth_writer.add_frame(&depth_rgb)?;

            if let Some(ref mut p) = progress {
                let frame_bytes = frame.rgb.len() as u64 + frame.depth.len() as u64 * 2;
                p.update(frame_bytes);
            }
        }

        rgb_writer.finish()?;
        depth_writer.finish()?;

        if let Some(p) = progress.take() {
            p.finish();
        }

        // Metadata sidecar
        let meta_path = out_dir.join(format!("{}.meta.json", stem));
        let meta = serde_json::json!({
            "source_file": f,
            "session_name": header.session_name_str(),
            "serial_number": header.serial_number_str(),
            "usb_type": header.usb_type_str(),
            "color_width": color_w,
            "color_height": color_h,
            "depth_width": depth_w,
            "depth_height": depth_h,
            "fps": fps,
            "total_frames": total_frames,
            "duration_s": duration_s,
            "start_timestamp_us": header.start_timestamp_us,
            "has_imu": header.has_imu(),
            "has_depth": true,
            "rgb_codec": header.rgb_codec,
            "depth_codec": header.depth_codec,
            "rgb_quality": header.rgb_quality,
            "zstd_level": header.zstd_level,
            "depth_colormap": "turbo",
            "depth_max_mm": max_depth_mm,
            "intrinsics": {
                "color": {
                    "fx": header.color_fx,
                    "fy": header.color_fy,
                    "ppx": header.color_ppx,
                    "ppy": header.color_ppy,
                    "distortion_model": header.color_distortion_model,
                    "distortion_coeffs": header.color_distortion_coeffs.to_vec(),
                },
                "depth": {
                    "fx": header.depth_fx,
                    "fy": header.depth_fy,
                    "ppx": header.depth_ppx,
                    "ppy": header.depth_ppy,
                    "distortion_model": header.depth_distortion_model,
                    "distortion_coeffs": header.depth_distortion_coeffs.to_vec(),
                    "scale": header.depth_scale,
                }
            },
            "extrinsics": {
                "rotation": header.extrinsic_rotation.to_vec(),
                "translation": header.extrinsic_translation.to_vec(),
            }
        });

        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        if !quiet {
            eprintln!("  RGB:   {}", mp4_path.display());
            eprintln!("  Depth: {}", depth_mp4_path.display());
            eprintln!("  Meta:  {}", meta_path.display());
        }
    }

    if !quiet {
        eprintln!("\nDone.");
    }

    Ok(())
}
