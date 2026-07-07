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
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // CSI sequences: ESC [ ... final_byte (0x40-0x7E)
            if chars.next_if_eq(&'[').is_some() {
                for c in chars.by_ref() {
                    if c.is_ascii() && c as u8 >= 0x40 && c as u8 <= 0x7E {
                        break;
                    }
                }
            } else {
                // OSC sequences: ESC ] ... ST (ESC \ or BEL)
                if chars.next_if_eq(&']').is_some() {
                    for c in chars.by_ref() {
                        if c == '\x07' {
                            // BEL terminated
                            break;
                        }
                        if c == '\x1b' {
                            // Possibly ESC \ terminated — consume backslash if present
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                // Lone ESC — ignore
            }
        } else {
            result.push(ch);
        }
    }
    Cow::Owned(result)
}
