//! Tool call and tool result rendering.
//!
//! Renders tool call cards with argument previews, expand/collapse for tool
//! results, pending/waiting states during streaming, and specialised
//! formatting for read/write/edit/bash/websearch/webfetch/task tools.

use std::collections::HashMap;

use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use tidev_types::message::{Message, MessageRole, ToolCall};
use tidev_types::tools::{canonical_tool_name, TaskArgs};
use crate::theme::ThemePalette;

use crate::ansi::ansi_to_styled_line;
use crate::components::chat::render::RenderContext;
use crate::components::chat::render_cache::SelectableRegionRange;
use crate::diff_render::render_unified_diff_text;
use crate::markdown;

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

    // Line counts for write/edit/apply_patch progress
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

    // Pending or waiting tool call
    if is_pending || is_waiting_result {
        let (progress_lines, progress_regions) = render_pending_tool_call(
            tool_call, body_width, palette, canonical_name,
            write_lines, edit_old_lines, edit_new_lines,
            patch_add_lines, patch_del_lines, patch_file_ops,
        );
        return (progress_lines, progress_regions);
    }

    // Regular tool call with result → rendered card with expand/collapse
    let mut lines = Vec::new();
    let mut regions = Vec::new();

    // Title line
    let title_style = Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD);
    let expand_indicator = if is_expanded { "▼" } else { "▶" };
    lines.push(Line::from(vec![
        Span::styled(format!(" {} {} ", expand_indicator, tool_call.name), title_style),
    ]));

    // Arguments (collapsible)
    if is_expanded {
        let pretty = pretty_tool_arguments(&tool_call.arguments);
        for line_text in pretty.lines() {
            lines.push(Line::from(Span::styled(
                format!("   {}", line_text),
                Style::default().fg(palette.muted),
            )));
        }
    } else {
        // Summary line
        let summary = summarize_tool_call(tool_call, body_width);
        lines.push(Line::from(Span::styled(
            format!("   {}", summary),
            Style::default().fg(palette.muted),
        )));
    }

    // Tool result
    if let Some(result_msg) = tool_result {
        let output = &result_msg.content;
        let has_ansi = output.contains('\x1b');

        // Try diff rendering for write/edit/apply_patch tools.
        let is_diff_candidate = matches!(canonical_name, "edit" | "write" | "apply_patch");
        if is_expanded && is_diff_candidate && !has_ansi {
            // Try rendering as a unified diff first.
            if let Some((diff_lines, diff_regions)) =
                render_unified_diff_text(output, body_width.saturating_sub(2), palette, 4)
            {
                let start_line = lines.len();
                for dl in &diff_lines {
                    lines.push(dl.clone());
                }
                for mut r in diff_regions {
                    r.start_line += start_line;
                    r.end_line += start_line;
                    regions.push(r);
                }
                // Also track the entire diff as a selectable region.
                regions.push(SelectableRegionRange {
                    start_line,
                    end_line: lines.len(),
                    min_x: 2,
                    max_x: None,
                });
            } else {
                // Fallback to plain text.
                let default_style = Style::default().fg(palette.text);
                let output_lines = if has_ansi {
                    ansi_to_styled_line(output, default_style)
                } else {
                    output.lines().map(|l| Line::from(Span::styled(
                        format!("  {}", l), default_style,
                    ))).collect()
                };
                let start_line = lines.len();
                for ol in &output_lines {
                    lines.push(ol.clone());
                }
                regions.push(SelectableRegionRange {
                    start_line,
                    end_line: lines.len(),
                    min_x: 2,
                    max_x: None,
                });
            }
        } else if is_expanded {
            // Full output (with ANSI if present) for non-diff-candidate tools.
            let default_style = Style::default().fg(palette.text);
            let output_lines = if has_ansi {
                ansi_to_styled_line(output, default_style)
            } else {
                output.lines().map(|l| Line::from(Span::styled(
                    format!("  {}", l), default_style,
                ))).collect()
            };
            let start_line = lines.len();
            for ol in &output_lines {
                lines.push(ol.clone());
            }
            regions.push(SelectableRegionRange {
                start_line,
                end_line: lines.len(),
                min_x: 2,
                max_x: None,
            });
        } else {
            // Preview (truncated, ANSI-styled)
            let preview_lines = render_output_preview_lines(output, body_width, palette);
            for pl in &preview_lines {
                lines.push(pl.clone());
            }
        }

        // Bash exit code
        if canonical_name == "bash" {
            let (exit_code, _) = parse_bash_exit_code(output);
            if let Some(code) = exit_code {
                let color = if code == 0 { palette.muted } else { palette.error };
                lines.push(Line::from(Span::styled(
                    format!("   Exit code: {}", code),
                    Style::default().fg(color),
                )));
            }
        }
    }

    (lines, regions)
}

// ---------------------------------------------------------------------------
// Pending/waiting tool call rendering
// ---------------------------------------------------------------------------

fn render_pending_tool_call(
    tool_call: &ToolCall,
    body_width: usize,
    palette: ThemePalette,
    canonical_name: &str,
    write_lines: usize,
    edit_old_lines: usize,
    edit_new_lines: usize,
    patch_add_lines: usize,
    patch_del_lines: usize,
    patch_file_ops: usize,
) -> (Vec<Line<'static>>, Vec<SelectableRegionRange>) {
    let title_style = Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    let preparing_text = preparing_text_for_tool(canonical_name);

    lines.push(Line::from(vec![
        Span::styled(format!(" ▶ {} ", tool_call.name), title_style),
        Span::styled(preparing_text, Style::default().fg(palette.muted)),
    ]));

    // Show a spinner line for ongoing operations
    lines.push(Line::from(Span::styled(
        format!("   {}...", match canonical_name {
            "write" => format!("Writing {} lines", write_lines),
            "edit" => format!("Editing {}→{} lines", edit_old_lines, edit_new_lines),
            "apply_patch" => format!("Patching ({} files, +{} -{})", patch_file_ops, patch_add_lines, patch_del_lines),
            "read" => "Reading...".to_string(),
            "glob" => "Searching...".to_string(),
            "grep" => "Searching...".to_string(),
            "bash" => "Running...".to_string(),
            "task" => "Agent working...".to_string(),
            _ => "Working...".to_string(),
        }),
        Style::default().fg(palette.muted),
    )));

    (lines, Vec::new())
}

