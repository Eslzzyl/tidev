use std::borrow::Cow;

use ansi_to_tui::IntoText;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Strip **all** ANSI escape sequences (CSI, OSC, bare ESC) from `text`.
///
/// Returns a borrowed `&str` when no escape sequences are present (zero-copy).
///
/// Uses SIMD-accelerated byte scanning via `memchr` to quickly locate ESC
/// characters, then only parses the ANSI sequences at those positions.
/// Plain-text segments are copied in bulk.
pub(crate) fn strip_ansi(text: &str) -> Cow<'_, str> {
    // Fast path: no escape sequences at all — zero-copy borrow
    if memchr::memchr(b'\x1b', text.as_bytes()).is_none() {
        return Cow::Borrowed(text);
    }

    let mut result = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut prev_end = 0usize;
    // Sequential scan: after processing an escape sequence, resume from
    // its end.  This avoids double-processing embedded ESC bytes that
    // were consumed as part of a larger sequence (e.g. an OSC containing
    // internal ESC bytes).
    let mut scan_offset = 0usize;

    while let Some(esc_offset) = memchr::memchr(b'\x1b', &bytes[scan_offset..]) {
        let abs_esc = scan_offset + esc_offset;

        // Copy plain text before this escape sequence
        if abs_esc > prev_end {
            result.push_str(&text[prev_end..abs_esc]);
        }

        // Determine the end of the escape sequence.
        // ESC followed by '[' → CSI; ESC followed by ']' → OSC; else lone ESC.
        let seq_end = if abs_esc + 1 < bytes.len() {
            match bytes[abs_esc + 1] {
                b'[' => {
                    // CSI: ESC [ ... final_byte (0x40-0x7E)
                    skip_csi(&bytes[abs_esc + 2..])
                        .map(|consumed| abs_esc + 2 + consumed)
                        .unwrap_or(bytes.len())
                }
                b']' => {
                    // OSC: ESC ] ... ST (BEL 0x07 or ESC \)
                    skip_osc(&bytes[abs_esc + 2..])
                        .map(|consumed| abs_esc + 2 + consumed)
                        .unwrap_or(bytes.len())
                }
                _ => {
                    // Lone ESC or unrecognized sequence — skip just the ESC
                    abs_esc + 1
                }
            }
        } else {
            // Trailing lone ESC
            abs_esc + 1
        };

        prev_end = seq_end;
        // Resume scanning from after the processed sequence so that any
        // ESC bytes inside a multi-byte sequence (e.g. OSC) are not
        // processed again.
        scan_offset = seq_end;
    }

    // Copy remaining text after last escape sequence
    if prev_end < bytes.len() {
        result.push_str(&text[prev_end..]);
    }

    Cow::Owned(result)
}

/// Skip a CSI sequence (starting after `ESC [` or `0x9B`) and return the number
/// of bytes consumed.  CSI ends at the first byte in range 0x40-0x7E.
///
/// This matches the original permissive behaviour: no validation of parameter
/// bytes is performed, any byte before the final byte is accepted.
fn skip_csi(rest: &[u8]) -> Option<usize> {
    if rest.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < rest.len() {
        let b = rest[i];
        if (0x40..=0x7E).contains(&b) {
            return Some(i + 1); // include the final byte
        }
        i += 1;
    }
    None // truncated CSI (no final byte found)
}

/// Skip an OSC sequence (starting after `ESC ]`) and return the number of
/// bytes consumed.  OSC is terminated by BEL (0x07) or ST (ESC \ 0x1B 0x5C).
fn skip_osc(rest: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            b'\x07' => return Some(i + 1), // BEL terminated
            b'\x1b' => {
                // Check for ST (ESC \) terminator
                if i + 1 < rest.len() && rest[i + 1] == b'\\' {
                    return Some(i + 2);
                }
                // Bare ESC inside OSC — treat as end (malformed)
                return Some(i + 1);
            }
            _ => i += 1,
        }
    }
    None // truncated OSC
}

