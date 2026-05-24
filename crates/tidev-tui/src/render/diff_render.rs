use crate::markdown::{WrapOptions, adaptive_wrap_lines, highlight_code_to_lines_for_path};
use crate::theme::{ThemeName, ThemePalette};
use crate::core::state::SelectableRegionRange;
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
    line_number: usize,
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
) -> Option<(Vec<Line<'static>>, Vec<SelectableRegionRange>)> {
    let sections = split_diff_sections(text);
    let mut out = Vec::new();
    let mut regions = Vec::new();
    let mut rendered_any = false;

    for section in sections {
        let (section_lines, section_regions) = render_diff_section(&section, width, palette)?;
        if !section_lines.is_empty() {
            if rendered_any {
                out.push(Line::from(String::new()));
            }
            let start_offset = out.len();
            for mut r in section_regions {
                r.start_line += start_offset;
                r.end_line += start_offset;
                regions.push(r);
            }
            out.extend(section_lines);
            rendered_any = true;
        }
    }

    rendered_any.then_some((out, regions))
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
) -> Option<(Vec<Line<'static>>, Vec<SelectableRegionRange>)> {
    let patch = Patch::from_str(section).ok()?;
    let rows = collect_rows(&patch);
    if rows.is_empty() {
        return None;
    }

    let syntax_path = patch.modified().or_else(|| patch.original()).map(Path::new);
    // Check if this is a new file. We detect this by:
    // 1. "new file mode" in the diff text (git format)
    // 2. Original filename is empty or /dev/null (diffy-generated diff for new files)
    // We can't rely on patch.original().is_none() because diffy still parses the
    // "--- a/foo.rs" line even for new files from git diff.
    let is_new_file = section.contains("new file mode")
        || patch
            .original()
            .map(|s| s.is_empty() || s == "/dev/null")
            .unwrap_or(true);
    let layout = if is_new_file {
        DiffLayout::Narrow
    } else if width >= WIDE_LAYOUT_THRESHOLD {
        DiffLayout::Wide
    } else {
        DiffLayout::Narrow
    };
    let line_number_width = line_number_width_for_rows(&rows);

    Some(match layout {
        DiffLayout::Wide => {
            let lines = render_wide_rows(&rows, width, line_number_width, syntax_path, palette);
            let left_width = width.saturating_sub(1) / 2;
            let _right_width = width.saturating_sub(1).saturating_sub(left_width);

            let min_x_left = (line_number_width.max(1) + 3) as u16;
            let max_x_left = left_width as u16;

            let min_x_right = (left_width + 1 + line_number_width.max(1) + 3) as u16;
            let max_x_right = width as u16;

            let region1 = SelectableRegionRange {
                start_line: 0,
                end_line: lines.len(),
                min_x: min_x_left,
                max_x: Some(max_x_left),
            };
            let region2 = SelectableRegionRange {
                start_line: 0,
                end_line: lines.len(),
                min_x: min_x_right,
                max_x: Some(max_x_right),
            };

            (lines, vec![region1, region2])
        }
        DiffLayout::Narrow => {
            let lines = render_narrow_rows(&rows, width, line_number_width, syntax_path, palette);
            let min_x = (line_number_width.max(1) + 3) as u16;
            let region = SelectableRegionRange {
                start_line: 0,
                end_line: lines.len(),
                min_x,
                max_x: Some(width as u16),
            };
            (lines, vec![region])
        }
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
        let mut old_line_number = hunk.old_range().start();
        let mut new_line_number = hunk.new_range().start();

        while cursor < lines.len() {
            match lines[cursor] {
                DiffLine::Context(text) => {
                    rows.push(DiffRow {
                        kind: RowKind::Context,
                        left: Some(DiffCell::new(old_line_number, text, DiffLineKind::Context)),
                        right: Some(DiffCell::new(new_line_number, text, DiffLineKind::Context)),
                    });
                    old_line_number += 1;
                    new_line_number += 1;
                    cursor += 1;
                }
                DiffLine::Delete(_) => {
                    let mut removed = Vec::new();
                    let mut added = Vec::new();

                    while cursor < lines.len() {
                        match lines[cursor] {
                            DiffLine::Delete(text) => {
                                removed.push((old_line_number, text.to_string()));
                                old_line_number += 1;
                                cursor += 1;
                            }
                            _ => break,
                        }
                    }

                    while cursor < lines.len() {
                        match lines[cursor] {
                            DiffLine::Insert(text) => {
                                added.push((new_line_number, text.to_string()));
                                new_line_number += 1;
                                cursor += 1;
                            }
                            _ => break,
                        }
                    }

                    let count = removed.len().max(added.len());
                    for offset in 0..count {
                        let left = removed
                            .get(offset)
                            .cloned()
                            .map(|(line_number, text)| DiffCell::delete(line_number, text));
                        let right = added
                            .get(offset)
                            .cloned()
                            .map(|(line_number, text)| DiffCell::insert(line_number, text));
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
                        right: Some(DiffCell::insert(new_line_number, text.to_string())),
                    });
                    new_line_number += 1;
                    cursor += 1;
                }
            }
        }
    }

    rows
}