// ---------------------------------------------------------------------------
// Summary rendering for read/glob/grep
// ---------------------------------------------------------------------------

fn render_tool_call_summary_line_inner(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    body_width: usize,
    palette: ThemePalette,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let title_style = Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD);
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);
    let mut lines = Vec::new();

    let mut spans = vec![
        Span::styled(format!(" ▶ {} ", tool_call.name), title_style),
    ];

    // Arguments summary
    let summary = summarize_tool_arguments(tool_call, body_width);
    if !summary.is_empty() {
        spans.push(Span::styled(summary, Style::default().fg(palette.muted)));
    }

    lines.push(Line::from(spans));

    // Tool result size/completion info
    if let Some(result_msg) = tool_result {
        let output = &result_msg.content;
        let mut info_parts = Vec::new();

        if canonical_name == "read" {
            let (metadata, _) = parse_read_content_metadata(output);
            if let Some(ref m) = metadata {
                if let Some(lines_count) = m.get("lines") {
                    info_parts.push(format!("{} lines", lines_count));
                }
                if let Some(size) = m.get("size") {
                    info_parts.push(format!("{}", size));
                }
            }
        }

        if canonical_name == "grep" || canonical_name == "glob" {
            let line_count = output.lines().count();
            if line_count > 0 {
                info_parts.push(format!("{} results", line_count));
            }
        }

        if !info_parts.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("   {}", info_parts.join(" · ")),
                Style::default().fg(palette.muted),
            )));
        }
    }

    lines
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
    let expand = if is_expanded { "▼" } else { "▶" };

    lines.push(Line::from(vec![
        Span::styled(format!(" {} task ", expand), Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD)),
        Span::styled(subagent_type.to_string(), Style::default().fg(palette.muted)),
    ]));
    lines.push(Line::from(Span::styled(
        format!("   {}", description),
        Style::default().fg(palette.text),
    )));

    if is_expanded {
        let preview = render_output_preview_lines(output, body_width, palette);
        lines.extend(preview);
    }

    lines
}

// ---------------------------------------------------------------------------
// Output preview
// ---------------------------------------------------------------------------

pub(crate) fn render_output_preview_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let has_ansi = output.contains('\x1b');
    let all_lines: Vec<Line<'static>> = if has_ansi {
        ansi_to_styled_line(output, Style::default().fg(palette.text))
    } else {
        output.lines().map(|l| Line::from(Span::styled(
            l.to_string(),
            Style::default().fg(palette.text),
        ))).collect()
    };

    let preview_count = TOOL_OUTPUT_PREVIEW_LINES.min(all_lines.len());
    let mut result = Vec::new();

    for line in all_lines.iter().take(preview_count) {
        // Truncate long lines
        let text_len: usize = line.spans.iter().map(|s| s.content.len()).sum();
        if text_len > body_width {
            // Simple truncation: take first body_width chars
            let mut truncated = Vec::new();
            let mut remaining = body_width;
            for span in &line.spans {
                if remaining == 0 {
                    truncated.push(Span::raw("..."));
                    break;
                }
                if span.content.len() <= remaining {
                    truncated.push(span.clone());
                    remaining -= span.content.len();
                } else {
                    let clipped: String = span.content.chars().take(remaining).collect();
                    truncated.push(Span::styled(clipped, span.style));
                    remaining = 0;
                }
            }
            result.push(Line::from(truncated));
        } else {
            result.push(line.clone());
        }
    }

    if all_lines.len() > preview_count {
        result.push(Line::from(Span::styled(
            format!("   ... ({} more lines)", all_lines.len() - preview_count),
            Style::default().fg(palette.muted),
        )));
    }

    result
}

pub(crate) fn tool_output_is_truncated(output: &str) -> bool {
    output.contains("output truncated:")
}

// ---------------------------------------------------------------------------
// Bash exit code
// ---------------------------------------------------------------------------

fn parse_bash_exit_code(output: &str) -> (Option<i32>, &str) {
    // Look for "Exit code: N" at the end of the output
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

fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}

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
    // Rough estimate: count files in patch_text or parse from args
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
    // Check if JSON has no null/undefined values and ends properly
    if arguments.trim().is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(arguments).is_ok()
}

fn preparing_text_for_tool(canonical_name: &str) -> &'static str {
    match canonical_name {
        "write" => "Preparing to write...",
        "edit" => "Preparing to edit...",
        "apply_patch" => "Preparing to apply patch...",
        "read" => "Preparing to read...",
        "glob" => "Preparing to search...",
        "grep" => "Preparing to search...",
        "bash" => "Preparing to run...",
        "task" => "Preparing to delegate...",
        "question" => "Preparing to ask...",
        _ => "Preparing...",
    }
}

fn parse_read_content_metadata(output: &str) -> (Option<HashMap<String, String>>, &str) {
    // Try to find metadata in first line like: "File has 42 lines (1.2 KB)"
    let first_line = output.lines().next().unwrap_or("");
    let mut metadata = HashMap::new();

    // Extract file path if present
    let body = output.trim_start_matches(first_line).trim_start();

    // Check for lines info: "X lines"
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
