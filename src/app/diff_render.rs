use crate::markdown_render::{WrapOptions, adaptive_wrap_lines, highlight_code_to_lines_for_path};
use crate::theme::{ThemeName, ThemePalette};
use diffy::{Line as DiffLine, Patch};
use ratatui::{
    prelude::{Modifier, Style},
    text::{Line, Span},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiffLineKind {
    Context,
    Delete,
    Insert,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiffLayout {
    Wide,
    Narrow,
}

#[derive(Clone)]
struct DiffCell {
    text: String,
    kind: DiffLineKind,
}

#[derive(Clone)]
struct DiffRow {
    kind: RowKind,
    left: Option<DiffCell>,
    right: Option<DiffCell>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Blank,
    Context,
    Removed,
    Added,
    Modified,
}

const WIDE_LAYOUT_THRESHOLD: usize = 100;
const DARK_ADD_BG: (u8, u8, u8) = (33, 58, 43);
const DARK_DEL_BG: (u8, u8, u8) = (74, 34, 29);
const LIGHT_ADD_BG: (u8, u8, u8) = (218, 251, 225);
const LIGHT_DEL_BG: (u8, u8, u8) = (255, 235, 233);

pub(super) fn render_unified_diff_text(
    text: &str,
    width: usize,
    palette: ThemePalette,
) -> Option<Vec<Line<'static>>> {
    let sections = split_diff_sections(text);
    let mut out = Vec::new();
    let mut rendered_any = false;

    for section in sections {
        let section_lines = render_diff_section(&section, width, palette)?;
        if !section_lines.is_empty() {
            if rendered_any {
                out.push(Line::from(String::new()));
            }
            out.extend(section_lines);
            rendered_any = true;
        }
    }

    rendered_any.then_some(out)
}

fn split_diff_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();

    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        sections.push(current);
    }

    if sections.is_empty() && !text.trim().is_empty() {
        sections.push(text.to_string());
    }

    sections
}

fn render_diff_section(
    section: &str,
    width: usize,
    palette: ThemePalette,
) -> Option<Vec<Line<'static>>> {
    let patch = Patch::from_str(section).ok()?;
    let rows = collect_rows(&patch);
    if rows.is_empty() {
        return None;
    }

    let syntax_path = patch.modified().or_else(|| patch.original()).map(Path::new);
    let layout = if width >= WIDE_LAYOUT_THRESHOLD {
        DiffLayout::Wide
    } else {
        DiffLayout::Narrow
    };

    Some(match layout {
        DiffLayout::Wide => render_wide_rows(&rows, width, syntax_path, palette),
        DiffLayout::Narrow => render_narrow_rows(&rows, width, syntax_path, palette),
    })
}

fn collect_rows(patch: &Patch<'_, str>) -> Vec<DiffRow> {
    let mut rows = Vec::new();

    for (index, hunk) in patch.hunks().iter().enumerate() {
        if index > 0 {
            rows.push(DiffRow::blank());
        }

        let lines = hunk.lines();
        let mut cursor = 0usize;

        while cursor < lines.len() {
            match lines[cursor] {
                DiffLine::Context(text) => {
                    rows.push(DiffRow {
                        kind: RowKind::Context,
                        left: Some(DiffCell::new(text, DiffLineKind::Context)),
                        right: Some(DiffCell::new(text, DiffLineKind::Context)),
                    });
                    cursor += 1;
                }
                DiffLine::Delete(_) => {
                    let mut removed = Vec::new();
                    let mut added = Vec::new();

                    while cursor < lines.len() {
                        match lines[cursor] {
                            DiffLine::Delete(text) => {
                                removed.push(text.to_string());
                                cursor += 1;
                            }
                            _ => break,
                        }
                    }

                    while cursor < lines.len() {
                        match lines[cursor] {
                            DiffLine::Insert(text) => {
                                added.push(text.to_string());
                                cursor += 1;
                            }
                            _ => break,
                        }
                    }

                    let count = removed.len().max(added.len());
                    for offset in 0..count {
                        let left = removed.get(offset).cloned().map(DiffCell::delete);
                        let right = added.get(offset).cloned().map(DiffCell::insert);
                        let kind = match (left.as_ref(), right.as_ref()) {
                            (Some(_), Some(_)) => RowKind::Modified,
                            (Some(_), None) => RowKind::Removed,
                            (None, Some(_)) => RowKind::Added,
                            (None, None) => RowKind::Blank,
                        };
                        rows.push(DiffRow { kind, left, right });
                    }
                }
                DiffLine::Insert(text) => {
                    rows.push(DiffRow {
                        kind: RowKind::Added,
                        left: None,
                        right: Some(DiffCell::insert(text.to_string())),
                    });
                    cursor += 1;
                }
            }
        }
    }

    rows
}

