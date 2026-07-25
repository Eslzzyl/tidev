use super::*;

// ---------------------------------------------------------------------------
// Question result pairs rendering
// ---------------------------------------------------------------------------

pub(super) fn render_question_result_pairs(
    output: &str,
    content_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![Span::styled(
        "Questions & Answers",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "(no output)",
            Style::default().fg(palette.muted),
        )));
        return lines;
    }

    let mut lines_iter = output.lines().peekable();
    while let Some(q_line) = lines_iter.next() {
        if q_line.trim().is_empty() {
            continue;
        }

        let question_text: String = q_line
            .strip_prefix("Q")
            .and_then(|rest| rest.split_once(':').map(|x| x.1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| q_line.to_string());

        let answer_text = lines_iter
            .next()
            .and_then(|a_line| a_line.strip_prefix("A: "))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let q_line_owned = Line::from(question_text.clone());
        let q_wrapped = word_wrap_line(
            &q_line_owned,
            WrapOptions::new(content_width)
                .initial_indent(Line::from(vec![Span::styled(
                    "  Q: ",
                    Style::default()
                        .fg(palette.accent_soft)
                        .add_modifier(Modifier::BOLD),
                )]))
                .subsequent_indent(Line::from(vec![Span::styled(
                    "     ",
                    Style::default().fg(palette.text),
                )])),
        );
        for wl in q_wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.fg(palette.text);
                }
            }
            lines.push(Line::from(owned_spans));
        }

        let a_line_owned = Line::from(answer_text.clone());
        let a_wrapped = word_wrap_line(
            &a_line_owned,
            WrapOptions::new(content_width)
                .initial_indent(Line::from(vec![Span::styled(
                    "  → ",
                    Style::default().fg(palette.success),
                )]))
                .subsequent_indent(Line::from(vec![Span::styled(
                    "     ",
                    Style::default().fg(palette.text),
                )])),
        );
        for wl in a_wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.fg(palette.success).add_modifier(Modifier::BOLD);
                }
            }
            lines.push(Line::from(owned_spans));
        }

        lines.push(Line::from(""));
    }

    lines
}
