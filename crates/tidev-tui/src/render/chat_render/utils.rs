use crate::render::render::shorten_single_line;
use tidev_engine::{

    markdown_render::render_markdown_text_with_width_and_cwd, theme::ThemePalette,
    tooling::builtin::utils::display_workspace_relative, tooling::canonical_tool_name,
};
use ratatui::{
    prelude::{Modifier, Style},
    text::{Line, Span},
};
use std::path::Path;

pub(super) fn render_reasoning_markdown_lines(
    reasoning: &str,
    body_width: usize,
    cwd: Option<&std::path::Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Use 0.5 ratio for a balanced dimmed appearance that works consistently across terminals
    // This avoids the inconsistent behavior of Modifier::DIM which varies significantly
    // between Windows Terminal (strong dimming) and Ghostty (weak/no dimming)
    let dimmed_color = tidev_engine::theme::mix_colors(palette.muted, palette.background, 0.5);
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
                    .fg(tidev_engine::theme::mix_colors(fg, palette.background, 0.4));
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
        spans.extend(
            line.spans
                .into_iter()
                .map(|mut span| {
                    // Mix the foreground color with background for all spans (including highlighted ones)
                    // Use 0.4 ratio for a slightly more visible dimmed text
                    if let Some(fg) = span.style.fg {
                        span.style =
                            span.style
                                .fg(tidev_engine::theme::mix_colors(fg, palette.background, 0.4));
                    } else {
                        span.style = span.style.patch(body_style);
                    }
                    span
                })
                .collect::<Vec<_>>(),
        );
        lines.push(Line::from(spans));
    }

    lines
}

pub(super) fn tool_output_is_error(output: &str) -> bool {
    let first_line = output.lines().next().unwrap_or("").trim_start();

    first_line.starts_with("Tool failed:")
        || first_line.starts_with("Tool '")
        || first_line.starts_with("Request failed:")
        || first_line.starts_with("failed to read")
        || first_line.contains("Cannot read binary file")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
}

pub(super) fn summarize_tool_call(
    tool_name: &str,
    arguments: &str,
    body_width: usize,
    workspace_root: &Path,
) -> String {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let fields = summarize_tool_arguments(tool_name, arguments);
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();

    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    let path_to_relative = |path: &str| display_workspace_relative(workspace_root, Path::new(path));

    let summary = match canonical_name {
        "read" => field("file_path")
            .map(|path| format!("Read {}", path_to_relative(path)))
            .unwrap_or_else(|| "Read file".to_string()),
        "write" => field("file_path")
            .map(|path| format!("Write {}", path_to_relative(path)))
            .unwrap_or_else(|| "Write file".to_string()),
        "edit" => field("file_path")
            .map(|path| format!("Edit {}", path_to_relative(path)))
            .unwrap_or_else(|| "Edit file".to_string()),
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
        "websearch" => {
            let query = field("query").unwrap_or("");
            format!("Search the web for \"{query}\"")
        }
        "webfetch" => {
            let url = field("url").unwrap_or("");
            format!("Fetch web page from {url}")
        }
        "memory" => {
            let op = field("operation").unwrap_or("query");
            match op {
                "store" => {
                    let mtype = field("memory_type").unwrap_or("");
                    let title = field("title").unwrap_or("untitled");
                    if mtype.is_empty() {
                        format!("Save memory: {title}")
                    } else {
                        format!("Save [{mtype}] {title}")
                    }
                }
                "search" => {
                    let query = field("query").unwrap_or("");
                    format!("Search memories for \"{query}\"")
                }
                "list" => "List all memories".to_string(),
                "read" => {
                    let id = field("memory_id").unwrap_or("");
                    format!("Read memory {id}")
                }
                "delete" => {
                    let id = field("memory_id").unwrap_or("");
                    format!("Delete memory {id}")
                }
                other => format!("Memory: {other}"),
            }
        }
        "apply_patch" => "Apply patch".to_string(),
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

pub(super) fn summarize_tool_arguments(tool_name: &str, arguments: &str) -> Vec<(String, String)> {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();
    let mut fields = Vec::new();

    let string_field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string())
    };

    match canonical_name {
        "read" => {
            if let Some(path) = string_field("file_path") {
                fields.push(("file_path".to_string(), path));
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
            if let Some(path) = string_field("file_path") {
                fields.push(("file_path".to_string(), path));
            }
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
            // Use raw JSON access for command to preserve the full text with newlines,
            // so the expanded view in tool.rs can word-wrap it properly.
            if let Some(command) = parsed
                .as_ref()
                .and_then(|v| v.get("command"))
                .and_then(serde_json::Value::as_str)
            {
                fields.push(("command".to_string(), command.to_string()));
            }
            if let Some(description) = string_field("description") {
                fields.push(("description".to_string(), description));
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
        "websearch" => {
            if let Some(query) = string_field("query") {
                fields.push(("query".to_string(), query));
            }
            if let Some(num) = parsed
                .as_ref()
                .and_then(|v| v.get("num_results"))
                .and_then(|v| v.as_i64())
            {
                fields.push(("num_results".to_string(), format!("{}", num)));
            }
            if let Some(st) = string_field("search_type") {
                fields.push(("search_type".to_string(), st));
            }
        }
        "webfetch" => {
            if let Some(url) = string_field("url") {
                fields.push(("url".to_string(), url));
            }
            if let Some(fmt) = string_field("format") {
                fields.push(("format".to_string(), fmt));
            }
            if let Some(to) = parsed
                .as_ref()
                .and_then(|v| v.get("timeout"))
                .and_then(|v| v.as_i64())
            {
                fields.push(("timeout".to_string(), format!("{}s", to)));
            }
        }
        "memory" => {
            if let Some(op) = string_field("operation") {
                fields.push(("operation".to_string(), op));
            }
            if let Some(query) = string_field("query") {
                fields.push(("query".to_string(), query));
            }
            if let Some(title) = string_field("title") {
                fields.push(("title".to_string(), title));
            }
            if let Some(mtype) = string_field("memory_type") {
                fields.push(("type".to_string(), mtype));
            }
        }
        "apply_patch" => {
            if let Some(patch) = string_field("patch_text") {
                let first_line = patch.lines().next().unwrap_or(patch.as_str());
                let preview = shorten_single_line(first_line, 80);
                fields.push(("patch".to_string(), preview));
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
pub(super) fn parse_line_range_from_read_output(output: &str) -> Option<(i64, i64)> {
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
pub(super) fn parse_range(s: &str) -> Option<(i64, i64)> {
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
    (i64, i64),         // line_range (start, end)
    Option<(i64, i64)>, // requested_range (None if model didn't specify)
    i64,                // file_total
    Option<String>,     // truncated_by (None | "size" | "lines")
)>;

pub(super) fn parse_read_content_metadata(content: &str) -> ReadContentMetadata {
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

pub(super) fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}

pub(super) fn display_tool_name(tool_name: &str) -> String {
    tidev_engine::tooling::canonical_tool_name(tool_name)
        .unwrap_or(tool_name)
        .to_string()
}
