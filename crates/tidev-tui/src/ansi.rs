use std::borrow::Cow;

use ansi_to_tui::IntoText;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Strip **all** ANSI escape sequences (CSI, OSC, bare ESC) from `text`.
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
            consume_escape(&mut chars);
        } else {
            result.push(c);
        }
    }
    Cow::Owned(result)
}

/// Strip **only OSC** escape sequences from `text`, preserving CSI (SGR)
/// sequences for downstream parsing by `ansi-to-tui`.
///
/// OSC format: `ESC ] <content> BEL` or `ESC ] <content> ST (ESC \)`
///
/// Returns a borrowed `&str` when no OSC sequences are present (zero-copy).
pub(crate) fn strip_osc(text: &str) -> Cow<'_, str> {
    if !text.contains("\x1b]") {
        return Cow::Borrowed(text);
    }
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&']') {
            chars.next(); // consume ']'
            // Consume until BEL or ST.
            loop {
                match chars.next() {
                    Some('\x07') => break,
                    Some('\x1b') if chars.peek() == Some(&'\\') => {
                        chars.next(); // consume '\'
                        break;
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
        } else {
            result.push(c);
        }
    }
    Cow::Owned(result)
}

/// Consume an ANSI escape sequence from the iterator.
///
/// The `\x1b` byte MUST have already been peeked but NOT consumed.
/// After this call the iterator is positioned after the entire sequence.
///
/// Handles CSI (`ESC [`), OSC (`ESC ]` with BEL or ST terminator),
/// and bare/unrecognized ESC.
fn consume_escape(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) {
    match chars.peek() {
        Some(&'[') => {
            // CSI: ESC [ <parameter bytes 0x30-0x3F>
            //            <intermediate bytes 0x20-0x2F>
            //            <final byte 0x40-0x7E>
            chars.next(); // consume '['
            for inner in chars.by_ref() {
                if (0x40..=0x7E).contains(&(inner as u8)) {
                    break;
                }
            }
        }
        Some(&']') => {
            // OSC: ESC ] <content> BEL (0x07) or ST (ESC \)
            chars.next(); // consume ']'
            loop {
                match chars.next() {
                    Some('\x07') => break,
                    Some('\x1b') if chars.peek() == Some(&'\\') => {
                        chars.next(); // consume '\'
                        break;
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
        }
        _ => {
            // Bare/unrecognized escape — nothing more to skip.
        }
    }
}

/// Parse a line of text that may contain ANSI escape sequences into a
/// ratatui `Line` with styled spans. If no ANSI codes are present, returns
/// a single span styled with `default_style` (fast path, zero allocation
/// beyond the string copy).
///
/// OSC sequences (including hyperlinks) are stripped before parsing because
/// `ansi-to-tui` only supports CSI/SGR sequences and mishandles
/// ST-terminated OSC by consuming visible text into the garbage.
///
/// On parse error the input is returned as a single uncolored span.
pub(crate) fn ansi_to_styled_line(text: &str, default_style: Style) -> Line<'static> {
    if !text.contains('\x1b') {
        return Line::from(Span::styled(text.to_string(), default_style));
    }
    let cleaned = strip_osc(text);
    match cleaned.as_ref().into_text() {
        Ok(parsed) => parsed.lines.into_iter().next().unwrap_or_default(),
        Err(_) => Line::from(Span::styled(text.to_string(), default_style)),
    }
}
