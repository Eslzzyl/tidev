//! Utility functions copied from tidev-tui (private, self-contained).

use ratatui::layout::Rect;
use ratatui::prelude::{Frame, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::ThemePalette;

/// Try to read text from the system clipboard.
///
/// Returns `None` when the clipboard is unavailable or doesn't contain text.
/// This is the single shared entry point for paste — all components (RenameDialog,
/// ConnectDialog, Composer) call this instead of reaching into `arboard`
/// directly.
pub(crate) fn paste_from_clipboard() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok().filter(|t| !t.is_empty())
}

/// Try to read an image from the system clipboard.
///
/// Returns `(filename, mime, data, file_size)` on success:
/// - `filename`: display name for the attachment
/// - `mime`: MIME type (e.g. `"image/png"`)
/// - `data`: raw encoded bytes (PNG)
/// - `file_size`: size in bytes
///
/// This consumes the clipboard — call it only after `paste_from_clipboard()`
/// returns `None` (i.e. clipboard does not contain text).
pub(crate) fn paste_image_from_clipboard() -> Option<(String, String, Vec<u8>, u64)> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let img = clipboard.get_image().ok()?;

    // Encode RGBA bytes as PNG.
    let rgba =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
    let mut png_bytes = Vec::new();
    rgba.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageFormat::Png,
    )
    .ok()?;

    let file_size = png_bytes.len() as u64;
    Some((
        format!("clipboard_{}x{}.png", img.width, img.height),
        "image/png".to_string(),
        png_bytes,
        file_size,
    ))
}

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

/// Compute a bottom-aligned rect inside `area`.
///
/// The rect fills the full width of `area` (no horizontal centering) and is
/// anchored to the bottom.  This matches the old TUI behaviour where
/// workspace-boundary, sensitive-file, and question dialogs fill the composer
/// area width instead of being centred.
pub(crate) fn bottom_centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(8);

    let x = area.x + 2;
    let y = area.y + area.height.saturating_sub(height).saturating_sub(1);

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

/// Pretty-print JSON tool arguments for display in the permission dialog.
pub(crate) fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}

/// Token count units: K (thousand), M (million), B (billion), T (trillion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TokenUsage {
    /// Input tokens (prompt tokens)
    pub input_tokens: u32,
    /// Output tokens (completion tokens)
    pub output_tokens: u32,
    /// Cache read tokens (cached prompt tokens)
    pub cache_read_tokens: u32,
    /// Cache write tokens (cache creation tokens)
    pub cache_write_tokens: u32,
}

impl TokenUsage {
    /// Total tokens (input + output)
    pub fn total(&self) -> u64 {
        self.input_tokens as u64 + self.output_tokens as u64
    }

    /// Create from individual values
    pub fn new(
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        }
    }

    /// Add two token usages together.
    pub fn add(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
    }
}

/// Format a token count with appropriate unit suffix.
pub(crate) fn format_token_count(count: u64) -> String {
    if count >= 1_000_000_000_000 {
        let value = count as f64 / 1_000_000_000_000.0;
        format!("{:.1}T", value)
    } else if count >= 1_000_000_000 {
        let value = count as f64 / 1_000_000_000.0;
        format!("{:.1}B", value)
    } else if count >= 1_000_000 {
        let value = count as f64 / 1_000_000.0;
        format!("{:.1}M", value)
    } else if count >= 1_000 {
        let value = count as f64 / 1_000.0;
        format!("{:.1}K", value)
    } else {
        count.to_string()
    }
}

/// Format a token count, accepting u32 input.
#[allow(dead_code)]
pub(crate) fn format_token_count_u32(count: u32) -> String {
    format_token_count(count as u64)
}

/// Shorten a string to fit within max_chars, appending "..." if truncated.
pub(crate) fn shorten(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
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