/// Convert an ANSI-escape-sequence-rich string into styled ratatui lines.
///
/// Each line in the input becomes a `Line<'static>` with foreground/background
/// colours applied from the ANSI codes.  This is the inverse of `strip_ansi`.
pub(crate) fn ansi_to_styled_line(text: &str, default_style: Style) -> Vec<Line<'static>> {
    // ansi_to_tui::IntoText produces one Line per input line.
    match text.into_text() {
        Ok(text) => text
            .lines
            .iter()
            .map(|line| {
                let spans: Vec<Span<'static>> = line
                    .spans
                    .iter()
                    .map(|s| {
                        let combined = default_style.patch(s.style);
                        Span::styled(s.content.clone(), combined)
                    })
                    .collect();
                Line::from(spans)
            })
            .collect(),
        Err(_) => {
            // Fallback: plain text with default style
            text.lines()
                .map(|l| Line::from(Span::styled(l.to_string(), default_style)))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_ansi_returns_borrowed() {
        let text = "Hello, world!";
        let result = strip_ansi(text);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "Hello, world!");
    }

    #[test]
    fn empty_string_returns_borrowed() {
        let result = strip_ansi("");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "");
    }

    #[test]
    fn strips_sgr_csi() {
        // ESC [ <params> m — the most common ANSI sequence
        let result = strip_ansi("\x1b[31mred\x1b[0m");
        assert_eq!(result, "red");
    }

    #[test]
    fn strips_csi_with_complex_parameters() {
        // CSI with multiple numeric params + true color
        let result = strip_ansi("\x1b[38;2;255;100;50mcolored\x1b[0m");
        assert_eq!(result, "colored");
    }

    #[test]
    fn strips_cursor_movement_csi() {
        // CSI cursor positioning
        let result = strip_ansi("line1\x1b[2G\x1b[Kline2");
        assert_eq!(result, "line1line2");
    }

    #[test]
    fn strips_osc_with_bel_terminator() {
        let result = strip_ansi("before\x1b]0;my title\x07after");
        assert_eq!(result, "beforeafter");
    }

    #[test]
    fn strips_osc_with_st_terminator() {
        // ST = ESC \ (0x1B 0x5C)
        let result = strip_ansi("before\x1b]0;my title\x1b\\after");
        assert_eq!(result, "beforeafter");
    }

    #[test]
    fn strips_lone_esc() {
        let result = strip_ansi("a\x1bb");
        assert_eq!(result, "ab");
    }

    #[test]
    fn strips_esc_followed_by_non_bracket() {
        // ESC X (not [ or ]) — lone ESC is skipped, X remains as plain text
        // (next_if_eq does NOT consume the non-matching character)
        let result = strip_ansi("a\x1bXb");
        assert_eq!(result, "aXb");
    }

    #[test]
    fn trailing_lone_esc() {
        let result = strip_ansi("text\x1b");
        assert_eq!(result, "text");
    }

    #[test]
    fn strips_multiple_sequences() {
        let result = strip_ansi("\x1b[1mBold\x1b[0m and \x1b[3mitalic\x1b[0m");
        assert_eq!(result, "Bold and italic");
    }

    #[test]
    fn strips_consecutive_escapes() {
        let result = strip_ansi("\x1b[1m\x1b[31mbold red\x1b[0m\x1b[0m");
        assert_eq!(result, "bold red");
    }

    #[test]
    fn preserves_unicode_text() {
        let result = strip_ansi("Hello \u{4e16}\u{754c} \x1b[31mred\x1b[0m!");
        assert_eq!(result, "Hello \u{4e16}\u{754c} red!");
    }

    #[test]
    fn only_ansi_returns_empty() {
        let result = strip_ansi("\x1b[1;32m");
        assert_eq!(result, "");
    }

    #[test]
    fn trailing_csi_after_last_char() {
        let result = strip_ansi("text\x1b[0m");
        assert_eq!(result, "text");
    }

    #[test]
    fn malformed_csi_no_final_byte() {
        // CSI with only parameter bytes (0x30-0x3F: digits, semicolons)
        // and no final byte in 0x40-0x7E before end of string.
        let result = strip_ansi("a\x1b[0;12;34");
        assert_eq!(result, "a");
    }

    #[test]
    fn csi_with_intermediate_bytes() {
        // CSI with intermediate bytes (space separated params)
        // ESC [ 1 2 SP 3 @ — valid CSI with intermediate
        let result = strip_ansi("a\x1b[1;2 3@b");
        assert_eq!(result, "ab");
    }

    #[test]
    fn osc_with_embedded_esc_inside() {
        // OSC where an ESC appears inside (not followed by \)
        // The ESC inside terminates the OSC; everything from OSC start
        // to that ESC (inclusive) is stripped.
        let result = strip_ansi("a\x1b]0;ti\x1btleb");
        assert_eq!(result, "atleb");
    }
}
