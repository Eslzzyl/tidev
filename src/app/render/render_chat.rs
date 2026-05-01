use crate::{
    markdown_render::{WrapOptions, render_markdown_text_with_width_and_cwd, word_wrap_line},
    session::{COMPACTION_MESSAGE_LABEL, Message, MessageRole, ToolCall},
    theme::ThemePalette,
    tooling::{TodoItem, canonical_tool_name},
    tooling::builtin::utils::display_workspace_relative,
    utils::{TokenUsage, format_token_count},
};
use chrono::Local;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use super::diff_render::render_unified_diff_text;
use super::permission::{RunningSubagentExecution, SubagentStatus};
use super::{
    App, MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind,
    MessageRenderCacheValue, render::*,
};

const TOOL_OUTPUT_PREVIEW_LINES: usize = 5;
const TOOL_OUTPUT_EXPANDED_MAX_LINES: usize = 100;

use crate::app::runtime::state::SelectableRegionRange;

#[derive(Clone, Debug)]
struct ToolResultCardRange {
    message_id: Uuid,
    start_line: usize,
    end_line: usize,
}

#[derive(Clone, Debug)]
struct RunningCardRange {
    execution_index: usize,
    start_line: usize,
    end_line: usize,
}

struct RenderContext<'a> {
    palette: ThemePalette,
    spinner: &'a str,
    #[allow(dead_code)]
    workspace_root: &'a Path,
    expanded_tool_results: &'a HashSet<Uuid>,
    expanded_tool_outputs: &'a HashMap<Uuid, String>,
}

fn render_tool_call_with_result(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    body_width: usize,
    is_streaming: bool,
    ctx: &RenderContext<'_>,
) -> (Vec<Line<'static>>, Vec<SelectableRegionRange>) {
    let palette = ctx.palette;
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);

    // Check if this is a pending call (arguments not complete)
    let is_pending = tool_result.is_none()
        && is_streaming
        && !matches!(canonical_name, "read" | "list" | "glob" | "grep")
        && !tool_call_arguments_are_complete(&tool_call.arguments);

    if matches!(canonical_name, "list" | "grep" | "glob" | "read" | "skill") {
        return (
            render_tool_call_summary_line(tool_call, tool_result, body_width, palette, ctx),
            vec![],
        );
    }

    // Get result lines and exit code (for bash)
    let (result_lines, exit_code, mut regions) = if let Some(result_msg) = tool_result {
        render_tool_result_detail_lines(result_msg, body_width, ctx)
    } else {
        (Vec::new(), None, vec![])
    };

    // Get rtk_rewritten from tool result message
    let rtk_rewritten = tool_result.map(|m| m.rtk_rewritten).unwrap_or(false);

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    let call_lines =
        render_tool_call_lines(tool_call, body_width, palette, exit_code, rtk_rewritten, ctx.workspace_root);
    lines.extend(call_lines);

    if is_pending {
        // Show calling status inline
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", ctx.spinner),
                Style::default().fg(palette.accent_soft),
            ),
            Span::styled("Calling...", Style::default().fg(palette.muted)),
        ]));
    } else if !result_lines.is_empty() {
        lines.push(Line::from(""));
        let offset = lines.len();
        for r in &mut regions {
            r.start_line += offset;
            r.end_line += offset;
        }
        lines.extend(result_lines);
    }

    lines.push(Line::from(""));
    (lines, regions)
}

fn render_compaction_divider_line(
    label: &str,
    width: usize,
    palette: ThemePalette,
) -> Line<'static> {
    let label_width = UnicodeWidthStr::width(label);
    if width <= label_width.saturating_add(2) {
        return line_with_style(label, palette.accent_soft);
    }

    let remaining = width - label_width - 2;
    let left = remaining / 2;
    let right = remaining - left;

    let mut spans = Vec::new();
    if left > 0 {
        spans.push(Span::styled(
            "─".repeat(left),
            Style::default().fg(palette.muted),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        label.to_string(),
        Style::default().fg(palette.accent_soft),
    ));
    spans.push(Span::raw(" "));
    if right > 0 {
        spans.push(Span::styled(
            "─".repeat(right),
            Style::default().fg(palette.muted),
        ));
    }

    Line::from(spans)
}

