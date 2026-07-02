//! zstd compression helpers for large text columns.

use zstd::stream::{decode_all, encode_all};

/// Compress text using zstd level 3.
pub fn compress_text(text: &str) -> Vec<u8> {
    encode_all(std::io::Cursor::new(text), 3).unwrap()
}

/// Decompress a zstd-compressed blob back into a `String`.
pub fn decompress_text(data: &[u8]) -> String {
    if let Ok(bytes) = decode_all(std::io::Cursor::new(data))
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
        let decompressed = decompress_text(&compressed);
        assert_eq!(input, decompressed);
    }

    #[test]
    fn fallback_for_uncompressed_text() {
        let input = "legacy plain text";
        let result = decompress_text(input.as_bytes());
        assert_eq!(input, result);
    }
}
