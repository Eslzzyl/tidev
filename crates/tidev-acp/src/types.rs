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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tc(name: &str, args: &str) -> tidev_types::message::ToolCall {
        tidev_types::message::ToolCall {
            id: "tc-1".into(),
            name: name.into(),
            arguments: args.into(),
            thought_signature: None,
        }
    }

    // ── tidev_tool_call_to_acp ──────────────────────────────────────────
    #[test]
    fn to_acp_kind_execute() {
        let tc = make_tc("shell", r#"{"cmd":"ls"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.tool_call_id.to_string(), "tc-1");
        assert_eq!(acp_tc.title, "shell");
        assert_eq!(acp_tc.kind, acp::ToolKind::Execute);
    }

    #[test]
    fn to_acp_kind_read() {
        let tc = make_tc("read", r#"{"path":"x"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Read);
    }

    #[test]
    fn to_acp_kind_edit() {
        let tc = make_tc("write", r#"{"path":"x","content":"hi"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Edit);
    }

    #[test]
    fn to_acp_kind_other() {
        let tc = make_tc("websearch", r#"{"query":"rust"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Other);
    }

    #[test]
    fn to_acp_raw_input_valid_json() {
        let tc = make_tc("read", r#"{"path":"Cargo.toml"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        // raw_input should be set when arguments are valid JSON
        let raw = acp_tc.raw_input.expect("raw_input should be Some");
        assert_eq!(raw.get("path").and_then(|v| v.as_str()), Some("Cargo.toml"));
    }

    #[test]
    fn to_acp_raw_input_invalid_json() {
        let tc = make_tc("read", "not-json");
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert!(acp_tc.raw_input.is_none(), "invalid JSON should produce None");
    }

    // ── tidev_tool_call_to_acp_update ───────────────────────────────────
    #[test]
    fn to_acp_update_no_status() {
        let tc = make_tc("read", "{}");
        let update = tidev_tool_call_to_acp_update(&tc, None);
        assert_eq!(update.tool_call_id.to_string(), "tc-1");
        assert_eq!(update.fields.status, None);
    }

    #[test]
    fn to_acp_update_in_progress() {
        let tc = make_tc("read", "{}");
        let update = tidev_tool_call_to_acp_update(&tc, Some(acp::ToolCallStatus::InProgress));
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::InProgress));
    }

    #[test]
    fn to_acp_update_completed() {
        let tc = make_tc("read", "{}");
        let update = tidev_tool_call_to_acp_update(&tc, Some(acp::ToolCallStatus::Completed));
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Completed));
    }

    // ── tidev_tool_result_to_acp_content ────────────────────────────────
    #[test]
    fn result_to_content_contains_output_text() {
        let result = tidev_types::message::ToolExecutionResult::new("hello world");
        let content = tidev_tool_result_to_acp_content(&result);
        assert_eq!(content.len(), 1);
        match &content[0] {
            acp::ToolCallContent::Content(c) => {
                match &c.content {
                    acp::ContentBlock::Text(t) => assert_eq!(t.text, "hello world"),
                    _ => panic!("expected Text"),
                }
            }
            _ => panic!("expected ToolCallContent::Content"),
        }
    }

    #[test]
    fn result_to_content_empty_output() {
        let result = tidev_types::message::ToolExecutionResult::new("");
        let content = tidev_tool_result_to_acp_content(&result);
        assert_eq!(content.len(), 1);
    }

    // ── tool_starting_update / tool_completed_update ────────────────────
    #[test]
    fn starting_update_in_progress() {
        let tc = make_tc("read", "{}");
        let update = tool_starting_update(&tc);
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::InProgress));
    }

    #[test]
    fn completed_update_completed() {
        let tc = make_tc("read", "{}");
        let update = tool_completed_update(&tc);
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Completed));
    }
}
