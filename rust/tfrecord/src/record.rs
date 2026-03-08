/// TFRecord wire format: length-prefixed records with masked CRC32C checksums.
///
/// Format per record:
///   [u64 length] [u32 masked_crc(length)] [data bytes] [u32 masked_crc(data)]
use std::io::{self, Write};

/// TFRecord masked CRC32C: rotate right by 15 bits, then add constant.
fn masked_crc32c(data: &[u8]) -> u32 {
    let crc = crc32c::crc32c(data);
    ((crc >> 15) | (crc << 17)).wrapping_add(0xa282ead8)
}

/// Write a single TFRecord to the writer.
pub fn write_record<W: Write>(writer: &mut W, data: &[u8]) -> io::Result<()> {
    let len = data.len() as u64;
    let len_bytes = len.to_le_bytes();

    // Length + CRC of length
    writer.write_all(&len_bytes)?;
    writer.write_all(&masked_crc32c(&len_bytes).to_le_bytes())?;

    // Data + CRC of data
    writer.write_all(data)?;
    writer.write_all(&masked_crc32c(data).to_le_bytes())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_masked_crc32c_known_value() {
        // Test that masking works
        let data = b"hello";
        let crc = crc32c::crc32c(data);
        let masked = masked_crc32c(data);
        assert_ne!(crc, masked);
        // Verify the masking formula
        let expected = ((crc >> 15) | (crc << 17)).wrapping_add(0xa282ead8);
        assert_eq!(masked, expected);
    }

    #[test]
    fn test_write_record() {
        let mut buf = Vec::new();
        write_record(&mut buf, b"test data").unwrap();
        // Should be: 8 (len) + 4 (len_crc) + 9 (data) + 4 (data_crc) = 25 bytes
        assert_eq!(buf.len(), 25);
        // First 8 bytes are length (9 as u64 LE)
        assert_eq!(&buf[0..8], &9u64.to_le_bytes());
    }
}
