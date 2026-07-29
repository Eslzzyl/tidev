//! Type conversions between tidev values and ACP v2 schema values.

use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v2 as acp;
use tidev_types::message::{Message, MessageAttachment, ToolCall, ToolExecutionResult};
use tidev_types::tools::canonical_tool_name;

/// Convert a possibly relative path into the absolute path required by ACP v2.
pub(crate) fn absolute_path(path: impl AsRef<Path>) -> acp::AbsolutePath {
    let path = path.as_ref();
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    acp::AbsolutePath::new(absolute)
}

/// Build the display title used for a tool call.
pub(crate) fn tool_title(tool_call: &ToolCall) -> String {
    crate::common::tool::tool_title(tool_call)
}

/// Map tidev's canonical tool names to ACP v2 tool kinds.
pub(crate) fn tool_kind(tool_call: &ToolCall) -> acp::ToolKind {
    match canonical_tool_name(&tool_call.name) {
        Some("shell") | Some("exec") => acp::ToolKind::Execute,
        Some("read") | Some("glob") | Some("grep") => acp::ToolKind::Read,
        Some("write") | Some("edit") | Some("apply_patch") => acp::ToolKind::Edit,
        Some("websearch") | Some("webfetch") => acp::ToolKind::Fetch,
        Some("task") | Some("question") | Some("todowrite") | Some("skill") => acp::ToolKind::Other,
        _ => acp::ToolKind::Other,
    }
}

/// Build absolute file locations from a tool call's arguments.
pub(crate) fn tool_locations(tool_call: &ToolCall) -> Vec<acp::ToolCallLocation> {
    let args: serde_json::Value = match serde_json::from_str(&tool_call.arguments) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let path = match canonical_tool_name(&tool_call.name) {
        Some("read") | Some("write") | Some("edit") => args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(serde_json::Value::as_str),
        _ => None,
    };

    path.filter(|value| !value.is_empty())
        .map(|value| vec![acp::ToolCallLocation::new(absolute_path(value))])
        .unwrap_or_default()
}

/// Convert a tidev tool call to a v2 upsert.
pub(crate) fn tool_call_update(
    tool_call: &ToolCall,
    status: Option<acp::ToolCallStatus>,
) -> acp::ToolCallUpdate {
    let raw_input = serde_json::from_str(&tool_call.arguments).ok();
    let mut update = acp::ToolCallUpdate::new(tool_call.id.clone())
        .title(tool_title(tool_call))
        .kind(tool_kind(tool_call))
        .locations(tool_locations(tool_call))
        .raw_input(raw_input);

    if let Some(status) = status {
        update = update.status(status);
    }

    update
}

/// Convert tool result output and file changes into v2 tool-call content.
pub(crate) fn tool_result_content(
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
) -> Vec<acp::ToolCallContent> {
    let mut content = vec![acp::ToolCallContent::Content(Box::new(acp::Content::new(
        acp::ContentBlock::Text(acp::TextContent::new(&result.output)),
    )))];

    let canonical = canonical_tool_name(&tool_call.name);
    if matches!(canonical, Some("write") | Some("edit")) {
        if let (Some(diff), Some(path)) = (
            result.metadata.diff.as_deref(),
            result.metadata.filepath.as_deref(),
        ) {
            content.push(acp::ToolCallContent::Diff(
                acp::Diff::new(vec![acp::DiffChange::modify(absolute_path(path))])
                    .with_patch(acp::DiffPatch::new(diff)),
            ));
        }
    } else if canonical == Some("apply_patch") {
        let changes = result
            .metadata
            .file_changes
            .iter()
            .map(|change| match change.operation.as_str() {
                "add" => acp::DiffChange::add(absolute_path(&change.path)),
                "delete" => acp::DiffChange::delete(absolute_path(&change.path)),
                _ => acp::DiffChange::modify(absolute_path(&change.path)),
            })
            .collect::<Vec<_>>();

        if !changes.is_empty() {
            let patch = result
                .metadata
                .file_changes
                .iter()
                .filter_map(|change| change.diff.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            let diff = if patch.is_empty() {
                acp::Diff::new(changes)
            } else {
                acp::Diff::new(changes).with_patch(acp::DiffPatch::new(patch))
            };
            content.push(acp::ToolCallContent::Diff(diff));
        }
    }

    content
}

/// Convert a persisted message into v2 content blocks.
pub(crate) fn message_content(message: &Message) -> Vec<acp::ContentBlock> {
    let mut blocks = Vec::new();
    if !message.content.is_empty() {
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(
            &message.content,
        )));
    }
    if !message.reasoning.is_empty() {
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(
            &message.reasoning,
        )));
    }
    blocks.extend(attachments_to_content(&message.attachments));
    if blocks.is_empty() {
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new("")));
    }
    blocks
}

/// Convert image attachments into v2 image content blocks.
pub(crate) fn attachments_to_content(attachments: &[MessageAttachment]) -> Vec<acp::ContentBlock> {
    use base64::Engine as _;

    attachments
        .iter()
        .filter_map(|attachment| match attachment {
            MessageAttachment::Image { mime, data, .. } => {
                Some(acp::ContentBlock::Image(acp::ImageContent::new(
                    base64::engine::general_purpose::STANDARD.encode(data),
                    mime.clone(),
                )))
            }
            _ => None,
        })
        .collect()
}

/// Build the v2 plan update represented by a todowrite call.
pub(crate) fn todo_plan_update(tool_call: &ToolCall) -> Option<acp::PlanUpdate> {
    let parsed: serde_json::Value = serde_json::from_str(&tool_call.arguments).ok()?;
    let todos = parsed.get("todos")?.as_array()?;
    let entries = todos
        .iter()
        .filter_map(|item| {
            let content = item.get("content")?.as_str()?.to_string();
            let status = match item.get("status").and_then(serde_json::Value::as_str) {
                Some("in_progress") => acp::PlanEntryStatus::InProgress,
                Some("completed") => acp::PlanEntryStatus::Completed,
                _ => acp::PlanEntryStatus::Pending,
            };
            Some(acp::PlanEntry::new(
                content,
                acp::PlanEntryPriority::Medium,
                status,
            ))
        })
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return None;
    }

    Some(acp::PlanUpdate::new(acp::PlanUpdateContent::items(
        "tidev-plan",
        entries,
    )))
}

/// Return a terminal ID derived from a shell tool call ID.
pub(crate) fn terminal_id(tool_call_id: &str) -> acp::TerminalId {
    acp::TerminalId::new(format!("terminal-{tool_call_id}"))
}

/// Extract a shell command from a shell tool call.
pub(crate) fn shell_command(tool_call: &ToolCall) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.arguments).ok()?;
    args.get("command")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}
