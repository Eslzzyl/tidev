//! Text and process-output encoding utilities.
//!
//! User files are decoded once into UTF-8 for the application and retain the
//! original encoding and bytes so edits can be written back without an
//! implicit conversion. Protocol payloads must continue to use their own
//! strict UTF-8 parsers; these helpers are for text-like external data only.

use anyhow::{Result, bail};
use chardetng::EncodingDetector;
use encoding_rs::{Encoding, UTF_8};

/// A byte-order mark that was present in the source document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bom {
    None,
    Utf8,
    Utf16Le,
    Utf16Be,
}

/// Options used when decoding a text-like byte stream.
#[derive(Clone, Copy)]
pub struct DecodeOptions {
    /// An explicit fallback used when detection cannot identify the encoding.
    pub fallback_encoding: Option<&'static Encoding>,
    /// Whether chardetng may guess an encoding for bytes without a BOM.
    pub allow_heuristic: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            fallback_encoding: system_codepage_encoding(),
            allow_heuristic: true,
        }
    }
}

/// Text decoded for application use while retaining its source representation.
pub struct TextDocument {
    text: String,
    encoding: &'static Encoding,
    bom: Bom,
    original_bytes: Vec<u8>,
    had_decode_errors: bool,
}

impl TextDocument {
    /// Return the decoded UTF-8 text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Consume the document and return its decoded UTF-8 text.
    pub fn into_text(self) -> String {
        self.text
    }

    /// Return the encoding label selected for the source bytes.
    pub fn encoding_name(&self) -> &'static str {
        self.encoding.name()
    }

    /// Return the source BOM, if any.
    pub fn bom(&self) -> Bom {
        self.bom
    }

    /// Return whether the decoder had to replace malformed source sequences.
    pub fn had_decode_errors(&self) -> bool {
        self.had_decode_errors
    }

    /// Return the original source bytes.
    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
    }

    /// Encode updated text using the source encoding and BOM.
    ///
    /// If the text is unchanged, the original bytes are returned verbatim.
    /// This preserves unusual but valid source byte sequences and makes a
    /// no-op edit byte-for-byte stable.
    pub fn encode_updated(&self, text: &str) -> Result<Vec<u8>> {
        if text == self.text {
            return Ok(self.original_bytes.clone());
        }

        if self.had_decode_errors {
            bail!(
                "cannot edit {} text containing malformed byte sequences",
                self.encoding.name()
            );
        }

        let encoded = encode_with_encoding(self.encoding, text)?;
        let mut output = Vec::with_capacity(encoded.len() + bom_len(self.bom));
        append_bom(&mut output, self.bom);
        output.extend_from_slice(&encoded);
        Ok(output)
    }
}

/// Decode a text file strictly enough for editing and LLM context.
pub fn decode_text(bytes: &[u8]) -> Result<TextDocument> {
    decode_text_with_options(bytes, DecodeOptions::default())
}

/// Decode text with an explicit fallback policy.
pub fn decode_text_with_options(bytes: &[u8], options: DecodeOptions) -> Result<TextDocument> {
    let document = decode_text_lossy_with_options(bytes, options);
    if document.had_decode_errors {
        bail!(
            "input contains malformed byte sequences for detected encoding {}",
            document.encoding.name()
        );
    }
    Ok(document)
}

/// Decode text while retaining replacement characters for display-only paths.
pub fn decode_text_lossy(bytes: &[u8]) -> TextDocument {
    decode_text_lossy_with_options(bytes, DecodeOptions::default())
}

