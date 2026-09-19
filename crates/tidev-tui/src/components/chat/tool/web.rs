use super::webfetch::strip_webfetch_content;
use super::*;

use crate::hyperlink::HyperlinkLine;
use crate::i18n::{TextKey, UiText};
use crate::markdown::{
    markdown_to_hyperlink_lines, render_markdown_text_with_width_and_cwd_with_ui,
};

// ---------------------------------------------------------------------------
// Web search result rendering
// ---------------------------------------------------------------------------

pub(super) fn render_websearch_result_lines(
    output: &str,
    content_width: usize,
    palette: ThemePalette,
    ui_text: &UiText,
    is_expanded: bool,
    is_error: bool,
) -> Vec<HyperlinkLine> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(HyperlinkLine::new(Line::from(Span::styled(
            ui_text.text(TextKey::NoMatches),
            Style::default().fg(palette.muted),
        ))));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(
            output,
            None,
            content_width,
            palette,
            ui_text,
            is_expanded,
            true,
        );
    }

    lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
        ui_text.text(TextKey::Search),
        Style::default().fg(palette.accent_soft),
    )])));
    lines.push(HyperlinkLine::new(Line::from("")));

    let rendered =
        render_markdown_text_with_width_and_cwd_with_ui(output, Some(content_width), None, ui_text);
    let md_lines: Vec<HyperlinkLine> = markdown_to_hyperlink_lines(&rendered);

    if is_expanded {
        let has_lines = !md_lines.is_empty();
        lines.extend(md_lines);
        if has_lines {
            lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
                ui_text.text(TextKey::ClickToCollapse),
                Style::default().fg(palette.muted),
            )])));
        }
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
                format!(
                    "  {}",
                    ui_text.text_with_value(
                        TextKey::MoreLinesClickExpand,
                        "count",
                        &(line_count - max_preview).to_string(),
                    )
                ),
                Style::default().fg(palette.muted),
            )])));
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
    ui_text: &UiText,
    is_expanded: bool,
    is_error: bool,
) -> Vec<HyperlinkLine> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(HyperlinkLine::new(Line::from(Span::styled(
            ui_text.text(TextKey::NoOutput),
            Style::default().fg(palette.muted),
        ))));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(
            output,
            None,
            content_width,
            palette,
            ui_text,
            is_expanded,
            true,
        );
    }

    lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
        ui_text.text(TextKey::Output),
        Style::default().fg(palette.accent_soft),
    )])));
    lines.push(HyperlinkLine::new(Line::from("")));

    // Strip line-number prefixes and metadata footers for clean TUI display
    let clean = strip_webfetch_content(output);
    let rendered =
        render_markdown_text_with_width_and_cwd_with_ui(&clean, Some(content_width), None, ui_text);
    let md_lines: Vec<HyperlinkLine> = markdown_to_hyperlink_lines(&rendered);

    if is_expanded {
        let has_lines = !md_lines.is_empty();
        lines.extend(md_lines);
        if has_lines {
            lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
                ui_text.text(TextKey::ClickToCollapse),
                Style::default().fg(palette.muted),
            )])));
        }
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(HyperlinkLine::new(Line::from(vec![Span::styled(
                format!(
                    "  {}",
                    ui_text.text_with_value(
                        TextKey::MoreLinesClickExpand,
                        "count",
                        &(line_count - max_preview).to_string(),
                    )
                ),
                Style::default().fg(palette.muted),
            )])));
        }
    }

    lines
}
