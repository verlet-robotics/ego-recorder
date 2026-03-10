use std::ptr;

// Raw FFI bindings
extern "C" {
    fn zdepth_decompressor_new() -> *mut std::ffi::c_void;
    fn zdepth_decompressor_free(d: *mut std::ffi::c_void);
    fn zdepth_decompressor_decompress(
        d: *mut std::ffi::c_void,
        compressed_data: *const u8,
        compressed_size: usize,
        out_width: *mut i32,
        out_height: *mut i32,
        out_data: *mut *const u16,
        out_count: *mut usize,
    ) -> i32;

    fn zdepth_compressor_new() -> *mut std::ffi::c_void;
    fn zdepth_compressor_free(c: *mut std::ffi::c_void);
    fn zdepth_compressor_compress(
        c: *mut std::ffi::c_void,
        depth_data: *const u16,
        width: i32,
        height: i32,
        keyframe: i32,
        out_data: *mut *const u8,
        out_size: *mut usize,
    ) -> i32;
}

/// Safe wrapper around the Zdepth decompressor.
pub struct ZdepthDecompressor {
    inner: *mut std::ffi::c_void,
}

// SAFETY: The C++ decompressor has no thread-local state.
unsafe impl Send for ZdepthDecompressor {}

#[derive(Debug, thiserror::Error)]
pub enum ZdepthError {
    #[error("zdepth decompression failed (code {0})")]
    DecompressFailed(i32),
    #[error("zdepth compression failed (code {0})")]
    CompressFailed(i32),
    #[error("null decompressor")]
    NullDecompressor,
    #[error("null compressor")]
    NullCompressor,
}

impl ZdepthDecompressor {
    pub fn new() -> Result<Self, ZdepthError> {
        let inner = unsafe { zdepth_decompressor_new() };
        if inner.is_null() {
            return Err(ZdepthError::NullDecompressor);
        }
        Ok(Self { inner })
    }

    /// Decompress a Zdepth-compressed buffer.
    /// Returns (width, height, depth_data) where depth_data is a Vec of uint16 values.
    pub fn decompress(&mut self, data: &[u8]) -> Result<(usize, usize, Vec<u16>), ZdepthError> {
        let mut width: i32 = 0;
        let mut height: i32 = 0;
        let mut out_data: *const u16 = ptr::null();
        let mut out_count: usize = 0;

        let ret = unsafe {
            zdepth_decompressor_decompress(
                self.inner,
                data.as_ptr(),
                data.len(),
                &mut width,
                &mut height,
                &mut out_data,
                &mut out_count,
            )
        };

        if ret != 0 {
            return Err(ZdepthError::DecompressFailed(ret));
        }

        // Copy the data out of the C++ internal buffer
        let depth_slice = unsafe { std::slice::from_raw_parts(out_data, out_count) };
        Ok((width as usize, height as usize, depth_slice.to_vec()))
    }
}

impl Drop for ZdepthDecompressor {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe { zdepth_decompressor_free(self.inner) };
        }
    }
}

impl Default for ZdepthDecompressor {
    fn default() -> Self {
        Self::new().expect("failed to create ZdepthDecompressor")
    }
}

/// Safe wrapper around the Zdepth compressor.
pub struct ZdepthCompressor {
    inner: *mut std::ffi::c_void,
}

// SAFETY: The C++ compressor has no thread-local state.
unsafe impl Send for ZdepthCompressor {}

impl ZdepthCompressor {
    pub fn new() -> Result<Self, ZdepthError> {
        let inner = unsafe { zdepth_compressor_new() };
        if inner.is_null() {
            return Err(ZdepthError::NullCompressor);
        }
        Ok(Self { inner })
    }

    /// Compress a depth buffer using Zdepth.
    /// Returns a slice of compressed bytes. The slice is valid until the next
    /// call to `compress` or until the compressor is dropped.
    pub fn compress(
        &mut self,
        depth: &[u16],
        width: i32,
        height: i32,
        keyframe: bool,
    ) -> Result<&[u8], ZdepthError> {
        let mut out_data: *const u8 = ptr::null();
        let mut out_size: usize = 0;

        let ret = unsafe {
            zdepth_compressor_compress(
                self.inner,
                depth.as_ptr(),
                width,
                height,
                keyframe as i32,
                &mut out_data,
                &mut out_size,
            )
        };

        if ret != 0 {
            return Err(ZdepthError::CompressFailed(ret));
        }

        Ok(unsafe { std::slice::from_raw_parts(out_data, out_size) })
    }
}

impl Drop for ZdepthCompressor {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe { zdepth_compressor_free(self.inner) };
        }
    }
}

impl Default for ZdepthCompressor {
    fn default() -> Self {
        Self::new().expect("failed to create ZdepthCompressor")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_decompressor() {
        let _d = ZdepthDecompressor::new().unwrap();
    }

    #[test]
    fn test_create_compressor() {
        let _c = ZdepthCompressor::new().unwrap();
    }

    #[test]
    fn test_compress_decompress_roundtrip() {
        let mut compressor = ZdepthCompressor::new().unwrap();
        let mut decompressor = ZdepthDecompressor::new().unwrap();

        let width = 64;
        let height = 64;
        let depth: Vec<u16> = (0..width * height).map(|i| (i % 4096) as u16).collect();

        let compressed = compressor
            .compress(&depth, width as i32, height as i32, true)
            .unwrap();
        let compressed_copy = compressed.to_vec();

        let (w, h, decompressed) = decompressor.decompress(&compressed_copy).unwrap();
        assert_eq!(w, width);
        assert_eq!(h, height);
        assert_eq!(decompressed.len(), depth.len());
    }
}