/// Decode text while retaining replacement characters for display-only paths.
pub fn decode_text_lossy_with_options(bytes: &[u8], options: DecodeOptions) -> TextDocument {
    let original_bytes = bytes.to_vec();
    let (encoding, bom, payload) = detect_encoding(bytes, options);
    let (text, had_decode_errors) = if bom == Bom::Utf16Le {
        decode_utf16(payload, true)
    } else if bom == Bom::Utf16Be {
        decode_utf16(payload, false)
    } else {
        let (decoded, had_decode_errors) = encoding.decode_without_bom_handling(payload);
        (decoded.into_owned(), had_decode_errors)
    };

    TextDocument {
        text,
        encoding,
        bom,
        original_bytes,
        had_decode_errors,
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> (String, bool) {
    let mut units = Vec::with_capacity(bytes.len().div_ceil(2));
    let mut had_decode_errors = !bytes.len().is_multiple_of(2);
    for chunk in bytes.chunks_exact(2) {
        let unit = if little_endian {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        };
        units.push(unit);
    }

    match String::from_utf16(&units) {
        Ok(text) => (text, had_decode_errors),
        Err(_) => {
            had_decode_errors = true;
            (String::from_utf16_lossy(&units), had_decode_errors)
        }
    }
}

/// Resolve an encoding label using the WHATWG/encoding_rs registry.
pub fn encoding_from_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.as_bytes())
}

fn detect_encoding(bytes: &[u8], options: DecodeOptions) -> (&'static Encoding, Bom, &[u8]) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (UTF_8, Bom::Utf8, &bytes[3..]);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return (encoding_rs::UTF_16LE, Bom::Utf16Le, &bytes[2..]);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return (encoding_rs::UTF_16BE, Bom::Utf16Be, &bytes[2..]);
    }
    if !options.allow_heuristic
        && let Some(encoding) = options.fallback_encoding
    {
        return (encoding, Bom::None, bytes);
    }
    // ISO-2022-JP uses ASCII escape sequences, so its bytes are also valid
    // UTF-8. Recognize its state-switch sequences before the UTF-8 fast path.
    if looks_like_iso_2022_jp(bytes) {
        return (encoding_rs::ISO_2022_JP, Bom::None, bytes);
    }
    if std::str::from_utf8(bytes).is_ok() {
        return (UTF_8, Bom::None, bytes);
    }

    if options.allow_heuristic && !bytes.is_empty() {
        let mut detector = EncodingDetector::new();
        detector.feed(bytes, true);
        let (encoding, certain) = detector.guess_assess(None, false);
        if certain {
            return (encoding, Bom::None, bytes);
        }
    }

    (options.fallback_encoding.unwrap_or(UTF_8), Bom::None, bytes)
}

fn looks_like_iso_2022_jp(bytes: &[u8]) -> bool {
    bytes.windows(3).any(|window| {
        matches!(
            window,
            [0x1B, b'$', b'@']
                | [0x1B, b'$', b'B']
                | [0x1B, b'(', b'B']
                | [0x1B, b'(', b'J']
                | [0x1B, b'(', b'I']
        )
    })
}

fn encode_with_encoding(encoding: &'static Encoding, text: &str) -> Result<Vec<u8>> {
    if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
        let mut output = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            let bytes = if encoding == encoding_rs::UTF_16LE {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            output.extend_from_slice(&bytes);
        }
        return Ok(output);
    }

    let (encoded, _, had_errors) = encoding.encode(text);
    if had_errors {
        bail!("updated text cannot be represented in {}", encoding.name());
    }
    Ok(encoded.into_owned())
}

fn bom_len(bom: Bom) -> usize {
    match bom {
        Bom::None => 0,
        Bom::Utf8 => 3,
        Bom::Utf16Le | Bom::Utf16Be => 2,
    }
}

fn append_bom(output: &mut Vec<u8>, bom: Bom) {
    match bom {
        Bom::None => {}
        Bom::Utf8 => output.extend_from_slice(&[0xEF, 0xBB, 0xBF]),
        Bom::Utf16Le => output.extend_from_slice(&[0xFF, 0xFE]),
        Bom::Utf16Be => output.extend_from_slice(&[0xFE, 0xFF]),
    }
}

/// Decode command output to UTF-8 using the same detection policy as files.
/// Command output is display data, so malformed sequences are replaced.
pub fn decode_command_output(bytes: &[u8]) -> String {
    decode_text_lossy(bytes).into_text()
}

