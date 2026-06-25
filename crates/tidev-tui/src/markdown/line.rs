use ratatui::text::{Line, Span};

pub fn line_to_static(line: &Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .iter()
            .map(|span| Span {
                style: span.style,
                content: std::borrow::Cow::Owned(span.content.to_string()),
            })
            .collect(),
    }
}

pub fn push_owned_lines<'a>(src: &[Line<'a>], out: &mut Vec<Line<'static>>) {
    for line in src {
        out.push(line_to_static(line));
    }
}
