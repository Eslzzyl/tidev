//! zstd compression helpers for large text columns.

use zstd::stream::{decode_all, encode_all};

/// Zstandard frame header magic number: 0xFD2FB528 (little-endian: 0x28, 0xB5, 0x2F, 0xFD).
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Compress text using zstd level 3. Empty input returns an empty byte vector.
pub fn compress_text(text: &str) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }
    encode_all(std::io::Cursor::new(text), 3).unwrap()
}

/// Decompress a zstd-compressed blob back into a `String`.
///
/// If the data is empty, returns an empty string without decompression overhead.
/// If the data begins with the zstd magic header, attempts decompression.
/// Falls back to interpreting raw bytes as UTF-8 for uncompressed legacy data.
pub fn decompress_text(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    if data.len() >= 4
        && data[..4] == ZSTD_MAGIC
        && let Ok(bytes) = decode_all(std::io::Cursor::new(data))
        && let Ok(s) = String::from_utf8(bytes)
    {
        return s;
    }
    String::from_utf8_lossy(data).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small() {
        let input = "Hello, tidev!";
        let compressed = compress_text(input);
        assert!(!compressed.is_empty());
        let decompressed = decompress_text(&compressed);
        assert_eq!(input, decompressed);
    }

    #[test]
    fn roundtrip_large() {
        let input = "The quick brown fox jumps over the lazy dog. ".repeat(1000);
        let compressed = compress_text(&input);
        let decompressed = decompress_text(&compressed);
        assert_eq!(input, decompressed);
        assert!(
            compressed.len() < input.len(),
            "zstd should compress repetitive text"
        );
    }

    #[test]
    fn empty_string() {
        let input = "";
        let compressed = compress_text(input);
        assert!(
            compressed.is_empty(),
            "empty string should return empty bytes"
        );
        let decompressed = decompress_text(&compressed);
        assert_eq!(input, decompressed);
    }

    #[test]
    fn legacy_empty_zstd_frame() {
        // A 9-byte zstd frame encoding an empty string from older versions.
        let legacy_empty_frame = encode_all(std::io::Cursor::new(""), 3).unwrap();
        assert_eq!(legacy_empty_frame.len(), 9);
        assert_eq!(legacy_empty_frame[..4], ZSTD_MAGIC);
        let decompressed = decompress_text(&legacy_empty_frame);
        assert_eq!(decompressed, "");
    }

    #[test]
    fn fallback_for_uncompressed_text() {
        let input = "legacy plain text";
        let result = decompress_text(input.as_bytes());
        assert_eq!(input, result);
    }
}
