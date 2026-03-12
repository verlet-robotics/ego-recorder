//! Fast proxy video generation via H.264 remux.
//!
//! Reads .egorec frame blocks header-only, extracts raw H.264 NAL data,
//! and pipes it into `ffmpeg -c copy` to produce a browser-playable MP4.
//! Zero decode/encode — purely I/O-bound.

use anyhow::{bail, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use egorec::format::*;

use crate::progress::ExportProgress;

pub fn run(files: &[String], output: Option<&str>, quiet: bool) -> Result<()> {
    for f in files {
        if !Path::new(f).exists() {
            bail!("file not found: {}", f);
        }
    }

    for f in files {
        remux_one(f, output, quiet)?;
    }

    if !quiet {
        eprintln!("\nDone.");
    }

    Ok(())
}

fn remux_one(path: &str, output: Option<&str>, quiet: bool) -> Result<()> {
    let mut file = std::io::BufReader::new(std::fs::File::open(path)?);

    let header = FileHeader::read_from(&mut file)?;
    if header.magic[..6] != FILE_MAGIC[..6] || header.magic[6] != 0x02 {
        bail!("{}: not a valid .egorec v2 file", path);
    }
    if header.rgb_codec != 2 {
        bail!(
            "{}: rgb_codec={} (need 2 for H.264 remux)",
            path,
            header.rgb_codec
        );
    }

    let file_len = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::End(-(FileFooter::SIZE as i64)))?;
    let footer = FileFooter::read_from(&mut file)?;
    if footer.footer_magic != FOOTER_MAGIC {
        bail!("{}: missing footer — file may be truncated", path);
    }

    let total_frames = footer.total_frames;
    let duration_s = footer.total_duration_us as f64 / 1e6;
    let fps = if duration_s > 0.0 && total_frames > 0 {
        (total_frames as f64 / duration_s).round() as u32
    } else {
        30
    };

    let stem = Path::new(path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let out_dir = match output {
        Some(o) => PathBuf::from(o),
        None => Path::new(path)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf(),
    };
    std::fs::create_dir_all(&out_dir)?;
    let mp4_path = out_dir.join(format!("{}.proxy.mp4", stem));

    if !quiet {
        eprintln!(
            "Remuxing {} -> {} ({} frames, {}x{} @ {}fps)",
            path,
            mp4_path.display(),
            total_frames,
            header.color_width,
            header.color_height,
            fps
        );
    }

    let mut ffmpeg = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "h264",
            "-r", &fps.to_string(),
            "-i", "pipe:0",
            "-c:v", "copy",
            "-movflags", "+faststart",
            "-f", "mp4",
        ])
        .arg(mp4_path.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(if quiet { Stdio::null() } else { Stdio::piped() })
        .spawn()?;

    let mut stdin = ffmpeg.stdin.take().unwrap();

    // Seek to first frame
    file.seek(SeekFrom::Start(FILE_HEADER_SIZE as u64))?;

    let mut progress = if !quiet {
        Some(ExportProgress::new(
            total_frames,
            &Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
        ))
    } else {
        None
    };

    let mut last_block_end: u64 = FILE_HEADER_SIZE as u64;

    for _ in 0..total_frames {
        let magic = file.read_u32::<LittleEndian>()?;
        if magic != FRAME_MAGIC {
            bail!("bad frame magic at offset {}", last_block_end);
        }
        let block_size = file.read_u32::<LittleEndian>()?;
        let _timestamp_us = file.read_u64::<LittleEndian>()?;
        let _frame_number = file.read_u64::<LittleEndian>()?;
        let rgb_compressed_size = file.read_u32::<LittleEndian>()?;
        let depth_compressed_size = file.read_u32::<LittleEndian>()?;
        let imu_sample_count = file.read_u16::<LittleEndian>()?;
        let _flags = file.read_u16::<LittleEndian>()?;

        if rgb_compressed_size > 0 {
            let mut buf = vec![0u8; rgb_compressed_size as usize];
            file.read_exact(&mut buf)?;
            stdin.write_all(&buf)?;
        }

        // Skip depth + IMU
        let skip = depth_compressed_size as u64
            + imu_sample_count as u64 * IMUSampleWire::SIZE as u64;
        if skip > 0 {
            file.seek(SeekFrom::Current(skip as i64))?;
        }

        last_block_end += block_size as u64;

        if let Some(ref mut p) = progress {
            p.update(rgb_compressed_size as u64);
        }
    }

    // Flush trailing H.264 data between last frame block and index table
    let trailing_size = footer.index_offset.saturating_sub(last_block_end);
    if trailing_size > 0 {
        let mut trailing = vec![0u8; trailing_size as usize];
        file.seek(SeekFrom::Start(last_block_end))?;
        file.read_exact(&mut trailing)?;
        stdin.write_all(&trailing)?;
    }

    drop(stdin);

    let status = ffmpeg.wait()?;
    if !status.success() {
        bail!("ffmpeg exited with status {}", status);
    }

    if let Some(p) = progress.take() {
        p.finish();
    }

    if !quiet {
        let out_size = std::fs::metadata(&mp4_path)?.len();
        let in_size = file_len;
        eprintln!(
            "  {} ({:.1} MB) -> {} ({:.1} MB, {:.0}% of original)",
            path,
            in_size as f64 / 1e6,
            mp4_path.display(),
            out_size as f64 / 1e6,
            out_size as f64 / in_size as f64 * 100.0
        );
    }

    Ok(())
}
