use crate::theme::ThemePalette;
use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::hyperlink::HyperlinkLine;
use crate::markdown::{WrapOptions, adaptive_wrap_line};

// ---------------------------------------------------------------------------
// Running subagent summary (used for inline cards)
// ---------------------------------------------------------------------------

pub struct RunningSubagentInfo {
    pub tool_call_id: String,
    pub description: String,
    pub subagent_type: String,
    pub status_text: String,
    pub child_session_id: Option<uuid::Uuid>,
    pub interrupted: bool,
}

#[derive(Clone, Debug)]
pub struct InlineRunningCardRange {
    pub execution_index: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// Render a running subagent inline card — 4+ lines with padding, word-wrapped
/// header, and status line. This mirrors the old v0.6.x implementation.
pub(crate) fn render_running_subagent_lines(
    info: &RunningSubagentInfo,
    content_width: usize,
    palette: ThemePalette,
) -> Vec<HyperlinkLine> {
    let mut lines = Vec::new();

    // Top padding
    lines.push(HyperlinkLine::new(Line::from("")));

    // Header: @type subagent: description (word-wrapped)
    let description = info.description.trim();
    let header_line = Line::from(vec![
        Span::styled(
            format!("@{}", info.subagent_type),
            Style::default().fg(palette.accent_soft),
        ),
        Span::styled(" subagent: ", Style::default().fg(palette.muted)),
        Span::styled(
            description.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    lines.extend(
        adaptive_wrap_line(
            &header_line,
            WrapOptions::new(content_width).break_words(true),
        )
        .into_iter()
        .map(|l| {
            HyperlinkLine::new(Line::from(
                l.spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style))
                    .collect::<Vec<_>>(),
            ))
        }),
    );

    // Status line with 2-space indent
    lines.push(HyperlinkLine::new(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            info.status_text.clone(),
            Style::default().fg(palette.accent_soft),
        ),
    ])));

    // Bottom padding
    lines.push(HyperlinkLine::new(Line::from("")));

    lines
}

/// Count how many visual lines a running subagent card will occupy.
/// Must stay in sync with render_running_subagent_lines.
pub(crate) fn count_running_subagent_card_lines(
    info: &RunningSubagentInfo,
    content_width: usize,
) -> usize {
    let mut count = 0;
    // Top padding
    count += 1;
    // Header line (word-wrapped)
    let header_text = format!(
        "@{} subagent: {}",
        info.subagent_type,
        info.description.trim()
    );
    count += adaptive_wrap_line(
        &Line::from(header_text),
        WrapOptions::new(content_width).break_words(true),
    )
    .len();
    // Status line
    count += 1;
    // Bottom padding
    count += 1;
    count
}
