//! Utility functions copied from tidev-tui (private, self-contained).

use ratatui::layout::Rect;
use ratatui::prelude::{Frame, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tidev_tui::theme::ThemePalette;

/// Expand tab characters to spaces.
pub(crate) fn expand_tabs(text: &str, tab_width: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len() + tab_width);
    let mut col = 0usize;
    for ch in text.chars() {
        match ch {
            '\t' => {
                let spaces = tab_width - (col % tab_width);
                for _ in 0..spaces {
                    result.push(' ');
                }
                col += spaces;
            }
            '\n' => {
                result.push(ch);
                col = 0;
            }
            _ => {
                result.push(ch);
                col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            }
        }
    }
    result
}

/// Compute a centred rect of the given dimensions inside `area`.
pub(crate) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(8);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect::new(x, y, width, height)
}

/// Render a 1-column vertical scrollbar.
pub(crate) fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    scroll: usize,
    content_height: usize,
    palette: ThemePalette,
    hovered: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let bg = if hovered {
        palette.hover_bg(palette.background)
    } else {
        palette.background
    };

    let track_style = Style::default().bg(bg).fg(palette.border);
    let thumb_style = Style::default().bg(bg).fg(palette.accent);
    let height = area.height as usize;
    let mut lines = Vec::with_capacity(height);

    if content_height <= height || height == 0 {
        for _ in 0..height {
            lines.push(Line::from(vec![Span::styled(" ", track_style)]));
        }
    } else {
        let max_scroll = content_height.saturating_sub(height);
        let thumb_height = ((height * height) / content_height.max(1))
            .clamp(1, height)
            .max(1);
        let track_span = height.saturating_sub(thumb_height);
        let thumb_top = if track_span == 0 {
            0
        } else {
            ((scroll as f32 / max_scroll as f32) * track_span as f32).round() as usize
        };

        for row in 0..height {
            let is_thumb = row >= thumb_top && row < thumb_top + thumb_height;
            let style = if is_thumb { thumb_style } else { track_style };
            let glyph = if is_thumb { "█" } else { "░" };
            lines.push(Line::from(vec![Span::styled(glyph, style)]));
        }
    }

    let paragraph = Paragraph::new(lines).style(Style::default().bg(bg));
    frame.render_widget(paragraph, area);
}

/// Strip `<system-reminder>...</system-reminder>` XML-like tags from text.
///
/// These tags are injected by the LLM prompt template to mark system instructions
/// and should not be shown in the UI. Also strips trailing whitespace/newlines
/// after the closing tag.
pub(crate) fn strip_system_reminder_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        if let Some(start) = rest.find("<system-reminder") {
            // Push content before the tag
            result.push_str(&rest[..start]);
            // Find the closing tag
            if let Some(end) = rest[start..].find("</system-reminder>") {
                let after_close = start + end + "</system-reminder>".len();
                rest = &rest[after_close..];
                // Skip trailing whitespace/newlines after the closing tag
                while rest.starts_with('\n') || rest.starts_with('\r') || rest.starts_with(' ') {
                    rest = &rest[1..];
                }
            } else {
                // No closing tag — keep the rest as-is
                result.push_str(&rest[start..]);
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result
}
