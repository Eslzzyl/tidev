//! Encoding utilities for decoding command output on all platforms.
//!
//! On Windows, native programs often output text using the system's ANSI
//! code page (e.g., CP-936/GBK for Chinese, CP-1252 for Western European)
//! instead of UTF-8.  This module detects the active code page and converts
//! output to UTF-8, providing a fallback when `String::from_utf8_lossy`
//! would produce garbled text.
//!
//! On Unix, this module is a thin wrapper around `String::from_utf8_lossy`.

/// Decode command output bytes into a `String`.
///
/// On all platforms, valid UTF-8 is passed through directly.  On Windows,
/// if the bytes are not valid UTF-8, the system's active ANSI code page
/// is detected and used to decode the output.  On non-Windows, falls back
/// to `String::from_utf8_lossy`.
pub fn decode_command_output(bytes: &[u8]) -> String {
    // Fast path: valid UTF-8 — use it directly on all platforms
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    #[cfg(windows)]
    return decode_from_system_codepage(bytes);

    #[cfg(not(windows))]
    String::from_utf8_lossy(bytes).into_owned()
}

/// Decode bytes using the Windows system ANSI code page.
#[cfg(windows)]
fn decode_from_system_codepage(bytes: &[u8]) -> String {
    // SAFETY: GetACP() is a simple Win32 API that returns a u32 and has
    // no safety requirements.
    let cp = unsafe { GetACP() };
    if cp == 65001 {
        // System is already UTF-8, but the data wasn't valid UTF-8.
        return String::from_utf8_lossy(bytes).into_owned();
    }
    if let Some(encoding) = codepage_to_encoding(cp) {
        let (cow, ..) = encoding.decode(bytes);
        return cow.into_owned();
    }
    // Unknown code page — best-effort lossy decode
    String::from_utf8_lossy(bytes).into_owned()
}

/// Map a Windows code page identifier to an `encoding_rs` encoding.
///
/// Supports the most common ANSI and East Asian code pages returned by
/// `GetACP()` on modern Windows systems.
#[cfg(windows)]
fn codepage_to_encoding(cp: u32) -> Option<&'static encoding_rs::Encoding> {
    match cp {
        // Thai
        874 => Some(encoding_rs::WINDOWS_874),
        // Japanese Shift-JIS
        932 => Some(encoding_rs::SHIFT_JIS),
        // Simplified Chinese (GBK)
        936 => Some(encoding_rs::GBK),
        // Korean
        949 => Some(encoding_rs::EUC_KR),
        // Traditional Chinese (Big5)
        950 => Some(encoding_rs::BIG5),
        // Cyrillic (OEM)
        866 => Some(encoding_rs::IBM866),
        // Central/Eastern European
        1250 => Some(encoding_rs::WINDOWS_1250),
        // Cyrillic
        1251 => Some(encoding_rs::WINDOWS_1251),
        // Western European / Latin-I
        1252 => Some(encoding_rs::WINDOWS_1252),
        // Greek
        1253 => Some(encoding_rs::WINDOWS_1253),
        // Turkish
        1254 => Some(encoding_rs::WINDOWS_1254),
        // Hebrew
        1255 => Some(encoding_rs::WINDOWS_1255),
        // Arabic
        1256 => Some(encoding_rs::WINDOWS_1256),
        // Baltic
        1257 => Some(encoding_rs::WINDOWS_1257),
        // Vietnamese
        1258 => Some(encoding_rs::WINDOWS_1258),
        // Mac Roman (legacy)
        10000 => Some(encoding_rs::MACINTOSH),
        // UTF-8
        65001 => Some(encoding_rs::UTF_8),
        _ => None,
    }
}

// Get the Windows active ANSI code page via the Win32 `GetACP()` API.
#[cfg(windows)]
unsafe extern "system" {
    // SAFETY: kernel32!GetACP is always available on Windows.
    pub fn GetACP() -> u32;
}

/// Prepend shell-specific encoding setup to a command to encourage
/// UTF-8 output on Windows.
///
/// On non-Windows, returns the command unchanged.
#[cfg(windows)]
pub fn prepare_command_for_shell(command: &str, shell_program: &str, _shell_arg: &str) -> String {
    let shell_lower = shell_program.to_lowercase();

    // PowerShell: set output encoding to UTF-8
    if shell_lower.contains("powershell") || shell_lower.contains("pwsh") {
        return format!(
            "$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); {}",
            command
        );
    }

    // cmd.exe: set code page to UTF-8
    if shell_lower.contains("cmd") {
        return format!("@chcp 65001 >nul 2>nul && {}", command);
    }

    // Bash / Git Bash / MSYS2 / Cygwin: encoding is handled via
    // environment variables (set in exec.rs).  No prefix needed.
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
    fn test_valid_utf8_passthrough() {
        let input = "hello world";
        let result = decode_command_output(input.as_bytes());
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_unicode_utf8_passthrough() {
        let input = "hello world 你好世界 中文測試 🎉";
        let result = decode_command_output(input.as_bytes());
        assert_eq!(result, "hello world 你好世界 中文測試 🎉");
    }

    #[test]
    fn test_empty_input() {
        let result = decode_command_output(b"");
        assert_eq!(result, "");
    }

    #[test]
    fn test_ascii_only() {
        let input = "plain ASCII text with symbols !@#$%^&*()";
        let result = decode_command_output(input.as_bytes());
        assert_eq!(result, "plain ASCII text with symbols !@#$%^&*()");
    }

    #[test]
    fn test_non_utf8_partial_sequence() {
        // A partial 3-byte UTF-8 sequence: first 2 bytes of U+4E16 (世)
        let partial = [0xe4, 0xb8];
        let result = decode_command_output(&partial);
        // Should not panic; will contain replacement chars or partial decode
        assert!(!result.is_empty());
    }

    #[test]
    fn test_prepare_command_for_shell_non_windows() {
        // On non-Windows or when shell is bash-like: should return unchanged
        let cmd = "echo hello";
        let result = prepare_command_for_shell(cmd, "bash", "-lc");
        assert_eq!(result, "echo hello");

        let result2 = prepare_command_for_shell(cmd, "sh", "-lc");
        assert_eq!(result2, "echo hello");
    }
}
