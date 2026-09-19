use std::path::Path;

use crate::theme::ThemePalette;
use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use tidev_llm::message::{Message, MessageAttachment, ToolCall};
use tidev_utils::path::display_workspace_relative;
use tidev_utils::tool_name::canonical_tool_name;
use unicode_width::UnicodeWidthStr;

use crate::components::chat::render::RenderContext;
use crate::i18n::{TextKey, UiText};
use crate::markdown::{WrapOptions, word_wrap_line};

use super::read::{
    format_file_size, format_read_result_label_with_locale, parse_line_range_from_read_output,
    parse_read_content_metadata,
};
use super::utils::{tool_output_is_error, truncate_utf8};

// ---------------------------------------------------------------------------
// Summary rendering for read/glob/grep (enhanced with action labels and rich suffixes)
// ---------------------------------------------------------------------------

pub(super) fn render_tool_call_summary_line_inner(
    tool_call: &ToolCall,
    tool_result: Option<&Message>,
    content_width: usize,
    palette: ThemePalette,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);
    let parsed = serde_json::from_str::<serde_json::Value>(&tool_call.arguments).ok();

    let string_field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(serde_json::Value::as_str)
            .map(|s| s.to_string())
    };

    let rel_path = |p: &str| display_workspace_relative(ctx.workspace_root, Path::new(p));

    let ui_text = &ctx.ui_text;
    let (action_label, target_spans) = match canonical_name {
        "grep" => {
            let pattern = string_field("pattern").unwrap_or_default();
            let path = string_field("path").unwrap_or_else(|| ".".to_string());
            let rel = rel_path(&path);
            (
                ui_text.text(TextKey::Search),
                vec![
                    Span::styled(
                        format!("\"{}\"", pattern),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} ", ui_text.text(TextKey::Within)),
                        Style::default().fg(palette.muted),
                    ),
                    Span::styled(
                        rel,
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ],
            )
        }
        "glob" => {
            let pattern = string_field("pattern").unwrap_or_else(|| "*".to_string());
            let path = string_field("path").unwrap_or_else(|| ".".to_string());
            let rel = rel_path(&path);
            (
                ui_text.text(TextKey::Find),
                vec![
                    Span::styled(
                        pattern.clone(),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} ", ui_text.text(TextKey::Within)),
                        Style::default().fg(palette.muted),
                    ),
                    Span::styled(
                        rel,
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ],
            )
        }
        "read" => {
            let path = string_field("file_path").unwrap_or_else(|| ui_text.text(TextKey::File));
            let rel = rel_path(&path);
            (
                ui_text.text(TextKey::Read),
                vec![Span::styled(
                    rel,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                )],
            )
        }
        "skill" => {
            let name = string_field("name");
            let path = normalize_skill_path(string_field("path"));
            match (name, path) {
                (Some(name), Some(path)) => (
                    format!(
                        "{} {}",
                        ui_text.text(TextKey::Read),
                        ui_text.text(TextKey::Skill)
                    ),
                    vec![Span::styled(
                        format!("{name}/{path}"),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    )],
                ),
                (Some(name), None) => (
                    format!(
                        "{} {}",
                        ui_text.text(TextKey::Loading),
                        ui_text.text(TextKey::Skill)
                    ),
                    vec![Span::styled(
                        name,
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    )],
                ),
                (None, _) => (ui_text.text(TextKey::Skills), Vec::new()),
            }
        }
        _ => {
            let summary = summarize_tool_call(tool_call, content_width);
            return vec![Line::from(vec![Span::styled(
                summary,
                Style::default().fg(palette.accent_soft),
            )])];
        }
    };

    let result_suffix = if let Some(result_msg) = tool_result {
        let output = &result_msg.content;
        compute_tool_result_suffix_with_locale(
            canonical_name,
            output,
            &result_msg.attachments,
            ui_text,
        )
    } else {
        " ...".to_string()
    };

    let mut all_spans = vec![Span::styled(
        format!("{} ", action_label),
        Style::default().fg(palette.accent_soft),
    )];
    all_spans.extend(target_spans);
    all_spans.push(Span::styled(
        result_suffix,
        Style::default().fg(palette.muted),
    ));

    let line = Line::from(all_spans);

    // Wrap the line if it exceeds content_width
    let indent_width = UnicodeWidthStr::width(action_label.as_str()) + 1;
    let indent = Line::from(" ".repeat(indent_width));
    let wrapped = word_wrap_line(
        &line,
        WrapOptions::new(content_width)
            .subsequent_indent(indent)
            .break_words(true),
    );
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

// ---------------------------------------------------------------------------
// Tool result suffix computation (for summary lines)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(super) fn compute_tool_result_suffix(
    canonical_name: &str,
    output: &str,
    attachments: &[MessageAttachment],
) -> String {
    compute_tool_result_suffix_with_locale(
        canonical_name,
        output,
        attachments,
        &UiText::from_preference("en-US"),
    )
}

