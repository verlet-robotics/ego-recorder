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
    #[error("null decompressor")]
    NullDecompressor,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_decompressor() {
        let _d = ZdepthDecompressor::new().unwrap();
    }
}
