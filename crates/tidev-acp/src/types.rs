//! Type conversions between tidev internal types and ACP v1 schema types.

use agent_client_protocol::schema::v1 as acp;

/// Convert a tidev [`ToolCall`](tidev_types::message::ToolCall) to an ACP
/// [`ToolCall`](acp::ToolCall) for sending as a `SessionUpdate::ToolCall` notification.
pub fn tidev_tool_call_to_acp(tc: &tidev_types::message::ToolCall) -> acp::ToolCall {
    let kind = match tidev_types::tools::canonical_tool_name(&tc.name) {
        Some("shell") | Some("exec") => acp::ToolKind::Execute,
        Some("read") | Some("glob") | Some("grep") => acp::ToolKind::Read,
        Some("write") | Some("edit") => acp::ToolKind::Edit,
        _ => acp::ToolKind::Other,
    };

    let raw_input: Option<serde_json::Value> =
        serde_json::from_str(&tc.arguments).ok();

    acp::ToolCall::new(tc.id.clone(), &tc.name)
        .kind(kind)
        .raw_input(raw_input)
}

/// Convert a tidev [`ToolCall`](tidev_types::message::ToolCall) to an ACP
/// [`ToolCallUpdate`](acp::ToolCallUpdate) with optional status.
pub fn tidev_tool_call_to_acp_update(
    tc: &tidev_types::message::ToolCall,
    status: Option<acp::ToolCallStatus>,
) -> acp::ToolCallUpdate {
    let fields = acp::ToolCallUpdateFields::new().status(status);
    acp::ToolCallUpdate::new(tc.id.clone(), fields)
}

/// Convert a tidev [`ToolExecutionResult`](tidev_types::message::ToolExecutionResult)
/// to ACP [`ToolCallContent`](acp::ToolCallContent) items.
pub fn tidev_tool_result_to_acp_content(
    result: &tidev_types::message::ToolExecutionResult,
) -> Vec<acp::ToolCallContent> {
    vec![acp::ToolCallContent::Content(acp::Content::new(
        acp::ContentBlock::Text(acp::TextContent::new(&result.output)),
    ))]
}

/// Build an ACP [`ToolCallUpdate`](acp::ToolCallUpdate) representing a tool
/// call that has started executing.
pub fn tool_starting_update(tc: &tidev_types::message::ToolCall) -> acp::ToolCallUpdate {
    tidev_tool_call_to_acp_update(tc, Some(acp::ToolCallStatus::InProgress))
}

/// Build an ACP [`ToolCallUpdate`](acp::ToolCallUpdate) representing a tool
/// call that has completed.
pub fn tool_completed_update(tc: &tidev_types::message::ToolCall) -> acp::ToolCallUpdate {
    tidev_tool_call_to_acp_update(tc, Some(acp::ToolCallStatus::Completed))
}

/// Build an ACP [`ToolCallUpdate`](acp::ToolCallUpdate) representing a tool
/// call that has errored.
pub fn tool_error_update(tc: &tidev_types::message::ToolCall) -> acp::ToolCallUpdate {
    tidev_tool_call_to_acp_update(tc, Some(acp::ToolCallStatus::Failed))
}

/// Build an ACP [`ContentChunk`](acp::ContentChunk) for an agent text delta.
pub fn text_delta_chunk(text: &str) -> acp::ContentChunk {
    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)))
}

/// Build an ACP [`ContentChunk`](acp::ContentChunk) for a reasoning/thinking delta.
pub fn reasoning_delta_chunk(text: &str) -> acp::ContentChunk {
    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)))
}
