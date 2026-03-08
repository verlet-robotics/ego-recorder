/// Packed structs matching src/storage/binary_format.h exactly.
/// All multi-byte values are little-endian.
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{self, Read};

/// 8-byte file magic: ASCII "EGOREC" + version 2.0
pub const FILE_MAGIC: [u8; 8] = [b'E', b'G', b'O', b'R', b'E', b'C', 0x02, 0x00];

/// Frame block boundary marker: 'FRME' (0x46524D45)
pub const FRAME_MAGIC: u32 = 0x46524D45;

/// Index table start marker: 'INDX' (0x58444E49)
pub const INDEX_MAGIC: u32 = 0x58444E49;

/// Footer marker: 'DONE' (0x454E4F44)
pub const FOOTER_MAGIC: u32 = 0x454E4F44;

/// File header at byte offset 0. Contains camera calibration, session metadata,
/// and compression settings.
#[derive(Debug, Clone)]
pub struct FileHeader {
    pub magic: [u8; 8],
    pub header_size: u32,
    pub flags: u32,
    pub serial_number: [u8; 32],
    pub depth_scale: f32,
    pub depth_width: u32,
    pub depth_height: u32,
    pub depth_fx: f32,
    pub depth_fy: f32,
    pub depth_ppx: f32,
    pub depth_ppy: f32,
    pub depth_distortion_model: u32,
    pub depth_distortion_coeffs: [f32; 5],
    pub color_width: u32,
    pub color_height: u32,
    pub color_fx: f32,
    pub color_fy: f32,
    pub color_ppx: f32,
    pub color_ppy: f32,
    pub color_distortion_model: u32,
    pub color_distortion_coeffs: [f32; 5],
    pub extrinsic_rotation: [f32; 9],
    pub extrinsic_translation: [f32; 3],
    pub session_name: [u8; 128],
    pub start_timestamp_us: u64,
    pub usb_type: [u8; 8],
    pub rgb_codec: u8,
    pub depth_codec: u8,
    pub rgb_quality: u8,
    pub zstd_level: u8,
    pub reserved: [u8; 128],
}

impl FileHeader {
    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        let header_size = r.read_u32::<LittleEndian>()?;
        let flags = r.read_u32::<LittleEndian>()?;
        let mut serial_number = [0u8; 32];
        r.read_exact(&mut serial_number)?;
        let depth_scale = r.read_f32::<LittleEndian>()?;
        let depth_width = r.read_u32::<LittleEndian>()?;
        let depth_height = r.read_u32::<LittleEndian>()?;
        let depth_fx = r.read_f32::<LittleEndian>()?;
        let depth_fy = r.read_f32::<LittleEndian>()?;
        let depth_ppx = r.read_f32::<LittleEndian>()?;
        let depth_ppy = r.read_f32::<LittleEndian>()?;
        let depth_distortion_model = r.read_u32::<LittleEndian>()?;
        let mut depth_distortion_coeffs = [0f32; 5];
        for c in &mut depth_distortion_coeffs {
            *c = r.read_f32::<LittleEndian>()?;
        }
        let color_width = r.read_u32::<LittleEndian>()?;
        let color_height = r.read_u32::<LittleEndian>()?;
        let color_fx = r.read_f32::<LittleEndian>()?;
        let color_fy = r.read_f32::<LittleEndian>()?;
        let color_ppx = r.read_f32::<LittleEndian>()?;
        let color_ppy = r.read_f32::<LittleEndian>()?;
        let color_distortion_model = r.read_u32::<LittleEndian>()?;
        let mut color_distortion_coeffs = [0f32; 5];
        for c in &mut color_distortion_coeffs {
            *c = r.read_f32::<LittleEndian>()?;
        }
        let mut extrinsic_rotation = [0f32; 9];
        for v in &mut extrinsic_rotation {
            *v = r.read_f32::<LittleEndian>()?;
        }
        let mut extrinsic_translation = [0f32; 3];
        for v in &mut extrinsic_translation {
            *v = r.read_f32::<LittleEndian>()?;
        }
        let mut session_name = [0u8; 128];
        r.read_exact(&mut session_name)?;
        let start_timestamp_us = r.read_u64::<LittleEndian>()?;
        let mut usb_type = [0u8; 8];
        r.read_exact(&mut usb_type)?;
        let rgb_codec = r.read_u8()?;
        let depth_codec = r.read_u8()?;
        let rgb_quality = r.read_u8()?;
        let zstd_level = r.read_u8()?;
        let mut reserved = [0u8; 128];
        r.read_exact(&mut reserved)?;

