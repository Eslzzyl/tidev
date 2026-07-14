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
