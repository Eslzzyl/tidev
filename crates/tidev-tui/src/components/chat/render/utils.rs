use super::*;

use std::sync::LazyLock;
use std::time::Instant;

use crate::hyperlink::HyperlinkLine;
use crate::markdown::WrapOptions;
use crate::theme::ThemePalette;
use fancy_regex::Regex;
use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use uuid::Uuid;

use crate::components::chat::render_cache::SelectableRegionRange;

/// Information about an image badge found in a rendered user message card.
/// Used for mouse hit-testing to open the ImageViewer overlay.
#[derive(Clone, Debug)]
pub(crate) struct ImageBadgeInfo {
    /// Absolute line number where the card starts in the render output.
    pub card_start_line: usize,
    /// Line offset within the card (0-indexed).
    pub badge_line_offset: usize,
    /// Column offset of the badge within the line.
    pub badge_col: usize,
    /// Width of the badge text in columns.
    pub badge_width: usize,
    /// The message that contains the image attachment.
    pub message_id: Uuid,
    /// Index into the message's Image attachments.
    pub attachment_index: usize,
}

// ---------------------------------------------------------------------------
// Badge regex patterns for user-message content
// ---------------------------------------------------------------------------

/// Regex for detecting @ file/directory references.
/// Look-behind ensures @ is not preceded by word chars or backticks.
static AT_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?<![\w`])@(\.?[^\s`.,]*(?:\.[^\s`.,]+)*)").unwrap());

/// Regex for image badge patterns like `[100.0 KB PNG]` produced by
/// `format_image_badge()`. The type label is uppercase (PNG, JPEG, etc.).
pub(super) static IMAGE_BADGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\d[\d.]*\s+(?:B|KB|MB|GB)\s+[A-Z][A-Z0-9]*\]").unwrap());

/// Kind of inline badge detected in user message content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageBadgeKind {
    AtReference,
    Image,
}

pub(super) fn decorate_card_lines(
    lines: Vec<HyperlinkLine>,
    bg: Color,
    geom: &CardGeom,
) -> Vec<HyperlinkLine> {
    let bg_style = Style::default().bg(bg);
    let left_prefix = " ".repeat(geom.left);
    lines
        .into_iter()
        .map(|mut line| {
            let has_visual_prefix = line.line.spans.first().is_some_and(|s| s.content == "┃ ");
            // Hyperlink columns are relative to the content line start; the
            // prepended padding (or the existing "┃ " visual prefix) shifts
            // every column right by its width.
            let shift = if has_visual_prefix {
                UnicodeWidthStr::width("┃ ")
            } else {
                geom.left
            };
            let mut spans = if has_visual_prefix {
                Vec::with_capacity(line.line.spans.len() + 1)
            } else {
                vec![Span::styled(left_prefix.clone(), bg_style)]
            };
            for mut span in line.line.spans {
                if span.style.bg.is_none() {
                    span.style = span.style.bg(bg);
                }
                spans.push(span);
            }
            // Explicit right padding
            let used: usize = spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            let remaining = geom.total.saturating_sub(used);
            if remaining > 0 {
                spans.push(Span::styled(" ".repeat(remaining), bg_style));
            }
            line.line = Line::from(spans);
            if shift > 0 {
                for hyperlink in &mut line.hyperlinks {
                    hyperlink.columns =
                        hyperlink.columns.start + shift..hyperlink.columns.end + shift;
                }
            }
            line
        })
        .collect()
}

pub(super) fn track_selectable_region(
    regions: &mut Vec<SelectableRegionRange>,
    card_lines: &[HyperlinkLine],
    start_line: usize,
) {
    let first_content = card_lines
        .iter()
        .position(|l| l.line.spans.iter().any(|s| !s.content.is_empty()));
    let last_content = card_lines
        .iter()
        .rposition(|l| l.line.spans.iter().any(|s| !s.content.is_empty()));
    if let (Some(first), Some(last)) = (first_content, last_content) {
        regions.push(SelectableRegionRange {
            start_line: start_line + first,
            end_line: start_line + last + 1,
            min_x: 2,
            max_x: None,
        });
    }
}

pub(super) fn build_header_lines(is_subsession: bool, palette: ThemePalette) -> Vec<HyperlinkLine> {
    if is_subsession {
        vec![
            HyperlinkLine::new(Line::from(Span::styled(
                "SUBSESSION active — viewing a child session.",
                Style::default().fg(palette.accent_soft),
            ))),
            HyperlinkLine::new(Line::from(Span::styled(
                "Press Ctrl+X then Up arrow to return to the parent session.",
                Style::default().fg(palette.muted),
            ))),
            HyperlinkLine::new(Line::from("")),
        ]
    } else {
        Vec::new()
    }
}

pub(super) fn loading_spinner(spinner_start: Instant) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    const FRAME_DURATION_MS: u128 = 100;
    let elapsed = spinner_start.elapsed().as_millis();
    let frame_index = (elapsed / FRAME_DURATION_MS) as usize;
    FRAMES[frame_index % FRAMES.len()]
}

/// Expand tab characters to spaces using configurable tab stops.
/// `unicode-width` measures `\t` as 0 columns, but terminals render it as
/// multiple spaces. Expanding tabs at parse time prevents width mismatches.
fn expand_tabs(text: &str, tab_width: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len() + tab_width);
    let mut col = 0usize;
    for ch in text.chars() {
        if ch == '\t' {
            let spaces = tab_width - (col % tab_width);
            result.extend(std::iter::repeat_n(' ', spaces));
            col += spaces;
        } else {
            result.push(ch);
            col += 1;
        }
    }
    result
}

/// Return the display width (in terminal columns) of `s`.
fn char_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate `s` to fit within `max_width` columns and append `…` if truncated.
fn shorten_by_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let total = char_width(s);
    if total <= max_width {
        return s.to_string();
    }
    let ellipsis = '…';
    let ellipsis_width = UnicodeWidthChar::width(ellipsis).unwrap_or(1);
    let target = max_width.saturating_sub(ellipsis_width);
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > target {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out.push(ellipsis);
    out
}

/// Wrap `text` into at most `max_lines` lines of `max_width` columns each.
/// Newlines are collapsed into spaces. Word boundaries are preferred for
/// line breaks; URL-like tokens are kept intact whenever they fit on a line
/// (an over-wide URL is hard-broken at the width boundary so no line exceeds
/// `max_width`); hard-breaks are used when a single word exceeds max_width.
/// When content is truncated by `max_lines`, the last line is shortened with
/// an ellipsis.
pub fn wrap_text_lines(text: &str, max_width: usize, max_lines: usize) -> Vec<String> {
    if max_width == 0 || max_lines == 0 {
        return vec![];
    }

    let normalized: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let normalized = expand_tabs(&normalized, 4);
    let trimmed = normalized.trim();

    if trimmed.is_empty() {
        return vec!["".to_string()];
    }

    let line = Line::from(trimmed.to_string());
    let wrapped = crate::markdown::adaptive_wrap_line(
        &line,
        WrapOptions::new(max_width).break_words(true),
    );
    let mut lines: Vec<String> = wrapped
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();

    if lines.len() > max_lines {
        lines.truncate(max_lines);
        // Content continues beyond the kept lines: mark the truncation with
        // an ellipsis.
        let last = &mut lines[max_lines - 1];
        *last = shorten_by_width(&format!("{last} …"), max_width);
    }

    if lines.is_empty() {
        lines.push("".to_string());
    }

    lines
}

/// Post-process rendered markdown lines to replace badge text with styled spans.
/// Scans each span for `@path` and `[size TYPE]` patterns and splits the span
/// at badge boundaries, applying bold accent for AtReference and white-on-teal
/// for Image badges. Hyperlink ranges are unaffected (spans are only split).
pub(super) fn apply_badge_styling(lines: &mut [HyperlinkLine], palette: ThemePalette) {
    for line in lines.iter_mut() {
        let old_spans: Vec<Span<'static>> = line.line.spans.drain(..).collect();
        for span in old_spans {
            let text = span.content.to_string();
            let mut parts: Vec<(String, Style)> = Vec::new();
            let mut offset = 0usize;

            let mut badges_in_span: Vec<(usize, usize, MessageBadgeKind)> = Vec::new();

            // @ references
            {
                let mut search_start = 0;
                while let Ok(Some(caps)) = AT_REF_RE.captures(&text[search_start..]) {
                    if let Some(path_match) = caps.get(1) {
                        if path_match.as_str().is_empty() {
                            break;
                        }
                        let abs_start = search_start + path_match.start() - 1;
                        let abs_end = search_start + path_match.end();
                        badges_in_span.push((abs_start, abs_end, MessageBadgeKind::AtReference));
                        search_start += path_match.end();
                    } else {
                        break;
                    }
                }
            }

            // Image badge patterns like `[100KB PNG]`
            {
                let mut search_start = 0;
                while let Ok(Some(m)) = IMAGE_BADGE_RE.find(&text[search_start..]) {
                    let abs_start = search_start + m.start();
                    let abs_end = search_start + m.end();
                    badges_in_span.push((abs_start, abs_end, MessageBadgeKind::Image));
                    search_start += m.end();
                }
            }

            badges_in_span.sort_by_key(|b| b.0);

            if badges_in_span.is_empty() {
                parts.push((text, span.style));
            } else {
                for (start, end, kind) in &badges_in_span {
                    if *start > offset {
                        parts.push((text[offset..*start].to_string(), span.style));
                    }
                    let badge_style = match kind {
                        MessageBadgeKind::AtReference => Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                        MessageBadgeKind::Image => Style::default()
                            .bg(palette.selection_bg)
                            .fg(palette.selection_fg)
                            .add_modifier(Modifier::BOLD),
                    };
                    parts.push((text[*start..*end].to_string(), badge_style));
                    offset = *end;
                }
                if offset < text.len() {
                    parts.push((text[offset..].to_string(), span.style));
                }
            }

            for (content, style) in parts {
                line.line.spans.push(Span::styled(content, style));
            }
        }
    }
}

/// Render a centered divider line with the compaction label, e.g.
/// `─── COMPACTED ───`.
pub(super) fn render_compaction_divider_line(
    label: &str,
    width: usize,
    palette: ThemePalette,
) -> HyperlinkLine {
    let label_width = UnicodeWidthStr::width(label);
    if width <= label_width.saturating_add(2) {
        return HyperlinkLine::new(Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(palette.accent_soft),
        )));
    }

    let remaining = width - label_width - 2;
    let left = remaining / 2;
    let right = remaining - left;

    let mut spans = Vec::new();
    if left > 0 {
        spans.push(Span::styled(
            "─".repeat(left),
            Style::default().fg(palette.muted),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        label.to_string(),
        Style::default().fg(palette.accent_soft),
    ));
    spans.push(Span::raw(" "));
    if right > 0 {
        spans.push(Span::styled(
            "─".repeat(right),
            Style::default().fg(palette.muted),
        ));
    }

    HyperlinkLine::new(Line::from(spans))
}