fn line_number_width_for_rows(rows: &[DiffRow]) -> usize {
    let max_line_number = rows.iter().fold(0usize, |max_line_number, row| {
        let left = row.left.as_ref().map(|cell| cell.line_number).unwrap_or(0);
        let right = row.right.as_ref().map(|cell| cell.line_number).unwrap_or(0);
        max_line_number.max(left.max(right))
    });

    line_number_width(max_line_number)
}

fn render_wide_rows(
    rows: &[DiffRow],
    width: usize,
    line_number_width: usize,
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
                    .map(|cell| {
                        render_cell_lines(cell, left_width, line_number_width, syntax_path, palette)
                    })
                    .unwrap_or_else(|| vec![blank_cell_line(left_width, left_bg.flatten())]);
                let right = row
                    .right
                    .as_ref()
                    .map(|cell| {
                        render_cell_lines(
                            cell,
                            right_width,
                            line_number_width,
                            syntax_path,
                            palette,
                        )
                    })
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
                    .map(|cell| {
                        render_cell_lines(cell, left_width, line_number_width, syntax_path, palette)
                    })
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
                    .map(|cell| {
                        render_cell_lines(
                            cell,
                            right_width,
                            line_number_width,
                            syntax_path,
                            palette,
                        )
                    })
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
                    .map(|cell| {
                        render_cell_lines(cell, left_width, line_number_width, syntax_path, palette)
                    })
                    .unwrap_or_else(|| vec![blank_cell_line(left_width, left_bg.flatten())]);
                let right = row
                    .right
                    .as_ref()
                    .map(|cell| {
                        render_cell_lines(
                            cell,
                            right_width,
                            line_number_width,
                            syntax_path,
                            palette,
                        )
                    })
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
    line_number_width: usize,
    syntax_path: Option<&Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    for row in rows {
        match row.kind {
            RowKind::Blank => out.push(Line::from(String::new())),
            RowKind::Context => {
                if let Some(cell) = row.left.as_ref() {
                    out.extend(render_cell_lines(
                        cell,
                        width,
                        line_number_width,
                        syntax_path,
                        palette,
                    ));
                }
            }
            RowKind::Removed => {
                if let Some(cell) = row.left.as_ref() {
                    out.extend(render_cell_lines(
                        cell,
                        width,
                        line_number_width,
                        syntax_path,
                        palette,
                    ));
                }
            }
            RowKind::Added => {
                if let Some(cell) = row.right.as_ref() {
                    out.extend(render_cell_lines(
                        cell,
                        width,
                        line_number_width,
                        syntax_path,
                        palette,
                    ));
                }
            }
            RowKind::Modified => {
                if let Some(cell) = row.left.as_ref() {
                    out.extend(render_cell_lines(
                        cell,
                        width,
                        line_number_width,
                        syntax_path,
                        palette,
                    ));
                }
                if let Some(cell) = row.right.as_ref() {
                    out.extend(render_cell_lines(
                        cell,
                        width,
                        line_number_width,
                        syntax_path,
                        palette,
                    ));
                }
            }
        }
    }

    out
}

