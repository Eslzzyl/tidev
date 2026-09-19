//! Theme preview content builder.
//!
//! Builds a static showcase of one theme's palette for the theme panel's
//! right pane: color swatches, a sample chat card, syntax-highlighted code,
//! a diff excerpt, mode badges, surface blocks and a real markdown snippet.
//! All palette-driven elements are styled from the previewed palette; the
//! markdown snippet uses the app's static markdown styles (labelled as such).

use ratatui::prelude::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tidev_config::ThemeDefinition;
use unicode_width::UnicodeWidthStr;

use crate::diff_render::render_unified_diff_text;
use crate::i18n::{TextKey, UiText};
use crate::markdown::{highlight_code_to_lines, render_markdown_text_with_width_and_cwd_with_ui};
use crate::theme::ThemePalette;

/// Build the complete preview pane content for one theme.
pub(crate) fn build_preview_lines(
    name: &str,
    palette: ThemePalette,
    def: Option<&ThemeDefinition>,
    width: usize,
    ui_text: &UiText,
) -> Vec<Line<'static>> {
    let width = width.max(10);
    let mut out = Vec::new();

    // ── Theme info ──────────────────────────────────────────────
    let mut header = vec![Span::styled(
        format!(" {name} "),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
    )];
    header.push(Span::styled(
        format!(
            " {} ",
            ui_text.text(if palette.is_dark {
                TextKey::ThemePreviewDark
            } else {
                TextKey::ThemePreviewLight
            })
        ),
        Style::default().fg(palette.text).bg(palette.panel_light),
    ));
    let syntax = def.map_or_else(
        || {
            if palette.is_dark {
                "base16-ocean.dark"
            } else {
                "InspiredGitHub"
            }
        },
        |d| d.syntax_theme_key(),
    );
    header.push(Span::styled(
        format!(
            "  {}",
            ui_text.text_with_value(TextKey::ThemePreviewSyntax, "value", syntax)
        ),
        Style::default().fg(palette.muted),
    ));
    out.push(Line::from(header));
    out.push(Line::from(String::new()));

    // ── Palette swatches ────────────────────────────────────────
    let entries: Vec<(String, Color)> = vec![
        (
            ui_text.text(TextKey::ThemePreviewBackground),
            palette.background,
        ),
        (ui_text.text(TextKey::ThemePreviewPanel), palette.panel),
        (
            ui_text.text(TextKey::ThemePreviewPanelAlt),
            palette.panel_alt,
        ),
        (
            ui_text.text(TextKey::ThemePreviewPanelLight),
            palette.panel_light,
        ),
        (ui_text.text(TextKey::ThemePreviewText), palette.text),
        (ui_text.text(TextKey::ThemePreviewMuted), palette.muted),
        (ui_text.text(TextKey::ThemePreviewBorder), palette.border),
        (ui_text.text(TextKey::ThemePreviewAccent), palette.accent),
        (
            ui_text.text(TextKey::ThemePreviewAccentSoft),
            palette.accent_soft,
        ),
        (ui_text.text(TextKey::ThemePreviewSuccess), palette.success),
        (ui_text.text(TextKey::ThemePreviewWarning), palette.warning),
        (ui_text.text(TextKey::ThemePreviewError), palette.error),
        (ui_text.text(TextKey::ThemePreviewDiffAdd), palette.diff_add),
        (
            ui_text.text(TextKey::ThemePreviewDiffDelete),
            palette.diff_delete,
        ),
        (
            ui_text.text(TextKey::ThemePreviewSelectionBg),
            palette.selection_bg,
        ),
        (
            ui_text.text(TextKey::ThemePreviewSelectionFg),
            palette.selection_fg,
        ),
        (
            ui_text.text(TextKey::ThemePreviewModeBuild),
            palette.mode_build,
        ),
        (
            ui_text.text(TextKey::ThemePreviewModePlan),
            palette.mode_plan,
        ),
    ];
    // Two columns when the pane is wide enough for "██ name #rrggbb" cells.
    let cols = if width >= 50 { 2 } else { 1 };
    let mut row_spans: Vec<Span<'static>> = Vec::new();
    for (i, (entry_name, color)) in entries.iter().enumerate() {
        row_spans.push(Span::styled("██", Style::default().fg(*color)));
        row_spans.push(Span::styled(
            format!(" {entry_name:<12} "),
            Style::default().fg(palette.text),
        ));
        row_spans.push(Span::styled(
            hex_of(*color),
            Style::default().fg(palette.muted),
        ));
        if (i + 1) % cols == 0 || i + 1 == entries.len() {
            out.push(Line::from(row_spans));
            row_spans = Vec::new();
        } else {
            row_spans.push(Span::raw("  "));
        }
    }
    out.push(Line::from(String::new()));

    // ── Sample chat message card ────────────────────────────────
    out.push(section_header(
        palette,
        ui_text.text(TextKey::ThemePreviewMessages),
        width,
    ));
    out.push(Line::from(vec![
        Span::styled("┌", Style::default().fg(palette.border)),
        Span::styled(
            "─".repeat(width.saturating_sub(2)),
            Style::default().fg(palette.border),
        ),
        Span::styled("┐", Style::default().fg(palette.border)),
    ]));
    out.push(card_row(
        palette,
        vec![
            Span::styled(
                "tidev",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(palette.muted)),
            Span::styled("1.2s", Style::default().fg(palette.muted)),
        ],
        width,
    ));
    out.push(card_row(
        palette,
        vec![
            Span::styled(
                ui_text.text(TextKey::ThemePreviewIWill),
                Style::default().fg(palette.text),
            ),
            Span::styled(
                ui_text.text(TextKey::ThemePreviewUpdate),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                ui_text.text(TextKey::ThemePreviewThe),
                Style::default().fg(palette.text),
            ),
            Span::styled("README.md", Style::default().fg(palette.accent)),
            Span::styled(
                ui_text.text(TextKey::ThemePreviewAndRun),
                Style::default().fg(palette.text),
            ),
            Span::styled("cargo test", Style::default().fg(palette.accent)),
            Span::styled(".", Style::default().fg(palette.text)),
        ],
        width,
    ));
    out.push(card_row(
        palette,
        vec![
            Span::styled(
                format!("✓ {}", ui_text.text(TextKey::ThemePreviewReady)),
                Style::default().fg(palette.success),
            ),
            Span::styled("  ", Style::default().fg(palette.muted)),
            Span::styled(
                format!("▲ {}", ui_text.text(TextKey::ThemePreviewPending)),
                Style::default().fg(palette.warning),
            ),
            Span::styled("  ", Style::default().fg(palette.muted)),
            Span::styled(
                format!("✗ {}", ui_text.text(TextKey::ThemePreviewFailed)),
                Style::default().fg(palette.error),
            ),
        ],
        width,
    ));
    out.push(card_row(
        palette,
        vec![
            Span::styled(
                format!("{} ", ui_text.text(TextKey::ThemePreviewTool)),
                Style::default().fg(palette.muted),
            ),
            Span::styled(
                ui_text.text(TextKey::ThemePreviewReadFile),
                Style::default().fg(palette.accent_soft),
            ),
            Span::styled(" · ", Style::default().fg(palette.muted)),
            Span::styled(
                "https://example.com",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ],
        width,
    ));
    out.push(card_row(
        palette,
        vec![Span::styled(
            ui_text.text(TextKey::ThemePreviewSelected),
            Style::default()
                .fg(palette.selection_fg)
                .bg(palette.selection_bg),
        )],
        width,
    ));
    out.push(Line::from(vec![
        Span::styled("└", Style::default().fg(palette.border)),
        Span::styled(
            "─".repeat(width.saturating_sub(2)),
            Style::default().fg(palette.border),
        ),
        Span::styled("┘", Style::default().fg(palette.border)),
    ]));
    out.push(Line::from(String::new()));

    // ── Syntax-highlighted code block ───────────────────────────
    out.push(section_header(
        palette,
        ui_text.text(TextKey::ThemePreviewSyntaxHighlighting),
        width,
    ));
    let code = "// tidev theme preview\nfn main() {\n    let msg = \"hello tidev\";\n    println!(\"{msg}\");\n}";
    out.push(pad_with_bg(
        Line::from(vec![
            Span::styled(
                "┌ rust",
                Style::default()
                    .fg(palette.accent_soft)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "─".repeat(width.saturating_sub(6)),
                Style::default().fg(palette.border),
            ),
        ]),
        width,
        palette.panel_alt,
    ));
    for line in highlight_code_to_lines(code, "rust") {
        out.push(pad_with_bg(line, width, palette.panel_alt));
    }
    out.push(Line::from(String::new()));

    // ── Diff excerpt ────────────────────────────────────────────
    out.push(section_header(
        palette,
        ui_text.text(TextKey::ThemePreviewDiff),
        width,
    ));
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,5 +10,7 @@ fn main() {
     let items = vec![1, 2, 3];
-    let total: i32 = items.iter().sum();
+    let total: i64 = items.iter().map(|&x| x as i64).sum();
     println!("total: {total}");
+    let doubled: Vec<i32> = items.iter().map(|x| x * 2).collect();
+    println!("doubled: {doubled:?}");
     // finish up
 }
"#;
    // The hunk header is not part of the diffy body rows — render it
    // manually, exactly like the real diff renderer's surroundings.
    out.push(pad_with_bg(
        Line::from(vec![Span::styled(
            "@@ -10,5 +10,7 @@ fn main() {",
            Style::default().fg(palette.accent_soft),
        )]),
        width,
        palette.panel_alt,
    ));
    if let Some((diff_lines, _regions)) = render_unified_diff_text(diff, width, palette, 4) {
        out.extend(diff_lines);
    } else {
        // Fallback: simple styled rows. Should never trigger — the snippet
        // above is a valid unified diff — but keep the preview robust.
        let (del_bg, add_bg) = (palette.diff_delete_bg, palette.diff_add_bg);
        for line in [
            Line::from(vec![Span::styled(
                "    let items = vec![1, 2, 3];",
                Style::default().fg(palette.text),
            )]),
            Line::from(vec![
                Span::styled("-", Style::default().fg(palette.diff_delete)),
                Span::styled(
                    " let total: i32 = items.iter().sum();",
                    Style::default()
                        .fg(palette.diff_delete)
                        .add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(vec![
                Span::styled("+", Style::default().fg(palette.diff_add)),
                Span::styled(
                    " let total: i64 = items.iter().map(|&x| x as i64).sum();",
                    Style::default().fg(palette.diff_add),
                ),
            ]),
            Line::from(vec![Span::styled(
                "    // finish up",
                Style::default().fg(palette.text),
            )]),
        ] {
            let bg = if line.spans.iter().any(|s| s.content == "+") {
                add_bg
            } else if line.spans.iter().any(|s| s.content == "-") {
                del_bg
            } else {
                palette.panel_alt
            };
            out.push(pad_with_bg(line, width, bg));
        }
    }
    out.push(Line::from(String::new()));

    // ── Mode badges ─────────────────────────────────────────────
    out.push(section_header(
        palette,
        ui_text.text(TextKey::ThemePreviewMode),
        width,
    ));
    out.push(Line::from(vec![
        Span::styled(
            ui_text.text(TextKey::Build),
            Style::default()
                .fg(palette.mode_build)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" → ", Style::default().fg(palette.muted)),
        Span::styled(
            ui_text.text(TextKey::Plan),
            Style::default()
                .fg(palette.mode_plan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(palette.muted)),
        Span::styled(
            ui_text.text(TextKey::ThemePreviewModel),
            Style::default().fg(palette.text),
        ),
        Span::styled(" · ", Style::default().fg(palette.muted)),
        Span::styled(
            ui_text.text(TextKey::ThemePreviewProvider),
            Style::default().fg(palette.muted),
        ),
        Span::styled(" · ", Style::default().fg(palette.muted)),
        Span::styled(
            ui_text.text(TextKey::Thinking),
            Style::default().fg(palette.accent_soft),
        ),
    ]));
    out.push(Line::from(String::new()));

    // ── Surface blocks ──────────────────────────────────────────
    out.push(section_header(
        palette,
        ui_text.text(TextKey::ThemePreviewSurfaces),
        width,
    ));
    for (surface_name, color) in [
        (
            ui_text.text(TextKey::ThemePreviewBackground),
            palette.background,
        ),
        (ui_text.text(TextKey::ThemePreviewPanel), palette.panel),
        (
            ui_text.text(TextKey::ThemePreviewPanelAlt),
            palette.panel_alt,
        ),
        (
            ui_text.text(TextKey::ThemePreviewPanelLight),
            palette.panel_light,
        ),
    ] {
        out.push(Line::from(vec![
            Span::styled(
                format!("  {surface_name:<12}"),
                Style::default().fg(palette.text),
            ),
            Span::styled("████████", Style::default().fg(color)),
            Span::styled("  ", Style::default().fg(palette.muted)),
            Span::styled(hex_of(color), Style::default().fg(palette.muted)),
        ]));
    }
    out.push(Line::from(String::new()));

    // ── Markdown sample (static styles) ─────────────────────────
    out.push(section_header(
        palette,
        ui_text.text(TextKey::ThemePreviewMarkdown),
        width,
    ));
    let md = r#"# Heading One
## Heading Two
### Heading Three

Text with **bold**, *italic*, ~~strikethrough~~, `inline code` and [a link](https://example.com).

- item one
- item two
  - nested item
- item three

1. first
2. second

> quote line one
> quote line two

| Name  | Kind  | Status |
|-------|-------|--------|
| dark  | deep  | ✓ used |
| light | soft  | ✓ used |

```rust
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
```
"#;
    let rendered = render_markdown_text_with_width_and_cwd_with_ui(md, Some(width), None, ui_text);
    out.extend(rendered.text.lines.iter().cloned());

    out
}

/// `── label ──────...` section divider, drawn in the border color.
fn section_header(palette: ThemePalette, label: String, width: usize) -> Line<'static> {
    let head = format!("── {label} ");
    let head_w = UnicodeWidthStr::width(head.as_str());
    Line::from(vec![
        Span::styled(head, Style::default().fg(palette.border)),
        Span::styled(
            "─".repeat(width.saturating_sub(head_w)),
            Style::default().fg(palette.border),
        ),
    ])
}

/// One row of the sample chat card: `│ content │` with the content padded
/// (or truncated with an ellipsis) to fit the card width.
fn card_row(palette: ThemePalette, content: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let content_w = width.saturating_sub(4);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in content {
        let w = UnicodeWidthStr::width(span.content.as_ref());
        if used + w > content_w {
            if used < content_w {
                spans.push(Span::styled("…", Style::default().fg(palette.muted)));
            }
            break;
        }
        used += w;
        spans.push(span);
    }
    if used < content_w {
        spans.push(Span::styled(" ".repeat(content_w - used), Style::default()));
    }
    let mut line = vec![Span::styled("│ ", Style::default().fg(palette.border))];
    line.extend(spans);
    line.push(Span::styled(" │", Style::default().fg(palette.border)));
    Line::from(line)
}

/// Pad a line to `width` and paint its whole background with `bg`.
fn pad_with_bg(mut line: Line<'static>, width: usize, bg: Color) -> Line<'static> {
    let used: usize = line
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    if used < width {
        line.spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(bg),
        ));
    }
    for span in &mut line.spans {
        span.style.bg = Some(bg);
    }
    line
}

/// Hex string for a palette color, e.g. `#1e1e2e`.
fn hex_of(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "-------".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_config::ThemeDefinition;

    /// Minimal theme definition parsed via serde_json (colors deserialize
    /// from plain strings).
    fn test_def(dark: bool) -> ThemeDefinition {
        let json = format!(
            r#"{{"dark":{dark},"background":"000000","panel":"111111","panel_alt":"222222","panel_light":"333333","text":"eeeeee","muted":"aaaaaa","border":"444444","accent":"00aaff","accent_soft":"66ccff","success":"00cc00","warning":"cccc00","error":"cc0000","diff_add":"00ff00","diff_delete":"ff0000","selection_bg":"0055ff","selection_fg":"ffffff","mode_build":"00cccc","mode_plan":"88aaaa"}}"#
        );
        serde_json::from_str(&json).expect("test theme parses")
    }

    fn joined(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
            .collect()
    }

    #[test]
    fn preview_builds_for_dark_theme() {
        let def = test_def(true);
        let palette = ThemePalette::from_definition(&def);
        let lines = build_preview_lines(
            "test-dark",
            palette,
            Some(&def),
            80,
            &UiText::from_preference("en-US"),
        );
        assert!(!lines.is_empty());
        let text = joined(&lines);
        // Theme info: name, badge, syntax theme key
        assert!(text.contains("test-dark"));
        assert!(text.contains("Dark"));
        assert!(text.contains("base16-ocean.dark"));
        // Every palette entry renders as a swatch with its hex code
        for hex in [
            "#000000", "#111111", "#222222", "#333333", "#eeeeee", "#aaaaaa", "#444444", "#00aaff",
            "#66ccff", "#00cc00", "#cccc00", "#cc0000", "#00ff00", "#ff0000", "#0055ff", "#ffffff",
            "#00cccc", "#88aaaa",
        ] {
            assert!(text.contains(hex), "missing swatch hex {hex}");
        }
        // Sample sections are present
        for label in [
            "Messages",
            "Syntax Highlighting",
            "Diff",
            "Mode",
            "Surfaces",
            "Markdown",
        ] {
            assert!(text.contains(label), "missing section {label}");
        }
        // Selection row uses the theme's selection colors
        assert!(text.contains("selected item"));
        // Diff section: hunk header plus syntax-highlighted body rows
        assert!(text.contains("@@ -10,5 +10,7 @@"));
        assert!(text.contains("let total: i64"), "missing diff insert row");
        assert!(text.contains("let doubled"), "missing diff insert row");
        assert!(text.contains("// finish up"), "missing diff context row");
        // Markdown section: headings, list nesting, blockquote, table, fence
        for snippet in [
            "Heading One",
            "Heading Two",
            "Heading Three",
            "strikethrough",
            "inline code",
            "nested item",
            "quote line one",
            "dark",
            "Hello, {name}!",
        ] {
            assert!(text.contains(snippet), "missing markdown element {snippet}");
        }
    }

    #[test]
    fn preview_builds_for_light_theme() {
        let def = test_def(false);
        let palette = ThemePalette::from_definition(&def);
        let lines = build_preview_lines(
            "test-light",
            palette,
            Some(&def),
            60,
            &UiText::from_preference("en-US"),
        );
        assert!(!lines.is_empty());
        let text = joined(&lines);
        assert!(text.contains("Light"));
        assert!(text.contains("InspiredGitHub"));
    }

    #[test]
    fn preview_builds_at_narrow_width() {
        let def = test_def(true);
        let palette = ThemePalette::from_definition(&def);
        let lines = build_preview_lines(
            "narrow",
            palette,
            Some(&def),
            24,
            &UiText::from_preference("en-US"),
        );
        assert!(!lines.is_empty());
        // Single-column swatch grid: every color still appears exactly once
        let text = joined(&lines);
        for hex in ["#00aaff", "#00ff00", "#ff0000"] {
            assert!(text.contains(hex), "missing swatch hex {hex}");
        }
    }

    #[test]
    fn preview_falls_back_without_definition() {
        let def = test_def(true);
        let palette = ThemePalette::from_definition(&def);
        // `def: None` — the syntax key falls back to the palette default
        let lines = build_preview_lines(
            "ghost",
            palette,
            None,
            60,
            &UiText::from_preference("en-US"),
        );
        assert!(!lines.is_empty());
        let text = joined(&lines);
        assert!(text.contains("base16-ocean.dark"));
    }
}