        Ok(Self {
            magic,
            header_size,
            flags,
            serial_number,
            depth_scale,
            depth_width,
            depth_height,
            depth_fx,
            depth_fy,
            depth_ppx,
            depth_ppy,
            depth_distortion_model,
            depth_distortion_coeffs,
            color_width,
            color_height,
            color_fx,
            color_fy,
            color_ppx,
            color_ppy,
            color_distortion_model,
            color_distortion_coeffs,
            extrinsic_rotation,
            extrinsic_translation,
            session_name,
            start_timestamp_us,
            usb_type,
            rgb_codec,
            depth_codec,
            rgb_quality,
            zstd_level,
            reserved,
        })
    }

    /// Session name as a trimmed string.
    pub fn session_name_str(&self) -> &str {
        let end = self
            .session_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.session_name.len());
        std::str::from_utf8(&self.session_name[..end]).unwrap_or("")
    }

    /// Serial number as a trimmed string.
    pub fn serial_number_str(&self) -> &str {
        let end = self
            .serial_number
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.serial_number.len());
        std::str::from_utf8(&self.serial_number[..end]).unwrap_or("")
    }

    /// USB type as a trimmed string.
    pub fn usb_type_str(&self) -> &str {
        let end = self
            .usb_type
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.usb_type.len());
        std::str::from_utf8(&self.usb_type[..end]).unwrap_or("")
    }

    pub fn has_imu(&self) -> bool {
        self.flags & 0x01 != 0
    }
}

/// Frame block header (36 bytes).
#[derive(Debug, Clone, Copy)]
pub struct FrameBlockHeader {
    pub magic: u32,
    pub block_size: u32,
    pub timestamp_us: u64,
    pub frame_number: u64,
    pub rgb_compressed_size: u32,
    pub depth_compressed_size: u32,
    pub imu_sample_count: u16,
    pub flags: u16,
}

impl FrameBlockHeader {
    pub const SIZE: usize = 36;

    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self {
            magic: r.read_u32::<LittleEndian>()?,
            block_size: r.read_u32::<LittleEndian>()?,
            timestamp_us: r.read_u64::<LittleEndian>()?,
            frame_number: r.read_u64::<LittleEndian>()?,
            rgb_compressed_size: r.read_u32::<LittleEndian>()?,
            depth_compressed_size: r.read_u32::<LittleEndian>()?,
            imu_sample_count: r.read_u16::<LittleEndian>()?,
            flags: r.read_u16::<LittleEndian>()?,
        })
    }
}

/// IMU sample wire format (32 bytes).
#[derive(Debug, Clone, Copy)]
pub struct IMUSampleWire {
    pub timestamp_us: u64,
    pub accel_x: f32,
    pub accel_y: f32,
    pub accel_z: f32,
    pub gyro_x: f32,
    pub gyro_y: f32,
    pub gyro_z: f32,
}

impl IMUSampleWire {
    pub const SIZE: usize = 32;
}

/// Index entry (24 bytes).
#[derive(Debug, Clone, Copy)]
pub struct IndexEntry {
    pub timestamp_us: u64,
    pub file_offset: u64,
    pub frame_number: u64,
}

impl IndexEntry {
    pub const SIZE: usize = 24;

    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self {
            timestamp_us: r.read_u64::<LittleEndian>()?,
            file_offset: r.read_u64::<LittleEndian>()?,
            frame_number: r.read_u64::<LittleEndian>()?,
        })
    }
}

/// File footer (36 bytes).
#[derive(Debug, Clone, Copy)]
pub struct FileFooter {
    pub index_magic: u32,
    pub index_offset: u64,
    pub index_entry_count: u32,
    pub total_frames: u64,
    pub total_duration_us: u64,
    pub footer_magic: u32,
}

impl FileFooter {
    pub const SIZE: usize = 36;

    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self {
            index_magic: r.read_u32::<LittleEndian>()?,
            index_offset: r.read_u64::<LittleEndian>()?,
            index_entry_count: r.read_u32::<LittleEndian>()?,
            total_frames: r.read_u64::<LittleEndian>()?,
            total_duration_us: r.read_u64::<LittleEndian>()?,
            footer_magic: r.read_u32::<LittleEndian>()?,
        })
    }
}

/// Compute the serialized size of the FileHeader by counting fields.
/// This must match sizeof(FileHeader) in C++ (with #pragma pack(push, 1)).
pub const FILE_HEADER_SIZE: usize =
    8       // magic
    + 4     // header_size
    + 4     // flags
    + 32    // serial_number
    + 4     // depth_scale
    + 4     // depth_width
    + 4     // depth_height
    + 4     // depth_fx
    + 4     // depth_fy
    + 4     // depth_ppx
    + 4     // depth_ppy
    + 4     // depth_distortion_model
    + 20    // depth_distortion_coeffs (5 * f32)
    + 4     // color_width
    + 4     // color_height
    + 4     // color_fx
    + 4     // color_fy
    + 4     // color_ppx
    + 4     // color_ppy
    + 4     // color_distortion_model
    + 20    // color_distortion_coeffs (5 * f32)
    + 36    // extrinsic_rotation (9 * f32)
    + 12    // extrinsic_translation (3 * f32)
    + 128   // session_name
    + 8     // start_timestamp_us
    + 8     // usb_type
    + 1     // rgb_codec
    + 1     // depth_codec
    + 1     // rgb_quality
    + 1     // zstd_level
    + 128;  // reserved

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_block_header_size() {
        assert_eq!(FrameBlockHeader::SIZE, 36);
    }

    #[test]
    fn test_imu_sample_size() {
        assert_eq!(IMUSampleWire::SIZE, 32);
    }

    #[test]
    fn test_index_entry_size() {
        assert_eq!(IndexEntry::SIZE, 24);
    }

    #[test]
    fn test_file_footer_size() {
        assert_eq!(FileFooter::SIZE, 36);
    }
}
