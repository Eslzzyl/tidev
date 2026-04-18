use crate::{
    markdown_render::{WrapOptions, render_markdown_text_with_width_and_cwd, word_wrap_line},
    session::{Message, MessageRole, ToolCall},
    theme::ThemePalette,
    tooling::{TodoItem, canonical_tool_name},
};
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::diff_render::render_unified_diff_text;
use super::permission::RunningStatus;
use super::permission::RunningSubagentExecution;
use super::{
    App, MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind,
    MessageRenderCacheValue, render::*,
};

const TOOL_OUTPUT_PREVIEW_LINES: usize = 5;
const TOOL_OUTPUT_EXPANDED_MAX_LINES: usize = 100;

#[derive(Clone, Debug)]
struct ToolResultCardRange {
    message_id: Uuid,
    start_line: usize,
    end_line: usize,
}

struct RenderContext<'a> {
    palette: ThemePalette,
    #[allow(dead_code)]
    workspace_root: &'a Path,
    expanded_tool_results: &'a HashSet<Uuid>,
    expanded_tool_outputs: &'a HashMap<Uuid, String>,
}

fn render_tool_call_with_result(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    body_width: usize,
    ctx: &RenderContext<'_>,
) -> Vec<Line<'static>> {
    let palette = ctx.palette;
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);

    if matches!(canonical_name, "list" | "grep" | "glob" | "read") {
        return render_tool_call_summary_line(
            tool_call,
            tool_result,
            body_width,
            palette,
            ctx,
        );
    }

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    let call_lines = render_tool_call_lines(tool_call, body_width, palette);
    lines.extend(call_lines);

    if let Some(result_msg) = tool_result {
        let result_lines = render_tool_result_detail_lines(result_msg, body_width, ctx);
        if !result_lines.is_empty() {
            lines.push(Line::from(""));
            lines.extend(result_lines);
        }
    }

    lines.push(Line::from(""));
    lines
}

fn render_tool_call_summary_line(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    body_width: usize,
    palette: ThemePalette,
    ctx: &RenderContext<'_>,
) -> Vec<Line<'static>> {
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);
    let fields = summarize_tool_arguments(&tool_call.name, &tool_call.arguments);

    let get_field = |name: &str| {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };

    let (action_label, target) = match canonical_name {
        "list" => {
            let path = get_field("path").unwrap_or(".");
            ("List", path.to_string())
        }
        "grep" => {
            let pattern = get_field("pattern").unwrap_or("");
            let path = get_field("path").unwrap_or(".");
            ("Search", format!("\"{}\" in {}", pattern, path))
        }
        "glob" => {
            let pattern = get_field("pattern").unwrap_or("*");
            let path = get_field("path").unwrap_or(".");
            ("Find", format!("{} in {}", pattern, path))
        }
        "read" => {
            let path = get_field("path").unwrap_or("file");
            ("Read", path.to_string())
        }
        _ => {
            let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width);
            return vec![Line::from(vec![Span::styled(
                summary,
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )])];
        }
    };

    let result_suffix = if let Some(result_msg) = tool_result {
        let output = tool_output_from_message(result_msg, ctx).trim();
        compute_tool_result_suffix(canonical_name, output)
    } else {
        " ...".to_string()
    };

    let line = Line::from(vec![
        Span::styled(
            format!("{} ", action_label),
            Style::default().fg(palette.accent_soft),
        ),
        Span::styled(
            target.clone(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(result_suffix, Style::default().fg(palette.muted)),
    ]);

    vec![line]
}

fn compute_tool_result_suffix(canonical_name: &str, output: &str) -> String {
    match canonical_name {
        "list" => {
            let count = if output.trim() == "(empty)" {
                0
            } else {
                output
                    .lines()
                    .skip(1)
                    .filter(|line| !line.trim().is_empty())
                    .count()
            };
            format!(" → {} items", count)
        }
        "grep" | "glob" => {
            if tool_output_is_error(output) {
                let count = if output.is_empty() {
                    0
                } else {
                    output.lines().count()
                };
                format!(" → failed ({} lines)", count)
            } else {
                let count = if output.is_empty() {
                    0
                } else {
                    output.lines().count()
                };
                format!(" → {} matches", count)
            }
        }
        "read" => {
            if tool_output_is_error(output) {
                " → error".to_string()
            } else {
                let line_range = parse_line_range_from_read_output(output);
                let truncated = tool_output_is_truncated(output);
                match line_range {
                    Some((start, end)) => {
                        if truncated {
                            format!(" → Line {}-{} (truncated)", start, end)
                        } else {
                            format!(" → Line {}-{}", start, end)
                        }
                    }
                    None => {
                        let total_lines = output.lines().count();
                        if total_lines == 0 {
                            " → empty".to_string()
                        } else if truncated {
                            format!(" → First {} lines (truncated)", total_lines)
                        } else {
                            format!(" → All {} lines", total_lines)
                        }
                    }
                }
            }
        }
        _ => String::new(),
    }
}

fn tool_output_is_truncated(output: &str) -> bool {
    output.contains("output truncated:")
        || output.contains("... (truncated)")
        || output.contains("(Output capped at")
        || output.contains("[truncated]")
}

fn render_tool_call_lines(
    tool_call: &ToolCall,
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let fields = summarize_tool_arguments(&tool_call.name, &tool_call.arguments);

    let get_field = |name: &str| {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };

    let mut lines = Vec::new();

    let canonical_display = canonical_tool_name(&tool_call.name)
        .map(|s| s.to_string())
        .unwrap_or_else(|| tool_call.name.clone());

    lines.push(Line::from(vec![
        Span::styled("Tool: ", Style::default().fg(palette.muted)),
        Span::styled(
            canonical_display.clone(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or("");
    match canonical_name {
        "bash" => {
            let command = get_field("command").unwrap_or("");
            let desc = get_field("description");

            if let Some(d) = desc {
                lines.push(Line::from(vec![
                    Span::styled("  Description: ", Style::default().fg(palette.muted)),
                    Span::styled(d.to_string(), Style::default().fg(palette.text)),
                ]));
            }

            for line in command.lines() {
                lines.push(Line::from(vec![
                    Span::styled("  $ ", Style::default().fg(palette.accent)),
                    Span::styled(
                        shorten_single_line(line, body_width.saturating_sub(4)),
                        Style::default().fg(palette.text),
                    ),
                ]));
            }
        }
        "write" => {
            let path = get_field("path").unwrap_or("file");
            lines.push(Line::from(vec![
                Span::styled("  Path: ", Style::default().fg(palette.muted)),
                Span::styled(path.to_string(), Style::default().fg(palette.text)),
            ]));
        }
        _ => {
            let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width);
            for line in summary.lines() {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(line.to_string(), Style::default().fg(palette.text)),
                ]));
            }
        }
    }

    lines
}

fn render_tool_result_detail_lines(
    message: &Message,
    body_width: usize,
    ctx: &RenderContext<'_>,
) -> Vec<Line<'static>> {
    let palette = ctx.palette;
    let output = tool_output_from_message(message, ctx);
    let is_error = tool_output_is_error(output);
    let tool_name = message.tool_name.as_deref().unwrap_or(message.role.label());
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);

    if !is_error
        && matches!(canonical_name, "edit" | "write" | "apply_patch")
        && let Some(diff_lines) = render_unified_diff_text(output, body_width, palette)
    {
        return diff_lines;
    }

    if canonical_name == "todowrite" && !is_error {
        #[derive(serde::Deserialize)]
        struct RawTodo {
            content: String,
            status: Option<String>,
            priority: Option<String>,
        }

        let raw_todos = if let Ok(todos) = serde_json::from_str::<Vec<RawTodo>>(output) {
            Some(todos)
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
            value
                .get("todos")
                .and_then(|v| serde_json::from_value::<Vec<RawTodo>>(v.clone()).ok())
        } else {
            None
        };

        if let Some(raw_todos) = raw_todos {
            let todos: Vec<TodoItem> = raw_todos
                .into_iter()
                .map(|r| TodoItem {
                    content: r.content,
                    status: r.status.unwrap_or_else(|| "pending".to_string()),
                    priority: r.priority.unwrap_or_else(|| "medium".to_string()),
                })
                .collect();
            return render_todos_checkbox_list(&todos, body_width, palette);
        }
    }

    render_output_preview_lines(
        output,
        body_width,
        is_error,
        Some(message.id),
        ctx.expanded_tool_results,
        palette,
    )
}