fn render_cell_lines(
    cell: &DiffCell,
    width: usize,
    line_number_width: usize,
    syntax_path: Option<&Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let width = width.max(2);
    let content = render_cell_content(cell, syntax_path, palette);
    let initial_indent = cell_prefix(cell.line_number, cell.kind, line_number_width, palette);
    let subsequent_indent = blank_prefix(line_number_width);
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

fn cell_prefix(
    line_number: usize,
    cell: DiffLineKind,
    line_number_width: usize,
    palette: ThemePalette,
) -> Line<'static> {
    let marker = match cell {
        DiffLineKind::Context => " ",
        DiffLineKind::Delete => "-",
        DiffLineKind::Insert => "+",
    };

    let line_number = format!("{:>width$}", line_number, width = line_number_width.max(1));

    Line::from(vec![
        Span::styled(line_number, Style::default().fg(palette.muted)),
        Span::styled(" ", Style::default()),
        Span::styled(marker, marker_style(cell, palette)),
        Span::styled(" ", Style::default()),
    ])
}

fn blank_prefix(line_number_width: usize) -> Line<'static> {
    Line::from(vec![Span::styled(
        " ".repeat(line_number_width.max(1) + 3),
        Style::default(),
    )])
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
            | ThemeName::Mocha
            | ThemeName::Solarized
            | ThemeName::Everforest
            | ThemeName::Dusk
            | ThemeName::Gruvbox
            | ThemeName::TokyoNight
            | ThemeName::RosePine
            | ThemeName::Contrast
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

fn line_number_width(max_line_number: usize) -> usize {
    if max_line_number == 0 {
        1
    } else {
        max_line_number.to_string().len()
    }
}

impl DiffCell {
    fn new(line_number: usize, text: &str, kind: DiffLineKind) -> Self {
        Self {
            line_number,
            text: text.to_string(),
            kind,
        }
    }

    fn delete(line_number: usize, text: String) -> Self {
        Self {
            line_number,
            text,
            kind: DiffLineKind::Delete,
        }
    }

    fn insert(line_number: usize, text: String) -> Self {
        Self {
            line_number,
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

        let (lines, _) =
            render_unified_diff_text(diff, 120, palette()).expect("diff should render");
        let rendered = flatten_lines(&lines);

        assert!(!rendered.iter().any(|line| line.contains("diff --git")));
        assert!(!rendered.iter().any(|line| line.contains("--- a/foo.rs")));
        assert!(!rendered.iter().any(|line| line.contains("+++ b/foo.rs")));
        assert!(!rendered.iter().any(|line| line.contains("@@ -1,3 +1,3 @@")));
        assert!(rendered.iter().any(|line| line.contains("2 - old")));
        assert!(rendered.iter().any(|line| line.contains("2 + new")));
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

        let (lines, _) = render_unified_diff_text(diff, 60, palette()).expect("diff should render");
        let rendered = flatten_lines(&lines);

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("1 - fn main() {}"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("1 + fn main() {}"))
        );
    }

    #[test]
    fn uses_single_column_for_new_file_even_on_wide_width() {
        let diff = r#"diff --git a/foo.rs b/foo.rs
new file mode 100644
--- a/foo.rs
+++ b/foo.rs
@@ -0,0 +1,2 @@
+fn main() {}
+println!("hello");
"#;

        let (lines, _) =
            render_unified_diff_text(diff, 120, palette()).expect("diff should render");
        let rendered = flatten_lines(&lines);

        assert!(!rendered.iter().any(|line| line.contains("│")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("1 + fn main() {}"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("2 + println!(\"hello\");"))
        );
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

        let (lines, _) =
            render_unified_diff_text(diff, 120, palette()).expect("diff should render");
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
