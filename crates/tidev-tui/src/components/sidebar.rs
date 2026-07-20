//! Sidebar — right-hand info panel displaying session metadata, model info,
//! token usage, changed files, todos, and workspace path.
//!
//! Mirrors the old `tidev_tui::render::chat_render::render_sidebar` behaviour.

use std::collections::HashMap;
use std::path::Path;

use ratatui::layout::{Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use tidev_core::FileDiff;
use tidev_types::tools::TodoItem;
use crate::chat_context::ChatContext;
use crate::theme::ThemePalette;
use crate::utils::{TokenUsage, format_token_count};
use unicode_width::UnicodeWidthStr;

use crate::app::ContextUsage;

/// Sidebar component — right-hand info panel.
pub(crate) struct Sidebar {
    /// Scroll offset for the sidebar content area.
    pub scroll_offset: usize,

    /// Cached total lines for scroll max computation.
    total_lines: usize,

    /// Viewport height in lines (set during render, used for scroll max).
    viewport_lines: usize,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            total_lines: 0,
            viewport_lines: 0,
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let max_scroll = self.total_lines.saturating_sub(self.viewport_lines.max(1));
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(max_scroll);
    }



    /// Render the sidebar into the given area.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        palette: ThemePalette,
        workspace_root: &Path,
        chat_context: Option<&ChatContext>,
        context_usage: Option<&ContextUsage>,
        todos: &[TodoItem],
    ) {
        if area.width < 4 || area.height < 4 {
            return;
        }

        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 0,
        });
        let sidebar_content_width = inner.width as usize;

        // ── Build content lines ──
        let mut lines: Vec<Line> = Vec::new();

        // Session title
        lines.push(Line::from(""));
        let session_title = chat_context
            .map(|ctx| shorten(&ctx.title, sidebar_content_width))
            .unwrap_or_default();
        lines.push(Line::from(vec![Span::styled(
            session_title,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(""));

        // Model section
        lines.push(Line::from(vec![Span::styled(
            "Model",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        if let Some(ctx) = chat_context {
            lines.push(Line::from(vec![Span::styled(
                &ctx.model_display_name,
                Style::default().fg(palette.text),
            )]));
            lines.push(Line::from(vec![Span::styled(
                &ctx.provider_display_name,
                Style::default().fg(palette.muted),
            )]));
        }

        // Tokens per second (average across session)
        if let Some(ctx) = chat_context {
            let session_tps: Vec<f32> = ctx
                .messages
                .iter()
                .filter(|m| matches!(m.role, tidev_types::message::MessageRole::Assistant))
                .filter_map(|m| m.tokens_per_second)
                .collect();

            if !session_tps.is_empty() {
                let avg_tps = session_tps.iter().sum::<f32>() / session_tps.len() as f32;
                lines.push(Line::from(vec![Span::styled(
                    format!("Speed: {:.1} t/s (avg)", avg_tps),
                    Style::default().fg(palette.muted),
                )]));
            } else if let Some(usage) = context_usage
                && let Some(current_tps) = usage.tokens_per_second {
                    lines.push(Line::from(vec![Span::styled(
                        format!("Speed: {:.1} t/s", current_tps),
                        Style::default().fg(palette.muted),
                    )]));
                }
        }

        // Token statistics
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Tokens",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        let mut token_usage = TokenUsage::default();
        if let Some(ctx) = chat_context {
            for m in ctx.messages.iter().filter(|m| matches!(m.role, tidev_types::message::MessageRole::Assistant)) {
                token_usage.add(TokenUsage::new(
                    m.input_tokens.unwrap_or(0),
                    m.output_tokens.unwrap_or(0),
                    m.cache_read_tokens.unwrap_or(0),
                    m.cache_write_tokens.unwrap_or(0),
                ));
            }
        }

        let total = token_usage.total();
        let total_cache = token_usage.total_cache();

        lines.push(Line::from(vec![Span::styled(
            format!("Total: {}", format_token_count(total)),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("In: {}", format_token_count(token_usage.input_tokens as u64)),
            Style::default().fg(palette.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("Cache: {}", format_token_count(total_cache)),
            Style::default().fg(palette.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("Out: {}", format_token_count(token_usage.output_tokens as u64)),
            Style::default().fg(palette.muted),
        )]));

        lines.push(Line::from(""));

        // Request count
        if let Some(ctx) = chat_context {
            let request_count = ctx
                .messages
                .iter()
                .filter(|m| matches!(m.role, tidev_types::message::MessageRole::Assistant))
                .count();
            lines.push(Line::from(vec![Span::styled(
                format!("Requests: {request_count}"),
                Style::default().fg(palette.text),
            )]));
        }

        // Changed Files section
        lines.push(Line::from(""));
        let mut all_diffs: Vec<FileDiff> = Vec::new();
        if let Some(ctx) = chat_context {
            // Use HashMap last-wins to merge diffs across messages.
            let mut file_map: HashMap<String, FileDiff> = HashMap::new();
            for msg in ctx.visible_messages() {
                if let Some(ref diffs_json) = msg.file_diffs
                    && let Ok(diffs) =
                        serde_json::from_str::<Vec<FileDiff>>(diffs_json)
                    {
                        for d in diffs {
                            file_map.insert(d.file.clone(), d);
                        }
                    }
            }
            all_diffs = file_map.into_values().collect();
        }

        lines.push(Line::from(vec![
            Span::styled(
                "Changed Files",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({})", all_diffs.len()),
                Style::default().fg(palette.muted),
            ),
        ]));

        if all_diffs.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "(no changes yet)",
                Style::default().fg(palette.muted),
            )]));
        } else {
            // Sort: modified first, then added, then deleted;
            // within each group, stable by filename to prevent visual jumping.
            all_diffs.sort_by(|a, b| {
                let a_key = match a.status.as_deref() {
                    Some("modified") => 0,
                    Some("added") => 1,
                    Some("deleted") => 2,
                    _ => 3,
                };
                let b_key = match b.status.as_deref() {
                    Some("modified") => 0,
                    Some("added") => 1,
                    Some("deleted") => 2,
                    _ => 3,
                };
                a_key.cmp(&b_key).then_with(|| a.file.cmp(&b.file))
            });

            let content_width = sidebar_content_width;

            for d in &all_diffs {
                let filename = Path::new(&d.file)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| d.file.clone());

                let show_add = d.additions > 0;
                let show_del = d.deletions > 0;

                let add_str = format!("+{}", d.additions);
                let del_str = format!("-{}", d.deletions);

                let file_span =
                    Span::styled(filename.clone(), Style::default().fg(palette.text));
                let add_span =
                    Span::styled(add_str.clone(), Style::default().fg(palette.diff_add));
                let del_span = Span::styled(
                    del_str.clone(),
                    Style::default().fg(palette.diff_delete),
                );

                // Right-align counts
                let fw = UnicodeWidthStr::width(filename.as_str());
                let aw = if show_add {
                    UnicodeWidthStr::width(add_str.as_str())
                } else {
                    0
                };
                let dw = if show_del {
                    UnicodeWidthStr::width(del_str.as_str())
                } else {
                    0
                };
                let gap_count = if show_add && show_del { 1 } else { 0 };
                let padding = content_width.saturating_sub(fw + aw + dw + gap_count);

                let mut spans = vec![file_span, Span::raw(" ".repeat(padding))];
                if show_add {
                    spans.push(add_span);
                }
                if show_del {
                    if show_add {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(del_span);
                }
                lines.push(Line::from(spans));
            }
        }

        // Todos section
        if !todos.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("Todos ({})", todos.len()),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]));

            for todo in todos {
                let (checkbox, style) = match todo.status.as_str() {
                    "completed" => (
                        "✔ ",
                        Style::default()
                            .fg(palette.muted)
                            .add_modifier(Modifier::CROSSED_OUT),
                    ),
                    "in_progress" => ("● ", Style::default().fg(palette.accent)),
                    "pending" => ("○ ", Style::default().fg(palette.text)),
                    _ => ("○ ", Style::default().fg(palette.text)),
                };

                let content = &todo.content;
                lines.push(Line::from(vec![
                    Span::styled(checkbox.to_string(), style),
                    Span::styled(content.as_str(), style),
                ]));
            }
        }

        // Undo state
        if let Some(ctx) = chat_context
            && ctx.is_reverted() {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    "⚠ Undo active",
                    Style::default().fg(palette.warning),
                )]));
            }

        // ── Background ──
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.panel)),
            area,
        );

        // ── Footer: workspace path ──
        let display_path = workspace_root.to_string_lossy().to_string();
        let display_path = display_path.replace(
            &dirs::home_dir().unwrap_or_default().to_string_lossy().to_string(),
            "~",
        );
        let display_path = shorten(&display_path, sidebar_content_width);
        let footer_lines: Vec<Line<'static>> = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Workspace",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![Span::styled(
                display_path,
                Style::default().fg(palette.muted),
            )]),
            Line::from(""),
            Line::from(""),
        ];
        let footer_height: u16 = footer_lines.len() as u16;

        // ── Layout: scrollable content + fixed footer ──
        let content_height = inner.height.saturating_sub(footer_height);
        let content_area = Rect {
            height: content_height,
            ..inner
        };
        let footer_area = Rect {
            y: area.y + area.height.saturating_sub(footer_height),
            height: footer_height,
            ..inner
        };

        // Estimate total lines for scroll max
        self.total_lines = lines
            .iter()
            .map(|line| {
                let w: usize = line
                    .spans
                    .iter()
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum();
                if w == 0 {
                    1
                } else {
                    (w + sidebar_content_width - 1) / sidebar_content_width.max(1)
                }
            })
            .sum();

        self.viewport_lines = content_height as usize;
        let max_scroll = self.total_lines.saturating_sub(self.viewport_lines.max(1));
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        // Render scrollable content
        let paragraph = Paragraph::new(lines)
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset as u16, 0));
        frame.render_widget(paragraph, content_area);

        // Render fixed footer (workspace path)
        let footer_paragraph =
            Paragraph::new(footer_lines).style(Style::default().fg(palette.text));
        frame.render_widget(footer_paragraph, footer_area);

    }
}

/// Truncate a string to fit within `max_width` characters, appending `…` when truncated.
fn shorten(s: &str, max_width: usize) -> String {
    let width = UnicodeWidthStr::width(s);
    if width <= max_width || max_width < 3 {
        return s.to_string();
    }
    let mut result = String::with_capacity(max_width);
    let mut current_width = 0;
    for ch in s.chars() {
        let w = UnicodeWidthStr::width(ch.to_string().as_str());
        if current_width + w + 1 > max_width {
            // +1 for the ellipsis
            result.push('…');
            break;
        }
        result.push(ch);
        current_width += w;
    }
    result
}
