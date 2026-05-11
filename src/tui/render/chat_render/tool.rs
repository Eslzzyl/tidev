use crate::{
    markdown_render::{WrapOptions, render_markdown_text_with_width_and_cwd, word_wrap_line},
    session::{Message, ToolCall},
    theme::ThemePalette,
    tooling::builtin::utils::display_workspace_relative,
    tooling::{TodoItem, canonical_tool_name},
    tui::core::state::SelectableRegionRange,
};
use ratatui::{
    prelude::{Modifier, Style},
    text::{Line, Span},
};
use std::collections::HashSet;
use std::path::Path;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use super::utils::{
    parse_line_range_from_read_output, parse_read_content_metadata, summarize_tool_arguments,
    summarize_tool_call, tool_output_is_error,
};
use super::{RenderContext, TOOL_OUTPUT_EXPANDED_MAX_LINES, TOOL_OUTPUT_PREVIEW_LINES};
use crate::tui::diff_render::render_unified_diff_text;
use crate::tui::render::render::{line_with_style, shorten, shorten_single_line};

pub(super) fn render_tool_call_with_result(
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

    let call_lines = render_tool_call_lines(
        tool_call,
        body_width,
        palette,
        exit_code,
        rtk_rewritten,
        ctx.workspace_root,
    );
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

pub(super) fn render_compaction_divider_line(
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

pub(super) fn tool_call_arguments_are_complete(arguments: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(arguments).is_ok()
}

pub(super) fn render_tool_call_summary_line(
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
            let summary = summarize_tool_call(
                &tool_call.name,
                &tool_call.arguments,
                body_width,
                ctx.workspace_root,
            );
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

pub(super) fn compute_tool_result_suffix(canonical_name: &str, output: &str) -> String {
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

pub(super) fn tool_output_is_truncated(output: &str) -> bool {
    output.contains("output truncated:")
        || output.contains("... (truncated)")
        || output.contains("(Output capped at")
        || output.contains("[truncated]")
}

/// Parse bash output to extract exit code and remaining output.
/// Format: "[exit N]\n<output>"
pub(super) fn parse_bash_exit_code(output: &str) -> (Option<i32>, &str) {
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

pub(super) fn render_tool_call_lines(
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
            let summary = summarize_tool_call(
                &tool_call.name,
                &tool_call.arguments,
                body_width,
                workspace_root,
            );
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

pub(super) fn render_tool_result_detail_lines(
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

    // Question tool results: render Q&A pairs with clear formatting
    if canonical_name == "question" {
        return (
            render_question_result_pairs(effective_output, body_width, palette),
            None,
            vec![],
        );
    }

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

    // Subagent task results: render markdown preview (collapsed) or full (expanded)
    if canonical_name == "task" {
        let is_expanded = ctx.expanded_tool_results.contains(&message.id);
        return (
            render_subagent_task_preview(effective_output, body_width, palette, is_expanded),
            None,
            vec![],
        );
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

/// Renders a compact or full markdown preview of a subagent task result.
pub(super) fn render_subagent_task_preview(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
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

    // Render the output as markdown
    let rendered =
        render_markdown_text_with_width_and_cwd(output, Some(body_width.saturating_sub(2)), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        // Show all lines when expanded
        let max_lines = TOOL_OUTPUT_EXPANDED_MAX_LINES;
        let line_count = md_lines.len();
        if line_count <= max_lines {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_lines));
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  ▼ {} more line(s) — Click to expand",
                    line_count - max_lines
                ),
                Style::default().fg(palette.muted),
            )]));
        }
        lines.push(Line::from(vec![Span::styled(
            if line_count > max_lines {
                "▲ Click to collapse"
            } else {
                "▲  Click to collapse"
            },
            Style::default().fg(palette.muted),
        )]));
    } else {
        // Preview mode: show first few lines
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
    }

    lines
}

pub(super) fn render_todos_checkbox_list(
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

/// Parses the formatted output of the question tool (produced by `QuestionDialogState::formatted_output`)
/// and renders each Q&A pair as a clearly styled block.
///
/// Expected input format:
///   Q1: question text
///   A: answer text
///   Q2: another question
///   A: another answer
pub(super) fn render_question_result_pairs(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let wrap_width = body_width.saturating_sub(4);

    // Title
    lines.push(Line::from(vec![Span::styled(
        "Questions & Answers",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    if output.trim().is_empty() {
        lines.push(line_with_style("(no output)", palette.muted));
        return lines;
    }

    let mut lines_iter = output.lines().peekable();
    while let Some(q_line) = lines_iter.next() {
        // Skip empty lines
        if q_line.trim().is_empty() {
            continue;
        }

        // Question line starts with "Q" (e.g. "Q1: ...")
        let question_text: String = q_line
            .strip_prefix("Q")
            .and_then(|rest| rest.split_once(':').map(|x| x.1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| q_line.to_string());

        // Answer line follows (starts with "A: ")
        let answer_text = lines_iter
            .next()
            .and_then(|a_line| a_line.strip_prefix("A: "))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Render question
        lines.push(Line::from(vec![Span::styled(
            "  Q: ",
            Style::default()
                .fg(palette.accent_soft)
                .add_modifier(Modifier::BOLD),
        )]));
        let q_line_owned = Line::from(question_text.clone());
        let q_wrapped = word_wrap_line(&q_line_owned, WrapOptions::new(wrap_width));
        if q_wrapped.len() <= 1 {
            lines.push(Line::from(vec![Span::styled(
                format!("     {}", question_text),
                Style::default().fg(palette.text),
            )]));
        } else {
            for (i, wl) in q_wrapped.iter().enumerate() {
                let prefix = if i == 0 { "     " } else { "       " };
                lines.push(Line::from(vec![Span::styled(
                    format!(
                        "{}{}",
                        prefix,
                        wl.spans.iter().map(|s| &*s.content).collect::<String>()
                    ),
                    Style::default().fg(palette.text),
                )]));
            }
        }

        // Render answer
        lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(palette.success)),
            Span::styled(
                answer_text,
                Style::default()
                    .fg(palette.success)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(""));
    }

    lines
}

pub(super) fn tool_output_from_message<'a>(
    message: &'a Message,
    ctx: &'a RenderContext<'_>,
) -> &'a str {
    ctx.expanded_tool_outputs
        .get(&message.id)
        .map(|output| output.as_str())
        .unwrap_or_else(|| message.content.as_str())
}

pub(super) fn render_output_preview_lines(
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