fn render_todos_checkbox_list(
    todos: &[TodoItem],
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = vec![line_with_style("Updated todo list:", palette.accent_soft)];

    if todos.is_empty() {
        lines.push(line_with_style("  (no items)", palette.muted));
        return lines;
    }

    let max_content_len = body_width.saturating_sub(6).max(1);

    for todo in todos {
        let (checkbox, style) = match todo.status.as_str() {
            "completed" => (
                "✔ ",
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
            "pending" => ("○ ", Style::default().fg(palette.text)),
            "cancelled" => ("✗ ", Style::default().fg(palette.muted)),
            _ => ("○ ", Style::default().fg(palette.text)),
        };

        let priority_marker = if todo.priority == "high" { "⚠ " } else { "" };

        let content = shorten(&todo.content, max_content_len);
        lines.push(Line::from(vec![
            Span::styled(format!("  {priority_marker}{checkbox}"), style),
            Span::styled(content, style),
        ]));
    }

    lines
}

fn tool_output_from_message<'a>(message: &'a Message, ctx: &'a RenderContext<'_>) -> &'a str {
    ctx.expanded_tool_outputs
        .get(&message.id)
        .map(|output| output.as_str())
        .unwrap_or_else(|| message.content.as_str())
}

fn render_output_preview_lines(
    output: &str,
    body_width: usize,
    is_error: bool,
    message_id: Option<Uuid>,
    expanded_tool_results: &HashSet<Uuid>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let is_expanded = message_id.is_some_and(|id| expanded_tool_results.contains(&id));
    let max_lines = if is_expanded {
        TOOL_OUTPUT_EXPANDED_MAX_LINES
    } else if is_error {
        4
    } else {
        TOOL_OUTPUT_PREVIEW_LINES
    };

    let prefix = if is_error { "!" } else { "↳" };
    let fg = if is_error {
        palette.error
    } else {
        palette.text
    };
    let prefix_style = Style::default().fg(if is_error {
        palette.error
    } else {
        palette.accent_soft
    });

    let total_output_lines = output.lines().count();
    let wrap_width = body_width.saturating_sub(2);

    for line in output.lines().take(max_lines) {
        if is_expanded {
            let owned_line = Line::from(line.to_string());
            let wrapped =
                word_wrap_line(&owned_line, WrapOptions::new(wrap_width).break_words(true));
            for (wrap_idx, wrapped_line) in wrapped.iter().enumerate() {
                let effective_prefix = if wrap_idx == 0 { prefix } else { " " };
                let mut spans = vec![Span::styled(format!("{} ", effective_prefix), prefix_style)];
                spans.extend(
                    wrapped_line.spans.iter().map(|span| {
                        Span::styled(span.content.to_string(), Style::default().fg(fg))
                    }),
                );
                lines.push(Line::from(spans));
            }
        } else {
            lines.push(line_with_prefix(
                prefix,
                &shorten_single_line(line, wrap_width),
                prefix_style,
                Style::default().fg(fg),
            ));
        }
    }

    if total_output_lines > max_lines {
        lines.push(line_with_prefix(
            prefix,
            &format!("... {} more line(s)", total_output_lines - max_lines),
            Style::default().fg(palette.muted),
            Style::default().fg(palette.muted),
        ));
    } else if total_output_lines > TOOL_OUTPUT_PREVIEW_LINES && message_id.is_some() {
        let hint = if is_expanded {
            "▲ Click to collapse"
        } else {
            "▼ Click to expand"
        };
        lines.push(line_with_prefix(
            prefix,
            hint,
            Style::default().fg(palette.muted),
            Style::default().fg(palette.muted),
        ));
    }

    if lines.is_empty() {
        lines.push(line_with_style("(no output)", palette.muted));
    }

    lines
}

impl App {
    pub(super) fn render_chat(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette();
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        let sidebar_visible = area.width >= self.config.ui.sidebar_width.saturating_add(55);
        let main_area = if sidebar_visible {
            let split = Layout::horizontal([
                Constraint::Min(20),
                Constraint::Length(self.config.ui.sidebar_width),
            ])
            .split(area);
            self.sidebar_area = Some(split[1]);
            self.render_sidebar(frame, split[1]);
            split[0]
        } else {
            area
        };

        let composer_height = self
            .composer
            .preferred_height(
                main_area.width.saturating_sub(4),
                self.config.ui.max_input_lines,
            )
            .min(main_area.height.saturating_sub(3).max(3));

        if let Some(dialog) = self.question_dialog.clone() {
            let question_height = dialog
                .prompt_height(main_area.width, composer_height)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(question_height),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_question_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            self.render_retrying_hint(frame, layout[3]);
            return;
        }

        let layout = Layout::vertical([
            Constraint::Min(6),
            Constraint::Length(composer_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(main_area);

        self.render_messages(frame, layout[0]);
        let prompt_title = self.mode.title().to_string();
        self.render_input_block(
            frame,
            layout[1],
            &prompt_title,
            self.composer.placeholder(),
            false,
        );
        self.render_at_mention_palette(frame, layout[1]);
        self.render_prompt_footer(frame, layout[2]);
        self.render_retrying_hint(frame, layout[3]);
        self.render_command_palette(frame, layout[1]);
    }

    pub(super) fn render_messages(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut title = format!("Conversation · {}", shorten(&self.conversation.title, 32),);
        if !self.message_follow_tail {
            title.push_str(" · history");
        }
        if self.conversation.parent_session_id.is_some() {
            title.push_str(" · SUBSESSION");
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(title);
        frame.render_widget(block, area);

        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let scrollbar_area = if inner.width > 1 {
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            (chunks[0], Some(chunks[1]))
        } else {
            (inner, None)
        };

        let content_area = scrollbar_area.0;
        self.message_content_area = Some(content_area);
        self.message_viewport_lines = content_area.height as usize;
        let content_width = content_area.width.max(1) as usize;
        let (mut text, mut total_lines, card_ranges, rendered_virtualized, virtualized_render_scroll) =
            self.messages_text(Some(content_width));

        // Add tool running state
        for running in &self.running_tool_executions {
            if running.status != RunningStatus::Running {
                continue;
            }
            let canonical_name =
                canonical_tool_name(&running.tool_call.name).unwrap_or(&running.tool_call.name);
            let action = match canonical_name {
                "edit" | "write" => "Editing",
                "read" => "Reading",
                "bash" => "Running",
                "grep" | "glob" | "list" => "Searching",
                _ => "Executing",
            };

            let fields =
                summarize_tool_arguments(&running.tool_call.name, &running.tool_call.arguments);
            let target = fields
                .iter()
                .find(|(k, _)| k == "path" || k == "filePath" || k == "command")
                .map(|(_, v)| v.as_str())
                .unwrap_or(&running.tool_call.name);

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", action),
                    Style::default().fg(palette.accent_soft),
                ),
                Span::styled(
                    shorten(target, 64),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("...", Style::default().fg(palette.muted)),
            ]);

            let card_lines = decorate_card_lines(
                vec![Line::from(""), line, Line::from("")],
                content_width,
                palette.panel_alt,
            );
            text.lines.extend(card_lines);
            total_lines += 3;
        }

        if self.conversation.parent_session_id.is_none() {
            for running_subagent in &self.running_subagent_executions {
                let card_lines =
                    self.render_running_subagent_lines(running_subagent, content_width);
                if card_lines.is_empty() {
                    continue;
                }

                let decorated_lines = decorate_card_lines(card_lines, content_width, palette.panel);
                total_lines += decorated_lines.len();
                text.lines.extend(decorated_lines);
            }
        }

        self.message_total_lines = total_lines;

        let max_scroll = total_lines.saturating_sub(self.message_viewport_lines);
        let scroll = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = scroll;
        self.message_follow_tail = scroll >= max_scroll;
        let render_scroll = if rendered_virtualized {
            virtualized_render_scroll
        } else {
            scroll
        };

        // Calculate screen positions for tool result cards
        self.tool_result_card_bounds.clear();
        for card_range in card_ranges {
            let screen_start = card_range.start_line.saturating_sub(render_scroll);
            let screen_end = card_range.end_line.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }

            let visible_start = screen_start.max(0) as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;

            if visible_start < visible_end {
                let card_rect = Rect {
                    x: content_area.x,
                    y: content_area.y.saturating_add(visible_start),
                    width: content_area.width,
                    height: visible_end.saturating_sub(visible_start),
                };
                self.tool_result_card_bounds
                    .push((card_range.message_id, card_rect));
            }
        }

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(palette.background).fg(palette.text))
            .scroll((render_scroll as u16, 0));

        frame.render_widget(paragraph, content_area);

        if let Some(scrollbar_area) = scrollbar_area.1 {
            self.render_scrollbar(frame, scrollbar_area, scroll, max_scroll);
        }
    }

