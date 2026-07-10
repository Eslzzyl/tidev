//! Tool call and tool result rendering.
//!
//! Renders tool call cards with argument previews, expand/collapse for tool
//! results, pending/waiting states during streaming, and specialised
//! formatting for read/write/edit/bash/websearch/webfetch/task/question/todowrite tools.

use std::collections::HashMap;
use std::path::Path;

use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use tidev_types::message::{Message, MessageAttachment, ToolCall};
use tidev_types::tools::{canonical_tool_name, TaskArgs};
use tidev_utils::path::display_workspace_relative;
use crate::theme::ThemePalette;
use unicode_width::UnicodeWidthStr;

use crate::ansi::ansi_to_styled_line;
use crate::components::chat::render::RenderContext;
use crate::components::chat::render_cache::SelectableRegionRange;
use crate::diff_render::render_unified_diff_text;
use crate::markdown::{WrapOptions, render_markdown_text_with_width_and_cwd, word_wrap_line};
use crate::utils::expand_tabs;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TOOL_OUTPUT_PREVIEW_LINES: usize = 5;

// ---------------------------------------------------------------------------
// Main entry
// ---------------------------------------------------------------------------

/// Render a tool call with its optional result.
///
/// Returns (rendered_lines, selectable_regions).
pub(crate) fn render_tool_call_with_result(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    body_width: usize,
    is_streaming: bool,
    ctx: &RenderContext,
    is_expanded: bool,
) -> (Vec<Line<'static>>, Vec<SelectableRegionRange>) {
    let palette = ctx.palette;
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);

    // Pending state: arguments not yet complete
    let is_pending = tool_result.is_none()
        && is_streaming
        && !matches!(canonical_name, "read" | "glob" | "grep")
        && !tool_call_arguments_are_complete(&tool_call.arguments);

    // Precompute line counts for write/edit/patch tools
    let write_lines = if matches!(canonical_name, "write") && tool_result.is_none() {
        count_lines_in_partial_json(&tool_call.arguments, "content")
    } else {
        0
    };
    let (edit_old_lines, edit_new_lines) =
        if matches!(canonical_name, "edit") && tool_result.is_none() {
            (
                count_lines_in_partial_json(&tool_call.arguments, "old_text")
                    .max(count_lines_in_partial_json(&tool_call.arguments, "old_string")),
                count_lines_in_partial_json(&tool_call.arguments, "new_text")
                    .max(count_lines_in_partial_json(&tool_call.arguments, "new_string")),
            )
        } else {
            (0, 0)
        };
    let (patch_add_lines, patch_del_lines, patch_file_ops) =
        if matches!(canonical_name, "apply_patch") && tool_result.is_none() {
            count_patch_changes(&tool_call.arguments)
        } else {
            (0, 0, 0)
        };

    let is_waiting_result = tool_result.is_none()
        && is_streaming
        && matches!(canonical_name, "write" | "edit" | "apply_patch")
        && match canonical_name {
            "write" => write_lines > 0,
            "edit" => edit_old_lines > 0 || edit_new_lines > 0,
            "apply_patch" => patch_file_ops > 0,
            _ => false,
        };

    // Summary-only tools (read/glob/grep/skill)
    if matches!(canonical_name, "grep" | "glob" | "read" | "skill") {
        return (
            render_tool_call_summary_line_inner(tool_call, tool_result, body_width, palette, ctx),
            Vec::new(),
        );
    }

    // Completed task tools → subagent result card
    if canonical_name == "task"
        && tool_result.is_some()
        && let Ok(task_args) = serde_json::from_str::<TaskArgs>(&tool_call.arguments)
    {
        let output = tool_result.map(|m| m.content.as_str()).unwrap_or("");
        let lines = render_subagent_task_preview(
            output, body_width, palette, is_expanded, &task_args.description, task_args.subagent_type.as_str(),
        );
        return (lines, Vec::new());
    }

    // Get result lines and exit code (for bash)
    let (result_lines, exit_code, mut regions) = if let Some(result_msg) = tool_result {
        render_tool_result_detail_lines(result_msg, body_width, ctx, is_expanded)
    } else {
        (Vec::new(), None, vec![])
    };

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    // For write/edit/apply_patch that have live line count info, skip the
    // generic "Preparing" state and show title + progress directly.
    let has_live_progress = match canonical_name {
        "write" => write_lines > 0,
        "edit" => edit_old_lines > 0 || edit_new_lines > 0,
        "apply_patch" => patch_file_ops > 0,
        _ => false,
    };

    if is_pending && !has_live_progress {
        // No live progress data yet → show generic preparing state
        let preparing_text = preparing_text_for_tool(canonical_name);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", ctx.spinner),
                Style::default().fg(palette.accent_soft),
            ),
            Span::styled(preparing_text, Style::default().fg(palette.muted)),
        ]));
    } else {
        // Arguments complete or write/edit with live progress: show header
        let call_lines = render_tool_call_lines(
            tool_call,
            body_width,
            palette,
            exit_code,
            ctx.workspace_root,
        );
        lines.extend(call_lines);
    }

    // Show live progress during streaming or waiting for write/edit
    if (is_pending && has_live_progress) || is_waiting_result {
        let progress_text = match canonical_name {
            "write" if write_lines > 0 => format!("Writing {} lines...", write_lines),
            "edit" if edit_old_lines > 0 && edit_new_lines > 0 => {
                format!("Replacing {} lines with {} lines...", edit_old_lines, edit_new_lines)
            }
            "edit" if edit_old_lines > 0 => {
                format!("Replacing {} lines with 0 lines...", edit_old_lines)
            }
            "edit" if edit_new_lines > 0 => {
                format!("Replacing 0 lines with {} lines...", edit_new_lines)
            }
            "apply_patch" if patch_file_ops > 0 => {
                let mut parts = vec![];
                if patch_add_lines > 0 { parts.push(format!("+{}", patch_add_lines)); }
                if patch_del_lines > 0 { parts.push(format!("-{}", patch_del_lines)); }
                let change_summary = if parts.is_empty() {
                    String::new()
                } else {
                    format!(" ({} lines)", parts.join(" "))
                };
                format!("Applying patch to {}{}...",
                    pluralize(patch_file_ops, "file", "files"), change_summary)
            }
            _ => unreachable!(),
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", ctx.spinner),
                Style::default().fg(palette.accent_soft),
            ),
            Span::styled(progress_text, Style::default().fg(palette.muted)),
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

// ---------------------------------------------------------------------------
// Summary rendering for read/glob/grep (enhanced with action labels and rich suffixes)
// ---------------------------------------------------------------------------

fn render_tool_call_summary_line_inner(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    body_width: usize,
    palette: ThemePalette,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);
    let parsed = serde_json::from_str::<serde_json::Value>(&tool_call.arguments).ok();

    let string_field = |key: &str| {
        parsed.as_ref()
            .and_then(|v| v.get(key))
            .and_then(serde_json::Value::as_str)
            .map(|s| s.to_string())
    };

    let rel_path = |p: &str| display_workspace_relative(ctx.workspace_root, Path::new(p));

    let (action_label, target) = match canonical_name {
        "grep" => {
            let pattern = string_field("pattern").unwrap_or_default();
            let path = string_field("path").unwrap_or_else(|| ".".to_string());
            ("Search", format!("\"{}\" in {}", pattern, rel_path(&path)))
        }
        "glob" => {
            let pattern = string_field("pattern").unwrap_or_else(|| "*".to_string());
            let path = string_field("path").unwrap_or_else(|| ".".to_string());
            ("Find", format!("{} in {}", pattern, rel_path(&path)))
        }
        "read" => {
            let path = string_field("file_path").unwrap_or_else(|| "file".to_string());
            ("Read", rel_path(&path).to_string())
        }
        "skill" => {
            let name = string_field("name").unwrap_or_default();
            ("Loaded skill", name)
        }
        _ => {
            let summary = summarize_tool_call(tool_call, body_width);
            return vec![Line::from(vec![Span::styled(
                summary,
                Style::default().fg(palette.text).add_modifier(Modifier::BOLD),
            )])];
        }
    };

    let result_suffix = if let Some(result_msg) = tool_result {
        let output = &result_msg.content;
        compute_tool_result_suffix(canonical_name, output, &result_msg.attachments)
    } else {
        " ...".to_string()
    };

    let line = Line::from(vec![
        Span::styled(format!("{} ", action_label), Style::default().fg(palette.accent_soft)),
        Span::styled(target.clone(), Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
        Span::styled(result_suffix, Style::default().fg(palette.muted)),
    ]);

    // Wrap the line if it exceeds body_width
    let indent_width = UnicodeWidthStr::width(action_label) + 1;
    let indent = Line::from(" ".repeat(indent_width));
    let wrapped = word_wrap_line(
        &line,
        WrapOptions::new(body_width)
            .subsequent_indent(indent)
            .break_words(true),
    );
    wrapped.into_iter().map(|l| {
        Line::from(l.spans.into_iter().map(|s| Span::styled(s.content.to_string(), s.style)).collect::<Vec<_>>())
    }).collect()
}

// ---------------------------------------------------------------------------
// Subagent task preview
// ---------------------------------------------------------------------------

fn render_subagent_task_preview(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    description: &str,
    subagent_type: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled("(empty result)", Style::default().fg(palette.muted))));
        return lines;
    }

    // Top padding
    lines.push(Line::from(""));

    // Header: [@type] subagent: description
    let header_line = Line::from(vec![
        Span::styled(format!("@{}", subagent_type), Style::default().fg(palette.accent_soft)),
        Span::styled(" subagent: ", Style::default().fg(palette.muted)),
        Span::styled(description.to_string(), Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
    ]);
    lines.extend(
        word_wrap_line(&header_line, WrapOptions::new(body_width).break_words(true))
            .into_iter().map(|l| {
                Line::from(l.spans.into_iter().map(|s| Span::styled(s.content.to_string(), s.style)).collect::<Vec<_>>())
            }),
    );
    lines.push(Line::from(""));

    // Render output as markdown
    let rendered = render_markdown_text_with_width_and_cwd(output, Some(body_width.saturating_sub(2)), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        lines.extend(md_lines);
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(Line::from(vec![Span::styled(
                format!("   {} more line(s)", line_count - max_preview),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Tool-type-specific title/command rendering
// ---------------------------------------------------------------------------

fn render_tool_call_lines(
    tool_call: &ToolCall,
    body_width: usize,
    palette: ThemePalette,
    exit_code: Option<i32>,
    workspace_root: &Path,
) -> Vec<Line<'static>> {
    let parsed = serde_json::from_str::<serde_json::Value>(&tool_call.arguments).ok();

    let string_field = |key: &str| {
        parsed.as_ref()
            .and_then(|v| v.get(key))
            .and_then(serde_json::Value::as_str)
            .map(|s| s.to_string())
    };

    let mut lines = Vec::new();
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or("");

    match canonical_name {
        "bash" => {
            let command = string_field("command").unwrap_or_default();
            let desc = string_field("description");

            let display = desc.as_deref().unwrap_or(&command);
            let mut title_spans = vec![
                Span::styled("Bash: ", Style::default().fg(palette.muted)),
                Span::styled(display.to_string(), Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
            ];

            if let Some(code) = exit_code {
                if code == 0 {
                    title_spans.push(Span::styled("  ✓", Style::default().fg(palette.success)));
                } else {
                    title_spans.push(Span::styled(format!("  ✗ {}", code), Style::default().fg(palette.error)));
                }
            }
            lines.extend(wrap_tool_title(Line::from(title_spans), body_width, "      "));

            for cmd_line in command.lines() {
                let owned_line = Line::from(cmd_line.to_string());
                let wrapped = word_wrap_line(
                    &owned_line,
                    WrapOptions::new(body_width.saturating_sub(4)).break_words(true),
                );
                for (i, wl) in wrapped.iter().enumerate() {
                    let mut spans = Vec::new();
                    if i == 0 {
                        spans.push(Span::styled("  $ ", Style::default().fg(palette.accent)));
                    } else {
                        spans.push(Span::styled("    ", Style::default()));
                    }
                    spans.extend(wl.spans.iter().map(|s| {
                        Span::styled(s.content.to_string(), Style::default().fg(palette.text))
                    }));
                    lines.push(Line::from(spans));
                }
            }
        }
        "write" => {
            let path = string_field("file_path").unwrap_or_else(|| "file".to_string());
            let rel = display_workspace_relative(workspace_root, Path::new(&path));
            lines.extend(wrap_tool_title(
                Line::from(vec![
                    Span::styled("Write ", Style::default().fg(palette.muted)),
                    Span::styled(rel, Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
                ]),
                body_width, "      ",
            ));
        }
        "edit" => {
            let path = string_field("file_path").unwrap_or_else(|| "file".to_string());
            let rel = display_workspace_relative(workspace_root, Path::new(&path));
            lines.extend(wrap_tool_title(
                Line::from(vec![
                    Span::styled("Edit ", Style::default().fg(palette.muted)),
                    Span::styled(rel, Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
                ]),
                body_width, "      ",
            ));
        }
        "websearch" => {
            let query = string_field("query").unwrap_or_default();
            let mut title_spans = vec![
                Span::styled("Search web for ", Style::default().fg(palette.accent_soft)),
                Span::styled(query, Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
            ];
            let mut suffix_parts: Vec<String> = Vec::new();
            if let Some(num) = parsed.as_ref()
                .and_then(|v| v.get("num_results"))
                .and_then(|v| v.as_i64())
            {
                suffix_parts.push(format!("max: {}", num));
            }
            if let Some(st) = string_field("search_type") {
                suffix_parts.push(st);
            }
            if !suffix_parts.is_empty() {
                title_spans.push(Span::styled(
                    format!("  ({})", suffix_parts.join(", ")),
                    Style::default().fg(palette.muted),
                ));
            }
            lines.extend(wrap_tool_title(Line::from(title_spans), body_width, "               "));
        }
        "webfetch" => {
            let url = string_field("url").unwrap_or_default();
            let mut title_spans = vec![
                Span::styled("Fetch web page from ", Style::default().fg(palette.accent)),
                Span::styled(url, Style::default().fg(palette.text).add_modifier(Modifier::BOLD)),
            ];
            let mut suffix_parts: Vec<String> = Vec::new();
            if let Some(fmt) = string_field("format") {
                suffix_parts.push(format!("format: {}", fmt));
            }
            if let Some(to) = parsed.as_ref()
                .and_then(|v| v.get("timeout"))
                .and_then(|v| v.as_i64())
            {
                suffix_parts.push(format!("{}s", to));
            }
            if !suffix_parts.is_empty() {
                title_spans.push(Span::styled(
                    format!("  ({})", suffix_parts.join(", ")),
                    Style::default().fg(palette.muted),
                ));
            }
            lines.extend(wrap_tool_title(Line::from(title_spans), body_width, "                    "));
        }
        "apply_patch" => {
            let patch_text = string_field("patch_text").unwrap_or_default();
            let file_paths: Vec<&str> = patch_text
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    trimmed.strip_prefix("*** Add File: ")
                        .or_else(|| trimmed.strip_prefix("*** Update File: "))
                        .or_else(|| trimmed.strip_prefix("*** Delete File: "))
                })
                .collect();
            let title = if file_paths.is_empty() {
                "Apply patch".to_string()
            } else if file_paths.len() == 1 {
                format!("Apply patch to {}", file_paths[0])
            } else {
                format!("Apply patch to {} files", file_paths.len())
            };
            lines.extend(wrap_tool_title(
                Line::from(vec![Span::styled(title, Style::default().fg(palette.text).add_modifier(Modifier::BOLD))]),
                body_width, "               ",
            ));
        }
        "question" => {
            let count = parsed.as_ref()
                .and_then(|v| v.get("questions"))
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let title = if count == 1 {
                "Ask 1 question".to_string()
            } else {
                format!("Ask {} questions", count)
            };
            lines.extend(wrap_tool_title(
                Line::from(vec![Span::styled(title, Style::default().fg(palette.text).add_modifier(Modifier::BOLD))]),
                body_width, "   ",
            ));
        }
        "todowrite" => {
            lines.extend(wrap_tool_title(
                Line::from(vec![Span::styled("Update todo list", Style::default().fg(palette.text).add_modifier(Modifier::BOLD))]),
                body_width, "   ",
            ));
        }
        _ => {
            let summary = summarize_tool_call(tool_call, body_width);
            lines.extend(wrap_tool_title(
                Line::from(vec![Span::styled(summary, Style::default().fg(palette.text).add_modifier(Modifier::BOLD))]),
                body_width, "  ",
            ));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Tool result detail rendering (dispatches by tool type)
// ---------------------------------------------------------------------------

fn render_tool_result_detail_lines(
    message: &Message,
    body_width: usize,
    ctx: &RenderContext,
    is_expanded: bool,
) -> (Vec<Line<'static>>, Option<i32>, Vec<SelectableRegionRange>) {
    let palette = ctx.palette;
    let output = message.content.as_str();
    let tool_name = message.tool_name.as_deref().unwrap_or("tool");
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);

    // For bash, parse exit code and strip it from output
    let (exit_code, effective_output) = if canonical_name == "bash" {
        parse_bash_exit_code(output)
    } else {
        (None, output)
    };

    let is_error = tool_output_is_error(effective_output);

    // Question tool results: render Q&A pairs
    if canonical_name == "question" {
        return (
            render_question_result_pairs(effective_output, body_width, palette),
            None,
            vec![],
        );
    }

    // apply_patch with structured file_changes
    if canonical_name == "apply_patch"
        && !is_error
        && !message.metadata.file_changes.is_empty()
    {
        let mut lines = Vec::new();
        let mut regions = Vec::new();
        let mut line_offset = 0usize;

        for change in &message.metadata.file_changes {
            let label = match change.operation.as_str() {
                "A" => "Write",
                "M" => "Edit",
                "D" => "Delete",
                _ => "Edit",
            };

            lines.push(Line::from(vec![Span::styled(
                format!("{} {}", label, change.path),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            line_offset += 1;

            if let Some(diff) = &change.diff
                && let Some((diff_lines, diff_regions)) =
                    render_unified_diff_text(diff, body_width.saturating_sub(2), palette, 4)
            {
                let n_diff = diff_lines.len();
                for mut r in diff_regions {
                    r.start_line += line_offset;
                    r.end_line += line_offset;
                    regions.push(r);
                }
                lines.extend(diff_lines);
                line_offset += n_diff;
            }

            lines.push(Line::from(""));
            line_offset += 1;
        }

        return (lines, None, regions);
    }

    // Try to render diff from metadata (preferred, not truncated)
    if !is_error
        && matches!(canonical_name, "edit" | "write" | "apply_patch")
        && let Some(diff) = message.metadata.diff.as_ref()
        && let Some((diff_lines, regions)) =
            render_unified_diff_text(diff, body_width.saturating_sub(2), palette, 4)
    {
        return (diff_lines, None, regions);
    }

    // Fallback: try to render diff from output (may be truncated)
    if !is_error
        && matches!(canonical_name, "edit" | "write" | "apply_patch")
        && let Some((diff_lines, regions)) =
            render_unified_diff_text(effective_output, body_width.saturating_sub(2), palette, 4)
    {
        return (diff_lines, None, regions);
    }

    // todowrite: render checkbox list
    if canonical_name == "todowrite" && !is_error {
        #[derive(serde::Deserialize)]
        struct RawTodo {
            content: String,
            status: Option<String>,
        }

        let raw_todos = if let Ok(todos) = serde_json::from_str::<Vec<RawTodo>>(effective_output) {
            Some(todos)
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(effective_output) {
            value.get("newTodos")
                .or_else(|| value.get("todos"))
                .and_then(|v| serde_json::from_value::<Vec<RawTodo>>(v.clone()).ok())
        } else {
            None
        };

        if let Some(raw_todos) = raw_todos {
            let todos: Vec<TodoItem> = raw_todos.into_iter()
                .map(|r| TodoItem {
                    content: r.content,
                    status: r.status.unwrap_or_else(|| "pending".to_string()),
                })
                .collect();
            return (
                render_todos_checkbox_list(&todos, body_width, palette),
                None,
                vec![],
            );
        }
    }

    // Web search results: styled markdown with header
    if canonical_name == "websearch" {
        return (
            render_websearch_result_lines(effective_output, body_width, palette, is_expanded, is_error),
            None,
            vec![],
        );
    }

    // Web fetch results: styled markdown with header
    if canonical_name == "webfetch" {
        return (
            render_webfetch_result_lines(effective_output, body_width, palette, is_expanded, is_error),
            None,
            vec![],
        );
    }

    // Fallback: standard output preview
    (
        render_output_preview_lines(effective_output, body_width, palette, is_expanded, is_error),
        exit_code,
        vec![],
    )
}

// ---------------------------------------------------------------------------
// Web search result rendering
// ---------------------------------------------------------------------------

fn render_websearch_result_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled("(no results)", Style::default().fg(palette.muted))));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(output, body_width, palette, is_expanded, true);
    }

    lines.push(Line::from(vec![Span::styled(
        "Search Results",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    let rendered = render_markdown_text_with_width_and_cwd(
        output, Some(body_width.saturating_sub(2)), None,
    );
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        let has_lines = !md_lines.is_empty();
        lines.extend(md_lines);
        if has_lines {
            lines.push(Line::from(vec![Span::styled(
                "▲ Click to collapse",
                Style::default().fg(palette.muted),
            )]));
        }
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(Line::from(vec![Span::styled(
                format!("  ▼ {} more line(s) — Click to expand", line_count - max_preview),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Web fetch result rendering
// ---------------------------------------------------------------------------

fn render_webfetch_result_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled("(empty page)", Style::default().fg(palette.muted))));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(output, body_width, palette, is_expanded, true);
    }

    lines.push(Line::from(vec![Span::styled(
        "Page Content",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    let rendered = render_markdown_text_with_width_and_cwd(
        output, Some(body_width.saturating_sub(2)), None,
    );
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        let has_lines = !md_lines.is_empty();
        lines.extend(md_lines);
        if has_lines {
            lines.push(Line::from(vec![Span::styled(
                "▲ Click to collapse",
                Style::default().fg(palette.muted),
            )]));
        }
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(Line::from(vec![Span::styled(
                format!("  ▼ {} more line(s) — Click to expand", line_count - max_preview),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Question result pairs rendering
// ---------------------------------------------------------------------------

fn render_question_result_pairs(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![Span::styled(
        "Questions & Answers",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    if output.trim().is_empty() {
        lines.push(Line::from(Span::styled("(no output)", Style::default().fg(palette.muted))));
        return lines;
    }

    let mut lines_iter = output.lines().peekable();
    while let Some(q_line) = lines_iter.next() {
        if q_line.trim().is_empty() {
            continue;
        }

        let question_text: String = q_line.strip_prefix("Q")
            .and_then(|rest| rest.split_once(':').map(|x| x.1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| q_line.to_string());

        let answer_text = lines_iter.next()
            .and_then(|a_line| a_line.strip_prefix("A: "))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let q_line_owned = Line::from(question_text.clone());
        let q_wrapped = word_wrap_line(
            &q_line_owned,
            WrapOptions::new(body_width)
                .initial_indent(Line::from(vec![Span::styled(
                    "  Q: ",
                    Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD),
                )]))
                .subsequent_indent(Line::from(vec![Span::styled(
                    "     ",
                    Style::default().fg(palette.text),
                )])),
        );
        for wl in q_wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl.spans.into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.fg(palette.text);
                }
            }
            lines.push(Line::from(owned_spans));
        }

        let a_line_owned = Line::from(answer_text.clone());
        let a_wrapped = word_wrap_line(
            &a_line_owned,
            WrapOptions::new(body_width)
                .initial_indent(Line::from(vec![Span::styled(
                    "  → ",
                    Style::default().fg(palette.success),
                )]))
                .subsequent_indent(Line::from(vec![Span::styled(
                    "     ",
                    Style::default().fg(palette.text),
                )])),
        );
        for wl in a_wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl.spans.into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.fg(palette.success).add_modifier(Modifier::BOLD);
                }
            }
            lines.push(Line::from(owned_spans));
        }

        lines.push(Line::from(""));
    }

    lines
}

// ---------------------------------------------------------------------------
// Todos checkbox list rendering
// ---------------------------------------------------------------------------

struct TodoItem {
    content: String,
    status: String,
}

fn render_todos_checkbox_list(
    todos: &[TodoItem],
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if todos.is_empty() {
        lines.push(Line::from(Span::styled("  (no items)", Style::default().fg(palette.muted))));
        return lines;
    }

    for todo in todos {
        let (checkbox, style) = match todo.status.as_str() {
            "completed" => (
                "x ",
                Style::default().fg(palette.muted).add_modifier(Modifier::CROSSED_OUT),
            ),
            "in_progress" => (
                "● ",
                Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
            ),
            _ => ("○ ", Style::default().fg(palette.text)),
        };

        let checkbox_prefix = format!("  {}", checkbox);
        let cb_width = UnicodeWidthStr::width(checkbox_prefix.as_str());
        let indent = " ".repeat(cb_width);

        let content_line = Line::from(todo.content.clone());
        let wrapped = word_wrap_line(
            &content_line,
            WrapOptions::new(body_width.saturating_sub(2))
                .initial_indent(Line::from(vec![Span::styled(checkbox_prefix, style)]))
                .subsequent_indent(Line::from(vec![Span::styled(indent, Style::default())])),
        );

        for wl in wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl.spans.into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.patch(style);
                }
            }
            lines.push(Line::from(owned_spans));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Tool result suffix computation (for summary lines)
// ---------------------------------------------------------------------------

fn compute_tool_result_suffix(
    canonical_name: &str,
    output: &str,
    attachments: &[MessageAttachment],
) -> String {
    match canonical_name {
        "grep" | "glob" => {
            if tool_output_is_error(output) {
                let count = if output.is_empty() { 0 } else { output.lines().count() };
                format!(" → failed ({} lines)", count)
            } else {
                let count = output.lines().next()
                    .and_then(|first_line| {
                        if first_line.starts_with("No files found") {
                            Some(0usize)
                        } else if let Some(rest) = first_line.strip_prefix("Found ") {
                            rest.split(' ').next().and_then(|n| n.parse().ok())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| output.lines().count());
                match count {
                    0 => " → no match".to_string(),
                    1 => " → 1 match".to_string(),
                    n => format!(" → {} matches", n),
                }
            }
        }
        "read" => {
            if tool_output_is_error(output) {
                if output.contains("file not found") && output.contains("Did you mean") {
                    " → not found (with suggestions)".to_string()
                } else {
                    " → error".to_string()
                }
            } else if has_image_attachment(attachments) {
                let image = attachments.iter().find_map(|a| {
                    if let MessageAttachment::Image { mime, file_size, .. } = a {
                        Some((mime.as_str(), *file_size))
                    } else {
                        None
                    }
                });
                if let Some((mime, size)) = image {
                    let type_label = mime.strip_prefix("image/").unwrap_or(mime);
                    format!(" → {}, {}", type_label, format_file_size(size))
                } else {
                    " → image".to_string()
                }
            } else if has_directory_attachment(attachments) {
                let mut files = 0u64;
                let mut dirs = 0u64;
                for line in output.lines().skip(1) {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    if trimmed.ends_with('/') { dirs += 1; } else { files += 1; }
                }
                let total = files + dirs;
                if total == 0 {
                    " → empty".to_string()
                } else if dirs == 0 {
                    format!(" → {} items ({} files)", total, files)
                } else if files == 0 {
                    format!(" → {} items ({} dirs)", total, dirs)
                } else {
                    format!(" → {} items ({} files, {} dirs)", total, files, dirs)
                }
            } else {
                let (metadata, _) = parse_read_content_metadata(output);
                match metadata {
                    Some(info) if !info.is_empty() => {
                        if let Some(lines_count) = info.get("lines") {
                            format!(" → {} lines", lines_count)
                        } else {
                            String::new()
                        }
                    }
                    _ => {
                        let total_lines = output.lines().count();
                        if total_lines == 0 {
                            " → empty".to_string()
                        } else if tool_output_is_truncated(output) {
                            format!(" → {} lines (truncated)", total_lines)
                        } else {
                            format!(" → {} lines", total_lines)
                        }
                    }
                }
            }
        }
        "skill" => {
            if tool_output_is_error(output) {
                " → error".to_string()
            } else {
                let content_lines: Vec<_> = output.lines()
                    .skip_while(|l| l.starts_with('#') || l.starts_with("**"))
                    .filter(|l| !l.is_empty())
                    .collect();
                format!(" → {} lines", content_lines.len())
            }
        }
        _ => String::new(),
    }
}

fn has_directory_attachment(attachments: &[MessageAttachment]) -> bool {
    attachments.iter().any(|a| matches!(a, MessageAttachment::DirectoryReference { .. }))
}

fn has_image_attachment(attachments: &[MessageAttachment]) -> bool {
    attachments.iter().any(|a| matches!(a, MessageAttachment::Image { .. }))
}

// ---------------------------------------------------------------------------
// Output preview
// ---------------------------------------------------------------------------

pub(crate) fn render_output_preview_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let total_output_lines = output.lines().count();

    let max_lines = if is_expanded {
        total_output_lines
    } else if is_error {
        TOOL_OUTPUT_PREVIEW_LINES.saturating_sub(1)
    } else {
        TOOL_OUTPUT_PREVIEW_LINES
    };

    let fg = if is_error { palette.error } else { palette.text };
    let default_style = Style::default().fg(fg);
    let wrap_width = body_width.saturating_sub(2);

    for line_text in output.lines().take(max_lines) {
        let expanded = expand_tabs(line_text, 4);
        let styled_lines = ansi_to_styled_line(&expanded, default_style);
        for sl in &styled_lines {
            let wrapped = word_wrap_line(sl, WrapOptions::new(wrap_width).break_words(true));
            for wl in wrapped.iter() {
                let spans: Vec<Span<'static>> = wl.spans.iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style))
                    .collect();
                lines.push(Line::from(spans));
            }
        }
    }

    if is_expanded {
        if total_output_lines > 0 {
            lines.push(Line::from(vec![Span::styled(
                "▲ Click to collapse",
                Style::default().fg(palette.muted),
            )]));
        }
    } else if total_output_lines > max_lines {
        lines.push(Line::from(vec![Span::styled(
            format!("  ▼ {} more line(s) — Click to expand", total_output_lines - max_lines),
            Style::default().fg(palette.muted),
        )]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled("(no output)", Style::default().fg(palette.muted))));
    }

    lines
}

pub(crate) fn tool_output_is_truncated(output: &str) -> bool {
    output.contains("output truncated:")
        || output.contains("... (truncated)")
        || output.contains("(Output capped at")
        || output.contains("[truncated]")
}

// ---------------------------------------------------------------------------
// Tool output error detection
// ---------------------------------------------------------------------------

fn tool_output_is_error(output: &str) -> bool {
    let first_line = output.lines().next().unwrap_or("").trim_start();
    first_line.starts_with("Tool failed:")
        || first_line.starts_with("Tool '")
        || first_line.starts_with("Request failed:")
        || first_line.starts_with("Error:")
        || first_line.starts_with("failed to read")
        || first_line.contains("Cannot read binary file")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
}

// ---------------------------------------------------------------------------
// Bash exit code
// ---------------------------------------------------------------------------

fn parse_bash_exit_code(output: &str) -> (Option<i32>, &str) {
    // Try [exit N] format (new tool output)
    if let Some(stripped) = output.strip_prefix("[exit ") {
        if let Some(end_idx) = stripped.find(']') {
            let code_str = &stripped[..end_idx];
            if let Ok(code) = code_str.parse::<i32>() {
                let remaining = &stripped[end_idx + 1..];
                let remaining = remaining.strip_prefix('\n').unwrap_or(remaining);
                return (Some(code), remaining);
            }
        }
    }
    // Fallback: "Exit code: N" format (legacy)
    if let Some(pos) = output.rfind("Exit code: ") {
        let rest = &output[pos + "Exit code: ".len()..];
        let code_str = rest.split_whitespace().next().unwrap_or("");
        if let Ok(code) = code_str.parse::<i32>() {
            let body = &output[..pos].trim_end();
            return (Some(code), body);
        }
    }
    (None, output)
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn summarize_tool_call(tool_call: &ToolCall, max_width: usize) -> String {
    let args = summarize_tool_arguments(tool_call, max_width);
    if args.len() > max_width {
        format!("{}...", truncate_utf8(&args, max_width.saturating_sub(3)))
    } else {
        args
    }
}

fn summarize_tool_arguments(tool_call: &ToolCall, max_width: usize) -> String {
    match serde_json::from_str::<serde_json::Value>(&tool_call.arguments) {
        Ok(serde_json::Value::Object(map)) => {
            let parts: Vec<String> = map.iter()
                .filter_map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => {
                            if s.len() > 40 {
                                format!("\"{}...\"", truncate_utf8(s, 37))
                            } else {
                                format!("\"{}\"", s)
                            }
                        }
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => "...".to_string(),
                    };
                    Some(format!("{}={}", k, val_str))
                })
                .collect();
            let joined = parts.join(", ");
            if joined.len() > max_width {
                format!("{}...", truncate_utf8(&joined, max_width.saturating_sub(3)))
            } else {
                joined
            }
        }
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn count_lines_in_partial_json(args: &str, field: &str) -> usize {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(args) {
        if let Some(content) = val.get(field).and_then(|v| v.as_str()) {
            return content.lines().count().max(1);
        }
    }
    0
}

fn count_patch_changes(args: &str) -> (usize, usize, usize) {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(args) {
        let patch = val.get("patch_text").and_then(|v| v.as_str()).unwrap_or("");
        let file_ops = val.get("files").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(1);
        let (add, del) = count_diff_lines(patch);
        return (add, del, file_ops);
    }
    (0, 0, 0)
}

fn count_diff_lines(diff: &str) -> (usize, usize) {
    let mut add = 0;
    let mut del = 0;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            add += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            del += 1;
        }
    }
    (add, del)
}

fn tool_call_arguments_are_complete(arguments: &str) -> bool {
    if arguments.trim().is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(arguments).is_ok()
}

fn preparing_text_for_tool(canonical_name: &str) -> &'static str {
    match canonical_name {
        "bash" => "Preparing shell command...",
        "write" => "Preparing write...",
        "edit" => "Preparing edit...",
        "websearch" => "Preparing web search...",
        "webfetch" => "Preparing web fetch...",
        "task" => "Preparing subagent task...",
        "question" => "Preparing questions...",
        "todowrite" => "Preparing todo list...",
        "apply_patch" => "Preparing patch...",
        _ => "Preparing...",
    }
}

fn parse_read_content_metadata(output: &str) -> (Option<HashMap<String, String>>, &str) {
    let first_line = output.lines().next().unwrap_or("");
    let mut metadata = HashMap::new();

    let body = output.trim_start_matches(first_line).trim_start();

    if let Some(pos) = first_line.find("lines") {
        let before = &first_line[..pos].trim();
        if let Some(num_start) = before.rfind(' ') {
            let num_str = before[num_start + 1..].trim_end_matches('(').trim();
            if let Ok(n) = num_str.parse::<usize>() {
                metadata.insert("lines".to_string(), n.to_string());
            }
        }
    }

    if metadata.is_empty() {
        (None, output)
    } else {
        (Some(metadata), if body.is_empty() { output } else { body })
    }
}

/// Format a file size in human-readable form.
pub(crate) fn format_file_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

/// Simple pluralization helper.
fn pluralize(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        format!("{n} {plural}")
    }
}

/// Wrap a tool title Line at body_width, indenting continuation lines.
fn wrap_tool_title(
    title: Line<'static>,
    body_width: usize,
    subsequent_indent: &str,
) -> Vec<Line<'static>> {
    let indent = Line::from(subsequent_indent.to_string());
    let wrapped = word_wrap_line(
        &title,
        WrapOptions::new(body_width)
            .subsequent_indent(indent)
            .break_words(true),
    );
    wrapped.into_iter().map(|l| {
        Line::from(l.spans.into_iter().map(|s| Span::styled(s.content.to_string(), s.style)).collect::<Vec<_>>())
    }).collect()
}

/// Render a tool call summary line for inline display (compact).
pub(crate) fn render_tool_call_summary_line(
    tool_call: &ToolCall,
    palette: ThemePalette,
    _expandable: bool,
) -> Line<'static> {
    let name_style = Style::default().fg(palette.accent).add_modifier(Modifier::BOLD);
    let args_style = Style::default().fg(palette.muted);
    let preview = summarize_tool_arguments(tool_call, 40);

    let summary = if preview.len() > 40 {
        format!("{}...", &preview[..40])
    } else {
        preview
    };

    Line::from(vec![
        Span::styled(format!(" {} ", tool_call.name), name_style),
        Span::styled(summary, args_style),
    ])
}