fn system_codepage_encoding() -> Option<&'static Encoding> {
    #[cfg(windows)]
    {
        let cp = unsafe { GetACP() };
        codepage_to_encoding(cp)
    }

    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn codepage_to_encoding(cp: u32) -> Option<&'static Encoding> {
    match cp {
        874 => Some(encoding_rs::WINDOWS_874),
        932 => Some(encoding_rs::SHIFT_JIS),
        936 => Some(encoding_rs::GBK),
        949 => Some(encoding_rs::EUC_KR),
        950 => Some(encoding_rs::BIG5),
        866 => Some(encoding_rs::IBM866),
        1250 => Some(encoding_rs::WINDOWS_1250),
        1251 => Some(encoding_rs::WINDOWS_1251),
        1252 => Some(encoding_rs::WINDOWS_1252),
        1253 => Some(encoding_rs::WINDOWS_1253),
        1254 => Some(encoding_rs::WINDOWS_1254),
        1255 => Some(encoding_rs::WINDOWS_1255),
        1256 => Some(encoding_rs::WINDOWS_1256),
        1257 => Some(encoding_rs::WINDOWS_1257),
        1258 => Some(encoding_rs::WINDOWS_1258),
        10000 => Some(encoding_rs::MACINTOSH),
        65001 => Some(encoding_rs::UTF_8),
        _ => None,
    }
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetACP() -> u32;
}

/// Prepare a shell command for execution with UTF-8 output where supported.
#[cfg(windows)]
pub fn prepare_command_for_shell(command: &str, shell_program: &str, _shell_arg: &str) -> String {
    let shell_lower = shell_program.to_lowercase();
    if shell_lower.contains("powershell") || shell_lower.contains("pwsh") {
        return format!(
            "$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); {}",
            command
        );
    }
    if shell_lower.contains("cmd") {
        return format!("@chcp 65001 >nul 2>nul && {}", command);
    }
    command.to_string()
}