fn render_wide_rows(
    rows: &[DiffRow],
    width: usize,
    syntax_path: Option<&Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let separator = Span::styled("│", Style::default().fg(palette.border));
    let left_width = width.saturating_sub(1) / 2;
    let right_width = width.saturating_sub(1).saturating_sub(left_width);
    let mut out = Vec::new();

    for row in rows {
        let left_bg = row.left.as_ref().map(|cell| cell_bg(cell.kind, palette));
        let right_bg = row.right.as_ref().map(|cell| cell_bg(cell.kind, palette));

        match row.kind {
            RowKind::Blank => out.push(Line::from(String::new())),
            RowKind::Context => {
                let left = row
                    .left
                    .as_ref()
                    .map(|cell| render_cell_lines(cell, left_width, syntax_path, palette))
                    .unwrap_or_else(|| vec![blank_cell_line(left_width, left_bg.flatten())]);
                let right = row
                    .right
                    .as_ref()
                    .map(|cell| render_cell_lines(cell, right_width, syntax_path, palette))
                    .unwrap_or_else(|| vec![blank_cell_line(right_width, right_bg.flatten())]);
                out.extend(merge_columns(
                    left,
                    right,
                    separator.clone(),
                    left_width,
                    right_width,
                ));
            }
            RowKind::Removed => {
                let left = row
                    .left
                    .as_ref()
                    .map(|cell| render_cell_lines(cell, left_width, syntax_path, palette))
                    .unwrap_or_else(|| vec![blank_cell_line(left_width, left_bg.flatten())]);
                let right = vec![blank_cell_line(right_width, right_bg.flatten())];
                out.extend(merge_columns(
                    left,
                    right,
                    separator.clone(),
                    left_width,
                    right_width,
                ));
            }
            RowKind::Added => {
                let left = vec![blank_cell_line(left_width, left_bg.flatten())];
                let right = row
                    .right
                    .as_ref()
                    .map(|cell| render_cell_lines(cell, right_width, syntax_path, palette))
                    .unwrap_or_else(|| vec![blank_cell_line(right_width, right_bg.flatten())]);
                out.extend(merge_columns(
                    left,
                    right,
                    separator.clone(),
                    left_width,
                    right_width,
                ));
            }
            RowKind::Modified => {
                let left = row
                    .left
                    .as_ref()
                    .map(|cell| render_cell_lines(cell, left_width, syntax_path, palette))
                    .unwrap_or_else(|| vec![blank_cell_line(left_width, left_bg.flatten())]);
                let right = row
                    .right
                    .as_ref()
                    .map(|cell| render_cell_lines(cell, right_width, syntax_path, palette))
                    .unwrap_or_else(|| vec![blank_cell_line(right_width, right_bg.flatten())]);
                out.extend(merge_columns(
                    left,
                    right,
                    separator.clone(),
                    left_width,
                    right_width,
                ));
            }
        }
    }

    out
}

fn render_narrow_rows(
    rows: &[DiffRow],
    width: usize,
    syntax_path: Option<&Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    for row in rows {
        match row.kind {
            RowKind::Blank => out.push(Line::from(String::new())),
            RowKind::Context => {
                if let Some(cell) = row.left.as_ref() {
                    out.extend(render_cell_lines(cell, width, syntax_path, palette));
                }
            }
            RowKind::Removed => {
                if let Some(cell) = row.left.as_ref() {
                    out.extend(render_cell_lines(cell, width, syntax_path, palette));
                }
            }
            RowKind::Added => {
                if let Some(cell) = row.right.as_ref() {
                    out.extend(render_cell_lines(cell, width, syntax_path, palette));
                }
            }
            RowKind::Modified => {
                if let Some(cell) = row.left.as_ref() {
                    out.extend(render_cell_lines(cell, width, syntax_path, palette));
                }
                if let Some(cell) = row.right.as_ref() {
                    out.extend(render_cell_lines(cell, width, syntax_path, palette));
                }
            }
        }
    }

    out
}