    fn render_sidebar(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut lines = Vec::new();

        // Model info
        lines.push(Line::from(vec![Span::styled(
            "Model",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "{} / {}",
                self.active_model.provider_id, self.active_model.model_id
            ),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            if self.active_model.api_key_present() {
                "✓ API key present"
            } else {
                "✗ API key missing"
            },
            if self.active_model.api_key_present() {
                Style::default().fg(palette.success)
            } else {
                Style::default().fg(palette.error)
            },
        )]));

        lines.push(Line::from(""));

        // Working directory
        lines.push(Line::from(shorten(
            &self.workspace_root.display().to_string(),
            32,
        )));

        // Todos section
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("Todos ({})", self.todos.len()),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        let sidebar_width = self.config.ui.sidebar_width as usize;
        let max_content_len = sidebar_width.saturating_sub(4);

        for todo in &self.todos {
            let (checkbox, style) = match todo.status.as_str() {
                "completed" => (
                    "✔ ",
                    Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                "in_progress" => ("● ", Style::default().fg(palette.accent)),
                "pending" => ("○ ", Style::default().fg(palette.text)),
                "cancelled" => ("✗ ", Style::default().fg(palette.muted)),
                _ => ("○ ", Style::default().fg(palette.text)),
            };

            let priority_marker = if todo.priority == "high" { "⚠ " } else { "" };

            let content = shorten(&todo.content, max_content_len);
            lines.push(Line::from(vec![
                Span::styled(format!("{priority_marker}{checkbox}"), style),
                Span::styled(content, style),
            ]));
        }

        // Undo state (only when active)
        if self.conversation.is_reverted() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "⚠ Undo active",
                Style::default().fg(palette.warning),
            )]));
        }

        let paragraph = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title("Sidebar"),
            )
            .style(Style::default().fg(palette.text));

        frame.render_widget(paragraph, area);
    }

    fn messages_text(
        &mut self,
        content_width: Option<usize>,
    ) -> (Text<'static>, usize, Vec<ToolResultCardRange>, bool, usize) {
        let started_at = Instant::now();
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);
        let messages = self.conversation.visible_messages();

        let mut lines = Vec::new();
        let mut card_ranges = Vec::new();

        // Header for subsessions (always visible at top)
        let header_lines = if self.conversation.parent_session_id.is_some() {
            vec![
                line_with_style(
                    "SUBSESSION active — viewing a child session.",
                    palette.accent_soft,
                ),
                line_with_style(
                    "Press Ctrl+X then Up arrow to return to the parent session.",
                    palette.muted,
                ),
                Line::from(""),
            ]
        } else {
            Vec::new()
        };

        // Handle empty messages case
        if messages.is_empty() {
            lines.extend(header_lines);
            lines.extend(decorate_card_lines(
                vec![
                    line_with_style("No messages yet.", palette.muted),
                    line_with_style("Start with a prompt in the input box below.", palette.muted),
                ],
                width,
                palette.panel,
            ));
            let total_lines = lines.len().max(1);
            return (Text::from(lines), total_lines, card_ranges, false, 0);
        }

        // Check if there are streaming messages (need to force rebuild index)
        let has_streaming = messages
            .iter()
            .any(|m| m.streaming && matches!(m.role, MessageRole::Assistant));

        // Update layout index (force rebuild for streaming messages)
        self.update_message_layout_index(width, body_width, has_streaming);
        if let Some(scroll_offset) =
            self.resolve_message_scroll_target(&messages, width, body_width)
        {
            self.message_scroll_offset = scroll_offset;
            self.message_follow_tail = false;
            self.message_scroll_target = None;
        }

        // Calculate visible range based on scroll position
        let viewport = self.message_viewport_lines.max(1);
        let total_message_lines = self.message_layout_index.borrow().total_lines;
        let max_scroll = total_message_lines.saturating_sub(viewport);
        let scroll = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };
        self.message_scroll_offset = scroll;

        // Find visible blocks
        let visible_blocks = self.find_visible_message_blocks(scroll, viewport);

        // Add header lines
        let header_line_count = header_lines.len();
        lines.extend(header_lines);

        // Calculate render_scroll for virtualized rendering
        // The visible blocks may start before 'scroll' (due to buffer zone),
        // so we need to skip those lines when rendering.
        // Also, if first block starts after 'scroll', we need padding.
        let first_block_start = visible_blocks.first().map(|b| b.start_line).unwrap_or(0);
        let (render_scroll, padding_lines) = if first_block_start < scroll {
            (scroll - first_block_start, 0)
        } else if first_block_start > scroll {
            (0, first_block_start - scroll)
        } else {
            (0, 0)
        };

        // Add padding lines if first block starts after scroll position
        for _ in 0..padding_lines {
            lines.push(Line::from(""));
        }

        // Create render context for tool calls
        let expanded_tool_outputs = self.load_expanded_tool_outputs(&messages);
        let ctx = RenderContext {
            palette,
            workspace_root: self.workspace_root.as_path(),
            expanded_tool_results: &self.expanded_tool_results,
            expanded_tool_outputs: &expanded_tool_outputs,
        };

        // Render visible blocks
        let mut current_line_offset = header_line_count + padding_lines;
        for block in &visible_blocks {
            let block_lines = self.render_message_block_to_lines(
                &messages,
                block,
                width,
                body_width,
                &mut card_ranges,
                current_line_offset,
                &ctx,
            );
            current_line_offset += block_lines.len();
            lines.extend(block_lines);
        }

        // Calculate total lines from layout index
        let total_lines = header_line_count + total_message_lines;

        let elapsed = started_at.elapsed();
        if elapsed > Duration::from_millis(12) {
            let (hits, misses, entries) = self.message_render_cache_stats();
            crate::log_debug!(
                "messages_text: messages={}, visible_blocks={}, width={}, took={:?}, cache_hits={}, cache_misses={}, cache_entries={}",
                messages.len(),
                visible_blocks.len(),
                width,
                elapsed,
                hits,
                misses,
                entries
            );
        }

        (Text::from(lines), total_lines, card_ranges, true, render_scroll)
    }

    fn cached_render_message_cards(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        if message.streaming && matches!(message.role, MessageRole::Assistant) {
            self.record_message_render_cache_miss();
            return self.render_message_cards(message, body_width);
        }

        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id,
            width: body_width,
            kind: MessageRenderCacheKind::Cards,
        };
        let tick = self.next_message_render_cache_tick();

        {
            let mut cache = self.message_render_cache.borrow_mut();
            if let Some(entry) = cache.get_mut(&key) {
                entry.last_used_tick = tick;
                self.record_message_render_cache_hit();
                match &entry.value {
                    MessageRenderCacheValue::Cards(cards) => return cards.clone(),
                }
            }
        }

        self.record_message_render_cache_miss();
        let cards = self.render_message_cards(message, body_width);

        {
            let mut cache = self.message_render_cache.borrow_mut();
            cache.insert(
                key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: tick,
                },
            );
        }

        self.prune_message_render_cache_if_needed();
        cards
    }

    fn load_expanded_tool_outputs(&self, messages: &[Message]) -> HashMap<Uuid, String> {
        let mut outputs = HashMap::new();

        for message in messages {
            if !self.expanded_tool_results.contains(&message.id) {
                continue;
            }

            if let Ok(Some(output)) = self
                .store
                .load_tool_event_output(self.conversation.session_id, message.id)
            {
                outputs.insert(message.id, output);
            }
        }

        outputs
    }

    fn render_message_cards(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        let palette = self.palette();

        match message.role {
            MessageRole::User => vec![(palette.panel_alt, {
                let mut content_lines = self.render_text_body_lines(
                    &message.content,
                    body_width.saturating_sub(2),
                    Some(self.workspace_root.as_path()),
                );

                for attachment in &message.attachments {
                    content_lines.push(line_with_style(&attachment.summary(), palette.accent_soft));
                }

                let mut lines = Vec::new();
                lines.push(Line::from(""));
                for line in content_lines {
                    let mut spans = vec![Span::styled(
                        "┃ ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )];
                    spans.extend(line.spans);
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(""));

                lines
            })],
            MessageRole::Assistant => {
                let mut cards = Vec::new();

                let body_lines = self.render_assistant_body_lines(message, body_width);
                if !body_lines.is_empty() {
                    let mut lines_with_margin = Vec::new();
                    lines_with_margin.push(Line::from(""));
                    lines_with_margin.extend(body_lines);
                    lines_with_margin.push(Line::from(""));
                    cards.push((palette.background, lines_with_margin));
                }

                for _tool_call in &message.tool_calls {
                    // Tool calls are rendered in messages_text along with their results
                }

                cards
            }
            MessageRole::Tool => {
                // Return nothing here; handled by the loop in messages_text
                Vec::new()
            }
            MessageRole::System => {
                let content_lines = self.render_text_body_lines(
                    &message.content,
                    body_width,
                    Some(self.workspace_root.as_path()),
                );
                let mut lines = Vec::new();
                lines.push(Line::from(""));
                lines.extend(content_lines);
                lines.push(Line::from(""));
                vec![(palette.background, lines)]
            }
            MessageRole::Error => {
                let error_lines = self.render_error_body_lines(message, body_width);
                let mut lines = Vec::new();
                lines.push(Line::from(""));
                lines.extend(error_lines);
                lines.push(Line::from(""));
                vec![(palette.panel_light, lines)]
            }
        }
    }

    fn render_assistant_body_lines(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if !message.reasoning.trim().is_empty() {
            lines.extend(self.render_reasoning_lines(&message.reasoning, body_width));
            if !message.content.trim().is_empty() {
                lines.push(Line::from(""));
            }
        }

        if message.streaming && matches!(message.role, MessageRole::Assistant) {
            for line in message.content.lines().skip_while(|line| line.is_empty()) {
                lines.push(Line::from(line.to_string()));
            }
        } else if !message.content.is_empty() {
            if let Some(diff_lines) =
                render_unified_diff_text(&message.content, body_width, self.palette())
            {
                lines.extend(diff_lines);
            } else {
                let rendered = render_markdown_text_with_width_and_cwd(
                    &message.content,
                    Some(body_width),
                    Some(self.workspace_root.as_path()),
                );
                lines.extend(rendered.lines);
            }
        }

        if lines.is_empty()
            && !message.streaming
            && message.reasoning.trim().is_empty()
            && message.tool_calls.is_empty()
        {
            lines.push(line_with_style("(empty)", self.palette().muted));
        }

        lines
    }

    fn render_text_body_lines(
        &self,
        text: &str,
        body_width: usize,
        cwd: Option<&std::path::Path>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if text.trim().is_empty() {
            lines.push(line_with_style("(empty)", self.palette().muted));
        } else {
            let rendered = render_markdown_text_with_width_and_cwd(text, Some(body_width), cwd);
            lines.extend(rendered.lines);
        }
        lines
    }

    fn render_error_body_lines(&self, message: &Message, body_width: usize) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();
        let error_text = if message.content.trim().is_empty() {
            "Request failed.".to_string()
        } else {
            message.content.clone()
        };

        for line in error_text.lines() {
            lines.push(line_with_prefix(
                "!",
                &shorten_single_line(line, body_width.saturating_sub(2)),
                Style::default().fg(palette.error),
                Style::default().fg(palette.error),
            ));
        }

        if lines.is_empty() {
            lines.push(line_with_style("! Request failed.", palette.error));
        }

        lines
    }

    fn render_reasoning_lines(&self, reasoning: &str, body_width: usize) -> Vec<Line<'static>> {
        render_reasoning_markdown_lines(
            reasoning,
            body_width,
            Some(self.workspace_root.as_path()),
            self.palette(),
        )
    }

    fn render_running_subagent_lines(
        &self,
        execution: &RunningSubagentExecution,
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let task_summary = summarize_tool_call(
            &execution.tool_call.name,
            &execution.tool_call.arguments,
            body_width,
        );
        let child_session_label = self
            .store
            .load_session_record(execution.child_session_id)
            .ok()
            .flatten()
            .map(|record| {
                format!(
                    "{} · {}",
                    shorten(&record.title, 44),
                    execution.child_session_id.simple()
                )
            })
            .unwrap_or_else(|| execution.child_session_id.simple().to_string());

        let mut lines = vec![line_with_style("Subagent running", palette.accent_soft)];
        lines.push(line_with_prefix(
            "↳",
            &task_summary,
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        ));
        lines.push(line_with_prefix(
            "↳",
            &execution.status_text,
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        ));

        if let Some(tool_call) = &execution.current_tool_call {
            let current_tool =
                summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width);
            lines.push(line_with_prefix(
                "↳",
                &format!("Tool: {current_tool}"),
                Style::default().fg(palette.accent_soft),
                Style::default().fg(palette.text),
            ));
        }

        lines.push(line_with_prefix(
            "↳",
            &format!("Session {child_session_label}"),
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        ));
        lines.push(line_with_style(
            "Ctrl+X then arrows to inspect the child session.",
            palette.muted,
        ));

        lines
    }

    #[allow(dead_code)]
    fn render_attachment_preview_lines(
        &self,
        attachments: &[String],
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        for attachment in attachments {
            lines.push(line_with_prefix(
                "↳",
                &shorten_single_line(attachment, body_width.saturating_sub(2)),
                Style::default().fg(palette.accent_soft),
                Style::default().fg(palette.text),
            ));
        }

        lines
    }

    #[allow(dead_code)]
    fn render_output_preview_lines(
        &self,
        output: &str,
        body_width: usize,
        is_error: bool,
        message_id: Option<Uuid>,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        let is_expanded = message_id.is_some_and(|id| self.expanded_tool_results.contains(&id));
        let max_lines = if is_expanded {
            TOOL_OUTPUT_EXPANDED_MAX_LINES
        } else if is_error {
            4
        } else {
            TOOL_OUTPUT_PREVIEW_LINES
        };

        let prefix = if is_error { "!" } else { "↳" };
        let fg = if is_error {
            palette.error
        } else {
            palette.text
        };
        let prefix_style = Style::default().fg(if is_error {
            palette.error
        } else {
            palette.accent_soft
        });

        let total_output_lines = output.lines().count();
        let wrap_width = body_width.saturating_sub(2);

        for line in output.lines().take(max_lines) {
            if is_expanded {
                let owned_line = Line::from(line.to_string());
                let wrapped =
                    word_wrap_line(&owned_line, WrapOptions::new(wrap_width).break_words(true));
                for (wrap_idx, wrapped_line) in wrapped.iter().enumerate() {
                    let effective_prefix = if wrap_idx == 0 { prefix } else { " " };
                    let mut spans =
                        vec![Span::styled(format!("{} ", effective_prefix), prefix_style)];
                    spans.extend(wrapped_line.spans.iter().map(|span| {
                        Span::styled(span.content.to_string(), Style::default().fg(fg))
                    }));
                    lines.push(Line::from(spans));
                }
            } else {
                lines.push(line_with_prefix(
                    prefix,
                    &shorten_single_line(line, wrap_width),
                    prefix_style,
                    Style::default().fg(fg),
                ));
            }
        }

        if total_output_lines > max_lines {
            lines.push(line_with_prefix(
                prefix,
                &format!("... {} more line(s)", total_output_lines - max_lines),
                Style::default().fg(palette.muted),
                Style::default().fg(palette.muted),
            ));
        } else if total_output_lines > TOOL_OUTPUT_PREVIEW_LINES && message_id.is_some() {
            let hint = if is_expanded {
                "▲ Click to collapse"
            } else {
                "▼ Click to expand"
            };
            lines.push(line_with_prefix(
                prefix,
                hint,
                Style::default().fg(palette.muted),
                Style::default().fg(palette.muted),
            ));
        }

        if lines.is_empty() {
            lines.push(line_with_style("(no output)", palette.muted));
        }

        lines
    }

    /// Updates the message layout index to enable viewport virtualization.
    ///
    /// The layout index maintains a mapping from messages to their positions in
    /// the rendered output. This enables O(log n) binary search to find visible
    /// messages without rendering everything.
    ///
    /// The index is rebuilt when:
    /// - Width changes (line counts become invalid)
    /// - Messages are added/removed
    /// - Cache is cleared
    /// - Force rebuild is requested (for streaming messages)
    ///
    /// For incremental updates, only the tail (last few messages) is rebuilt,
    /// preserving existing block positions for unchanged messages.
    fn update_message_layout_index(&self, width: usize, body_width: usize, force_rebuild: bool) {
        let messages = self.conversation.visible_messages();
        let mut index = self.message_layout_index.borrow_mut();

        // Check if message count changed (new messages added or removed)
        let indexed_message_count = index
            .blocks
            .last()
            .map(|b| b.message_start_idx + b.message_count)
            .unwrap_or(0);
        let message_count_changed = indexed_message_count != messages.len();

        // Check if we need a full rebuild
        let needs_full_rebuild = force_rebuild
            || !index.valid
            || index.width != width
            || message_count_changed
            || index.blocks.is_empty() && !messages.is_empty();

        if needs_full_rebuild {
            index.blocks.clear();
            index.total_lines = 0;
            index.width = width;
            index.valid = true;

            let expanded_tool_outputs = self.load_expanded_tool_outputs(&messages);
            let ctx = RenderContext {
                palette: self.palette(),
                workspace_root: self.workspace_root.as_path(),
                expanded_tool_results: &self.expanded_tool_results,
                expanded_tool_outputs: &expanded_tool_outputs,
            };

            let mut current_line = 0;
            let mut i = 0;

            while i < messages.len() {
                // Build block without start_line (calculated below)
                let (message_id, message_count, line_count) =
                    self.build_message_block_data(&messages, i, width, body_width, &ctx);

                let block = super::MessageBlock {
                    message_id,
                    message_start_idx: i,
                    message_count,
                    start_line: current_line,
                    line_count,
                };

                current_line += line_count;
                i += message_count;
                index.blocks.push(block);
            }

            index.total_lines = current_line;
        }
    }

    fn resolve_message_scroll_target(
        &self,
        messages: &[Message],
        width: usize,
        body_width: usize,
    ) -> Option<usize> {
        let message_id = self.message_scroll_target?;

        // Create a minimal context for block data calculation
        let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
        let ctx = RenderContext {
            palette: self.palette(),
            workspace_root: self.workspace_root.as_path(),
            expanded_tool_results: &self.expanded_tool_results,
            expanded_tool_outputs: &expanded_tool_outputs,
        };

        let mut offset = 0;
        let mut i = 0;

        while i < messages.len() {
            if messages[i].id == message_id {
                return Some(offset);
            }

            let (_message_id, message_count, line_count) =
                self.build_message_block_data(messages, i, width, body_width, &ctx);
            offset += line_count;
            i += message_count;
        }

        None
    }

    /// Builds data for a single message block (without start_line).
    ///
    /// Returns (message_id, message_count, line_count).
    fn build_message_block_data(
        &self,
        messages: &[Message],
        start_idx: usize,
        width: usize,
        body_width: usize,
        ctx: &RenderContext<'_>,
    ) -> (Uuid, usize, usize) {
        let message = &messages[start_idx];
        let message_id = message.id;
        let palette = self.palette();

        let (message_count, line_count) = match message.role {
            MessageRole::Assistant => {
                // Count tool result messages that follow
                let mut count = 1;
                while start_idx + count < messages.len()
                    && matches!(messages[start_idx + count].role, MessageRole::Tool)
                {
                    count += 1;
                }

                // Calculate lines for assistant message
                let cards = self.cached_render_message_cards(message, body_width);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.background).len();
                }

                // Calculate lines for tool calls with results
                let tool_results_by_id: std::collections::HashMap<String, &Message> = {
                    let mut map = std::collections::HashMap::new();
                    let mut j = start_idx + 1;
                    while j < messages.len() && matches!(messages[j].role, MessageRole::Tool) {
                        if let Some(id) = &messages[j].tool_call_id {
                            map.insert(id.clone(), &messages[j]);
                        }
                        j += 1;
                    }
                    map
                };

                if !message.tool_calls.is_empty() {
                    for tool_call in &message.tool_calls {
                        let tool_result = tool_results_by_id.get(&tool_call.id).copied();
                        let card_lines =
                            render_tool_call_with_result(tool_call, tool_result, body_width, ctx);
                        if !card_lines.is_empty() {
                            lines +=
                                decorate_card_lines(card_lines, width, palette.panel_light).len();
                        }
                    }
                    lines += 1; // Empty line after tool calls
                }

                (count, lines)
            }
            MessageRole::User => {
                let cards = self.cached_render_message_cards(message, body_width);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.panel_alt).len();
                }
                lines += 1; // Empty line after user message
                (1, lines)
            }
            MessageRole::System => {
                let cards = self.cached_render_message_cards(message, body_width);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.background).len();
                }
                (1, lines)
            }
            MessageRole::Error => {
                let cards = self.cached_render_message_cards(message, body_width);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.panel_light).len();
                }
                (1, lines)
            }
            MessageRole::Tool => {
                // Tool messages are included in Assistant blocks, skip
                (1, 0)
            }
        };

        (message_id, message_count, line_count)
    }

    /// Finds message blocks that intersect with the visible viewport.
    ///
    /// Uses binary search for O(log n) complexity. Returns blocks with a
    /// buffer zone to ensure smooth scrolling.
    fn find_visible_message_blocks(
        &self,
        scroll: usize,
        viewport_height: usize,
    ) -> Vec<super::MessageBlock> {
        let index = self.message_layout_index.borrow();

        if index.blocks.is_empty() {
            return Vec::new();
        }

        let viewport_height = viewport_height.max(1);
        let max_scroll = index.total_lines.saturating_sub(viewport_height);
        let clamped_scroll = scroll.min(max_scroll);

        let visible_start = clamped_scroll.saturating_sub(5); // Buffer above
        let visible_end = clamped_scroll
            .saturating_add(viewport_height)
            .saturating_add(5); // Buffer below

        // Binary search for first block that could be visible
        let first_visible = index
            .blocks
            .partition_point(|block| block.start_line + block.line_count <= visible_start);

        // Collect all visible blocks
        let mut visible_blocks = Vec::new();
        for block in index.blocks.iter().skip(first_visible) {
            if block.start_line >= visible_end {
                break;
            }
            visible_blocks.push(block.clone());
        }

        visible_blocks
    }

    /// Renders a single message block to lines.
    ///
    /// This is the actual rendering logic, extracted for reuse in virtualization.
    fn render_message_block_to_lines(
        &self,
        messages: &[Message],
        block: &super::MessageBlock,
        width: usize,
        body_width: usize,
        card_ranges: &mut Vec<ToolResultCardRange>,
        current_line_offset: usize,
        ctx: &RenderContext<'_>,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        // Skip Tool messages - they're rendered as part of Assistant blocks
        if block.message_count == 0 {
            return lines;
        }

        let start_idx = block.message_start_idx;
        let message = &messages[start_idx];

        match message.role {
            MessageRole::Assistant => {
                // Render assistant message cards
                let assistant_cards = self.cached_render_message_cards(message, body_width);
                for (card_bg, card_lines) in assistant_cards {
                    if !card_lines.is_empty() {
                        lines.extend(decorate_card_lines(card_lines, width, card_bg));
                    }
                }

                // Collect tool results
                let tool_results_by_id: std::collections::HashMap<String, &Message> = {
                    let mut map = std::collections::HashMap::new();
                    let mut j = start_idx + 1;
                    while j < messages.len() && j < start_idx + block.message_count {
                        if matches!(messages[j].role, MessageRole::Tool)
                            && let Some(id) = &messages[j].tool_call_id
                        {
                            map.insert(id.clone(), &messages[j]);
                        }
                        j += 1;
                    }
                    map
                };

                // Render tool calls with results
                if !message.tool_calls.is_empty() {
                    for tool_call in &message.tool_calls {
                        let tool_result = tool_results_by_id.get(&tool_call.id).copied();
                        let tool_card_lines =
                            render_tool_call_with_result(tool_call, tool_result, body_width, ctx);
                        if !tool_card_lines.is_empty() {
                            let decorated =
                                decorate_card_lines(tool_card_lines, width, palette.panel_light);
                            if let Some(result_msg) = tool_result {
                                let start_line = current_line_offset + lines.len();
                                lines.extend(decorated);
                                let end_line = current_line_offset + lines.len();
                                card_ranges.push(ToolResultCardRange {
                                    message_id: result_msg.id,
                                    start_line,
                                    end_line,
                                });
                            } else {
                                lines.extend(decorated);
                            }
                        }
                    }
                    lines.push(Line::from(""));
                }
            }
            MessageRole::User | MessageRole::System | MessageRole::Error => {
                let cards = self.cached_render_message_cards(message, body_width);
                let bg = match message.role {
                    MessageRole::User => palette.panel_alt,
                    MessageRole::Error => palette.panel_light,
                    _ => palette.background,
                };
                for (_, card_lines) in cards {
                    if !card_lines.is_empty() {
                        lines.extend(decorate_card_lines(card_lines, width, bg));
                    }
                }
                if matches!(message.role, MessageRole::User) {
                    lines.push(Line::from(""));
                }
            }
            MessageRole::Tool => {
                // Tool messages are handled within Assistant blocks
            }
        }

        lines
    }

    fn render_scrollbar(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        scroll: usize,
        max_scroll: usize,
    ) {
        let palette = self.palette();
        if area.width == 0 || area.height == 0 {
            return;
        }

        let track_style = Style::default().bg(palette.background).fg(palette.border);
        let thumb_style = Style::default().bg(palette.background).fg(palette.accent);
        let height = area.height as usize;
        let mut lines = Vec::with_capacity(height);

        if max_scroll == 0 || height == 0 {
            for _ in 0..height {
                lines.push(Line::from(vec![Span::styled(" ", track_style)]));
            }
        } else {
            let thumb_height = ((height * height) / self.message_total_lines.max(1))
                .clamp(1, height)
                .max(1);
            let track_span = height.saturating_sub(thumb_height);
            let thumb_top = if track_span == 0 {
                0
            } else {
                ((scroll as f32 / max_scroll as f32) * track_span as f32).round() as usize
            };

            for row in 0..height {
                let is_thumb = row >= thumb_top && row < thumb_top + thumb_height;
                let style = if is_thumb { thumb_style } else { track_style };
                let glyph = if is_thumb { "█" } else { "░" };
                lines.push(Line::from(vec![Span::styled(glyph, style)]));
            }
        }

        let paragraph =
            Paragraph::new(Text::from(lines)).style(Style::default().bg(palette.background));
        frame.render_widget(paragraph, area);
    }
}

