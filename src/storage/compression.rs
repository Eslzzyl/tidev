//! zstd compression helpers for large text columns.
//!
//! We compress large text columns before writing to SQLite, extending
//! the original three-column strategy to cover more large fields:
//!
//! | Table | Column | Original type | Status |
//! |-------|--------|--------------|--------|
//! | `messages` | `content` | BLOB | Compressed |
//! | `messages` | `reasoning` | BLOB | Compressed |
//! | `messages` | `patch_files` | BLOB | Compressed |
//! | `messages` | `file_diffs` | BLOB | Compressed |
//! | `tool_events` | `input_json` | BLOB | Compressed |
//! | `tool_events` | `output_text` | BLOB | Compressed |
//! | `session_reverts` | `redo_snapshot` | BLOB | Compressed |
//!
//! With zstd level 3 we expect a 3–5× reduction, saving significant
//! storage for the large JSON and diff content stored in these columns.

use zstd::stream::{decode_all, encode_all};

/// Compress text using zstd level 3.
///
/// Level 3 offers a good trade-off between speed and ratio for English
/// text (typical ratio 3–5×).  Decompression is ~500 MB/s.
pub fn compress_text(text: &str) -> Vec<u8> {
    encode_all(std::io::Cursor::new(text), 3).unwrap()
}

/// Decompress a zstd-compressed blob back into a `String`.
///
/// If the data is not valid zstd (e.g. it is plain text from an older
/// database version), returns the raw bytes as a lossy UTF-8 string.
/// This ensures backwards compatibility with uncompressed columns.
pub fn decompress_text(data: &[u8]) -> String {
    // Try zstd decompression first
    if let Ok(bytes) = decode_all(std::io::Cursor::new(data))
        && let Ok(s) = String::from_utf8(bytes)
    {
        return s;
    }
    // Fall back: data might be uncompressed text (old database)
    String::from_utf8_lossy(data).to_string()
}

/// Read a column that may be either TEXT or BLOB and decompress it.
///
/// Use this in `load_messages` / `load_tool_event_output` when you do
/// not know whether the stored value is compressed or not (handles
/// old databases with TEXT columns gracefully).
pub fn read_decompress_column(data: &[u8]) -> String {
    decompress_text(data)
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
        // Verify it actually compressed
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
        // Passing plain text (no zstd header) should fall through
        let input = "legacy plain text";
        let result = decompress_text(input.as_bytes());
        assert_eq!(input, result);
    }
}