fn render_cell_lines(
    cell: &DiffCell,
    width: usize,
    syntax_path: Option<&Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let width = width.max(2);
    let content = render_cell_content(cell, syntax_path, palette);
    let initial_indent = cell_prefix(cell.kind, palette);
    let subsequent_indent = blank_prefix();
    let bg = cell_bg(cell.kind, palette);

    let wrapped = adaptive_wrap_lines(
        std::iter::once(content),
        WrapOptions::new(width)
            .initial_indent(initial_indent)
            .subsequent_indent(subsequent_indent),
    );

    let mut out = Vec::new();
    for line in wrapped {
        let styled = apply_bg(line, bg);
        out.push(pad_line_to_width(styled, width, bg));
    }

    if out.is_empty() {
        out.push(blank_cell_line(width, bg));
    }

    out
}

fn render_cell_content(
    cell: &DiffCell,
    syntax_path: Option<&Path>,
    palette: ThemePalette,
) -> Line<'static> {
    if let Some(mut lines) = highlight_code_to_lines_for_path(&cell.text, syntax_path) {
        let mut line = lines.pop().unwrap_or_default();
        if matches!(cell.kind, DiffLineKind::Delete) {
            for span in &mut line.spans {
                span.style = span.style.add_modifier(Modifier::DIM);
            }
        }
        return line;
    }

    let style = match cell.kind {
        DiffLineKind::Context => Style::default().fg(palette.text),
        DiffLineKind::Delete => Style::default()
            .fg(palette.error)
            .add_modifier(Modifier::DIM),
        DiffLineKind::Insert => Style::default().fg(palette.success),
    };

    Line::from(vec![Span::styled(cell.text.clone(), style)])
}

fn merge_columns(
    left: Vec<Line<'static>>,
    right: Vec<Line<'static>>,
    separator: Span<'static>,
    left_width: usize,
    right_width: usize,
) -> Vec<Line<'static>> {
    let height = left.len().max(right.len()).max(1);
    let mut out = Vec::with_capacity(height);

    for index in 0..height {
        let left_line = left
            .get(index)
            .cloned()
            .unwrap_or_else(|| blank_cell_line(left_width, None));
        let right_line = right
            .get(index)
            .cloned()
            .unwrap_or_else(|| blank_cell_line(right_width, None));

        let mut spans = left_line.spans;
        spans.push(separator.clone());
        spans.extend(right_line.spans);
        out.push(Line::from(spans));
    }

    out
}

fn cell_prefix(cell: DiffLineKind, palette: ThemePalette) -> Line<'static> {
    let marker = match cell {
        DiffLineKind::Context => " ",
        DiffLineKind::Delete => "-",
        DiffLineKind::Insert => "+",
    };

    Line::from(vec![
        Span::styled(marker, marker_style(cell, palette)),
        Span::styled(" ", Style::default()),
    ])
}

fn blank_prefix() -> Line<'static> {
    Line::from(vec![Span::styled("  ", Style::default())])
}

fn marker_style(kind: DiffLineKind, palette: ThemePalette) -> Style {
    match kind {
        DiffLineKind::Context => Style::default().fg(palette.muted),
        DiffLineKind::Delete => Style::default().fg(palette.error),
        DiffLineKind::Insert => Style::default().fg(palette.success),
    }
}

fn cell_bg(kind: DiffLineKind, palette: ThemePalette) -> Option<ratatui::style::Color> {
    match kind {
        DiffLineKind::Context => None,
        DiffLineKind::Delete => Some(if is_dark_theme(palette.name) {
            ratatui::style::Color::Rgb(DARK_DEL_BG.0, DARK_DEL_BG.1, DARK_DEL_BG.2)
        } else {
            ratatui::style::Color::Rgb(LIGHT_DEL_BG.0, LIGHT_DEL_BG.1, LIGHT_DEL_BG.2)
        }),
        DiffLineKind::Insert => Some(if is_dark_theme(palette.name) {
            ratatui::style::Color::Rgb(DARK_ADD_BG.0, DARK_ADD_BG.1, DARK_ADD_BG.2)
        } else {
            ratatui::style::Color::Rgb(LIGHT_ADD_BG.0, LIGHT_ADD_BG.1, LIGHT_ADD_BG.2)
        }),
    }
}