fn render_reasoning_markdown_lines(
    reasoning: &str,
    body_width: usize,
    cwd: Option<&std::path::Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let label_style = Style::default()
        .fg(palette.muted)
        .add_modifier(Modifier::DIM);
    let body_style = Style::default()
        .fg(palette.muted)
        .add_modifier(Modifier::DIM);

    lines.push(Line::from(vec![
        Span::styled("┃ ", label_style),
        Span::styled("Thinking:", body_style),
    ]));

    if reasoning.trim().is_empty() {
        lines.push(Line::from(vec![
            Span::styled("┃ ", label_style),
            Span::styled(String::new(), body_style),
        ]));
        return lines;
    }

    let content_width = body_width.saturating_sub(2).max(1);
    let rendered = render_markdown_text_with_width_and_cwd(reasoning, Some(content_width), cwd);

    if rendered.lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("┃ ", label_style),
            Span::styled(String::new(), body_style),
        ]));
        return lines;
    }

    for line in rendered.lines {
        let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
        spans.push(Span::styled("┃ ", label_style));
        spans.extend(line.spans.into_iter().map(|mut span| {
            span.style = span.style.patch(body_style);
            span
        }));
        lines.push(Line::from(spans));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::{render_reasoning_markdown_lines, render_tool_result_detail_lines, RenderContext};
    use crate::session::{Message, MessageRole};
    use crate::theme::ThemePalette;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use std::collections::{HashMap, HashSet};

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn text_lines_to_string(lines: &[Line<'static>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    fn test_app() -> super::App {
        let temp_root =
            std::env::temp_dir().join(format!("tidev-render-tests-{}", uuid::Uuid::new_v4()));
        let paths = crate::config::ConfigPaths {
            config_dir: temp_root.join(".config").join("tidev"),
            data_dir: temp_root.join(".local").join("share").join("tidev"),
            config_file: temp_root.join(".config").join("tidev").join("config.toml"),
            auth_file: temp_root
                .join(".local")
                .join("share")
                .join("tidev")
                .join("auth.json"),
            database_file: temp_root
                .join(".local")
                .join("share")
                .join("tidev")
                .join("sessions.sqlite3"),
        };

        super::App::new_with_paths(paths).unwrap()
    }

    #[test]
    fn reasoning_lines_render_markdown_code_blocks() {
        let lines = render_reasoning_markdown_lines(
            "```rust\nfn main() { println!(\"hi\"); }\n```\n",
            80,
            None,
            ThemePalette::dark(),
        );

        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
        assert_eq!(line_text(&lines[1]), "┃ fn main() { println!(\"hi\"); }");
        assert!(
            lines[1].spans.len() > 2,
            "expected highlighted spans in code line"
        );
        assert!(
            lines[1]
                .spans
                .iter()
                .skip(1)
                .any(|span| span.style != Style::default()),
            "expected syntax highlighting styles on code spans"
        );
    }

    #[test]
    fn reasoning_lines_preserve_empty_state() {
        let lines = render_reasoning_markdown_lines("", 80, None, ThemePalette::dark());

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
        assert_eq!(line_text(&lines[1]), "┃ ");
    }

    #[test]
    fn render_tool_result_detail_lines_list_shows_output_preview() {
        use crate::session::{Message, ToolExecutionResult};

        let message = Message::tool_result(
            "tool-call-id",
            "list",
            ToolExecutionResult::new("./\nfile1.txt\nfile2.txt"),
        );

        let ctx = RenderContext {
            palette: ThemePalette::dark(),
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
        };

        let lines = render_tool_result_detail_lines(&message, 80, &ctx);
        let text = text_lines_to_string(&lines);
        assert!(
            text.contains("file1.txt"),
            "should contain file listing: {}",
            text
        );
    }

    #[test]
    fn render_tool_result_detail_lines_todowrite_formats_checkbox_list() {
        use crate::session::{Message, ToolExecutionResult};
        use crate::tooling::TodoItem;

        let todos = vec![
            TodoItem {
                content: "Task 1".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
            },
            TodoItem {
                content: "Task 2".to_string(),
                status: "in_progress".to_string(),
                priority: "medium".to_string(),
            },
            TodoItem {
                content: "Task 3".to_string(),
                status: "pending".to_string(),
                priority: "low".to_string(),
            },
        ];
        let output = serde_json::to_string_pretty(&todos).unwrap();
        let message = Message::tool_result(
            "tool-call-id",
            "todowrite",
            ToolExecutionResult::new(output),
        );

        let ctx = RenderContext {
            palette: ThemePalette::dark(),
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
        };

        let lines = render_tool_result_detail_lines(&message, 80, &ctx);

        let text = text_lines_to_string(&lines);
        assert!(
            text.contains("Updated todo list"),
            "should contain header: {}",
            text
        );
        assert!(text.contains("Task 1"), "should contain Task 1: {}", text);
        assert!(text.contains("Task 2"), "should contain Task 2: {}", text);
        assert!(text.contains("Task 3"), "should contain Task 3: {}", text);
    }

    #[test]
    fn message_render_cache_hits_on_second_render_same_width() {
        let mut app = test_app();
        app.conversation
            .push(Message::new(MessageRole::User, "show file list"));
        app.conversation.push(Message::new(
            MessageRole::Assistant,
            "Summary with **markdown** and `inline code`.",
        ));

        let _ = app.messages_text(Some(80));
        let (_, misses_before, entries_before) = app.message_render_cache_stats();

        let _ = app.messages_text(Some(80));
        let (hits_after, misses_after, entries_after) = app.message_render_cache_stats();

        assert!(misses_before >= 2, "first render should have cache misses");
        assert!(entries_before >= 2, "first render should populate cache");
        assert!(hits_after > 0, "second render should have cache hits");
        assert_eq!(misses_after, misses_before, "second render should use cache");
        assert_eq!(entries_after, entries_before, "cache size should be stable");
    }

    #[test]
    fn message_render_cache_width_change_causes_miss() {
        let mut app = test_app();
        app.conversation
            .push(Message::new(MessageRole::User, "open README"));
        app.conversation.push(Message::new(
            MessageRole::Assistant,
            "A longer paragraph that should wrap differently at another width.",
        ));

        let _ = app.messages_text(Some(72));
        let (_, misses_before, entries_before) = app.message_render_cache_stats();

        let _ = app.messages_text(Some(100));
        let (_, misses_after, entries_after) = app.message_render_cache_stats();

        assert!(misses_after > misses_before);
        assert!(entries_after > entries_before);
    }

    #[test]
    fn message_render_cache_invalidation_refreshes_updated_content() {
        let mut app = test_app();
        app.conversation
            .push(Message::new(MessageRole::Assistant, "old cached content"));

        let (before, _, _, _, _) = app.messages_text(Some(80));
        let before_text = text_lines_to_string(&before.lines);
        assert!(before_text.contains("old cached content"));

        let message_id = app.conversation.messages[0].id;
        app.conversation.messages[0].content = "new refreshed content".to_string();
        app.invalidate_active_message_render_cache_for(message_id);

        let (after, _, _, _, _) = app.messages_text(Some(80));
        let after_text = text_lines_to_string(&after.lines);
        assert!(after_text.contains("new refreshed content"));
    }

    #[test]
    fn virtualized_render_clamps_scroll_and_keeps_content_visible() {
        let mut app = test_app();
        app.message_viewport_lines = 8;
        app.message_follow_tail = false;
        app.message_scroll_offset = usize::MAX;

        for idx in 0..24 {
            app.conversation.push(Message::new(
                MessageRole::Assistant,
                format!(
                    "message {idx}\n\n```rust\nfn item_{idx}() {{\n    println!(\"ok\");\n}}\n```"
                ),
            ));
        }

        let (text, total_lines, _, used_virtualization, _) = app.messages_text(Some(80));

        assert!(used_virtualization);
        assert!(total_lines > 0);
        assert!(!text.lines.is_empty());
        assert!(text_lines_to_string(&text.lines).contains("message"));

        let max_scroll = total_lines.saturating_sub(app.message_viewport_lines.max(1));
        assert!(app.message_scroll_offset <= max_scroll);
    }
}

fn tool_output_is_error(output: &str) -> bool {
    let first_line = output.lines().next().unwrap_or("").trim_start();

    first_line.starts_with("Tool failed:")
        || first_line.starts_with("Tool '")
        || first_line.starts_with("Request failed:")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
}

fn summarize_tool_call(tool_name: &str, arguments: &str, body_width: usize) -> String {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let fields = summarize_tool_arguments(tool_name, arguments);
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();

    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    let summary = match canonical_name {
        "read" => field("path")
            .map(|path| format!("Read {path}"))
            .unwrap_or_else(|| "Read file".to_string()),
        "write" => field("path")
            .map(|path| format!("Write {path}"))
            .unwrap_or_else(|| "Write file".to_string()),
        "edit" => field("path")
            .map(|path| format!("Edit {path}"))
            .unwrap_or_else(|| "Edit file".to_string()),
        "list" => field("path")
            .map(|path| format!("List {path}"))
            .unwrap_or_else(|| "List items".to_string()),
        "glob" => {
            let pattern = field("pattern").unwrap_or("*");
            let path = field("path").unwrap_or(".");
            format!("Find {pattern} in {path}")
        }
        "grep" => {
            let pattern = field("pattern").unwrap_or("");
            let path = field("path").unwrap_or(".");
            if pattern.is_empty() {
                format!("Search in {path}")
            } else {
                format!("Search \"{pattern}\" in {path}")
            }
        }
        "bash" => field("command")
            .map(|command| format!("Run shell command: {command}"))
            .unwrap_or_else(|| "Run shell command".to_string()),
        "task" => {
            let description = field("description").unwrap_or("task");
            let subagent_type = field("subagent_type").unwrap_or("general");
            format!("Spawn {subagent_type} subagent: {description}")
        }
        "question" => {
            let count = parsed
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(serde_json::Value::as_array)
                .map(|questions| questions.len())
                .unwrap_or(0);

            if count == 1 {
                field("question")
                    .map(|question| format!("Ask: {question}"))
                    .unwrap_or_else(|| "Ask 1 question".to_string())
            } else {
                format!("Ask {count} question{}", if count == 1 { "" } else { "s" })
            }
        }
        "todowrite" => "Update todo list".to_string(),
        _ => {
            let mut summary = display_tool_name(tool_name);
            summary = summary[0..1].to_uppercase() + &summary[1..];
            for (label, value) in fields.iter().take(2) {
                summary.push(' ');
                summary.push_str(label);
                summary.push(' ');
                summary.push_str(value);
            }
            summary
        }
    };

    shorten_single_line(&summary, body_width.saturating_sub(2))
}

fn summarize_tool_arguments(tool_name: &str, arguments: &str) -> Vec<(String, String)> {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();
    let mut fields = Vec::new();

    let string_field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_str)
            .map(|value| shorten_single_line(value, 96))
    };

    match canonical_name {
        "read" => {
            if let Some(path) = string_field("path") {
                fields.push(("path".to_string(), path));
            }
            // Extract offset and limit for read tool
            if let Some(offset) = parsed
                .as_ref()
                .and_then(|v| v.get("offset"))
                .and_then(|v| v.as_i64())
            {
                fields.push(("offset".to_string(), format!("{}", offset)));
            }
            if let Some(limit) = parsed
                .as_ref()
                .and_then(|v| v.get("limit"))
                .and_then(|v| v.as_i64())
            {
                fields.push(("limit".to_string(), format!("{}", limit)));
            }
        }
        "write" | "edit" => {
            if let Some(path) = string_field("path") {
                fields.push(("path".to_string(), path));
            }
        }
        "list" => {
            fields.push((
                "path".to_string(),
                string_field("path").unwrap_or_else(|| ".".to_string()),
            ));
        }
        "glob" => {
            if let Some(pattern) = string_field("pattern") {
                fields.push(("pattern".to_string(), pattern));
            }
            fields.push((
                "path".to_string(),
                string_field("path").unwrap_or_else(|| ".".to_string()),
            ));
        }
        "grep" => {
            if let Some(pattern) = string_field("pattern") {
                fields.push(("pattern".to_string(), pattern));
            }
            fields.push((
                "path".to_string(),
                string_field("path").unwrap_or_else(|| ".".to_string()),
            ));
            if let Some(include) = string_field("include") {
                fields.push(("include".to_string(), include));
            }
        }
        "bash" => {
            if let Some(command) = string_field("command") {
                fields.push(("command".to_string(), command));
            }
        }
        "task" => {
            if let Some(description) = string_field("description") {
                fields.push(("description".to_string(), description));
            }
            if let Some(subagent_type) = string_field("subagent_type") {
                fields.push(("subagent_type".to_string(), subagent_type));
            }
        }
        "question" => {
            let question_count = parsed
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(serde_json::Value::as_array)
                .map(|questions| questions.len())
                .unwrap_or(0);

            fields.push((
                "questions".to_string(),
                format!("{question_count} question(s)"),
            ));

            if let Some(first_question) = parsed
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(serde_json::Value::as_array)
                .and_then(|questions| questions.first())
                .and_then(|question| question.get("question"))
                .and_then(serde_json::Value::as_str)
            {
                fields.push((
                    "question".to_string(),
                    shorten_single_line(first_question, 96),
                ));
            }
        }
        "todowrite" => {
            let todo_count = parsed
                .as_ref()
                .and_then(|value| value.get("todos"))
                .and_then(serde_json::Value::as_array)
                .map(|todos| format!("{} item(s)", todos.len()));

            if let Some(todo_count) = todo_count {
                fields.push(("todos".to_string(), todo_count));
            }
        }
        _ => {}
    }

    if fields.is_empty() {
        fields.push((
            "arguments".to_string(),
            shorten_single_line(&pretty_tool_arguments(arguments), 120),
        ));
    }

    fields
}

