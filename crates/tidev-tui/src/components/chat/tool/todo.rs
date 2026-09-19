use super::*;
use crate::i18n::{TextKey, UiText};

// ---------------------------------------------------------------------------
// Todos checkbox list rendering
// ---------------------------------------------------------------------------

pub(super) struct TodoItem {
    pub(super) content: String,
    pub(super) status: String,
}

pub(super) fn render_todos_checkbox_list(
    todos: &[TodoItem],
    content_width: usize,
    palette: ThemePalette,
    ui_text: &UiText,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if todos.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", ui_text.text(TextKey::NoItems)),
            Style::default().fg(palette.muted),
        )));
        return lines;
    }

    for todo in todos {
        let (checkbox, style) = match todo.status.as_str() {
            "completed" => (
                "x ",
                Style::default()
                    .fg(palette.muted)
                    .add_modifier(Modifier::CROSSED_OUT),
            ),
            "in_progress" => (
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            _ => ("○ ", Style::default().fg(palette.text)),
        };

        let checkbox_prefix = format!("  {}", checkbox);
        let cb_width = UnicodeWidthStr::width(checkbox_prefix.as_str());
        let indent = " ".repeat(cb_width);

        let content_line = Line::from(todo.content.clone());
        let wrapped = word_wrap_line(
            &content_line,
            WrapOptions::new(content_width)
                .initial_indent(Line::from(vec![Span::styled(checkbox_prefix, style)]))
                .subsequent_indent(Line::from(vec![Span::styled(indent, Style::default())])),
        );

        for wl in wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.patch(style);
                }
            }
            lines.push(Line::from(owned_spans));
        }
    }

    lines
}
