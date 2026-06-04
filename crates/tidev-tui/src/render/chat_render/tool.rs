use crate::markdown::{WrapOptions, render_markdown_text_with_width_and_cwd, word_wrap_line};
use crate::theme::ThemePalette;
use tidev_engine::{
    tooling::builtin::utils::display_workspace_relative,
    tooling::{TodoItem, canonical_tool_name},
};
use tidev_session::session::{Message, ToolCall};

use crate::core::state::SelectableRegionRange;
use ratatui::{
    prelude::{Color, Modifier, Style},
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
use super::{RenderContext, TOOL_OUTPUT_PREVIEW_LINES};
use crate::diff_render::render_unified_diff_text;
use crate::render::render::line_with_style;

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
        && !matches!(canonical_name, "read" | "glob" | "grep")
        && !tool_call_arguments_are_complete(&tool_call.arguments);

    // Precompute line counts for write/edit/patch tools (used by both
    // is_waiting_result and the progress render block below).
    let write_lines = if matches!(canonical_name, "write") && tool_result.is_none() {
        count_lines_in_partial_json(&tool_call.arguments, "content")
    } else {
        0
    };
    let (edit_old_lines, edit_new_lines) =
        if matches!(canonical_name, "edit") && tool_result.is_none() {
            (
                count_lines_in_partial_json(&tool_call.arguments, "old_text"),
                count_lines_in_partial_json(&tool_call.arguments, "new_text"),
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

    // For write/edit/apply_patch, also show progress when arguments are
    // complete but the tool hasn't executed yet (covers rapid chunks that
    // skip is_pending).
    // Requires is_streaming so the spinner stops when streaming ends—otherwise
    // a rejected/abandoned tool call would keep the spinner spinning forever.
    let is_waiting_result = tool_result.is_none()
        && is_streaming
        && matches!(canonical_name, "write" | "edit" | "apply_patch")
        && match canonical_name {
            "write" => write_lines > 0,
            "edit" => edit_old_lines > 0 || edit_new_lines > 0,
            "apply_patch" => patch_file_ops > 0,
            _ => false,
        };

    if matches!(canonical_name, "grep" | "glob" | "read" | "skill") {
        return (
            render_tool_call_summary_line(tool_call, tool_result, body_width, palette, ctx),
            vec![],
        );
    }

    // For completed task tools, render a unified subagent result card
    // with the description and subagent_type from the tool call arguments.
    if canonical_name == "task"
        && tool_result.is_some()
        && let Ok(task_args) =
            serde_json::from_str::<tidev_engine::tooling::TaskArgs>(&tool_call.arguments)
    {
        let output = tool_output_from_message(tool_result.unwrap(), ctx);
        let is_expanded = ctx.expanded_tool_results.contains(&tool_result.unwrap().id);
        let lines = render_subagent_task_preview(
            output,
            body_width,
            palette,
            is_expanded,
            &task_args.description,
            &task_args.subagent_type,
        );
        return (lines, vec![]);
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
            rtk_rewritten,
            ctx.workspace_root,
        );
        lines.extend(call_lines);
    }

    // Show live progress during streaming or waiting for write/edit
    if (is_pending && has_live_progress) || is_waiting_result {
        let progress_text = match canonical_name {
            "write" if write_lines > 0 => format!("Writing {} lines...", write_lines),
            "edit" if edit_old_lines > 0 && edit_new_lines > 0 => {
                format!(
                    "Replacing {} lines with {} lines...",
                    edit_old_lines, edit_new_lines
                )
            }
            "edit" if edit_old_lines > 0 => {
                format!("Replacing {} lines with 0 lines...", edit_old_lines)
            }
            "edit" if edit_new_lines > 0 => {
                format!("Replacing 0 lines with {} lines...", edit_new_lines)
            }
            "apply_patch" if patch_file_ops > 0 => {
                let mut parts = vec![];
                if patch_add_lines > 0 {
                    parts.push(format!("+{}", patch_add_lines));
                }
                if patch_del_lines > 0 {
                    parts.push(format!("-{}", patch_del_lines));
                }
                let change_summary = if parts.is_empty() {
                    String::new()
                } else {
                    format!(" ({} lines)", parts.join(" "))
                };
                format!(
                    "Applying patch to {}{}...",
                    pluralize(patch_file_ops, "file", "files"),
                    change_summary,
                )
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

/// Count lines of content in a partial JSON field value from streaming
/// tool call arguments. Returns the number of lines counted so far
/// (newlines + 1), or 0 if the field hasn't been streamed in yet.
///
/// This is designed to work on incomplete JSON as it arrives chunk by chunk
/// during LLM streaming. It searches for `"field":` (with optional whitespace
/// after the colon) and reads the string value, counting both JSON `\n` escape
/// sequences and literal newline characters. Handles escaped quotes correctly.
fn count_lines_in_partial_json(args: &str, field: &str) -> usize {
    // Match "fieldname":  (with optional whitespace after colon)
    let key = format!("\"{}\":", field);
    if let Some(start) = args.find(&key) {
        let after_colon = &args[start + key.len()..];
        // Skip whitespace after colon
        let value_start = after_colon.trim_start();
        // Expect opening quote of the string value
        if !value_start.starts_with('"') {
            return 0;
        }
        let rest = &value_start[1..]; // skip opening quote

        // Single pass: find closing quote while counting newlines
        let mut i = 0usize;
        let mut newlines = 0usize;
        let bytes = rest.as_bytes();
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                // JSON escape: check for \n, then skip 2 bytes
                if i + 1 < bytes.len() && bytes[i + 1] == b'n' {
                    newlines += 1;
                }
                i += 2;
            } else if bytes[i] == b'"' {
                break; // unescaped quote = end of value
            } else {
                if bytes[i] == b'\n' {
                    newlines += 1;
                }
                i += 1;
            }
        }
        if i == 0 {
            return 0;
        }
        newlines + 1
    } else {
        0
    }
}

/// Count patch changes (additions, deletions, file operations) from a partial
/// `patch_text` JSON field. Returns `(additions, deletions, file_ops)`.
///
/// This works on incomplete JSON during LLM streaming — it finds the string
/// value for `"patch_text":` and counts `+` lines, `-` lines, and `***` file
/// operation markers inside it.
fn count_patch_changes(args: &str) -> (usize, usize, usize) {
    let key = "\"patch_text\":";
    let start = match args.find(key) {
        Some(s) => s,
        None => return (0, 0, 0),
    };
    let after_colon = &args[start + key.len()..];
    let value_start = after_colon.trim_start();
    if !value_start.starts_with('"') {
        return (0, 0, 0);
    }
    let rest = &value_start[1..]; // skip opening quote

    let mut i = 0usize;
    let mut adds = 0usize;
    let mut dels = 0usize;
    let mut ops = 0usize;
    let bytes = rest.as_bytes();
    let mut line_start = true;
    let mut was_cr = false; // track \r for \r\n sequences

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // JSON escape: check for \n (which represents a literal newline
            // in JSON), then process the decoded line.
            if i + 1 < bytes.len() && bytes[i + 1] == b'n' {
                line_start = true;
            }
            // Skip other escapes like \\, \", \t, etc.
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            // Unescaped quote = end of value (or nested JSON — skip)
            break;
        }
        if bytes[i] == b'\r' {
            was_cr = true;
            line_start = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'\n' {
            was_cr = false;
            line_start = true;
            i += 1;
            continue;
        }
        if line_start && !was_cr {
            match bytes[i] {
                b'+' => adds += 1,
                b'-' => dels += 1,
                b'*'
                    // Check if this starts a *** marker like *** Update File:
                    if bytes[i..].starts_with(b"*** ") => {
                        ops += 1;
                    }
                _ => {}
            }
        }
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            // handled above
        } else {
            line_start = false;
        }
        was_cr = false;
        i += 1;
    }

    (adds, dels, ops)
}

/// Simple pluralization helper.
fn pluralize(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        format!("{n} {plural}")
    }
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
            let path = get_field("file_path").unwrap_or("file");
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
        "grep" | "glob" => {
            if tool_output_is_error(output) {
                let count = if output.is_empty() {
                    0
                } else {
                    output.lines().count()
                };
                format!(" → failed ({} lines)", count)
            } else {
                let count = output
                    .lines()
                    .next()
                    .and_then(|first_line| {
                        if first_line.starts_with("No files found") {
                            Some(0usize)
                        } else if let Some(rest) = first_line.strip_prefix("Found ") {
                            // "Found 42 matches" or "Found 10 files"
                            rest.split(' ').next().and_then(|n| n.parse().ok())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        // Fallback: count non-empty lines
                        output.lines().count()
                    });
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
        "memory" => {
            if output.starts_with("Memory saved:") || output.starts_with("Memory updated:") {
                if output.starts_with("Memory saved:") {
                    " → saved".to_string()
                } else {
                    " → updated".to_string()
                }
            } else if output.starts_with("Memory ") && output.ends_with(" deleted.") {
                " → deleted".to_string()
            } else if output.starts_with("Found ") {
                // "Found N memories:"
                if let Some(count) = output.split_whitespace().nth(1) {
                    format!(" → {} memories", count)
                } else {
                    String::new()
                }
            } else if let Some(rest) = output.strip_prefix("Workspace memories (") {
                // "Workspace memories (N active):"
                if let Some(count) = rest.split_whitespace().next() {
                    format!(" → {} memories", count)
                } else {
                    String::new()
                }
            } else if output.starts_with("# [") {
                // Read output: count content lines after metadata
                let content_lines: Vec<&str> = output
                    .lines()
                    .skip_while(|l| l.starts_with('#') || l.starts_with("**"))
                    .filter(|l| !l.is_empty())
                    .collect();
                format!(" → {} lines", content_lines.len())
            } else {
                String::new()
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

/// Returns a localized preparing text for the given canonical tool name,
/// shown as a spinner status while tool arguments are still streaming.
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
        "memory" => "Preparing memory operation...",
        "apply_patch" => "Preparing patch...",
        _ => "Preparing...",
    }
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

    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or("");

    match canonical_name {
        "bash" => {
            let command = get_field("command").unwrap_or("");
            let desc = get_field("description");

            // Title: Bash: [description]  ✓/✗ N  [rtk]
            let display = desc.unwrap_or(command);
            let mut title_spans = vec![
                Span::styled("Bash: ", Style::default().fg(palette.muted)),
                Span::styled(
                    display.to_string(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ];

            // Add [rtk] marker if command was rewritten
            if rtk_rewritten {
                title_spans.push(Span::styled(
                    " [rtk]",
                    Style::default().fg(palette.accent_soft),
                ));
            }

            // Add exit code status
            if let Some(code) = exit_code {
                if code == 0 {
                    title_spans.push(Span::styled("  ✓", Style::default().fg(palette.success)));
                } else {
                    title_spans.push(Span::styled(
                        format!("  ✗ {}", code),
                        Style::default().fg(palette.error),
                    ));
                }
            }
            lines.push(Line::from(title_spans));

            // Command line (below title)
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
            let path = get_field("file_path").unwrap_or("file");
            let rel_path = display_workspace_relative(workspace_root, Path::new(path));
            lines.push(Line::from(vec![
                Span::styled("Write ", Style::default().fg(palette.muted)),
                Span::styled(
                    rel_path,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        "websearch" => {
            let query = get_field("query").unwrap_or("");
            let mut title_spans = vec![
                Span::styled("Search web for ", Style::default().fg(palette.accent_soft)),
                Span::styled(
                    query.to_string(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            let mut suffix_parts: Vec<String> = Vec::new();
            if let Some(num) = get_field("num_results") {
                suffix_parts.push(format!("max: {}", num));
            }
            if let Some(st) = get_field("search_type") {
                suffix_parts.push(st.to_string());
            }
            if !suffix_parts.is_empty() {
                title_spans.push(Span::styled(
                    format!("  ({})", suffix_parts.join(", ")),
                    Style::default().fg(palette.muted),
                ));
            }
            lines.push(Line::from(title_spans));
        }
        "webfetch" => {
            let url = get_field("url").unwrap_or("");
            let mut title_spans = vec![
                Span::styled("Fetch web page from ", Style::default().fg(palette.accent)),
                Span::styled(
                    url.to_string(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            let mut suffix_parts: Vec<String> = Vec::new();
            if let Some(fmt) = get_field("format") {
                suffix_parts.push(format!("format: {}", fmt));
            }
            if let Some(to) = get_field("timeout") {
                suffix_parts.push(format!("{}s", to));
            }
            if !suffix_parts.is_empty() {
                title_spans.push(Span::styled(
                    format!("  ({})", suffix_parts.join(", ")),
                    Style::default().fg(palette.muted),
                ));
            }
            lines.push(Line::from(title_spans));
        }
        _ => {
            let summary = summarize_tool_call(
                &tool_call.name,
                &tool_call.arguments,
                body_width,
                workspace_root,
            );
            lines.push(Line::from(vec![Span::styled(
                summary,
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )]));
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
        && let Some((diff_lines, regions)) =
            render_unified_diff_text(diff, body_width.saturating_sub(2), palette)
    {
        return (diff_lines, None, regions);
    }

    // Fallback: try to render diff from output (may be truncated)
    if !is_error
        && matches!(canonical_name, "edit" | "write" | "apply_patch")
        && let Some((diff_lines, regions)) =
            render_unified_diff_text(effective_output, body_width.saturating_sub(2), palette)
    {
        return (diff_lines, None, regions);
    }

    if canonical_name == "todowrite" && !is_error {
        #[derive(serde::Deserialize)]
        struct RawTodo {
            content: String,
            status: Option<String>,
        }

        let raw_todos = if let Ok(todos) = serde_json::from_str::<Vec<RawTodo>>(effective_output) {
            Some(todos)
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(effective_output) {
            value
                .get("newTodos")
                .or_else(|| value.get("todos"))
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
                })
                .collect();
            return (
                render_todos_checkbox_list(&todos, body_width, palette),
                None,
                vec![],
            );
        }
    }

    // Web search results: render as styled markdown with header
    if canonical_name == "websearch" {
        let is_expanded = ctx.expanded_tool_results.contains(&message.id);
        return (
            render_websearch_result_lines(
                effective_output,
                body_width,
                palette,
                is_expanded,
                is_error,
                Some(message.id),
                ctx.expanded_tool_results,
            ),
            None,
            vec![],
        );
    }

    // Web fetch results: render page content as styled markdown with header
    if canonical_name == "webfetch" {
        let is_expanded = ctx.expanded_tool_results.contains(&message.id);
        return (
            render_webfetch_result_lines(
                effective_output,
                body_width,
                palette,
                is_expanded,
                is_error,
                Some(message.id),
                ctx.expanded_tool_results,
            ),
            None,
            vec![],
        );
    }

    // Memory tool results: structured cards for search/list/read
    if canonical_name == "memory" && !is_error {
        let is_expanded = ctx.expanded_tool_results.contains(&message.id);
        return (
            render_memory_result_lines(
                effective_output,
                body_width,
                palette,
                is_expanded,
                is_error,
                Some(message.id),
                ctx.expanded_tool_results,
            ),
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
/// `description` and `subagent_type` are parsed from the original task tool call
/// arguments and used for the unified card header.
pub(super) fn render_subagent_task_preview(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    description: &str,
    subagent_type: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(line_with_style("(empty result)", palette.muted));
        return lines;
    }

    // Unified header: [@type] subagent: description (word-wrapped)
    let header_line = Line::from(vec![
        Span::styled(
            format!("@{}", subagent_type),
            Style::default().fg(palette.accent_soft),
        ),
        Span::styled(" subagent: ", Style::default().fg(palette.muted)),
        Span::styled(
            description.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    lines.extend(
        word_wrap_line(&header_line, WrapOptions::new(body_width).break_words(true))
            .into_iter()
            .map(|l| {
                Line::from(
                    l.spans
                        .into_iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style))
                        .collect::<Vec<_>>(),
                )
            }),
    );
    lines.push(Line::from(""));

    // Render the output as markdown
    let rendered =
        render_markdown_text_with_width_and_cwd(output, Some(body_width.saturating_sub(2)), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        // Show all lines when expanded
        lines.extend(md_lines);
    } else {
        // Preview mode: show first few lines
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();

        if line_count <= max_preview {
            lines.extend(md_lines);
        } else {
            lines.extend(md_lines.into_iter().take(max_preview));
            lines.push(Line::from(vec![Span::styled(
                format!("  {} more line(s)", line_count - max_preview),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    // Bottom padding
    lines.push(Line::from(""));

    lines
}

/// Renders websearch results: styled markdown with a "Search Results" header.
pub(super) fn render_websearch_result_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
    message_id: Option<Uuid>,
    expanded_tool_results: &HashSet<Uuid>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(line_with_style("(no results)", palette.muted));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(
            output,
            body_width,
            true,
            message_id,
            expanded_tool_results,
            palette,
        );
    }

    // Title header
    lines.push(Line::from(vec![Span::styled(
        "Search Results",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    // Render the output as markdown
    let rendered =
        render_markdown_text_with_width_and_cwd(output, Some(body_width.saturating_sub(2)), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        // Show all lines when expanded
        lines.extend(md_lines);
        lines.push(Line::from(vec![Span::styled(
            "▲ Click to collapse",
            Style::default().fg(palette.muted),
        )]));
    } else {
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
    }

    lines
}

/// Renders webfetch results: styled markdown with a "Page Content" header.
pub(super) fn render_webfetch_result_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
    message_id: Option<Uuid>,
    expanded_tool_results: &HashSet<Uuid>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if output.trim().is_empty() {
        lines.push(line_with_style("(empty page)", palette.muted));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(
            output,
            body_width,
            true,
            message_id,
            expanded_tool_results,
            palette,
        );
    }

    // Title header
    lines.push(Line::from(vec![Span::styled(
        "Page Content",
        Style::default().fg(palette.accent_soft),
    )]));
    lines.push(Line::from(""));

    // Render the content as markdown
    let rendered =
        render_markdown_text_with_width_and_cwd(output, Some(body_width.saturating_sub(2)), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;

    if is_expanded {
        // Show all lines when expanded
        lines.extend(md_lines);
        lines.push(Line::from(vec![Span::styled(
            "▲ Click to collapse",
            Style::default().fg(palette.muted),
        )]));
    } else {
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
    }

    lines
}

/// Renders memory tool results with structured cards.
pub(super) fn render_memory_result_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    is_error: bool,
    message_id: Option<Uuid>,
    expanded_tool_results: &HashSet<Uuid>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let trimmed = output.trim();

    if trimmed.is_empty() {
        lines.push(line_with_style("(no results)", palette.muted));
        return lines;
    }

    if is_error {
        return render_output_preview_lines(
            output,
            body_width,
            true,
            message_id,
            expanded_tool_results,
            palette,
        );
    }

    // Store / Update / Delete: simple confirmation
    if trimmed.starts_with("Memory saved:")
        || trimmed.starts_with("Memory updated:")
        || (trimmed.starts_with("Memory ") && trimmed.ends_with(" deleted."))
    {
        let style =
            if trimmed.starts_with("Memory saved:") || trimmed.starts_with("Memory updated:") {
                Style::default().fg(palette.success)
            } else {
                Style::default().fg(palette.muted)
            };
        // Show only the first line as the confirmation; hints/footnotes follow below
        let first_line = trimmed.lines().next().unwrap_or(trimmed);
        lines.push(Line::from(vec![
            Span::styled("  ✓ ", style),
            Span::styled(first_line.to_string(), style),
        ]));
        // Render any additional lines (e.g. dedup hints) in muted style as subsequent lines
        for extra in trimmed.lines().skip(1) {
            if !extra.trim().is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("    {}", extra.trim()),
                    palette.muted,
                )));
            }
        }
        return lines;
    }

    // Read: metadata header + markdown content
    if trimmed.starts_with("# [") {
        return render_memory_read_lines(
            trimmed,
            body_width,
            palette,
            is_expanded,
            message_id,
            expanded_tool_results,
        );
    }

    // Search / List: parse result lines
    let all_lines: Vec<&str> = trimmed.lines().collect();
    let is_search = trimmed.starts_with("Found ") || trimmed.starts_with("No memories found");
    let is_list =
        trimmed.starts_with("Workspace memories") || trimmed.starts_with("No memories yet");

    if is_search || is_list {
        let data_lines: Vec<&str> = all_lines
            .iter()
            .skip(1)
            .filter(|l| !l.trim().is_empty())
            .copied()
            .collect();

        if data_lines.is_empty() {
            lines.push(line_with_style(trimmed, palette.muted));
            return lines;
        }

        let max_items = if is_expanded {
            usize::MAX // show all items when expanded
        } else {
            (TOOL_OUTPUT_PREVIEW_LINES / 2).max(2)
        };
        let total = data_lines.len();
        let shown = data_lines
            .iter()
            .take(max_items)
            .copied()
            .collect::<Vec<_>>();

        for (i, line) in shown.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(""));
            }
            if is_search {
                if let Some((label, title, content)) = parse_memory_search_line(line) {
                    lines.push(render_memory_card_line(label, title, palette));
                    lines.push(Line::from(Span::styled(
                        format!("    {}", content),
                        Style::default().fg(palette.muted),
                    )));
                } else {
                    lines.push(line_with_style(line, palette.text));
                }
            } else {
                if let Some((label, title, content)) = parse_memory_list_line(line) {
                    lines.push(render_memory_card_line(label, title, palette));
                    lines.push(Line::from(Span::styled(
                        format!("    {}", content),
                        Style::default().fg(palette.muted),
                    )));
                } else {
                    lines.push(line_with_style(line, palette.text));
                }
            }
        }

        if is_expanded {
            if total > 0 {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    "▲ Click to collapse",
                    Style::default().fg(palette.muted),
                )]));
            }
        } else {
            let remaining = total.saturating_sub(shown.len());
            if remaining > 0 {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    format!("  ▼ {} more result(s) — Click to expand", remaining),
                    Style::default().fg(palette.muted),
                )]));
            }
        }

        return lines;
    }

    // Fallback: plain preview
    render_output_preview_lines(
        output,
        body_width,
        false,
        message_id,
        expanded_tool_results,
        palette,
    )
}

/// Render a single memory card title line: "  ◉ [proj] Title"
fn render_memory_card_line(label: &str, title: &str, palette: ThemePalette) -> Line<'static> {
    let badge_color = memory_type_color(label, palette);
    Line::from(vec![
        Span::styled("  ◉ ", Style::default().fg(badge_color)),
        Span::styled(
            format!("[{}] ", label),
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// Render read output: metadata header + content
fn render_memory_read_lines(
    output: &str,
    body_width: usize,
    palette: ThemePalette,
    is_expanded: bool,
    message_id: Option<Uuid>,
    expanded_tool_results: &HashSet<Uuid>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let content_width = body_width.saturating_sub(4).max(20);
    let all_lines: Vec<&str> = output.lines().collect();

    let title_line = all_lines.first().unwrap_or(&"");
    let (type_label, entry_title) = if let Some(rest) = title_line.strip_prefix("# [") {
        if let Some((label, title_rest)) = rest.split_once(']') {
            (label.trim(), title_rest.trim())
        } else {
            ("", "")
        }
    } else {
        ("", "")
    };

    if type_label.is_empty() {
        return render_output_preview_lines(
            output,
            body_width,
            false,
            message_id,
            expanded_tool_results,
            palette,
        );
    }

    let badge_color = memory_type_color(type_label, palette);

    // Card header
    lines.push(Line::from(vec![
        Span::styled(
            format!("  [{}] ", type_label),
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            entry_title.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Metadata lines
    let mut in_metadata = true;
    let mut content_start = all_lines.len();
    for (i, line) in all_lines.iter().enumerate().skip(1) {
        if in_metadata {
            if line.starts_with("**Type**:")
                || line.starts_with("**Created**:")
                || line.starts_with("**Updated**:")
                || line.starts_with("**Used**:")
                || line.starts_with("Tags:")
            {
                lines.push(Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default().fg(palette.muted),
                )));
            } else if line.trim().is_empty() {
                in_metadata = false;
                content_start = i + 1;
                lines.push(Line::from(Span::styled(
                    "  ─────────────────────────────────",
                    Style::default().fg(palette.muted),
                )));
            }
        }
    }

    if in_metadata {
        content_start = all_lines.len();
    }

    let content_text: String = all_lines[content_start..].join("\n");
    if content_text.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no content)",
            Style::default().fg(palette.muted),
        )));
        return lines;
    }

    let rendered =
        render_markdown_text_with_width_and_cwd(&content_text, Some(content_width), None);
    let md_lines: Vec<Line<'static>> = rendered.lines;
    let prefix = Span::styled("  ", Style::default());

    if is_expanded {
        // Show all lines when expanded
        for l in md_lines {
            let mut spans = vec![prefix.clone()];
            spans.extend(l.spans);
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(vec![Span::styled(
            "▲ Click to collapse",
            Style::default().fg(palette.muted),
        )]));
    } else {
        let max_preview = TOOL_OUTPUT_PREVIEW_LINES;
        let line_count = md_lines.len();
        if line_count <= max_preview {
            for l in md_lines {
                let mut spans = vec![prefix.clone()];
                spans.extend(l.spans);
                lines.push(Line::from(spans));
            }
        } else {
            for l in md_lines.into_iter().take(max_preview) {
                let mut spans = vec![prefix.clone()];
                spans.extend(l.spans);
                lines.push(Line::from(spans));
            }
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  ▼ {} more line(s) — Click to expand",
                    line_count - max_preview
                ),
                Style::default().fg(palette.muted),
            )]));
        }
    }

    lines
}

/// Get a colour for a memory type badge label ("proj", "usr", "ref", "feed").
fn memory_type_color(label: &str, palette: ThemePalette) -> Color {
    match label {
        "proj" | "project" => palette.accent,
        "usr" | "user" => palette.accent_soft,
        "ref" | "reference" => palette.warning,
        "feed" | "feedback" => palette.success,
        _ => palette.muted,
    }
}

/// Parse a search result line: `- [proj] **Title**: content preview…`
fn parse_memory_search_line(line: &str) -> Option<(&str, &str, &str)> {
    let line = line.trim();
    let line = line.strip_prefix("- [")?;
    let (label, rest) = line.split_once(']')?;
    let rest = rest.strip_prefix(" **")?;
    let (title, rest) = rest.split_once("**: ")?;
    Some((label, title.trim(), rest.trim()))
}

/// Parse a list result line: `` `uuid` [proj] Title — content preview… ``
fn parse_memory_list_line(line: &str) -> Option<(&str, &str, &str)> {
    let line = line.trim();
    let line = line.strip_prefix('`')?;
    let (_uuid, rest) = line.split_once('`')?;
    let rest = rest.trim();
    let rest = rest.strip_prefix('[')?;
    let (label, rest) = rest.split_once(']')?;
    let rest = rest.trim();
    let (title, content) = rest.split_once(" — ")?;
    Some((label, title.trim(), content.trim()))
}

pub(super) fn render_todos_checkbox_list(
    todos: &[TodoItem],
    body_width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if todos.is_empty() {
        lines.push(line_with_style("  (no items)", palette.muted));
        return lines;
    }

    for todo in todos {
        let (checkbox, style) = match todo.status.as_str() {
            "completed" => (
                "x ",
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
            _ => ("○ ", Style::default().fg(palette.text)),
        };

        let checkbox_prefix = format!("  {}", checkbox);
        let checkbox_width = UnicodeWidthStr::width(checkbox_prefix.as_str());
        let indent = " ".repeat(checkbox_width);

        let content_line = Line::from(todo.content.clone());
        let wrapped = word_wrap_line(
            &content_line,
            WrapOptions::new(body_width.saturating_sub(2))
                .initial_indent(Line::from(vec![Span::styled(checkbox_prefix, style)]))
                .subsequent_indent(Line::from(vec![Span::styled(indent, Style::default())])),
        );

        for wl in wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            // Apply the todo item style to content parts (after the prefix)
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

        // Render question with "  Q: " label merged with text (word-wrapped)
        // Uses initial_indent/subsequent_indent so word_wrap_line correctly
        // accounts for the prefix width — same approach as message content area.
        let q_line_owned = Line::from(question_text.clone());
        let q_wrapped = word_wrap_line(
            &q_line_owned,
            WrapOptions::new(body_width)
                .initial_indent(Line::from(vec![Span::styled(
                    "  Q: ",
                    Style::default()
                        .fg(palette.accent_soft)
                        .add_modifier(Modifier::BOLD),
                )]))
                .subsequent_indent(Line::from(vec![Span::styled(
                    "     ",
                    Style::default().fg(palette.text),
                )])),
        );
        for wl in q_wrapped {
            let mut owned_spans: Vec<Span<'static>> = wl
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            // Style content parts (after prefix) with text color
            for span in owned_spans.iter_mut().skip(1) {
                if span.style.fg.is_none() {
                    span.style = span.style.fg(palette.text);
                }
            }
            lines.push(Line::from(owned_spans));
        }

        // Render answer with word wrapping
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
            let mut owned_spans: Vec<Span<'static>> = wl
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect();
            // Style content parts (after prefix) with answer text styling
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
    let total_output_lines = output.lines().count();

    let max_lines = if is_expanded {
        total_output_lines
    } else if is_error {
        TOOL_OUTPUT_PREVIEW_LINES.saturating_sub(1)
    } else {
        TOOL_OUTPUT_PREVIEW_LINES
    };

    let fg = if is_error {
        palette.error
    } else {
        palette.text
    };

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
            // Preview (collapsed) mode: show each line with full wrapping
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
            format!(
                "  ▼ {} more line(s) — Click to expand",
                total_output_lines - max_lines
            ),
            Style::default().fg(palette.muted),
        )]));
    }

    if lines.is_empty() {
        lines.push(line_with_style("(no output)", palette.muted));
    }

    lines
}
