use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{self, Read};

pub const FILE_MAGIC: [u8; 8] = [b'E', b'G', b'O', b'R', b'E', b'C', 0x02, 0x00];
pub const FRAME_MAGIC: u32 = 0x46524D45;
pub const INDEX_MAGIC: u32 = 0x58444E49;
pub const FOOTER_MAGIC: u32 = 0x454E4F44;

#[derive(Debug, Clone)]
pub struct FileHeader {
    pub magic: [u8; 8],
    pub header_size: u32,
    pub color_width: u32,
    pub color_height: u32,
    pub rgb_codec: u8,
}

impl FileHeader {
    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        let header_size = r.read_u32::<LittleEndian>()?;
        let _flags = r.read_u32::<LittleEndian>()?;

        let mut scratch = [0u8; 32];
        r.read_exact(&mut scratch)?;
        let _depth_scale = r.read_f32::<LittleEndian>()?;
        let _depth_width = r.read_u32::<LittleEndian>()?;
        let _depth_height = r.read_u32::<LittleEndian>()?;
        let _depth_fx = r.read_f32::<LittleEndian>()?;
        let _depth_fy = r.read_f32::<LittleEndian>()?;
        let _depth_ppx = r.read_f32::<LittleEndian>()?;
        let _depth_ppy = r.read_f32::<LittleEndian>()?;
        let _depth_distortion_model = r.read_u32::<LittleEndian>()?;
        for _ in 0..5 {
            let _ = r.read_f32::<LittleEndian>()?;
        }

        let color_width = r.read_u32::<LittleEndian>()?;
        let color_height = r.read_u32::<LittleEndian>()?;
        let _color_fx = r.read_f32::<LittleEndian>()?;
        let _color_fy = r.read_f32::<LittleEndian>()?;
        let _color_ppx = r.read_f32::<LittleEndian>()?;
        let _color_ppy = r.read_f32::<LittleEndian>()?;
        let _color_distortion_model = r.read_u32::<LittleEndian>()?;
        for _ in 0..5 {
            let _ = r.read_f32::<LittleEndian>()?;
        }
        for _ in 0..9 {
            let _ = r.read_f32::<LittleEndian>()?;
        }
        for _ in 0..3 {
            let _ = r.read_f32::<LittleEndian>()?;
        }

        let mut session_name = [0u8; 128];
        r.read_exact(&mut session_name)?;
        let _start_timestamp_us = r.read_u64::<LittleEndian>()?;
        let mut usb_type = [0u8; 8];
        r.read_exact(&mut usb_type)?;
        let rgb_codec = r.read_u8()?;
        let _depth_codec = r.read_u8()?;
        let _rgb_quality = r.read_u8()?;
        let _zstd_level = r.read_u8()?;

        let mut reserved = [0u8; 128];
        r.read_exact(&mut reserved)?;

        Ok(Self {
            magic,
            header_size,
            color_width,
            color_height,
            rgb_codec,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameBlockHeader {
    pub magic: u32,
    pub block_size: u32,
    pub rgb_compressed_size: u32,
}

impl FrameBlockHeader {
    pub const SIZE: usize = 36;

    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        let magic = r.read_u32::<LittleEndian>()?;
        let block_size = r.read_u32::<LittleEndian>()?;
        let _timestamp_us = r.read_u64::<LittleEndian>()?;
        let _frame_number = r.read_u64::<LittleEndian>()?;
        let rgb_compressed_size = r.read_u32::<LittleEndian>()?;
        let _depth_compressed_size = r.read_u32::<LittleEndian>()?;
        let _imu_sample_count = r.read_u16::<LittleEndian>()?;
        let _flags = r.read_u16::<LittleEndian>()?;

        Ok(Self {
            magic,
            block_size,
            rgb_compressed_size,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndexEntry {
    pub file_offset: u64,
}

impl IndexEntry {
    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        let _timestamp_us = r.read_u64::<LittleEndian>()?;
        let file_offset = r.read_u64::<LittleEndian>()?;
        let _frame_number = r.read_u64::<LittleEndian>()?;
        Ok(Self { file_offset })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FileFooter {
    pub index_magic: u32,
    pub index_offset: u64,
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
            total_frames: {
                let _index_entry_count = r.read_u32::<LittleEndian>()?;
                r.read_u64::<LittleEndian>()?
            },
            total_duration_us: r.read_u64::<LittleEndian>()?,
            footer_magic: r.read_u32::<LittleEndian>()?,
        })
    }
}

pub const FILE_HEADER_SIZE: usize = 472;
