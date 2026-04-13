use ratatui::text::Line;
use std::path::Path;

pub(crate) fn append_markdown(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
    lines: &mut Vec<Line<'static>>,
) {
    let rendered = crate::markdown_render::render_markdown_text_with_width_and_cwd(
        markdown_source,
        width,
        cwd,
    );
    crate::render::line_utils::push_owned_lines(&rendered.lines, lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_to_strings(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.clone())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn append_markdown_renders_heading() {
        let mut out = Vec::new();
        append_markdown("# Title\n", None, None, &mut out);
        assert_eq!(lines_to_strings(&out), vec!["# Title".to_string()]);
    }
}
