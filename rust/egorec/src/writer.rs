//! Binary writer for .egorec v2 files.
//!
//! Supports bulk span transfer from a source file — copies contiguous frame
//! blocks without decoding. Used by splice to extract active segments.

use crate::format::*;
use byteorder::{LittleEndian, WriteBytesExt};
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::scanner::FrameInfo;

const COPY_BUF_SIZE: usize = 64 * 1024; // 64KB scratch buffer

/// Writer for creating new .egorec v2 files from copied frame blocks.
pub struct EgorecWriter {
    file: BufWriter<File>,
    index: Vec<IndexEntry>,
    frame_count: u64,
    first_ts: Option<u64>,
    last_ts: Option<u64>,
}

impl EgorecWriter {
    /// Create a new .egorec file and write the file header.
    pub fn create(path: &Path, header: &FileHeader) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Write header
        Self::write_header(&mut writer, header)?;

        Ok(Self {
            file: writer,
            index: Vec::new(),
            frame_count: 0,
            first_ts: None,
            last_ts: None,
        })
    }

    fn write_header(w: &mut BufWriter<File>, h: &FileHeader) -> io::Result<()> {
        w.write_all(&h.magic)?;
        w.write_u32::<LittleEndian>(h.header_size)?;
        w.write_u32::<LittleEndian>(h.flags)?;
        w.write_all(&h.serial_number)?;
        w.write_f32::<LittleEndian>(h.depth_scale)?;
        w.write_u32::<LittleEndian>(h.depth_width)?;
        w.write_u32::<LittleEndian>(h.depth_height)?;
        w.write_f32::<LittleEndian>(h.depth_fx)?;
        w.write_f32::<LittleEndian>(h.depth_fy)?;
        w.write_f32::<LittleEndian>(h.depth_ppx)?;
        w.write_f32::<LittleEndian>(h.depth_ppy)?;
        w.write_u32::<LittleEndian>(h.depth_distortion_model)?;
        for &c in &h.depth_distortion_coeffs {
            w.write_f32::<LittleEndian>(c)?;
        }
        w.write_u32::<LittleEndian>(h.color_width)?;
        w.write_u32::<LittleEndian>(h.color_height)?;
        w.write_f32::<LittleEndian>(h.color_fx)?;
        w.write_f32::<LittleEndian>(h.color_fy)?;
        w.write_f32::<LittleEndian>(h.color_ppx)?;
        w.write_f32::<LittleEndian>(h.color_ppy)?;
        w.write_u32::<LittleEndian>(h.color_distortion_model)?;
        for &c in &h.color_distortion_coeffs {
            w.write_f32::<LittleEndian>(c)?;
        }
        for &v in &h.extrinsic_rotation {
            w.write_f32::<LittleEndian>(v)?;
        }
        for &v in &h.extrinsic_translation {
            w.write_f32::<LittleEndian>(v)?;
        }
        w.write_all(&h.session_name)?;
        w.write_u64::<LittleEndian>(h.start_timestamp_us)?;
        w.write_all(&h.usb_type)?;
        w.write_u8(h.rgb_codec)?;
        w.write_u8(h.depth_codec)?;
        w.write_u8(h.rgb_quality)?;
        w.write_u8(h.zstd_level)?;
        w.write_all(&h.reserved)?;
        Ok(())
    }

    /// Write a single frame with pre-compressed RGB and depth data.
    pub fn write_frame(
        &mut self,
        timestamp_us: u64,
        frame_number: u64,
        is_keyframe: bool,
        rgb_data: &[u8],
        depth_data: &[u8],
    ) -> io::Result<()> {
        let block_size = (FrameBlockHeader::SIZE + rgb_data.len() + depth_data.len()) as u32;
        let flags: u16 = if is_keyframe { 1 } else { 0 };

        // Write FrameBlockHeader
        let offset = self.file.seek(SeekFrom::Current(0))?;
        self.file.write_u32::<LittleEndian>(FRAME_MAGIC)?;
        self.file.write_u32::<LittleEndian>(block_size)?;
        self.file.write_u64::<LittleEndian>(timestamp_us)?;
        self.file.write_u64::<LittleEndian>(frame_number)?;
        self.file
            .write_u32::<LittleEndian>(rgb_data.len() as u32)?;
        self.file
            .write_u32::<LittleEndian>(depth_data.len() as u32)?;
        self.file.write_u16::<LittleEndian>(0)?; // imu_sample_count
        self.file.write_u16::<LittleEndian>(flags)?;

        // Write compressed data
        self.file.write_all(rgb_data)?;
        self.file.write_all(depth_data)?;

        // Update index
        self.index.push(IndexEntry {
            timestamp_us,
            file_offset: offset,
            frame_number,
        });
        if self.first_ts.is_none() {
            self.first_ts = Some(timestamp_us);
        }
        self.last_ts = Some(timestamp_us);
        self.frame_count += 1;

        Ok(())
    }

    /// Copy a contiguous span of frame blocks from source file.
    /// `frame_infos` provides the index entries for the frames being copied.
    pub fn copy_span(
        &mut self,
        source: &mut File,
        frame_infos: &[FrameInfo],
    ) -> io::Result<()> {
        if frame_infos.is_empty() {
            return Ok(());
        }

        let source_start = frame_infos[0].file_offset;
        let last = &frame_infos[frame_infos.len() - 1];
        let source_end = last.file_offset + last.block_size as u64;
        let total_bytes = source_end - source_start;

        // Record index entries with updated offsets
        let dest_base = self.file.seek(SeekFrom::Current(0))?;
        for fi in frame_infos {
            let dest_offset = dest_base + (fi.file_offset - source_start);
            self.index.push(IndexEntry {
                timestamp_us: fi.timestamp_us,
                file_offset: dest_offset,
                frame_number: fi.frame_number,
            });
            if self.first_ts.is_none() {
                self.first_ts = Some(fi.timestamp_us);
            }
            self.last_ts = Some(fi.timestamp_us);
            self.frame_count += 1;
        }

        // Try copy_file_range on Linux, fall back to buffered copy
        source.seek(SeekFrom::Start(source_start))?;

        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let src_fd = source.as_raw_fd();
            // Flush BufWriter so we can use raw fd
            self.file.flush()?;
            let dst_fd = self.file.get_ref().as_raw_fd();
            let mut src_off = source_start as i64;
            let mut dst_off = dest_base as i64;
            let mut remaining = total_bytes as usize;

            while remaining > 0 {
                let chunk = remaining.min(usize::MAX / 2);
                let n = unsafe {
                    libc::copy_file_range(
                        src_fd,
                        &mut src_off,
                        dst_fd,
                        &mut dst_off,
                        chunk,
                        0,
                    )
                };
                if n < 0 {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::ENOSYS)
                        || err.raw_os_error() == Some(libc::EXDEV)
                    {
                        // Fallback to buffered copy
                        source.seek(SeekFrom::Start(source_start))?;
                        self.file.seek(SeekFrom::Start(dest_base))?;
                        Self::buffered_copy(source, &mut self.file, total_bytes)?;
                        return Ok(());
                    }
                    return Err(err);
                }
                remaining -= n as usize;
            }
            // Seek BufWriter past the copied data
            self.file.seek(SeekFrom::Start(dest_base + total_bytes))?;
            return Ok(());
        }

        #[cfg(not(target_os = "linux"))]
        {
            Self::buffered_copy(source, &mut self.file, total_bytes)?;
            return Ok(());
        }
    }

    fn buffered_copy<R: Read, W: Write>(
        source: &mut R,
        dest: &mut W,
        total: u64,
    ) -> io::Result<()> {
        let mut buf = vec![0u8; COPY_BUF_SIZE];
        let mut remaining = total;
        while remaining > 0 {
            let to_read = (remaining as usize).min(COPY_BUF_SIZE);
            source.read_exact(&mut buf[..to_read])?;
            dest.write_all(&mut buf[..to_read])?;
            remaining -= to_read as u64;
        }
        Ok(())
    }

    /// Write index table and footer, close file.
    pub fn finalize(mut self) -> io::Result<()> {
        self.file.flush()?;
        let index_offset = self.file.seek(SeekFrom::Current(0))?;

        // Write index entries
        for entry in &self.index {
            self.file.write_u64::<LittleEndian>(entry.timestamp_us)?;
            self.file.write_u64::<LittleEndian>(entry.file_offset)?;
            self.file.write_u64::<LittleEndian>(entry.frame_number)?;
        }

        // Compute duration
        let duration_us = match (self.first_ts, self.last_ts) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        };

        // Write footer
        self.file.write_u32::<LittleEndian>(INDEX_MAGIC)?;
        self.file.write_u64::<LittleEndian>(index_offset)?;
        self.file
            .write_u32::<LittleEndian>(self.index.len() as u32)?;
        self.file.write_u64::<LittleEndian>(self.frame_count)?;
        self.file.write_u64::<LittleEndian>(duration_us)?;
        self.file.write_u32::<LittleEndian>(FOOTER_MAGIC)?;

        self.file.flush()?;

        // fsync
        self.file.get_ref().sync_all()?;

        Ok(())
    }
}