#[cfg(not(windows))]
pub fn prepare_command_for_shell(command: &str, _shell_program: &str, _shell_arg: &str) -> String {
    command.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_roundtrip_preserves_original_bytes() {
        let bytes = "hello 你好".as_bytes();
        let document = decode_text(bytes).unwrap();
        assert_eq!(document.text(), "hello 你好");
        assert_eq!(document.encode_updated(document.text()).unwrap(), bytes);
    }

    #[test]
    fn utf8_bom_is_restored() {
        let bytes = [0xEF, 0xBB, 0xBF, b'h', b'i'];
        let document = decode_text(&bytes).unwrap();
        assert_eq!(document.text(), "hi");
        assert_eq!(document.bom(), Bom::Utf8);
        assert_eq!(
            document.encode_updated("bye").unwrap(),
            [0xEF, 0xBB, 0xBF, b'b', b'y', b'e']
        );
    }

    #[test]
    fn utf16_bom_is_decoded_and_restored() {
        let encoded = encode_with_encoding(encoding_rs::UTF_16LE, "hello 你好").unwrap();
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&encoded);
        let document = decode_text(&bytes).unwrap();
        assert_eq!(document.text(), "hello 你好");
        assert_eq!(document.bom(), Bom::Utf16Le);
        assert_eq!(document.encode_updated(document.text()).unwrap(), bytes);

        let encoded = encode_with_encoding(encoding_rs::UTF_16BE, "hello 你好").unwrap();
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend_from_slice(&encoded);
        let document = decode_text(&bytes).unwrap();
        assert_eq!(document.text(), "hello 你好");
        assert_eq!(document.bom(), Bom::Utf16Be);
        assert_eq!(document.encode_updated(document.text()).unwrap(), bytes);
    }

    #[test]
    fn explicit_legacy_encoding_roundtrips() {
        let (encoded, _, had_errors) = encoding_rs::GBK.encode("你好");
        assert!(!had_errors);
        let document = decode_text_lossy_with_options(
            &encoded,
            DecodeOptions {
                fallback_encoding: Some(encoding_rs::GBK),
                allow_heuristic: false,
            },
        );
        assert_eq!(document.text(), "你好");
        assert_eq!(document.encode_updated("您好").unwrap(), {
            let (expected, _, had_errors) = encoding_rs::GBK.encode("您好");
            assert!(!had_errors);
            expected.into_owned()
        });
    }

    #[test]
    fn gb18030_roundtrips_with_explicit_fallback() {
        let (encoded, _, had_errors) = encoding_rs::GB18030.encode("你好𠀀");
        assert!(!had_errors);
        let document = decode_text_with_options(
            &encoded,
            DecodeOptions {
                fallback_encoding: Some(encoding_rs::GB18030),
                allow_heuristic: false,
            },
        )
        .unwrap();
        assert_eq!(document.text(), "你好𠀀");
        assert_eq!(document.encode_updated("您好𠀀").unwrap(), {
            let (expected, _, had_errors) = encoding_rs::GB18030.encode("您好𠀀");
            assert!(!had_errors);
            expected.into_owned()
        });
    }

    #[test]
    fn gbk_is_detected_for_a_realistic_text_sample() {
        let (encoded, _, had_errors) =
            encoding_rs::GBK.encode("这是一个用于检测传统编码的中文文本样本。");
        assert!(!had_errors);
        let document = decode_text(&encoded).unwrap();
        assert_eq!(document.text(), "这是一个用于检测传统编码的中文文本样本。");
    }

    #[test]
    fn common_legacy_encoding_labels_are_supported() {
        assert_eq!(encoding_from_label("gbk"), Some(encoding_rs::GBK));
        assert_eq!(encoding_from_label("gb2312"), Some(encoding_rs::GBK));
        assert_eq!(encoding_from_label("gb18030"), Some(encoding_rs::GB18030));
        assert_eq!(
            encoding_from_label("shift_jis"),
            Some(encoding_rs::SHIFT_JIS)
        );
        assert_eq!(encoding_from_label("euc-jp"), Some(encoding_rs::EUC_JP));
        assert_eq!(
            encoding_from_label("iso-2022-jp"),
            Some(encoding_rs::ISO_2022_JP)
        );
    }

    #[test]
    fn japanese_encodings_roundtrip() {
        for encoding in [
            encoding_rs::SHIFT_JIS,
            encoding_rs::EUC_JP,
            encoding_rs::ISO_2022_JP,
        ] {
            let (encoded, _, had_errors) = encoding.encode("日本語");
            assert!(!had_errors, "{} cannot encode the fixture", encoding.name());
            let document = decode_text_lossy_with_options(
                &encoded,
                DecodeOptions {
                    fallback_encoding: Some(encoding),
                    allow_heuristic: false,
                },
            );
            assert_eq!(
                document.text(),
                "日本語",
                "{} decoded incorrectly",
                encoding.name()
            );
            assert_eq!(
                document.encode_updated(document.text()).unwrap(),
                encoded.into_owned()
            );
        }
    }

    #[test]
    fn iso_2022_jp_is_not_misclassified_as_utf8() {
        let (encoded, _, had_errors) = encoding_rs::ISO_2022_JP.encode("日本語");
        assert!(!had_errors);
        let document = decode_text(&encoded).unwrap();
        assert_eq!(document.encoding_name(), "ISO-2022-JP");
        assert_eq!(document.text(), "日本語");
    }

    #[test]
    fn unrepresentable_legacy_edit_is_rejected() {
        let (encoded, _, _) = encoding_rs::GBK.encode("你好");
        let document = decode_text_lossy_with_options(
            &encoded,
            DecodeOptions {
                fallback_encoding: Some(encoding_rs::GBK),
                allow_heuristic: false,
            },
        );
        assert!(document.encode_updated("hello 😀").is_err());
    }

    #[test]
    fn command_output_accepts_utf8() {
        assert_eq!(decode_command_output("hello 你好".as_bytes()), "hello 你好");
    }

    #[test]
    fn prepare_command_for_shell_is_stable_on_non_windows() {
        let command = "echo hello";
        assert_eq!(prepare_command_for_shell(command, "bash", "-lc"), command);
    }

    #[test]
    #[cfg(windows)]
    fn codepage_to_encoding_mapping() {
        assert!(codepage_to_encoding(936).is_some());
        assert!(codepage_to_encoding(932).is_some());
        assert!(codepage_to_encoding(99999).is_none());
    }
}