/// Parse line range from read tool output (e.g., "Showing lines 10-50 of 100")
fn parse_line_range_from_read_output(output: &str) -> Option<(i64, i64)> {
    // Match patterns like "Showing lines 10-50 of 100" or "Showing lines 10-50"
    if let Some(start) = output.find("Showing lines ") {
        let after_prefix = &output[start + 14..];
        // Parse start number
        let mut end_idx = 0;
        let mut start_num = 0i64;
        for (i, c) in after_prefix.chars().enumerate() {
            if c.is_ascii_digit() {
                start_num = start_num * 10 + (c as i64 - '0' as i64);
                end_idx = i;
            } else {
                break;
            }
        }
        // Look for "-{end}" after "Showing lines {start}-"
        let after_start = &after_prefix[end_idx + 1..];
        if let Some(stripped) = after_start.strip_prefix('-') {
            let mut end_num = 0i64;
            for c in stripped.chars() {
                if c.is_ascii_digit() {
                    end_num = end_num * 10 + (c as i64 - '0' as i64);
                } else {
                    break;
                }
            }
            if end_num > start_num {
                return Some((start_num, end_num));
            }
        }
    }
    None
}

fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}

fn display_tool_name(tool_name: &str) -> String {
    canonical_tool_name(tool_name)
        .unwrap_or(tool_name)
        .to_string()
}
