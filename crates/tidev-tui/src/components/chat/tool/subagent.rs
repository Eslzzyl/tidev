use super::*;

use crate::hyperlink::HyperlinkLine;
use crate::markdown::markdown_to_hyperlink_lines;

// ---------------------------------------------------------------------------
// Subagent task preview
// ---------------------------------------------------------------------------

pub(super) fn render_subagent_task_preview(
    output: &str,
    content_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    description: &str,
    subagent_type: &str,
) -> Vec<HyperlinkLine> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(HyperlinkLine::new(Line::from(Span::styled(
            "(empty result)",
            Style::default().fg(palette.muted),
        ))));
        return lines;
    }

    // Top padding
    lines.push(HyperlinkLine::new(Line::from("")));

    // Header: [@type] subagent: description
    let header_line = Line::from(vec![
        Span::styled(
            format!("@{}", subagent_type),
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
        word_wrap_line(
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
    lines.push(HyperlinkLine::new(Line::from("")));

    // Render output as markdown
    let rendered = render_markdown_text_with_width_and_cwd(output, Some(content_width), None);
    let md_lines: Vec<HyperlinkLine> = markdown_to_hyperlink_lines(&rendered);

    if is_expanded {
        lines.extend(md_lines);
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
                format!("   {} more line(s)", line_count - max_preview),
                Style::default().fg(palette.muted),
            )])));
        }
    }

    // Bottom padding
    lines.push(HyperlinkLine::new(Line::from("")));

    lines
}
