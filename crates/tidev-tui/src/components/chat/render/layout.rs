use crate::theme::ThemePalette;
use ratatui::layout::Rect;
use ratatui::prelude::{Frame, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const LEFT_MARGIN: u16 = 2;
pub const SCROLLBAR_WIDTH: u16 = 1;
const GAP: u16 = 1;

// ---------------------------------------------------------------------------
// CardGeom — describes a card's horizontal geometry
// ---------------------------------------------------------------------------

/// Describes a card's horizontal layout: total width, left padding, right padding.
/// Content width = total - left - right.
/// All card rendering functions receive `content_width` (the usable width).
/// `decorate_card_lines` uses the full `CardGeom` to lay out left/right padding.
#[derive(Clone, Copy, Debug)]
pub struct CardGeom {
    pub total: usize,
    pub left: usize,
    pub right: usize,
}

impl CardGeom {
    /// Build a card geometry that fills the given total width with symmetric padding.
    pub fn new(total: usize) -> Self {
        Self {
            total,
            left: 2,
            right: 2,
        }
    }

    /// The usable content width inside the padding.
    pub fn content(&self) -> usize {
        self.total.saturating_sub(self.left + self.right)
    }
}

pub(super) fn compute_content_layout(area: Rect) -> (Rect, Option<Rect>) {
    if area.width > LEFT_MARGIN + GAP + SCROLLBAR_WIDTH {
        let content_width = area.width - LEFT_MARGIN - GAP - SCROLLBAR_WIDTH;
        (
            Rect {
                x: area.x + LEFT_MARGIN,
                y: area.y,
                width: content_width,
                height: area.height,
            },
            Some(Rect {
                x: area.x + area.width - SCROLLBAR_WIDTH,
                y: area.y,
                width: SCROLLBAR_WIDTH,
                height: area.height,
            }),
        )
    } else if area.width > LEFT_MARGIN + SCROLLBAR_WIDTH {
        let content_width = area.width - LEFT_MARGIN - SCROLLBAR_WIDTH;
        (
            Rect {
                x: area.x + LEFT_MARGIN,
                y: area.y,
                width: content_width,
                height: area.height,
            },
            Some(Rect {
                x: area.x + area.width - SCROLLBAR_WIDTH,
                y: area.y,
                width: SCROLLBAR_WIDTH,
                height: area.height,
            }),
        )
    } else if area.width > LEFT_MARGIN {
        (
            Rect {
                x: area.x + LEFT_MARGIN,
                y: area.y,
                width: area.width - LEFT_MARGIN,
                height: area.height,
            },
            None,
        )
    } else {
        (area, None)
    }
}

pub(super) fn render_scrollbar(
    frame: &mut Frame,
    sb: Rect,
    scroll_offset: usize,
    total_lines: usize,
    viewport: usize,
    palette: ThemePalette,
    hovered: bool,
) {
    let bg = if hovered {
        palette.hover_bg(palette.background)
    } else {
        palette.background
    };
    let height = sb.height as usize;

    if total_lines <= viewport || height == 0 {
        let lines: Vec<Line> = (0..height)
            .map(|_| Line::from(Span::styled(" ", Style::default().bg(bg))))
            .collect();
        frame.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), sb);
        return;
    }

    let max_scroll = total_lines.saturating_sub(viewport);
    let scrolled = (scroll_offset as f32 / max_scroll as f32).clamp(0.0, 1.0);
    let thumb_height = ((sb.height as f32 * sb.height as f32 / total_lines.max(1) as f32)
        .clamp(1.0, sb.height as f32))
    .round() as u16;
    let track_span = sb.height.saturating_sub(thumb_height);
    let thumb_pos = if track_span == 0 {
        0
    } else {
        (scrolled * track_span as f32).round() as u16
    };

    let track_style = Style::default().bg(bg).fg(palette.border);
    let thumb_style = Style::default().bg(bg).fg(palette.accent);
    let lines: Vec<Line> = (0..sb.height)
        .map(|row| {
            if row >= thumb_pos && row < thumb_pos + thumb_height {
                Line::from(Span::styled("█", thumb_style))
            } else {
                Line::from(Span::styled("░", track_style))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), sb);
}