fn compute_tool_result_suffix_with_locale(
    canonical_name: &str,
    output: &str,
    attachments: &[MessageAttachment],
    ui_text: &UiText,
) -> String {
    let suffix = match canonical_name {
        "grep" | "glob" => {
            if tool_output_is_error(output) {
                let count = if output.is_empty() {
                    0
                } else {
                    output.lines().count()
                };
                ui_text.text_with_value(TextKey::ToolResultFailed, "count", &count.to_string())
            } else {
                let count = output
                    .lines()
                    .next()
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
                    0 => ui_text.text(TextKey::ToolResultNoMatch),
                    1 => ui_text.text(TextKey::ToolResultOneMatch),
                    n => {
                        ui_text.text_with_value(TextKey::ToolResultMatches, "count", &n.to_string())
                    }
                }
            }
        }
        "read" => {
            if output.contains("file not found") {
                if output.contains("Did you mean") {
                    ui_text.text(TextKey::ToolResultNotFoundSuggestions)
                } else {
                    ui_text.text(TextKey::ToolResultNotFound)
                }
            } else if output.contains("escapes the workspace root")
                || output.contains(" was denied")
            {
                ui_text.text(TextKey::ToolResultBlocked)
            } else if tool_output_is_error(output) {
                ui_text.text(TextKey::ToolResultError)
            } else if has_image_attachment(attachments) {
                // Enriched image suffix: " → TYPE, SIZE"
                let image = attachments.iter().find_map(|a| {
                    if let MessageAttachment::Image {
                        mime, file_size, ..
                    } = a
                    {
                        Some((mime.as_str(), *file_size))
                    } else {
                        None
                    }
                });
                if let Some((mime, size)) = image {
                    let type_label = mime.strip_prefix("image/").unwrap_or(mime);
                    ui_text.text_with_values(
                        TextKey::ToolResultImageType,
                        &[("type", type_label), ("size", &format_file_size(size))],
                    )
                } else {
                    ui_text.text(TextKey::ToolResultImage)
                }
            } else if has_directory_attachment(attachments) {
                // Count files and subdirectories from list_dir output
                let mut files = 0u64;
                let mut dirs = 0u64;
                for line in output.lines().skip(1) {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed.ends_with('/') {
                        dirs += 1;
                    } else {
                        files += 1;
                    }
                }
                if files == 0 && dirs == 0 {
                    ui_text.text(TextKey::ToolResultEmpty)
                } else if dirs == 0 {
                    ui_text.text_with_value(TextKey::ToolResultFiles, "count", &files.to_string())
                } else if files == 0 {
                    ui_text.text_with_value(TextKey::ToolResultDirs, "count", &dirs.to_string())
                } else {
                    ui_text.text_with_values(
                        TextKey::ToolResultFilesDirs,
                        &[("files", &files.to_string()), ("dirs", &dirs.to_string())],
                    )
                }
            } else {
                let metadata = parse_read_content_metadata(output);
                let is_size_truncated = output.contains("Output capped at 50 KB");

                match metadata {
                    Some(((start, end), requested_range, total, truncated_by)) => {
                        if is_size_truncated
                            || truncated_by.as_deref() == Some("lines")
                            || (start == 1 && end == total)
                        {
                            format_read_result_label_with_locale(
                                start,
                                end,
                                total,
                                requested_range,
                                truncated_by.as_deref(),
                                is_size_truncated,
                                ui_text,
                            )
                        } else {
                            // Partial read without truncation
                            ui_text.text_with_values(
                                TextKey::ToolResultLineRange,
                                &[
                                    ("start", &start.to_string()),
                                    ("end", &end.to_string()),
                                    ("total", &total.to_string()),
                                ],
                            )
                        }
                    }
                    None => {
                        // Fallback: try old format parsing
                        let line_range = parse_line_range_from_read_output(output);
                        let truncated = tool_output_is_truncated(output);

                        match line_range {
                            Some((start, end)) => {
                                if truncated && output.contains("Output capped at 50 KB") {
                                    ui_text.text_with_values(
                                        TextKey::ToolResultLineRangeTruncated,
                                        &[
                                            ("start", &start.to_string()),
                                            ("end", &end.to_string()),
                                            ("suffix", &ui_text.text(TextKey::ToolResultTruncated)),
                                        ],
                                    )
                                } else {
                                    ui_text.text_with_values(
                                        TextKey::ToolResultLineRangeShort,
                                        &[("start", &start.to_string()), ("end", &end.to_string())],
                                    )
                                }
                            }
                            None => {
                                let total_lines = output.lines().count();
                                if total_lines == 0 {
                                    ui_text.text(TextKey::ToolResultEmpty)
                                } else if truncated {
                                    ui_text.text_with_value(
                                        TextKey::ToolResultLinesTruncated,
                                        "count",
                                        &total_lines.to_string(),
                                    )
                                } else {
                                    ui_text.text_with_value(
                                        TextKey::ToolResultLines,
                                        "count",
                                        &total_lines.to_string(),
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
        "skill" => skill_result_suffix(output, ui_text),
        _ => String::new(),
    };
    if suffix.starts_with('→') {
        format!(" {suffix}")
    } else {
        suffix
    }
}

fn normalize_skill_path(path: Option<String>) -> Option<String> {
    path.filter(|path| !path.is_empty())
}

fn skill_result_suffix(output: &str, ui_text: &UiText) -> String {
    if tool_output_is_error(output) {
        if output.contains("unknown skill '") {
            ui_text.text(TextKey::ToolResultSkillNotFound)
        } else if output.contains("file not found") {
            ui_text.text(TextKey::ToolResultFileNotFound)
        } else if output.contains("escapes the skill directory")
            || output.contains("path must be relative to the skill directory")
        {
            ui_text.text(TextKey::ToolResultBlocked)
        } else if output.contains("a path requires a skill name")
            || output.contains("path must not be empty")
        {
            ui_text.text(TextKey::ToolResultInvalidRequest)
        } else if output.contains("Cannot read binary file") {
            ui_text.text(TextKey::ToolResultBinaryFile)
        } else {
            ui_text.text(TextKey::ToolResultError)
        }
    } else if output.starts_with("Available skills") {
        // List mode: report the catalog total when the page header
        // carries it ("showing A-B of N").
        let total = output
            .split("of ")
            .nth(1)
            .and_then(|s| s.split([')', ':']).next())
            .and_then(|s| s.trim().parse::<usize>().ok());
        match total {
            Some(n) => {
                ui_text.text_with_value(TextKey::ToolResultAvailable, "count", &n.to_string())
            }
            None => ui_text.text(TextKey::ToolResultListed),
        }
    } else {
        let content_lines: Vec<_> = output
            .lines()
            .skip_while(|l| l.starts_with('#') || l.starts_with("**"))
            .filter(|l| !l.is_empty())
            .collect();
        ui_text.text_with_value(
            TextKey::ToolResultLines,
            "count",
            &content_lines.len().to_string(),
        )
    }
}

fn has_directory_attachment(attachments: &[MessageAttachment]) -> bool {
    attachments
        .iter()
        .any(|a| matches!(a, MessageAttachment::DirectoryReference { .. }))
}

fn has_image_attachment(attachments: &[MessageAttachment]) -> bool {
    attachments
        .iter()
        .any(|a| matches!(a, MessageAttachment::Image { .. }))
}

pub(super) fn summarize_tool_call(tool_call: &ToolCall, max_width: usize) -> String {
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
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| {
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
                    format!("{}={}", k, val_str)
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
        Err(_) => {
            // Fallback for incomplete/partial JSON during streaming: show raw arguments
            crate::utils::pretty_tool_arguments(&tool_call.arguments)
        }
    }
}

pub(crate) fn tool_output_is_truncated(output: &str) -> bool {
    output.contains("output truncated:")
        || output.contains("... (truncated)")
        || output.contains("(Output capped at")
        || output.contains("[truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_result_suffix_distinguishes_failure_states() {
        assert_eq!(
            compute_tool_result_suffix(
                "read",
                "Error: failed to read src/missing.rs: file not found",
                &[],
            ),
            " → not found"
        );
        assert_eq!(
            compute_tool_result_suffix(
                "read",
                "Error: failed to read src/mian.rs: file not found. Did you mean one of these?\nsrc/main.rs",
                &[],
            ),
            " → not found (with suggestions)"
        );
        assert_eq!(
            compute_tool_result_suffix("read", "Error: Path '/tmp/file' was denied.", &[]),
            " → blocked by policy"
        );
        assert_eq!(
            compute_tool_result_suffix(
                "read",
                "Error: failed to read src/file.rs: permission denied",
                &[],
            ),
            " → error"
        );
    }

    #[test]
    fn read_result_suffix_handles_legacy_denial_without_error_prefix() {
        assert_eq!(
            compute_tool_result_suffix("read", "Path '/tmp/file' was denied.", &[]),
            " → blocked by policy"
        );
    }

    #[test]
    fn skill_result_suffix_distinguishes_failure_states() {
        assert_eq!(
            compute_tool_result_suffix("skill", "Error: unknown skill 'missing'", &[]),
            " → skill not found"
        );
        assert_eq!(
            compute_tool_result_suffix(
                "skill",
                "Error: failed to read docs/missing.md in skill 'demo': file not found",
                &[],
            ),
            " → file not found"
        );
        assert_eq!(
            compute_tool_result_suffix(
                "skill",
                "Error: path '../secret' escapes the skill directory",
                &[],
            ),
            " → blocked by policy"
        );
        assert_eq!(
            compute_tool_result_suffix(
                "skill",
                "Error: skill: a path requires a skill name to read from",
                &[],
            ),
            " → invalid request"
        );
        assert_eq!(
            compute_tool_result_suffix("skill", "Error: unexpected failure", &[]),
            " → error"
        );
    }

    #[test]
    fn empty_skill_path_is_not_rendered_as_a_child_path() {
        assert_eq!(normalize_skill_path(Some(String::new())), None);
        assert_eq!(
            normalize_skill_path(Some("docs/guide.md".to_string())),
            Some("docs/guide.md".to_string())
        );
    }
}
