use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde::Deserialize;
use serde_json::Value;

use crate::i18n::{TextKey, UiText};
use crate::markdown::{WrapOptions, word_wrap_line};
use crate::theme::ThemePalette;

#[derive(Debug, Deserialize)]
struct McpServerRecord {
    server: String,
    kind: String,
    status: String,
    tool_count: usize,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpToolRecord {
    server: String,
    tool: String,
    description: String,
    input_schema: Value,
    read_only: bool,
}

pub(super) fn render_catalog_result_lines(
    tool_name: &str,
    tool_arguments: &str,
    output: &str,
    content_width: usize,
    palette: ThemePalette,
    ui_text: &UiText,
    is_expanded: bool,
) -> Option<Vec<Line<'static>>> {
    let list_server = server_argument(tool_arguments);
    match tool_name {
        "mcp_list" if list_server.is_none() => {
            if output.trim().is_empty() {
                return Some(vec![muted_line(&ui_text.text(TextKey::NoServers), palette)]);
            }
            let records = parse_jsonl::<McpServerRecord>(output)?;
            Some(render_server_records(
                &records,
                content_width,
                palette,
                ui_text,
            ))
        }
        "mcp_list" | "mcp_search" => {
            if output.trim().is_empty() {
                let label = if tool_name == "mcp_search" {
                    ui_text.text(TextKey::NoMatches)
                } else {
                    ui_text.text(TextKey::NoTools)
                };
                return Some(vec![muted_line(&label, palette)]);
            }
            let records = parse_jsonl::<McpToolRecord>(output)?;
            Some(render_tool_records(
                &records,
                content_width,
                palette,
                ui_text,
                is_expanded,
            ))
        }
        _ => None,
    }
}

fn server_argument(arguments: &str) -> Option<String> {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("server")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|server| !server.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn parse_jsonl<T: for<'de> Deserialize<'de>>(output: &str) -> Option<Vec<T>> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn render_server_records(
    records: &[McpServerRecord],
    content_width: usize,
    palette: ThemePalette,
    ui_text: &UiText,
) -> Vec<Line<'static>> {
    records
        .iter()
        .flat_map(|record| {
            let status_color = match record.status.as_str() {
                "connected" => palette.success,
                "failed" => palette.error,
                "disabled" => palette.muted,
                _ => palette.warning,
            };
            let status_label = match record.status.as_str() {
                "connected" => ui_text.text(TextKey::Connected),
                "failed" => ui_text.text(TextKey::Failed),
                "disabled" => ui_text.text(TextKey::Disabled),
                _ => record.status.clone(),
            };
            let mut lines = wrap_line(
                Line::from(vec![
                    Span::styled("  ● ", Style::default().fg(status_color)),
                    Span::styled(
                        record.server.clone(),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "  {}",
                            ui_text.text_with_values(
                                TextKey::McpKindTools,
                                &[
                                    ("kind", &record.kind),
                                    ("count", &record.tool_count.to_string()),
                                ],
                            )
                        ),
                        Style::default().fg(palette.muted),
                    ),
                    Span::styled(
                        format!("  {status_label}"),
                        Style::default().fg(status_color),
                    ),
                ]),
                content_width,
                "    ",
            );
            if let Some(error) = record.error.as_deref().filter(|error| !error.is_empty()) {
                lines.extend(wrap_line(
                    Line::from(Span::styled(
                        format!("    {error}"),
                        Style::default().fg(palette.error),
                    )),
                    content_width,
                    "    ",
                ));
            }
            lines
        })
        .collect()
}

fn render_tool_records(
    records: &[McpToolRecord],
    content_width: usize,
    palette: ThemePalette,
    ui_text: &UiText,
    is_expanded: bool,
) -> Vec<Line<'static>> {
    records
        .iter()
        .flat_map(|record| {
            let mut title = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    record.server.clone(),
                    Style::default().fg(palette.accent_soft),
                ),
                Span::styled(" / ", Style::default().fg(palette.muted)),
                Span::styled(
                    record.tool.clone(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if record.read_only {
                title.push(Span::styled(
                    format!("  {}", ui_text.text(TextKey::ReadOnly)),
                    Style::default().fg(palette.success),
                ));
            }

            let mut lines = wrap_line(Line::from(title), content_width, "    ");
            if !record.description.is_empty() {
                lines.extend(wrap_line(
                    Line::from(Span::styled(
                        format!("    {}", record.description),
                        Style::default().fg(palette.muted),
                    )),
                    content_width,
                    "    ",
                ));
            }
            if is_expanded {
                lines.push(Line::from(Span::styled(
                    format!("    {}", ui_text.text(TextKey::InputSchema)),
                    Style::default().fg(palette.muted),
                )));
                let schema = serde_json::to_string_pretty(&record.input_schema)
                    .unwrap_or_else(|_| record.input_schema.to_string());
                for schema_line in schema.lines() {
                    lines.extend(wrap_line(
                        Line::from(Span::styled(
                            format!("      {schema_line}"),
                            Style::default().fg(palette.muted),
                        )),
                        content_width,
                        "      ",
                    ));
                }
            }
            lines
        })
        .collect()
}

fn wrap_line(
    line: Line<'static>,
    content_width: usize,
    subsequent_indent: &str,
) -> Vec<Line<'static>> {
    let indent = Line::from(subsequent_indent.to_string());
    word_wrap_line(
        &line,
        WrapOptions::new(content_width)
            .subsequent_indent(indent)
            .break_words(true),
    )
    .into_iter()
    .map(|line| {
        Line::from(
            line.spans
                .into_iter()
                .map(|span| Span::styled(span.content.to_string(), span.style))
                .collect::<Vec<_>>(),
        )
    })
    .collect()
}

fn muted_line(text: &str, palette: ThemePalette) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(palette.muted),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_jsonl_records() {
        let records = parse_jsonl::<McpServerRecord>(
            r#"{"server":"blender","kind":"stdio","status":"disabled","tool_count":0}
{"server":"github","kind":"http","status":"connected","tool_count":12}"#,
        )
        .expect("valid server JSONL");

        assert_eq!(records.len(), 2);
        assert_eq!(records[1].server, "github");
        assert_eq!(records[1].tool_count, 12);
    }

    #[test]
    fn rejects_non_jsonl_catalog_output() {
        assert!(parse_jsonl::<McpToolRecord>("[Output truncated: out-123]").is_none());
    }
}
