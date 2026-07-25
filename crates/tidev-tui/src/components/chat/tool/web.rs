use super::*;

// ---------------------------------------------------------------------------
// Web search result rendering
// ---------------------------------------------------------------------------

pub(super) fn render_websearch_result_lines(
    output: &str,
    content_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "(no results)",
            Style::default().fg(palette.muted),
        )));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(output, content_width, palette, is_expanded, true);
    }

    lines.push(Line::from(vec![Span::styled(
        "Search Results",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    let rendered = render_markdown_text_with_width_and_cwd(output, Some(content_width), None);
    let md_lines: Vec<Line<'static>> = rendered.lines.clone();

    if is_expanded {
        let has_lines = !md_lines.is_empty();
        lines.extend(md_lines);
        if has_lines {
            lines.push(Line::from(vec![Span::styled(
                "▲ Click to collapse",
                Style::default().fg(palette.muted),
            )]));
        }
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  ▼ {} more line(s) — Click to expand",
                    line_count - max_preview
                ),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Web fetch result rendering
// ---------------------------------------------------------------------------

pub(super) fn render_webfetch_result_lines(
    output: &str,
    content_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "(empty page)",
            Style::default().fg(palette.muted),
        )));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(output, content_width, palette, is_expanded, true);
    }

    lines.push(Line::from(vec![Span::styled(
        "Page Content",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    let rendered = render_markdown_text_with_width_and_cwd(output, Some(content_width), None);
    let md_lines: Vec<Line<'static>> = rendered.lines.clone();

    if is_expanded {
        let has_lines = !md_lines.is_empty();
        lines.extend(md_lines);
        if has_lines {
            lines.push(Line::from(vec![Span::styled(
                "▲ Click to collapse",
                Style::default().fg(palette.muted),
            )]));
        }
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  ▼ {} more line(s) — Click to expand",
                    line_count - max_preview
                ),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    lines
}
