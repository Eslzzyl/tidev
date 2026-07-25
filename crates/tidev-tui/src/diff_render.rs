//! Diff rendering — renders unified diff text with side-by-side (wide) or
//! single-column (narrow) layout, syntax highlighting, and selectable regions.
//!
//! Mirrors the old `tidev_tui::render::diff_render` behaviour.

use std::path::Path;

use crate::theme::{ThemeName, ThemePalette};
use diffy::{Line as DiffLine, Patch};
use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::components::chat::render_cache::SelectableRegionRange;
use crate::markdown::{WrapOptions, adaptive_wrap_lines, highlight_code_to_lines_for_path};
use crate::utils::expand_tabs;

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
    Context,
    Removed,
    Added,
}

const WIDE_LAYOUT_THRESHOLD: usize = 100;
const DARK_ADD_BG: (u8, u8, u8) = (33, 58, 43);
const DARK_DEL_BG: (u8, u8, u8) = (74, 34, 29);
const LIGHT_ADD_BG: (u8, u8, u8) = (218, 251, 225);
const LIGHT_DEL_BG: (u8, u8, u8) = (255, 235, 233);

pub(crate) fn render_unified_diff_text(
    text: &str,
    width: usize,
    palette: ThemePalette,
    tab_width: usize,
) -> Option<(Vec<Line<'static>>, Vec<SelectableRegionRange>)> {
    let sections = split_diff_sections(text);
    let mut out = Vec::new();
    let mut regions = Vec::new();
    let mut rendered_any = false;

    for section in sections {
        let (section_lines, section_regions) =
            render_diff_section(&section, width, palette, tab_width)?;
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
    tab_width: usize,
) -> Option<(Vec<Line<'static>>, Vec<SelectableRegionRange>)> {
    let patch = Patch::from_str(section).ok()?;
    let rows = collect_rows(&patch, tab_width);
    if rows.is_empty() {
        return None;
    }

    let syntax_path = patch.modified().or_else(|| patch.original()).map(Path::new);
    let is_new_file = section.contains("new file mode")
        || patch
            .original()
            .map(|s| s.is_empty() || s == "/dev/null")
            .unwrap_or(true);
    let layout = if is_new_file || width < WIDE_LAYOUT_THRESHOLD {
        DiffLayout::Narrow
    } else {
        DiffLayout::Wide
    };

    let max_line_number = rows.iter().fold(0usize, |max, row| {
        let left = row.left.as_ref().map(|cell| cell.line_number).unwrap_or(0);
        let right = row.right.as_ref().map(|cell| cell.line_number).unwrap_or(0);
        max.max(left.max(right))
    });
    let line_number_w = line_number_width(max_line_number);

    let lines = match layout {
        DiffLayout::Wide => render_wide_rows(&rows, width, line_number_w, syntax_path, palette),
        DiffLayout::Narrow => render_narrow_rows(&rows, width, line_number_w, syntax_path, palette),
    };

    Some((lines, Vec::new()))
}

fn collect_rows(patch: &Patch<'_, str>, tab_width: usize) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    let mut old_line_number;
    let mut new_line_number;

    for hunk in patch.hunks() {
        // Update line numbers from hunk header.
        old_line_number = hunk.old_range().start();
        new_line_number = hunk.new_range().start();

        for line in hunk.lines() {
            match line {
                DiffLine::Context(text) | DiffLine::Delete(text) | DiffLine::Insert(text) => {
                    let trimmed = text.strip_suffix('\n').unwrap_or(text);
                    match line {
                        DiffLine::Context(_) => {
                            old_line_number += 1;
                            new_line_number += 1;
                            rows.push(DiffRow {
                                kind: RowKind::Context,
                                left: Some(DiffCell::context(
                                    old_line_number - 1,
                                    trimmed,
                                    tab_width,
                                )),
                                right: Some(DiffCell::context(
                                    new_line_number - 1,
                                    trimmed,
                                    tab_width,
                                )),
                            });
                        }
                        DiffLine::Delete(_) => {
                            old_line_number += 1;
                            rows.push(DiffRow {
                                kind: RowKind::Removed,
                                left: Some(DiffCell::delete(
                                    old_line_number - 1,
                                    trimmed.to_string(),
                                    tab_width,
                                )),
                                right: None,
                            });
                        }
                        DiffLine::Insert(_) => {
                            new_line_number += 1;
                            rows.push(DiffRow {
                                kind: RowKind::Added,
                                left: None,
                                right: Some(DiffCell::insert(
                                    new_line_number - 1,
                                    trimmed.to_string(),
                                    tab_width,
                                )),
                            });
                        }
                    }
                }
            }
        }
    }

    rows
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
    let expected_width = left_width + 1 + right_width;
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

        let mut line = Line::from(spans);
        let actual = line_display_width(&line);
        if actual < expected_width {
            let mut sp = line.spans;
            sp.push(Span::styled(
                " ".repeat(expected_width - actual),
                Style::default(),
            ));
            line = Line::from(sp);
        }

        out.push(line);
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
    fn delete(line_number: usize, text: String, tab_width: usize) -> Self {
        Self {
            line_number,
            text: expand_tabs(&text, tab_width),
            kind: DiffLineKind::Delete,
        }
    }

    fn insert(line_number: usize, text: String, tab_width: usize) -> Self {
        Self {
            line_number,
            text: expand_tabs(&text, tab_width),
            kind: DiffLineKind::Insert,
        }
    }

    fn context(line_number: usize, text: &str, tab_width: usize) -> Self {
        Self {
            line_number,
            text: expand_tabs(text, tab_width),
            kind: DiffLineKind::Context,
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
            render_unified_diff_text(diff, 120, palette(), 4).expect("diff should render");
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

        let (lines, _) =
            render_unified_diff_text(diff, 60, palette(), 4).expect("diff should render");
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
            render_unified_diff_text(diff, 120, palette(), 4).expect("diff should render");
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
        assert!(render_unified_diff_text("hello world", 80, palette(), 4).is_none());
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
            render_unified_diff_text(diff, 120, palette(), 4).expect("diff should render");
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

    #[test]
    fn wide_layout_separator_alignment() {
        let diff = r#"diff --git a/foo.rs b/foo.rs
index 1111111..2222222 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,4 +1,5 @@
 fn main() {
-    old();
-    another();
+    new();
+    another();
+    extra();
 }
"#;
        let (lines, _) =
            render_unified_diff_text(diff, 120, palette(), 4).expect("diff should render");
        let flat: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        // All lines containing the separator must have it at the same column.
        let positions: Vec<usize> = flat.iter().filter_map(|l| l.find('\u{2502}')).collect();
        assert!(
            !positions.is_empty(),
            "expected at least one separator line"
        );
        let first = positions[0];
        assert!(
            positions.iter().all(|&p| p == first),
            "separator column positions differ: {:?}",
            positions
        );
    }

    #[test]
    fn wide_layout_all_lines_same_display_width() {
        let diff = r#"diff --git a/foo.rs b/foo.rs
index 1111111..2222222 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,4 +1,5 @@
 fn main() {
-    old();
-    another();
+    new();
+    another();
+    extra();
 }
"#;
        let width = 120usize;
        let (lines, _) =
            render_unified_diff_text(diff, width, palette(), 4).expect("diff should render");
        let flat: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        // Every non-empty line should have the same display width.
        let widths: Vec<usize> = flat
            .iter()
            .filter(|l| !l.is_empty())
            .map(|l| unicode_width::UnicodeWidthStr::width(l.as_str()))
            .collect();
        let expected = widths[0];
        for (i, &w) in widths.iter().enumerate() {
            assert_eq!(
                w,
                expected,
                "line {i} has width {w}, expected {expected}: {:?}",
                flat.iter().enumerate().find(|(j, _)| *j == i).unwrap().1
            );
        }
    }

    #[test]
    fn expand_tabs_replaces_tabs_with_spaces() {
        let text = "col0\tcol1\tcol2";
        let expanded = expand_tabs(text, 4);
        // Tab at col 0 -> 4 spaces (col0 is 4 cols), tab at col 8 -> 4 spaces
        assert_eq!(expanded, "col0    col1    col2");
        assert!(!expanded.contains('\t'));
    }

    #[test]
    fn expand_tabs_short_circuits_when_no_tabs() {
        let text = "no tabs here";
        let result = expand_tabs(text, 4);
        // Should be a direct to_string() — same content
        assert_eq!(result, text);
    }

    #[test]
    fn expand_tabs_at_line_start() {
        // Tab at column 0 → 4 spaces
        let result = expand_tabs("\thello", 4);
        assert_eq!(result, "    hello");
    }

    #[test]
    fn expand_tabs_at_line_end() {
        // Tab at end of line
        let result = expand_tabs("hello\t", 4);
        assert_eq!(result, "hello   ");
    }

    #[test]
    fn expand_tabs_multiple_on_same_line() {
        let result = expand_tabs("a\tb\tc", 4);
        // col 0: a, col 1: tab → 3 spaces, col 4: b, col 5: tab → 3 spaces, col 8: c
        assert_eq!(result, "a   b   c");
    }

    #[test]
    fn expand_tabs_with_newlines_reset() {
        // Tab after newline should start at column 0 again
        let result = expand_tabs("\tstart\n\tindented", 4);
        assert_eq!(result, "    start\n    indented");
    }

    #[test]
    fn expand_tabs_tab_width_2() {
        let result = expand_tabs("a\tb", 2);
        assert_eq!(result, "a b");
    }

    #[test]
    fn expand_tabs_tab_width_8() {
        let result = expand_tabs("a\tb", 8);
        assert_eq!(result, "a       b");
    }

    #[test]
    fn expand_tabs_with_unicode() {
        // CJK chars have width 2
        let result = expand_tabs("a\tb\u{4e2d}\tce", 4);
        // "a" col=0: tab → 3 spaces, col=4: "b" col=5, "中" col=7,
        // tab → 4-(7%4)=1 space, col=8: "ce" col=10
        assert_eq!(result, "a   b\u{4e2d} ce");
    }

    #[test]
    fn expand_tabs_empty_string() {
        let result = expand_tabs("", 4);
        assert_eq!(result, "");
    }

    #[test]
    fn expand_tabs_tab_only() {
        let result = expand_tabs("\t", 4);
        assert_eq!(result, "    ");
    }

    #[test]
    fn expand_tabs_tab_at_exact_column_multiple() {
        // When col % tab_width == 0, tab expands to tab_width spaces
        let result = expand_tabs("abcd\t", 4);
        // "abcd" is 4 chars, col=4, 4%4=0, so 4 spaces
        assert_eq!(result, "abcd    ");
    }
}
