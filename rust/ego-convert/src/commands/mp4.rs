use crate::progress::ExportProgress;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Turbo colormap — attempt to match matplotlib's "turbo".
/// Maps a normalized 0..1 value to (R, G, B) in 0..255.
fn turbo_colormap(t: f32) -> (u8, u8, u8) {
    // Piecewise polynomial approximation of the turbo colormap
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

/// Render depth Z16 buffer to RGB24 using the turbo colormap.
/// Clips to `max_depth_mm` then normalizes to 0..1.
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
            // Zero depth = no data → dark gray
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

pub fn run(files: &[String], output: Option<&str>, quiet: bool) -> Result<()> {
    // Validate files exist
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

        // Compute FPS from footer data, fallback 30
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

        let mut rgb_writer =
            lerobot_writer::video::Mp4Writer::new(&mp4_path, color_w, color_h, fps)?;
        let mut depth_writer =
            lerobot_writer::video::Mp4Writer::new(&depth_mp4_path, depth_w, depth_h, fps)?;

        let frames = reader.frames()?;

        let mut progress = if !quiet {
            Some(ExportProgress::new(total_frames, &filename))
        } else {
            None
        };

        // Max depth for colormap normalization (2m in mm; reasonable indoor range)
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

        // Write metadata sidecar JSON
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