fn is_dark_theme(name: ThemeName) -> bool {
    matches!(
        name,
        ThemeName::Dark
            | ThemeName::Nord
            | ThemeName::OneDark
            | ThemeName::Catppuccin
            | ThemeName::Solarized
    )
}

fn apply_bg(mut line: Line<'static>, bg: Option<ratatui::style::Color>) -> Line<'static> {
    if let Some(bg) = bg {
        for span in &mut line.spans {
            span.style.bg = Some(bg);
        }
    }
    line
}

fn pad_line_to_width(
    line: Line<'static>,
    width: usize,
    bg: Option<ratatui::style::Color>,
) -> Line<'static> {
    let used = line_display_width(&line);
    if used >= width {
        return line;
    }

    let mut spans = line.spans;
    let style = bg.map_or_else(Style::default, |color| Style::default().bg(color));
    spans.push(Span::styled(" ".repeat(width - used), style));
    Line::from(spans)
}

fn blank_cell_line(width: usize, bg: Option<ratatui::style::Color>) -> Line<'static> {
    let style = bg.map_or_else(Style::default, |color| Style::default().bg(color));
    Line::from(vec![Span::styled(" ".repeat(width.max(1)), style)])
}

fn line_display_width(line: &Line<'static>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

impl DiffCell {
    fn new(text: &str, kind: DiffLineKind) -> Self {
        Self {
            text: text.to_string(),
            kind,
        }
    }

    fn delete(text: String) -> Self {
        Self {
            text,
            kind: DiffLineKind::Delete,
        }
    }

    fn insert(text: String) -> Self {
        Self {
            text,
            kind: DiffLineKind::Insert,
        }
    }
}

impl DiffRow {
    fn blank() -> Self {
        Self {
            kind: RowKind::Blank,
            left: None,
            right: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> ThemePalette {
        ThemePalette::dark()
    }

    fn flatten_lines(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn removes_metadata_and_renders_body_rows() {
        let diff = r#"diff --git a/foo.rs b/foo.rs
index 1111111..2222222 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@ fn main
 fn main() {
-old
+new
 }
"#;

        let lines = render_unified_diff_text(diff, 120, palette()).expect("diff should render");
        let rendered = flatten_lines(&lines);

        assert!(!rendered.iter().any(|line| line.contains("diff --git")));
        assert!(!rendered.iter().any(|line| line.contains("--- a/foo.rs")));
        assert!(!rendered.iter().any(|line| line.contains("+++ b/foo.rs")));
        assert!(!rendered.iter().any(|line| line.contains("@@ -1,3 +1,3 @@")));
        assert!(rendered.iter().any(|line| line.contains("fn main() {")));
        assert!(rendered.iter().any(|line| line.contains("old")));
        assert!(rendered.iter().any(|line| line.contains("new")));
    }

    #[test]
    fn uses_single_column_on_narrow_width() {
        let diff = r#"--- a/foo.rs
+++ b/foo.rs
@@ -1 +1 @@
-fn main() {}
+fn main() {}
"#;

        let lines = render_unified_diff_text(diff, 60, palette()).expect("diff should render");
        let rendered = flatten_lines(&lines);

        assert!(rendered.iter().any(|line| line.contains("- fn main() {}")));
        assert!(rendered.iter().any(|line| line.contains("+ fn main() {}")));
    }

    #[test]
    fn returns_none_for_plain_text() {
        assert!(render_unified_diff_text("hello world", 80, palette()).is_none());
    }

    #[test]
    fn highlights_body_lines_for_known_language() {
        let diff = r#"--- a/src/main.rs
+++ b/src/main.rs
@@ -1 +1 @@
-fn main() {}
+fn main() {}
"#;

        let lines = render_unified_diff_text(diff, 120, palette()).expect("diff should render");
        let body_line = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .contains("fn main() {}")
            })
            .expect("expected highlighted body line");

        assert!(body_line.spans.len() > 1);
    }
}
