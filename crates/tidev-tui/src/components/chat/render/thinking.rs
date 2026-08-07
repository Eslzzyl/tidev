use super::*;

use std::collections::HashSet;

use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use tidev_llm::message::Message;
use uuid::Uuid;

use crate::hyperlink::HyperlinkLine;
use crate::markdown;

/// Compute the thinking duration string for display.
///
/// Format rules:
/// - < 1s       → "999ms"        (integer milliseconds)
/// - ≥ 1s, <1m  → "42.5s"        (one decimal place)
/// - ≥ 1m, <1h  → "3min 15s"
/// - ≥ 1h       → "1h 5min 30s"
pub(super) fn thinking_duration_str(message: &Message) -> Option<String> {
    let started = message.reasoning_started_at?;
    let ended = message
        .reasoning_completed_at
        .unwrap_or_else(chrono::Utc::now);
    if message.reasoning.trim().is_empty() {
        return None;
    }
    let elapsed = ended - started;
    let total_secs = elapsed.num_seconds().max(0);

    if total_secs >= 3600 {
        let hours = total_secs / 3600;
        let minutes = (total_secs % 3600) / 60;
        let seconds = total_secs % 60;
        Some(format!("{}h {}min {}s", hours, minutes, seconds))
    } else if total_secs >= 60 {
        let minutes = total_secs / 60;
        let seconds = total_secs % 60;
        Some(format!("{}min {}s", minutes, seconds))
    } else if total_secs >= 1 {
        let total_millis = elapsed.num_milliseconds().max(0) as f64;
        Some(format!("{:.1}s", total_millis / 1000.0))
    } else {
        let total_millis = elapsed.num_milliseconds().max(0) as u64;
        Some(format!("{}ms", total_millis))
    }
}

/// Determine whether a message's reasoning content should be collapsed.
pub(super) fn is_reasoning_collapsed(
    msg_id: &Uuid,
    overrides: &HashSet<Uuid>,
    default_collapse: bool,
) -> bool {
    let toggled = overrides.contains(msg_id);
    // Default: if `default_collapse` is true → collapsed.
    // If user toggled, invert the default.
    if toggled {
        !default_collapse
    } else {
        default_collapse
    }
}

/// Render reasoning content with ┃ prefix, dimmed colours, and the
/// Thinking:/Thought: label.  Matches the old implementation exactly.
/// When `collapsed` is true, only the header line is rendered.
/// Hyperlink columns are relative to the line start (before the "┃ " prefix);
/// `decorate_card_lines` shifts them by the visual prefix width.
pub(super) fn render_reasoning_lines(
    ctx: &RenderContext,
    reasoning: &str,
    content_width: usize,
    is_streaming: bool,
    collapsed: bool,
    duration: Option<&str>,
) -> Vec<HyperlinkLine> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    let dimmed_color = crate::theme::mix_colors(palette.muted, palette.background, 0.5);
    let label_style = Style::default().fg(dimmed_color);
    let label_italic_style = Style::default()
        .fg(dimmed_color)
        .add_modifier(Modifier::ITALIC);

    // Label: ┃ Thinking: or ┃ Thought:
    let label = if is_streaming {
        "Thinking:"
    } else {
        "Thought:"
    };

    // Build header with duration and fold indicator
    let mut header_spans = vec![Span::styled("┃ ", label_style)];

    // Duration suffix — shown during streaming once reasoning started,
    // and after the turn is complete.
    let duration_suffix = duration.map(|d| format!(" ({})", d)).unwrap_or_default();

    // Fold indicator: ▶ when collapsed, ▼ when expanded
    let fold_indicator = if collapsed {
        "  ▶"
    } else if !reasoning.trim().is_empty() {
        "  ▼"
    } else {
        ""
    };

    header_spans.push(Span::styled(
        format!("{}{}{}", label, duration_suffix, fold_indicator),
        label_italic_style,
    ));
    lines.push(HyperlinkLine::new(Line::from(header_spans)));

    // If collapsed or empty, stop here
    if collapsed || reasoning.trim().is_empty() {
        return lines;
    }

    let body_style = Style::default().fg(dimmed_color);
    let effective = content_width.saturating_sub(2).max(1); // 2 for ┃ prefix
    let rendered = markdown::render_markdown_text_with_width_and_cwd(
        reasoning,
        Some(effective),
        Some(ctx.workspace_root),
    );

    // Skip leading blank lines
    let mut rendered_lines = markdown::markdown_to_hyperlink_lines(&rendered).into_iter();
    let mut first_line = rendered_lines.next();
    while let Some(ref line) = first_line {
        if line
            .line
            .spans
            .iter()
            .all(|s| s.content.trim().is_empty() && s.style == Style::default())
        {
            first_line = rendered_lines.next();
        } else {
            break;
        }
    }

    // First content line
    if let Some(line) = first_line {
        let mut spans = vec![Span::styled("┃ ", label_style)];
        for mut span in line.line.spans {
            if let Some(fg) = span.style.fg {
                span.style = span
                    .style
                    .fg(crate::theme::mix_colors(fg, palette.background, 0.4));
            } else {
                span.style = span.style.patch(body_style);
            }
            spans.push(span);
        }
        lines.push(HyperlinkLine {
            line: Line::from(spans),
            hyperlinks: line.hyperlinks,
        });
    }

    // Subsequent lines
    for line in rendered_lines {
        let mut spans = vec![Span::styled("┃ ", label_style)];
        for mut span in line.line.spans {
            if let Some(fg) = span.style.fg {
                span.style = span
                    .style
                    .fg(crate::theme::mix_colors(fg, palette.background, 0.4));
            } else {
                span.style = span.style.patch(body_style);
            }
            spans.push(span);
        }
        lines.push(HyperlinkLine {
            line: Line::from(spans),
            hyperlinks: line.hyperlinks,
        });
    }

    lines
}