fn tool_call_arguments_are_complete(arguments: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(arguments).is_ok()
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

    let rel_path = |p: &str| display_workspace_relative(ctx.workspace_root, Path::new(p));

    let (action_label, target) = match canonical_name {
        "list" => {
            let path = get_field("path").unwrap_or(".");
            ("List", rel_path(path).to_string())
        }
        "grep" => {
            let pattern = get_field("pattern").unwrap_or("");
            let path = get_field("path").unwrap_or(".");
            ("Search", format!("\"{}\" in {}", pattern, rel_path(path)))
        }
        "glob" => {
            let pattern = get_field("pattern").unwrap_or("*");
            let path = get_field("path").unwrap_or(".");
            ("Find", format!("{} in {}", pattern, rel_path(path)))
        }
        "read" => {
            let path = get_field("path").unwrap_or("file");
            ("Read", rel_path(path).to_string())
        }
        "skill" => {
            let name = get_field("name").unwrap_or("");
            ("Loaded skill", name.to_string())
        }
        _ => {
            let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width, ctx.workspace_root);
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

    // Wrap the line if it exceeds body_width, with subsequent lines indented to align with target
    let indent_width = UnicodeWidthStr::width(action_label) + 1; // +1 for the space after label
    let indent = Line::from(" ".repeat(indent_width));
    let wrapped = word_wrap_line(
        &line,
        WrapOptions::new(body_width)
            .subsequent_indent(indent)
            .break_words(true),
    );
    // Convert to owned lines to satisfy lifetime requirements
    wrapped
        .into_iter()
        .map(|l| {
            Line::from(
                l.spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn compute_tool_result_suffix(canonical_name: &str, output: &str) -> String {
    match canonical_name {
        "list" => {
            if output == "(empty)" {
                " → empty".to_string()
            } else {
                let count = output
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
                    .saturating_sub(1); // Subtract the path line
                format!(" → {} items", count)
            }
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
                if output.contains("file not found") && output.contains("Did you mean") {
                    " → not found (with suggestions)".to_string()
                } else {
                    " → error".to_string()
                }
            } else if output.contains("Image read successfully") {
                " → image".to_string()
            } else if output.contains("<type>directory</type>") {
                let count = output
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
                    .saturating_sub(2); // Approximate, list_dir output is different now
                format!(" → directory ({} entries)", count)
            } else {
                let metadata = parse_read_content_metadata(output);
                let is_size_truncated = output.contains("Output capped at 50 KB");

                match metadata {
                    Some(((start, end), requested_range, total, truncated_by)) => {
                        // Check if this is a full file read (start==1 && end==total)
                        let is_full_file = start == 1 && end == total;
                        let has_requested_range = requested_range.is_some();

                        if is_size_truncated {
                            // 50KB truncation
                            if has_requested_range {
                                let (req_start, req_end) = requested_range.unwrap();
                                if is_full_file {
                                    format!(
                                        " → All {} lines (requested {}-{}, truncated due to 50KB cap)",
                                        total, req_start, req_end
                                    )
                                } else {
                                    format!(
                                        " → Line {}-{} of {} (requested {}-{}, truncated due to 50KB cap)",
                                        start, end, total, req_start, req_end
                                    )
                                }
                            } else if is_full_file {
                                format!(
                                    " → All {} lines (requested all lines, truncated due to 50KB cap)",
                                    total
                                )
                            } else {
                                format!(
                                    " → Line {}-{} of {} (requested all lines, truncated due to 50KB cap)",
                                    start, end, total
                                )
                            }
                        } else if truncated_by.as_deref() == Some("lines") {
                            // 2000-line cap (more flag, but no 50KB cutoff)
                            if has_requested_range {
                                let (req_start, req_end) = requested_range.unwrap();
                                if is_full_file {
                                    format!(
                                        " → All {} lines (requested {}-{}, truncated due to 2000 lines cap)",
                                        total, req_start, req_end
                                    )
                                } else {
                                    format!(
                                        " → Line {}-{} of {} (requested {}-{}, truncated due to 2000 lines cap)",
                                        start, end, total, req_start, req_end
                                    )
                                }
                            } else if is_full_file {
                                format!(
                                    " → All {} lines (requested all lines, truncated due to 2000 lines cap)",
                                    total
                                )
                            } else {
                                format!(
                                    " → Line {}-{} of {} (requested all lines, truncated due to 2000 lines cap)",
                                    start, end, total
                                )
                            }
                        } else if is_full_file {
                            // Complete file read without truncation
                            format!(" → All {} lines", total)
                        } else {
                            // Partial read without truncation
                            format!(" → Line {}-{} of {}", start, end, total)
                        }
                    }
                    None => {
                        // Fallback: try old format parsing
                        let line_range = parse_line_range_from_read_output(output);
                        let truncated = tool_output_is_truncated(output);

                        match line_range {
                            Some((start, end)) => {
                                if truncated && output.contains("Output capped at 50 KB") {
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

/// Parse bash output to extract exit code and remaining output.
/// Format: "[exit N]\n<output>"
fn parse_bash_exit_code(output: &str) -> (Option<i32>, &str) {
    // Look for "[exit N]" prefix
    if let Some(stripped) = output.strip_prefix("[exit ") {
        // Find the closing bracket
        if let Some(end_idx) = stripped.find(']') {
            let code_str = &stripped[..end_idx];
            if let Ok(code) = code_str.parse::<i32>() {
                // Skip the "] " or "]" and newline
                let remaining = &stripped[end_idx + 1..];
                let remaining = remaining.strip_prefix('\n').unwrap_or(remaining);
                return (Some(code), remaining);
            }
        }
    }
    (None, output)
}

fn render_tool_call_lines(
    tool_call: &ToolCall,
    body_width: usize,
    palette: ThemePalette,
    exit_code: Option<i32>,
    rtk_rewritten: bool,
    workspace_root: &Path,
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

    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or("");

    // Build title line with optional exit code for bash
    let title_spans = if canonical_name == "bash" {
        // Build the base spans for bash tool
        let mut spans = vec![
            Span::styled("Tool: ", Style::default().fg(palette.muted)),
            Span::styled(
                canonical_display.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ];

        // Add [rtk] marker after tool name if command was rewritten
        if rtk_rewritten {
            spans.push(Span::styled(
                " [rtk]",
                Style::default().fg(palette.accent_soft),
            ));
        }

        // Add exit code status
        if let Some(code) = exit_code {
            if code == 0 {
                spans.push(Span::styled(" ✓", Style::default().fg(palette.success)));
            } else {
                spans.push(Span::styled(
                    format!(" ✗ {}", code),
                    Style::default().fg(palette.error),
                ));
            }
        }
        spans
    } else {
        vec![
            Span::styled("Tool: ", Style::default().fg(palette.muted)),
            Span::styled(
                canonical_display.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    };
    lines.push(Line::from(title_spans));

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
                let owned_line = Line::from(line.to_string());
                let wrapped = word_wrap_line(
                    &owned_line,
                    WrapOptions::new(body_width.saturating_sub(4)).break_words(true),
                );
                for (i, wrapped_line) in wrapped.iter().enumerate() {
                    let mut spans = Vec::new();
                    if i == 0 {
                        spans.push(Span::styled("  $ ", Style::default().fg(palette.accent)));
                    } else {
                        spans.push(Span::styled("    ", Style::default()));
                    }
                    spans.extend(wrapped_line.spans.iter().map(|s| {
                        Span::styled(s.content.to_string(), Style::default().fg(palette.text))
                    }));
                    lines.push(Line::from(spans));
                }
            }
        }
        "write" => {
            let path = get_field("path").unwrap_or("file");
            let rel_path = display_workspace_relative(workspace_root, Path::new(path));
            lines.push(Line::from(vec![
                Span::styled("  Path: ", Style::default().fg(palette.muted)),
                Span::styled(rel_path, Style::default().fg(palette.text)),
            ]));
        }
        _ => {
            let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width, workspace_root);
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
) -> (Vec<Line<'static>>, Option<i32>, Vec<SelectableRegionRange>) {
    let palette = ctx.palette;
    let output = tool_output_from_message(message, ctx);
    let tool_name = message.tool_name.as_deref().unwrap_or(message.role.label());
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);

    // For bash, parse exit code and strip it from output
    let (exit_code, effective_output) = if canonical_name == "bash" {
        parse_bash_exit_code(output)
    } else {
        (None, output)
    };

    let is_error = tool_output_is_error(effective_output);

    // Try to render diff from metadata first (preferred, full diff not truncated)
    if !is_error
        && matches!(canonical_name, "edit" | "write" | "apply_patch")
        && let Some(diff) = message.metadata.diff.as_ref()
        && let Some((diff_lines, regions)) = render_unified_diff_text(diff, body_width, palette)
    {
        return (diff_lines, None, regions);
    }

    // Fallback: try to render diff from output (may be truncated)
    if !is_error
        && matches!(canonical_name, "edit" | "write" | "apply_patch")
        && let Some((diff_lines, regions)) =
            render_unified_diff_text(effective_output, body_width, palette)
    {
        return (diff_lines, None, regions);
    }

    if canonical_name == "todowrite" && !is_error {
        #[derive(serde::Deserialize)]
        struct RawTodo {
            content: String,
            status: Option<String>,
            priority: Option<String>,
        }

        let raw_todos = if let Ok(todos) = serde_json::from_str::<Vec<RawTodo>>(effective_output) {
            Some(todos)
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(effective_output) {
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
            return (
                render_todos_checkbox_list(&todos, body_width, palette),
                None,
                vec![],
            );
        }
    }

    // Subagent task results: render markdown preview (collapsed) or full raw (expanded)
    if canonical_name == "task" {
        let is_expanded = ctx.expanded_tool_results.contains(&message.id);
        if !is_expanded {
            return (
                render_subagent_task_preview(effective_output, body_width, palette),
                None,
                vec![],
            );
        }
        // Expanded: fall through to full raw output below
    }

    (
        render_output_preview_lines(
            effective_output,
            body_width,
            is_error,
            Some(message.id),
            ctx.expanded_tool_results,
            palette,
        ),
        exit_code,
        vec![],
    )
}

/// Renders a compact markdown preview of a subagent task result.
fn render_subagent_task_preview(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(line_with_style("(empty result)", palette.muted));
        return lines;
    }

    // Title header
    lines.push(Line::from(vec![
        Span::styled("Task ", Style::default().fg(palette.accent_soft)),
        Span::styled("· subagent result", Style::default().fg(palette.muted)),
    ]));
    lines.push(Line::from(""));

    // Render the output as markdown, then take the first few lines
    let rendered =
        render_markdown_text_with_width_and_cwd(output, Some(body_width.saturating_sub(2)), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;

    let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
    let line_count = md_lines.len();

    if line_count <= max_preview {
        lines.extend(md_lines);
    } else {
        lines.extend(md_lines.into_iter().take(max_preview));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  ▼ {} more line(s) — Click to expand",
                line_count - max_preview
            ),
            Style::default().fg(palette.muted),
        )]));
    }

    // Always add Ctrl+Click hint
    lines.push(Line::from(vec![Span::styled(
        "  Ctrl+Click to enter subsession",
        Style::default().fg(palette.muted),
    )]));

    lines
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

    let fg = if is_error {
        palette.error
    } else {
        palette.text
    };

    let total_output_lines = output.lines().count();
    let wrap_width = body_width.saturating_sub(2);

    for line in output.lines().take(max_lines) {
        let owned_line = Line::from(line.to_string());
        if is_expanded {
            let wrapped =
                word_wrap_line(&owned_line, WrapOptions::new(wrap_width).break_words(true));
            for wrapped_line in wrapped.iter() {
                let mut spans = Vec::new();
                spans.extend(
                    wrapped_line.spans.iter().map(|span| {
                        Span::styled(span.content.to_string(), Style::default().fg(fg))
                    }),
                );
                lines.push(Line::from(spans));
            }
        } else {
            let wrapped =
                word_wrap_line(&owned_line, WrapOptions::new(wrap_width).break_words(true));
            if let Some(first_wrapped) = wrapped.first() {
                let mut content = first_wrapped
                    .spans
                    .iter()
                    .map(|s| &*s.content)
                    .collect::<String>();
                if wrapped.len() > 1 || total_output_lines > max_lines {
                    // Try to fit "..."
                    if content.width() >= wrap_width {
                        content = shorten_single_line(&content, wrap_width.saturating_sub(3));
                        content.push_str("...");
                    } else {
                        content.push_str("...");
                    }
                }
                lines.push(Line::from(vec![Span::styled(
                    content,
                    Style::default().fg(fg),
                )]));
            }
        }
    }

    if total_output_lines > max_lines {
        lines.push(Line::from(vec![Span::styled(
            format!("... {} more line(s)", total_output_lines - max_lines),
            Style::default().fg(palette.muted),
        )]));
    } else if total_output_lines > TOOL_OUTPUT_PREVIEW_LINES && message_id.is_some() {
        let hint = if is_expanded {
            "▲ Click to collapse"
        } else {
            "▼ Click to expand"
        };
        lines.push(Line::from(vec![Span::styled(
            hint,
            Style::default().fg(palette.muted),
        )]));
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

        let sidebar_visible = area.width >= self.config.ui.sidebar_width.saturating_add(70);
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

        // In subsession, show navigation panel instead of input box
        if self.conversation.parent_session_id.is_some() {
            self.render_subsession_navigation(frame, layout[1]);
        } else {
            let prompt_title = match self.pending_mode.as_ref() {
                Some(pending) if self.pending_request => {
                    format!(
                        "{} (current), {} (on completion)",
                        self.mode.title(),
                        pending.title()
                    )
                }
                _ => self.mode.title().to_string(),
            };
            self.render_input_block(
                frame,
                layout[1],
                &prompt_title,
                self.composer.placeholder(),
                false,
            );
            self.render_at_mention_palette(frame, layout[1]);
            self.render_snippet_palette(frame, layout[1]);
            self.render_command_palette(frame, layout[1]);
            self.render_snippet_palette(frame, layout[1]);
        }
        self.render_prompt_footer(frame, layout[2]);
        self.render_retrying_hint(frame, layout[3]);
    }

    fn render_subsession_navigation(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(" Subsession ");

        frame.render_widget(block, area);

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Center the navigation hints
        let hint = Line::from(vec![
            Span::styled("Up", Style::default().fg(palette.accent_soft)),
            Span::styled(": return to parent  ", Style::default().fg(palette.muted)),
            Span::styled("Left", Style::default().fg(palette.accent_soft)),
            Span::styled("/", Style::default().fg(palette.muted)),
            Span::styled("Right", Style::default().fg(palette.accent_soft)),
            Span::styled(": switch subagent", Style::default().fg(palette.muted)),
        ]);

        let paragraph = Paragraph::new(hint)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.text));

        frame.render_widget(paragraph, inner);
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

        let scrollbar_area = if inner.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1), // gap between content and scrollbar
                Constraint::Length(1),
            ])
            .split(inner);
            (chunks[0], Some(chunks[2]))
        } else if inner.width > 1 {
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
        let (
            text,
            total_lines,
            card_ranges,
            selectable_regions_ranges,
            rendered_virtualized,
            virtualized_render_scroll,
            running_card_ranges,
        ) = self.messages_text(Some(content_width));

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

        self.selectable_regions.clear();
        for r in selectable_regions_ranges {
            let screen_start = r.start_line.saturating_sub(render_scroll);
            let screen_end = r.end_line.saturating_sub(render_scroll);
            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }
            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;
            if visible_start < visible_end {
                let y = content_area.y.saturating_add(visible_start);
                let height = visible_end.saturating_sub(visible_start);
                let min_x = content_area.x.saturating_add(r.min_x);
                let max_x = r
                    .max_x
                    .map(|mx| content_area.x.saturating_add(mx))
                    .unwrap_or(content_area.x.saturating_add(content_area.width));
                let width = max_x.saturating_sub(min_x);
                if width > 0 {
                    self.selectable_regions.push(Rect {
                        x: min_x,
                        y,
                        width,
                        height,
                    });
                }
            }
        }

        // Calculate screen positions for tool result cards
        self.tool_result_card_bounds.clear();
        for card_range in card_ranges {
            let screen_start = card_range.start_line.saturating_sub(render_scroll);
            let screen_end = card_range.end_line.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }

            let visible_start = screen_start as u16;
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

        // Calculate screen positions for running subagent cards
        // running_card_ranges contain positions within the running_lines block,
        // which starts at (header_line_count + total_message_lines) in the full text.
        let header_line_count = if self.conversation.parent_session_id.is_some() { 3 } else { 0 };
        let total_msg_lines = self.message_layout_index.borrow().total_lines;
        let running_block_start = header_line_count + total_msg_lines;

        self.running_subagent_card_bounds.clear();
        for card_range in &running_card_ranges {
            let abs_start = running_block_start + card_range.start_line;
            let abs_end = running_block_start + card_range.end_line;

            let screen_start = abs_start.saturating_sub(render_scroll);
            let screen_end = abs_end.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }

            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;

            if visible_start < visible_end {
                let card_rect = Rect {
                    x: content_area.x,
                    y: content_area.y.saturating_add(visible_start),
                    width: content_area.width,
                    height: visible_end.saturating_sub(visible_start),
                };
                self.running_subagent_card_bounds
                    .push((card_range.execution_index, card_rect));
            }
        }

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(palette.background).fg(palette.text))
            .scroll((render_scroll as u16, 0));

        frame.render_widget(paragraph, content_area);

        if let Some(scrollbar_area) = scrollbar_area.1 {
            self.message_scrollbar_area = Some(scrollbar_area);
            self.render_scrollbar(frame, scrollbar_area, scroll, max_scroll);
        }
    }

    fn render_sidebar(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut lines = Vec::new();

        // Workspace directory (top)
        let workspace_path = self.workspace_root.display().to_string();
        let display_path = workspace_path.replace(
            &dirs::home_dir().unwrap_or_default().display().to_string(),
            "~",
        );
        lines.push(Line::from(vec![Span::styled(
            "Workspace",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            display_path,
            Style::default().fg(palette.muted),
        )]));

        lines.push(Line::from(""));

        // Model info
        lines.push(Line::from(vec![Span::styled(
            "Model",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            self.active_model.label(),
            Style::default().fg(palette.text),
        )]));

        if let Some(usage) = &self.context_usage {
            let session_tps: Vec<f32> = self
                .conversation
                .messages
                .iter()
                .filter(|m| matches!(m.role, MessageRole::Assistant))
                .filter_map(|m| m.tokens_per_second)
                .collect();

            if !session_tps.is_empty() {
                let avg_tps = session_tps.iter().sum::<f32>() / session_tps.len() as f32;
                lines.push(Line::from(vec![Span::styled(
                    format!("Speed: {:.1} t/s (avg)", avg_tps),
                    Style::default().fg(palette.muted),
                )]));
            } else if let Some(current_tps) = usage.tokens_per_second {
                lines.push(Line::from(vec![Span::styled(
                    format!("Speed: {:.1} t/s", current_tps),
                    Style::default().fg(palette.muted),
                )]));
            }
        }

        // Token statistics (session cumulative)
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Tokens",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        let mut token_usage = TokenUsage::default();
        for m in self
            .conversation
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Assistant))
        {
            token_usage.add(m.token_usage());
        }

        let total = token_usage.total();
        let total_cache = token_usage.total_cache();

        lines.push(Line::from(vec![Span::styled(
            format!("Total: {}", format_token_count(total)),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "In: {}",
                format_token_count(token_usage.input_tokens as u64)
            ),
            Style::default().fg(palette.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("Cache: {}", format_token_count(total_cache)),
            Style::default().fg(palette.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Out: {}",
                format_token_count(token_usage.output_tokens as u64)
            ),
            Style::default().fg(palette.muted),
        )]));

        lines.push(Line::from(""));

        // Request count
        let request_count = self
            .conversation
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Assistant))
            .count();
        lines.push(Line::from(vec![Span::styled(
            format!("Requests: {request_count}"),
            Style::default().fg(palette.text),
        )]));

        // Changed Files section
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Changed Files",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        {
            let mut all_diffs = Vec::new();
            let mut seen_files = std::collections::HashSet::new();
            for msg in self.conversation.visible_messages() {
                if let Some(diffs_json) = &msg.file_diffs {
                    if let Ok(diffs) =
                        serde_json::from_str::<Vec<crate::snapshot::FileDiff>>(diffs_json)
                    {
                        for d in &diffs {
                            if seen_files.insert(d.file.clone()) {
                                all_diffs.push(d.clone());
                            }
                        }
                    }
                }
            }

            if all_diffs.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "(no changes yet)",
                    Style::default().fg(palette.muted),
                )]));
            } else {
                // Sort: modified first, then added, then deleted
                all_diffs.sort_by_key(|d| match d.status.as_deref() {
                    Some("modified") => 0,
                    Some("added") => 1,
                    Some("deleted") => 2,
                    _ => 3,
                });

                let count = all_diffs.len().min(10); // limit display to 10 files
                for d in &all_diffs[..count] {
                    let (status_icon, style) = match d.status.as_deref() {
                        Some("added") => ("+ ", Style::default().fg(palette.success)),
                        Some("deleted") => ("- ", Style::default().fg(palette.error)),
                        _ => ("~ ", Style::default().fg(palette.warning)),
                    };

                    let filename = Path::new(&d.file)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| d.file.clone());

                    let summary = format!(
                        "{}{} (+{}/-{})",
                        status_icon,
                        filename,
                        d.additions,
                        d.deletions
                    );
                    lines.push(Line::from(vec![Span::styled(summary, style)]));
                }
                if all_diffs.len() > 10 {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  ... {} more", all_diffs.len() - 10),
                        Style::default().fg(palette.muted),
                    )]));
                }
            }
        }

        // Todos section
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("Todos ({})", self.todos.len()),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

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

            let content = &todo.content;
            lines.push(Line::from(vec![
                Span::styled(format!("{priority_marker}{checkbox}"), style),
                Span::styled(content.as_str(), style),
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

        // Estimate total lines for scroll max (accounts for word wrapping)
        let sidebar_content_width = (area.width.saturating_sub(2)) as usize;
        self.sidebar_total_lines = lines
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

        let sidebar_viewport_lines = area.height.saturating_sub(2) as usize;
        let max_scroll = self.sidebar_total_lines.saturating_sub(sidebar_viewport_lines);
        self.sidebar_scroll_offset = self.sidebar_scroll_offset.min(max_scroll);

        let paragraph = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title("Sidebar"),
            )
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((self.sidebar_scroll_offset as u16, 0));

        frame.render_widget(paragraph, area);
    }

    fn messages_text(
        &mut self,
        content_width: Option<usize>,
    ) -> (
        Text<'static>,
        usize,
        Vec<ToolResultCardRange>,
        Vec<SelectableRegionRange>,
        bool,
        usize,
        Vec<RunningCardRange>,
    ) {
        let started_at = Instant::now();
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);
        let messages = self.conversation.visible_messages();

        let mut lines = Vec::new();
        let mut card_ranges = Vec::new();
        let mut selectable_regions_ranges = Vec::new();
        let mut running_card_ranges = Vec::new();

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
            return (
                Text::from(lines),
                total_lines,
                card_ranges,
                selectable_regions_ranges,
                false,
                0,
                running_card_ranges,
            );
        }

        // Update layout index
        self.update_message_layout_index(width, body_width, false);
        if let Some(scroll_offset) = self.resolve_message_scroll_target(messages, width, body_width)
        {
            self.message_scroll_offset = scroll_offset;
            self.message_follow_tail = false;
            self.message_scroll_target = None;
        }

        let mut running_lines = Vec::new();
        if self.conversation.parent_session_id.is_none() {
            for (index, running_subagent) in self.running_subagent_executions.iter().enumerate() {
                let card_lines = self.render_running_subagent_lines(running_subagent, width);
                if card_lines.is_empty() {
                    continue;
                }

                let card_start = running_lines.len();
                let decorated_lines =
                    super::render::decorate_card_lines(card_lines, width, palette.panel);
                running_lines.extend(decorated_lines);
                let card_end = running_lines.len();

                running_card_ranges.push(RunningCardRange {
                    execution_index: index,
                    start_line: card_start,
                    end_line: card_end,
                });
            }
        }
        let total_running_lines = running_lines.len();

        // Calculate visible range based on scroll position
        let viewport = self.message_viewport_lines.max(1);
        let total_message_lines = self.message_layout_index.borrow().total_lines;
        let total_overall_lines = total_message_lines + total_running_lines;
        let header_line_count = header_lines.len();

        let max_scroll = (header_line_count + total_overall_lines).saturating_sub(viewport);
        let scroll = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };
        self.message_scroll_offset = scroll;

        // the 'scroll' includes header lines. To find the correct message block, we must
        // offset the scroll past the header
        let message_scroll = scroll.saturating_sub(header_line_count);

        // Find visible blocks using the message-relative scroll
        let visible_blocks = self.find_visible_message_blocks(message_scroll, viewport);

        lines.extend(header_lines);

        // Calculate render_scroll for virtualized rendering
        // The visible blocks may start before 'message_scroll' (due to buffer zone),
        // so we need to skip those lines when rendering.
        // Also, if first block starts after 'message_scroll', we need padding.
        let first_block_start = visible_blocks.first().map(|b| b.start_line).unwrap_or(0);

        let (mut render_scroll, padding_lines) = if first_block_start < message_scroll {
            (message_scroll - first_block_start, 0)
        } else if first_block_start > message_scroll {
            (0, first_block_start - message_scroll)
        } else {
            (0, 0)
        };

        // Important: if we are scrolled inside the header, the render_scroll applies entirely to the header.
        // Otherwise, it skips the entire header PLUS block-relative scroll.
        if scroll < header_line_count {
            render_scroll = scroll;
        } else {
            render_scroll += header_line_count;
        }

        // Add padding lines if first block starts after scroll position
        for _ in 0..padding_lines {
            lines.push(Line::from(""));
        }

        // Create render context for tool calls
        let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
        let spinner = self.loading_spinner();
        let ctx = RenderContext {
            palette,
            spinner,
            workspace_root: self.workspace_root.as_path(),
            expanded_tool_results: &self.expanded_tool_results,
            expanded_tool_outputs: &expanded_tool_outputs,
        };

        // Render visible blocks
        let mut current_line_offset = header_line_count + padding_lines;
        for block in &visible_blocks {
            // Round end = no next message (session end) OR next message is User (new round)
            let next_idx = block.message_start_idx + block.message_count;
            let is_round_end =
                next_idx >= messages.len() || matches!(messages[next_idx].role, MessageRole::User);
            let block_lines = self.render_message_block_to_lines(
                messages,
                block,
                width,
                body_width,
                &mut card_ranges,
                &mut selectable_regions_ranges,
                current_line_offset,
                &ctx,
                is_round_end,
            );
            current_line_offset += block_lines.len();
            lines.extend(block_lines);
        }

        let last_block_end = visible_blocks
            .last()
            .map(|b| b.start_line + b.line_count)
            .unwrap_or(0);
        let missing_lines = total_message_lines.saturating_sub(last_block_end);
        for _ in 0..missing_lines {
            lines.push(Line::from(""));
        }

        lines.extend(running_lines);

        // Calculate total lines from layout index
        let total_lines = header_line_count + total_overall_lines;

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

        (
            Text::from(lines),
            total_lines,
            card_ranges,
            selectable_regions_ranges,
            true,
            render_scroll,
            running_card_ranges,
        )
    }

    fn cached_render_tool_call_with_result(
        &self,
        message: &Message,
        tool_call: &ToolCall,
        tool_result: Option<&Message>,
        body_width: usize,
        is_streaming: bool,
        ctx: &RenderContext<'_>,
    ) -> (Vec<Line<'static>>, Vec<SelectableRegionRange>) {
        if body_width == 0 {
            return (Vec::new(), Vec::new());
        }

        let tick = self.next_message_render_cache_tick();
        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id, // Binds the cache to the Assistant message hosting this tool call
            width: body_width,
            is_round_end: !is_streaming, // Approximation, cache differs when streaming is done
            kind: MessageRenderCacheKind::ToolCall(tool_call.id.clone()),
        };

        {
            let mut cache = self.message_render_cache.borrow_mut();
            if let Some(entry) = cache.get_mut(&key) {
                entry.last_used_tick = tick;
                self.record_message_render_cache_hit();
                match &entry.value {
                    MessageRenderCacheValue::ToolResult(lines, regions) => {
                        return (lines.clone(), regions.clone());
                    }
                    MessageRenderCacheValue::Cards(..) => {}
                }
            }
        }

        self.record_message_render_cache_miss();
        let result =
            render_tool_call_with_result(tool_call, tool_result, body_width, is_streaming, ctx);

        {
            let mut cache = self.message_render_cache.borrow_mut();
            cache.insert(
                key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::ToolResult(result.0.clone(), result.1.clone()),
                    last_used_tick: tick,
                },
            );
        }

        result
    }

    fn cached_render_message_cards(
        &self,
        message: &Message,
        body_width: usize,
        is_round_end: bool,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id,
            width: body_width,
            is_round_end,
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
                    MessageRenderCacheValue::ToolResult(..) => {} // Should never happen with .Cards kind
                }
            }
        }

        self.record_message_render_cache_miss();
        let cards = self.render_message_cards(message, body_width, is_round_end);

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
        is_round_end: bool,
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

                let body_lines =
                    self.render_assistant_body_lines(message, body_width, is_round_end);
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
                if message.content.starts_with(COMPACTION_MESSAGE_LABEL) {
                    let summary = message
                        .content
                        .split_once("\n\n")
                        .map(|(_, summary)| summary)
                        .unwrap_or("");

                    let mut lines = Vec::new();
                    lines.push(Line::from(""));
                    lines.push(render_compaction_divider_line(
                        COMPACTION_MESSAGE_LABEL,
                        body_width,
                        palette,
                    ));
                    lines.push(Line::from(""));
                    lines.extend(self.render_text_body_lines(
                        summary,
                        body_width,
                        Some(self.workspace_root.as_path()),
                    ));
                    lines.push(Line::from(""));
                    return vec![(palette.background, lines)];
                }

                if message.content.starts_with("Loaded instructions from")
                    || message.content.starts_with("Loaded ")
                        && message.content.contains(" instruction files:")
                {
                    let lines = vec![Line::from(vec![
                        Span::styled("󱁤 ", Style::default().fg(palette.accent_soft)),
                        Span::styled(
                            message.content.clone(),
                            Style::default()
                                .fg(palette.text)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ])];
                    return vec![(palette.background, lines)];
                }

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
        is_round_end: bool,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if !message.reasoning.trim().is_empty() {
            lines.extend(self.render_reasoning_lines(&message.reasoning, body_width));
            if !message.content.trim().is_empty() {
                lines.push(Line::from(""));
            }
        }

        if !message.content.is_empty() {
            if let Some((diff_lines, _)) =
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

        // Add model name, duration, end time, and mode at the end (only for round end)
        if is_round_end && !message.streaming {
            let model_display_name = message
                .model_id
                .as_ref()
                .and_then(|model_id| {
                    self.config
                        .resolve_model_by_ids(&self.auth, &self.conversation.provider_id, model_id)
                        .ok()
                        .map(|model| model.display_name)
                })
                .unwrap_or_else(|| self.conversation.model_display_name.clone());

            let duration = message.completed_at.map(|completed| {
                let elapsed = completed - message.created_at;
                let secs = elapsed.as_seconds_f64();
                format!("{:.1}s", secs)
            });

            let end_time = message.completed_at.map(|completed| {
                completed
                    .with_timezone(&Local)
                    .format("%H:%M:%S")
                    .to_string()
            });

            let tps = message
                .tokens_per_second
                .map(|val| format!("{:.1} t/s", val));

            let mode_label = message
                .mode
                .or_else(|| {
                    // Fallback to the preceding user message mode if not set on assistant message
                    self.conversation
                        .messages
                        .iter()
                        .take_while(|m| m.id != message.id)
                        .filter(|m| m.role == MessageRole::User)
                        .last()
                        .and_then(|m| m.mode)
                })
                .unwrap_or(self.mode);

            let parts: Vec<String> = [
                Some(model_display_name),
                duration,
                tps,
                end_time,
                Some(mode_label.title().to_string()),
            ]
            .into_iter()
            .flatten()
            .collect();

            let suffix = parts.join(" · ");
            lines.push(line_with_style_right_aligned(
                &suffix,
                body_width,
                self.palette().accent_soft,
            ));
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

        // Render reasoning first if present (preserves thinking at interruption point)
        if !message.reasoning.trim().is_empty() {
            lines.extend(self.render_reasoning_lines(&message.reasoning, body_width));
            lines.push(Line::from(""));
        }

        let error_text = if message.content.trim().is_empty() {
            "Request cancelled.".to_string()
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
            lines.push(line_with_style("! Request cancelled.", palette.error));
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

        // Title line: description (@subagent_type)
        let description = shorten(&execution.task_description, body_width.saturating_sub(30));
        let subagent_type = execution.subagent_type.clone();

        let mut lines = Vec::new();

        // Header line with description and subagent type
        lines.push(Line::from(vec![
            Span::styled(
                description,
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" (@{})", subagent_type),
                Style::default().fg(palette.muted),
            ),
        ]));

        // Status line - always shown to maintain consistent height
        let status_text = execution.status.display();
        let status_line = match &execution.status {
            SubagentStatus::Tool => {
                if let Some(tool_call) = &execution.current_tool_call {
                    let tool_summary = if tool_call_arguments_are_complete(&tool_call.arguments) {
                        summarize_tool_call(
                            &tool_call.name,
                            &tool_call.arguments,
                            body_width.saturating_sub(10),
                            self.workspace_root.as_path(),
                        )
                    } else {
                        let canonical_display = canonical_tool_name(&tool_call.name)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| tool_call.name.clone());
                        format!("{} ...", canonical_display)
                    };
                    format!("{}: {}", status_text, tool_summary)
                } else {
                    status_text.to_string()
                }
            }
            _ => status_text.to_string(),
        };

        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(status_line, Style::default().fg(palette.accent_soft)),
        ]));

        // Navigation hint
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(
                "Ctrl+X then ".to_string(),
                Style::default().fg(palette.muted),
            ),
            Span::styled("Up".to_string(), Style::default().fg(palette.accent_soft)),
            Span::styled("/".to_string(), Style::default().fg(palette.muted)),
            Span::styled("Down".to_string(), Style::default().fg(palette.accent_soft)),
            Span::styled(
                " to navigate".to_string(),
                Style::default().fg(palette.muted),
            ),
        ]));

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
        let streaming_mode_changed = index.contains_streaming_messages != force_rebuild;

        // Check if we need a full rebuild
        let needs_full_rebuild = force_rebuild
            || streaming_mode_changed
            || !index.valid
            || index.width != width
            || message_count_changed
            || index.blocks.is_empty() && !messages.is_empty();

        if needs_full_rebuild {
            index.blocks.clear();
            index.total_lines = 0;
            index.width = width;
            index.valid = true;
            index.contains_streaming_messages = force_rebuild;

            let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
            let spinner = self.loading_spinner();
            let ctx = RenderContext {
                palette: self.palette(),
                spinner,
                workspace_root: self.workspace_root.as_path(),
                expanded_tool_results: &self.expanded_tool_results,
                expanded_tool_outputs: &expanded_tool_outputs,
            };

            let mut current_line = 0;
            let mut i = 0;

            while i < messages.len() {
                // Build block without start_line (calculated below)
                // Determine is_round_end before generation
                let count = if matches!(messages[i].role, MessageRole::Assistant) {
                    let mut c = 1;
                    while i + c < messages.len()
                        && matches!(messages[i + c].role, MessageRole::Tool)
                    {
                        c += 1;
                    }
                    c
                } else {
                    1
                };
                let next_idx = i + count;
                let is_round_end = next_idx >= messages.len()
                    || matches!(messages[next_idx].role, MessageRole::User);

                let (message_id, message_count, line_count) = self.build_message_block_data(
                    messages,
                    i,
                    width,
                    body_width,
                    &ctx,
                    is_round_end,
                );

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
        let spinner = self.loading_spinner();
        let ctx = RenderContext {
            palette: self.palette(),
            spinner,
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

            let count = if matches!(messages[i].role, MessageRole::Assistant) {
                let mut c = 1;
                while i + c < messages.len() && matches!(messages[i + c].role, MessageRole::Tool) {
                    c += 1;
                }
                c
            } else {
                1
            };
            let next_idx = i + count;
            let is_round_end =
                next_idx >= messages.len() || matches!(messages[next_idx].role, MessageRole::User);

            let (_message_id, message_count, line_count) =
                self.build_message_block_data(messages, i, width, body_width, &ctx, is_round_end);
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
        is_round_end: bool,
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
                let cards = self.cached_render_message_cards(message, body_width, is_round_end);
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
                        let (card_lines, _) = self.cached_render_tool_call_with_result(
                            message,
                            tool_call,
                            tool_result,
                            body_width,
                            message.streaming,
                            ctx,
                        );
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
                let cards = self.cached_render_message_cards(message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.panel_alt).len();
                }
                lines += 1; // Empty line after user message
                (1, lines)
            }
            MessageRole::System => {
                let cards = self.cached_render_message_cards(message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.background).len();
                }
                (1, lines)
            }
            MessageRole::Error => {
                let cards = self.cached_render_message_cards(message, body_width, is_round_end);
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
    #[allow(clippy::too_many_arguments)]
    fn render_message_block_to_lines(
        &self,
        messages: &[Message],
        block: &super::MessageBlock,
        width: usize,
        body_width: usize,
        card_ranges: &mut Vec<ToolResultCardRange>,
        selectable_regions_ranges: &mut Vec<SelectableRegionRange>,
        current_line_offset: usize,
        ctx: &RenderContext<'_>,
        is_round_end: bool,
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
                let assistant_cards =
                    self.cached_render_message_cards(message, body_width, is_round_end);
                for (card_bg, card_lines) in assistant_cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();

                        let mut block_start = start_line;
                        let mut current_min_x = 1;
                        for (i, line) in card_lines.iter().enumerate() {
                            let is_reasoning =
                                line.spans.first().is_some_and(|s| s.content == "┃ ");
                            let line_min_x = if is_reasoning { 3 } else { 1 };

                            if line_min_x != current_min_x {
                                if i > 0 {
                                    selectable_regions_ranges.push(SelectableRegionRange {
                                        start_line: block_start,
                                        end_line: start_line + i,
                                        min_x: current_min_x,
                                        max_x: None,
                                    });
                                }
                                block_start = start_line + i;
                                current_min_x = line_min_x;
                            }
                        }
                        if block_start < start_line + card_lines.len() {
                            selectable_regions_ranges.push(SelectableRegionRange {
                                start_line: block_start,
                                end_line: start_line + card_lines.len(),
                                min_x: current_min_x,
                                max_x: None,
                            });
                        }

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
                        let (tool_card_lines, mut regions) = self
                            .cached_render_tool_call_with_result(
                                message,
                                tool_call,
                                tool_result,
                                body_width,
                                message.streaming,
                                ctx,
                            );
                        if !tool_card_lines.is_empty() {
                            let start_line = current_line_offset + lines.len();

                            // Adjust regions mapping
                            for r in &mut regions {
                                r.start_line += start_line;
                                r.end_line += start_line;
                                r.min_x += 1; // decorate_card_lines left padding
                                if let Some(max_x) = &mut r.max_x {
                                    *max_x += 1;
                                }
                                selectable_regions_ranges.push(r.clone());
                            }

                            // Calculate fallback region for bash or non-diff output
                            if regions.is_empty() {
                                selectable_regions_ranges.push(SelectableRegionRange {
                                    start_line,
                                    end_line: start_line + tool_card_lines.len(),
                                    min_x: 1,
                                    max_x: None,
                                });
                            }

                            let card_bg = if canonical_tool_name(&tool_call.name)
                                == Some("task")
                            {
                                palette.panel
                            } else {
                                palette.panel_light
                            };
                            let decorated =
                                decorate_card_lines(tool_card_lines, width, card_bg);
                            if let Some(result_msg) = tool_result {
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
                let cards = self.cached_render_message_cards(message, body_width, is_round_end);
                let bg = match message.role {
                    MessageRole::User => palette.panel_alt,
                    MessageRole::Error => palette.panel_light,
                    _ => palette.background,
                };
                for (_, card_lines) in cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();

                        let mut block_start = start_line;
                        let mut current_min_x = 1;
                        for (i, line) in card_lines.iter().enumerate() {
                            let is_reasoning =
                                line.spans.first().is_some_and(|s| s.content == "┃ ");
                            let line_min_x = if is_reasoning { 3 } else { 1 };

                            if line_min_x != current_min_x {
                                if i > 0 {
                                    selectable_regions_ranges.push(SelectableRegionRange {
                                        start_line: block_start,
                                        end_line: start_line + i,
                                        min_x: current_min_x,
                                        max_x: None,
                                    });
                                }
                                block_start = start_line + i;
                                current_min_x = line_min_x;
                            }
                        }
                        if block_start < start_line + card_lines.len() {
                            selectable_regions_ranges.push(SelectableRegionRange {
                                start_line: block_start,
                                end_line: start_line + card_lines.len(),
                                min_x: current_min_x,
                                max_x: None,
                            });
                        }

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
    // Use 0.5 ratio for a balanced dimmed appearance that works consistently across terminals
    // This avoids the inconsistent behavior of Modifier::DIM which varies significantly
    // between Windows Terminal (strong dimming) and Ghostty (weak/no dimming)
    let dimmed_color = crate::theme::mix_colors(palette.muted, palette.background, 0.5);
    let label_style = Style::default().fg(dimmed_color);
    let label_italic_style = Style::default()
        .fg(dimmed_color)
        .add_modifier(Modifier::ITALIC);
    let body_style = Style::default().fg(dimmed_color);

    lines.push(Line::from(vec![
        Span::styled("┃ ", label_style),
        Span::styled("Thinking:", label_italic_style),
    ]));

    if reasoning.trim().is_empty() {
        return lines;
    }

    let content_width = body_width.saturating_sub(2).max(1);
    let rendered = render_markdown_text_with_width_and_cwd(reasoning, Some(content_width), cwd);

    if rendered.lines.is_empty() {
        return lines;
    }

    let mut rendered_lines = rendered.lines.into_iter();

    // Skip leading blank lines to fix extra top spacing
    let mut first_line = rendered_lines.next();
    while let Some(line) = first_line {
        if line
            .spans
            .iter()
            .all(|s| s.content.trim().is_empty() && s.style == Style::default())
        {
            first_line = rendered_lines.next();
        } else {
            first_line = Some(line);
            break;
        }
    }

    if let Some(line) = first_line {
        let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
        spans.push(Span::styled("┃ ", label_style));
        spans.extend(line.spans.into_iter().map(|mut span| {
            // Mix the foreground color with background for all spans (including highlighted ones)
            // Use 0.4 ratio for a slightly more visible dimmed text
            // Note: We intentionally do NOT use Modifier::DIM here because its behavior
            // varies significantly between terminals (Windows Terminal dims heavily,
            // Ghostty barely dims at all), causing inconsistent appearance.
            if let Some(fg) = span.style.fg {
                span.style = span
                    .style
                    .fg(crate::theme::mix_colors(fg, palette.background, 0.4));
            } else {
                span.style = span.style.patch(body_style);
            }
            span
        }));
        lines.push(Line::from(spans));
    }

    for line in rendered_lines {
        let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
        spans.push(Span::styled("┃ ", label_style));
        spans.extend(line.spans.into_iter().map(|mut span| {
            // Mix the foreground color with background for all spans (including highlighted ones)
            // Use 0.4 ratio for a slightly more visible dimmed text
            if let Some(fg) = span.style.fg {
                span.style = span
                    .style
                    .fg(crate::theme::mix_colors(fg, palette.background, 0.4));
            } else {
                span.style = span.style.patch(body_style);
            }
            span
        }));
        lines.push(Line::from(spans));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::{
        RenderContext, render_reasoning_markdown_lines, render_tool_call_with_result,
        render_tool_result_detail_lines,
    };
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

        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
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
            spinner: "|",
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
        };

        let (lines, _, _) = render_tool_result_detail_lines(&message, 80, &ctx);
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
            spinner: "|",
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
        };

        let (lines, _, _) = render_tool_result_detail_lines(&message, 80, &ctx);

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
    fn streaming_tool_call_switches_to_summary_after_arguments_parse() {
        use crate::session::ToolCall;

        let tool_call = ToolCall {
            id: "tool-call-id".to_string(),
            name: "read".to_string(),
            arguments: "{\"path\": \"/tmp/example.txt\"}".to_string(),
        };

        let ctx = RenderContext {
            palette: ThemePalette::dark(),
            spinner: "|",
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
        };

        let (lines, _) = render_tool_call_with_result(&tool_call, None, 80, true, &ctx);
        let text = text_lines_to_string(&lines);

        assert!(
            text.contains("Read example.txt"),
            "should show parsed summary: {}",
            text
        );
        assert!(
            !text.contains("Calling..."),
            "pending state should be replaced: {}",
            text
        );
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
        assert_eq!(
            misses_after, misses_before,
            "second render should use cache"
        );
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

        let (before, _, _, _, _, _, _) = app.messages_text(Some(80));
        let before_text = text_lines_to_string(&before.lines);
        assert!(before_text.contains("old cached content"));

        let message_id = app.conversation.messages[0].id;
        app.conversation.messages[0].content = "new refreshed content".to_string();
        app.invalidate_active_message_render_cache_for(message_id);

        let (after, _, _, _, _, _, _) = app.messages_text(Some(80));
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

        let (text, total_lines, _, _, _, _, _) = app.messages_text(Some(80));

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
        || first_line.starts_with("failed to read")
        || first_line.contains("Cannot read binary file")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
}

fn summarize_tool_call(tool_name: &str, arguments: &str, body_width: usize, workspace_root: &Path) -> String {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let fields = summarize_tool_arguments(tool_name, arguments);
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();

    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    let path_to_relative = |path: &str| {
        display_workspace_relative(workspace_root, Path::new(path))
    };

    let summary = match canonical_name {
        "read" => field("path")
            .map(|path| format!("Read {}", path_to_relative(path)))
            .unwrap_or_else(|| "Read file".to_string()),
        "write" => field("path")
            .map(|path| format!("Write {}", path_to_relative(path)))
            .unwrap_or_else(|| "Write file".to_string()),
        "edit" => field("path")
            .map(|path| format!("Edit {}", path_to_relative(path)))
            .unwrap_or_else(|| "Edit file".to_string()),
        "list" => field("path")
            .map(|path| format!("List {}", path_to_relative(path)))
            .unwrap_or_else(|| "List items".to_string()),
        "glob" => {
            let pattern = field("pattern").unwrap_or("*");
            let path = field("path").unwrap_or(".");
            format!("Find {} in {}", pattern, path_to_relative(path))
        }
        "grep" => {
            let pattern = field("pattern").unwrap_or("");
            let path = field("path").unwrap_or(".");
            if pattern.is_empty() {
                format!("Search in {}", path_to_relative(path))
            } else {
                format!("Search \"{pattern}\" in {}", path_to_relative(path))
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
        "skill" => field("name")
            .map(|name| format!("Loaded skill {name}"))
            .unwrap_or_else(|| "Load skill".to_string()),
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
        "skill" => {
            if let Some(name) = string_field("name") {
                fields.push(("name".to_string(), name));
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

/// Parse range string "start-end" into (start, end).
fn parse_range(s: &str) -> Option<(i64, i64)> {
    let parts: Vec<_> = s.split('-').collect();
    if parts.len() == 2 {
        let start = parts[0].trim().parse().ok()?;
        let end = parts[1].trim().parse().ok()?;
        Some((start, end))
    } else {
        None
    }
}

/// Parsed metadata from a read tool output's XML-style metadata block.
/// Returns (line_range, optional_requested_range, file_total, optional_truncation_reason).
type ReadContentMetadata = Option<(
    (i64, i64),              // line_range (start, end)
    Option<(i64, i64)>,      // requested_range (None if model didn't specify)
    i64,                     // file_total
    Option<String>,          // truncated_by (None | "size" | "lines")
)>;

fn parse_read_content_metadata(content: &str) -> ReadContentMetadata {
    let mut line_range = None;
    let mut requested_range = None;
    let mut file_total = None;
    let mut truncated_by = None;

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("<line_range>") {
            if let Some(end) = val.strip_suffix("</line_range>") {
                line_range = parse_range(end);
            }
        } else if let Some(val) = line.strip_prefix("<requested_range>") {
            if let Some(end) = val.strip_suffix("</requested_range>") {
                requested_range = parse_range(end);
            }
        } else if let Some(val) = line.strip_prefix("<file_total>") {
            if let Some(end) = val.strip_suffix("</file_total>") {
                file_total = end.trim().parse().ok();
            }
        } else if let Some(val) = line.strip_prefix("<truncated_by>")
            && let Some(end) = val.strip_suffix("</truncated_by>")
            && matches!(end.trim(), "size" | "lines")
        {
            truncated_by = Some(end.trim().to_string());
        }
    }

    match (line_range, file_total) {
        (Some(lr), Some(ft)) => Some((lr, requested_range, ft, truncated_by)),
        _ => None,
    }
}

fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}

fn display_tool_name(tool_name: &str) -> String {
    crate::tooling::canonical_tool_name(tool_name)
        .unwrap_or(tool_name)
        .to_string()
}
