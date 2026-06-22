use std::borrow::Cow;

use ansi_to_tui::IntoText;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Parse a line of text that may contain ANSI escape sequences into a
/// ratatui `Line` with styled spans. If no ANSI codes are present, returns
/// a single span styled with `default_style` (fast path, zero allocation
/// beyond the string copy).
///
/// On parse error the input is returned as a single uncolored span.
pub(crate) fn ansi_to_styled_line(text: &str, default_style: Style) -> Line<'static> {
    if !text.contains('\x1b') {
        return Line::from(Span::styled(text.to_string(), default_style));
    }
    match text.into_text() {
        Ok(parsed) => parsed.lines.into_iter().next().unwrap_or_default(),
        Err(_) => Line::from(Span::styled(text.to_string(), default_style)),
    }
}

/// Strip ANSI CSI escape sequences (e.g. `\x1b[32m`) from `text`.
///
/// Returns a borrowed `&str` when no escape sequences are present (zero-copy).
pub(crate) fn strip_ansi(text: &str) -> Cow<'_, str> {
    if !text.contains('\x1b') {
        return Cow::Borrowed(text);
    }
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI sequence: ESC [ <params> <final byte>
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Parameter bytes 0x30-0x3F, intermediate bytes 0x20-0x2F
                // Final byte 0x40-0x7E terminates the sequence.
                for inner in chars.by_ref() {
                    if (0x40..=0x7E).contains(&(inner as u8)) {
                        break;
                    }
                }
            }
            // Other escape types (OSC, etc.) are rare in tool output and
            // silently dropped here.
        } else {
            result.push(c);
        }
    }
    Cow::Owned(result)
}
