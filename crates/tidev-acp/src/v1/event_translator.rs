//! Translates tidev [`BackendEvent`]s into ACP [`SessionNotification`]s.
//!
//! The translator maintains per-session state (message ID counters, tool call
//! tracking) and converts the stream of backend events into the ACP session
//! update protocol.

use std::collections::HashMap;

use agent_client_protocol::schema::v1 as acp;
use tidev_types::message::BackendEvent;
use uuid::Uuid;

/// Stateful translator that converts [`BackendEvent`]s into ACP
/// [`SessionNotification`](acp::SessionNotification) values.
///
/// One translator instance is created per ACP session.
pub struct EventTranslator {
    /// ACP session ID (string form of the tidev UUID).
    session_id: acp::SessionId,
    /// Monotonically increasing message ID counter.
    message_counter: u64,
    /// Map from tidev `request_id` to ACP `MessageId`.
    request_message_ids: HashMap<u64, acp::MessageId>,
    /// Model's maximum context window size in tokens (for UsageUpdate.size).
    context_window: usize,
}

impl EventTranslator {
    /// Create a new translator for the given session and context window.
    pub fn new(session_id: Uuid, context_window: usize) -> Self {
        Self {
            session_id: acp::SessionId::new(session_id.to_string()),
            message_counter: 0,
            request_message_ids: HashMap::new(),
            context_window,
        }
    }

    /// Update the context window size (e.g. after a model switch).
    pub fn set_context_window(&mut self, window: usize) {
        self.context_window = window;
    }

    /// Allocate a new ACP message ID for the given backend `request_id`.
    fn next_message_id(&mut self, request_id: u64) -> acp::MessageId {
        self.message_counter += 1;
        let id = acp::MessageId::new(format!("msg-{}", self.message_counter));
        self.request_message_ids.insert(request_id, id.clone());
        id
    }

    /// Look up the ACP message ID for a backend `request_id`.
    fn message_id_for(&self, request_id: u64) -> acp::MessageId {
        self.request_message_ids
            .get(&request_id)
            .cloned()
            .unwrap_or_else(|| acp::MessageId::new(format!("msg-{}", self.message_counter)))
    }

    /// Translate a single [`BackendEvent`] into zero or more ACP session
    /// update notifications.
    ///
    /// Returns `None` for events that have no ACP representation (e.g.
    /// `SidebarSnapshotReady`, `ContextCompacted`).
    pub fn translate(&mut self, event: &BackendEvent) -> Vec<acp::SessionNotification> {
        match event {
            BackendEvent::TurnStarting {
                session_id: _,
                request_id,
            } => {
                let message_id = self.next_message_id(*request_id);
                // Send an empty agent_message_chunk to signal the start of a new message.
                let chunk =
                    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new("")))
                        .message_id(message_id);
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::AgentMessageChunk(chunk),
                )]
            }

            BackendEvent::Delta {
                session_id: _,
                request_id,
                content,
            } => {
                let message_id = self.message_id_for(*request_id);
                let chunk =
                    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(content)))
                        .message_id(message_id);
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::AgentMessageChunk(chunk),
                )]
            }

            BackendEvent::ReasoningDelta {
                session_id: _,
                request_id,
                content,
            } => {
                let message_id = self.message_id_for(*request_id);
                let chunk =
                    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(content)))
                        .message_id(message_id);
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::AgentThoughtChunk(chunk),
                )]
            }

            BackendEvent::ToolCallUpdated {
                session_id: _,
                request_id: _,
                tool_call,
            } => {
                let update = crate::v1::types::tidev_tool_call_to_acp_update(tool_call, None);
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                )]
            }

            BackendEvent::ToolStarting {
                session_id: _,
                request_id: _,
                tool_call,
            } => {
                // First, send the initial ToolCall notification.
                let acp_tc = crate::v1::types::tidev_tool_call_to_acp(tool_call);
                let mut notifs = vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCall(acp_tc),
                )];
                // Then send a rich status update with title, kind, locations.
                let update = crate::v1::types::tool_starting_update_rich(tool_call);
                notifs.push(acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                ));

                // If this is a todowrite tool call, emit a Plan notification
                // so the client can display the updated task list.
                if tool_call.name == "todowrite" {
                    if let Some(plan) = todo_args_to_plan(&tool_call.arguments) {
                        notifs.push(acp::SessionNotification::new(
                            self.session_id.clone(),
                            acp::SessionUpdate::Plan(plan),
                        ));
                    }
                }

                notifs
            }

            BackendEvent::ToolCompleted {
                session_id: _,
                request_id: _,
                tool_call,
                result,
            } => {
                let mut notifs = Vec::new();

                // Prefer Diff content for write/edit/apply_patch.
                let content = crate::v1::types::tidev_result_to_diff_content(tool_call, result);
                let content = if !content.is_empty() {
                    content
                } else {
                    // Fallback: text content + any image attachments.
                    let mut c = crate::v1::types::tidev_tool_result_to_acp_content(result);
                    c.extend(crate::v1::types::tidev_attachments_to_content(
                        &result.attachments,
                    ));
                    c
                };

                if !content.is_empty() {
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            tool_call.id.clone(),
                            acp::ToolCallUpdateFields::new().content(Some(content)),
                        )),
                    ));
                }

                // Send the completed status update with raw_output.
                let update = crate::v1::types::tool_completed_update_rich(tool_call, result);
                notifs.push(acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                ));
                notifs
            }

            BackendEvent::ShellOutput { .. } => {
                // Shell streaming output is deferred to ToolCompleted in v1
                // to avoid redundant content transfers. Upgrade to v2 for
                // terminal_output_chunk support.
                vec![]
            }

            BackendEvent::SubagentStatus {
                session_id: _,
                request_id: _,
                tool_call_id,
                status_text,
                current_tool_call,
                content_delta,
                ..
            } => {
                // Transparent subagent: report as the parent tool's status update.
                let mut notifs = Vec::new();

                // If there's a current tool call from the subagent, report it as a
                // tool call update on the parent tool.
                if let Some(child_tc) = current_tool_call {
                    let child_content = acp::ToolCallContent::Content(acp::Content::new(
                        acp::ContentBlock::Text(acp::TextContent::new(format!(
                            "[subagent] {}: {}",
                            child_tc.name, status_text
                        ))),
                    ));
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            tool_call_id.clone(),
                            acp::ToolCallUpdateFields::new().content(Some(vec![child_content])),
                        )),
                    ));
                }

                // Stream content deltas from the subagent as parent tool content.
                if let Some(delta) = content_delta {
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            tool_call_id.clone(),
                            acp::ToolCallUpdateFields::new().content(Some(vec![
                                acp::ToolCallContent::Content(acp::Content::new(
                                    acp::ContentBlock::Text(acp::TextContent::new(delta)),
                                )),
                            ])),
                        )),
                    ));
                }

                notifs
            }

            BackendEvent::SubagentCompleted {
                session_id: _,
                request_id: _,
                tool_call,
                result,
                ..
            } => {
                let mut notifs = Vec::new();

                // Send the subagent result as tool content.
                let content = crate::v1::types::tidev_tool_result_to_acp_content(result);
                if !content.is_empty() {
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            tool_call.id.clone(),
                            acp::ToolCallUpdateFields::new().content(Some(content)),
                        )),
                    ));
                }

                // Mark the parent tool as completed with raw_output.
                let update = crate::v1::types::tool_completed_update_rich(tool_call, result);
                notifs.push(acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                ));
                notifs
            }

            BackendEvent::UsageStats {
                session_id: _,
                request_id: _,
                input_tokens,
                output_tokens,
                ..
            } => {
                let used = (*input_tokens + *output_tokens) as u64;
                let size = self.context_window as u64;
                let usage = acp::UsageUpdate::new(used, size);
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::UsageUpdate(usage),
                )]
            }

            BackendEvent::UserMessageCreated {
                session_id: _,
                message,
            } => {
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
                    &message.content,
                )));
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::UserMessageChunk(chunk),
                )]
            }

            BackendEvent::Failed {
                session_id: _,
                request_id: _,
                error,
            } => {
                // Send an error as an agent message chunk.
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
                    format!("Error: {error}"),
                )));
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::AgentMessageChunk(chunk),
                )]
            }

            // Events that are handled by the event loop's deferred response
            // logic, or have no ACP representation.
            BackendEvent::Finished { .. }
            | BackendEvent::StreamEnd { .. }
            | BackendEvent::InstructionsLoaded { .. }
            | BackendEvent::ContextCompacted { .. }
            | BackendEvent::UndoCompleted { .. }
            | BackendEvent::SidebarSnapshotReady { .. }
            | BackendEvent::MessagesTruncated { .. } => vec![],

            // ── Retrying ─────────────────────────────────────────────
            BackendEvent::Retrying {
                session_id: _,
                request_id: _,
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
            } => {
                let msg = if let Some(delay) = retry_after_secs {
                    format!("[Retry {attempt}/{max_attempts}] {reason} (retrying after {delay}s)")
                } else {
                    format!("[Retry {attempt}/{max_attempts}] {reason}")
                };
                let chunk =
                    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(&msg)));
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::AgentMessageChunk(chunk),
                )]
            }
        }
    }
}

/// Try to parse a `todowrite` tool call arguments JSON and convert it into
/// an ACP [`Plan`] notification payload.
///
/// Returns `None` if the arguments cannot be parsed as todo items.
fn todo_args_to_plan(arguments: &str) -> Option<acp::Plan> {
    // The arguments JSON shape is: { "todos": [{ "content": "...", "status": "..." }, ...] }
    // We parse directly from serde_json::Value to avoid depending on the
    // tidev_tools crate's TodoWriteArgs deserialization internals.
    let parsed: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let todos = parsed.get("todos")?.as_array()?;

    let entries: Vec<acp::PlanEntry> = todos
        .iter()
        .filter_map(|item| {
            let content = item.get("content")?.as_str()?.to_string();
            let status_str = item.get("status")?.as_str().unwrap_or("pending");
            let status = match status_str {
                "in_progress" => acp::PlanEntryStatus::InProgress,
                "completed" => acp::PlanEntryStatus::Completed,
                _ => acp::PlanEntryStatus::Pending,
            };
            Some(acp::PlanEntry::new(
                content,
                acp::PlanEntryPriority::Medium,
                status,
            ))
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    Some(acp::Plan::new(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn sid() -> Uuid {
        Uuid::parse_str(SESSION_ID).unwrap()
    }

    fn make_translator() -> EventTranslator {
        EventTranslator::new(sid(), 200000)
    }

    fn make_tc(name: &str, args: &str) -> tidev_types::message::ToolCall {
        tidev_types::message::ToolCall {
            id: "tc-1".into(),
            name: name.into(),
            arguments: args.into(),
            thought_signature: None,
        }
    }

    // ── Helper: assert a single notification with the expected session ID ──
    fn assert_session_id(notifs: &[acp::SessionNotification]) {
        assert_eq!(notifs[0].session_id.to_string(), SESSION_ID);
    }

    // ── TurnStarting ─────────────────────────────────────────────────────
    #[test]
    fn turn_starting_sends_empty_agent_message_chunk() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::TurnStarting {
            session_id: sid(),
            request_id: 1,
        });
        assert_eq!(notifs.len(), 1);
        assert_session_id(&notifs);
        match &notifs[0].update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => assert_eq!(t.text, ""),
                _ => panic!("expected Text content block"),
            },
            _ => panic!("expected AgentMessageChunk"),
        }
    }

    #[test]
    fn turn_starting_increments_message_id() {
        let mut tr = make_translator();
        let _n1 = tr.translate(&BackendEvent::TurnStarting {
            session_id: sid(),
            request_id: 1,
        });
        let n2 = tr.translate(&BackendEvent::TurnStarting {
            session_id: sid(),
            request_id: 2,
        });
        // Second TurnStarting still produces a valid notification.
        assert_eq!(n2.len(), 1);
        assert_session_id(&n2);
    }

    // ── Delta ────────────────────────────────────────────────────────────
    #[test]
    fn delta_sends_agent_message_chunk() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::Delta {
            session_id: sid(),
            request_id: 1,
            content: "Hello world".into(),
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => assert_eq!(t.text, "Hello world"),
                _ => panic!("expected Text"),
            },
            _ => panic!("expected AgentMessageChunk"),
        }
    }

    // ── ReasoningDelta ───────────────────────────────────────────────────
    #[test]
    fn reasoning_delta_sends_agent_thought_chunk() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::ReasoningDelta {
            session_id: sid(),
            request_id: 1,
            content: "thinking...".into(),
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::AgentThoughtChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => assert_eq!(t.text, "thinking..."),
                _ => panic!("expected Text"),
            },
            _ => panic!("expected AgentThoughtChunk"),
        }
    }

    // ── ToolCallUpdated ──────────────────────────────────────────────────
    #[test]
    fn tool_call_updated_sends_tool_call_update() {
        let mut tr = make_translator();
        let tc = make_tc("read", r#"{"path":"Cargo.toml"}"#);
        let notifs = tr.translate(&BackendEvent::ToolCallUpdated {
            session_id: sid(),
            request_id: 1,
            tool_call: tc,
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::ToolCallUpdate(update) => {
                assert_eq!(update.tool_call_id.to_string(), "tc-1");
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    // ── ToolStarting (normal) ────────────────────────────────────────────
    #[test]
    fn tool_starting_sends_tool_call_and_update() {
        let mut tr = make_translator();
        let tc = make_tc("read", r#"{"path":"Cargo.toml"}"#);
        let notifs = tr.translate(&BackendEvent::ToolStarting {
            session_id: sid(),
            request_id: 1,
            tool_call: tc,
        });
        // Should emit ToolCall + ToolCallUpdate (in_progress)
        assert_eq!(notifs.len(), 2);
        match &notifs[0].update {
            acp::SessionUpdate::ToolCall(tc) => {
                assert_eq!(tc.tool_call_id.to_string(), "tc-1");
                assert_eq!(tc.title, "read");
            }
            _ => panic!("expected ToolCall"),
        }
        match &notifs[1].update {
            acp::SessionUpdate::ToolCallUpdate(update) => {
                assert_eq!(update.tool_call_id.to_string(), "tc-1");
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    // ── ToolStarting (todowrite → Plan notification) ─────────────────────
    #[test]
    fn tool_starting_todowrite_emits_plan() {
        let mut tr = make_translator();
        let args = r#"{"todos":[
            {"content":"Task A","status":"pending"},
            {"content":"Task B","status":"in_progress"},
            {"content":"Task C","status":"completed"}
        ]}"#;
        let tc = make_tc("todowrite", args);
        let notifs = tr.translate(&BackendEvent::ToolStarting {
            session_id: sid(),
            request_id: 1,
            tool_call: tc,
        });
        // 3 notifications: ToolCall, ToolCallUpdate, Plan
        assert_eq!(notifs.len(), 3);
        match &notifs[2].update {
            acp::SessionUpdate::Plan(plan) => {
                assert_eq!(plan.entries.len(), 3);
                assert_eq!(plan.entries[0].content, "Task A");
                assert_eq!(plan.entries[1].content, "Task B");
                assert_eq!(plan.entries[2].content, "Task C");
            }
            _ => panic!("expected Plan"),
        }
    }

    #[test]
    fn tool_starting_todowrite_empty_todos_no_plan() {
        let mut tr = make_translator();
        let args = r#"{"todos":[]}"#;
        let tc = make_tc("todowrite", args);
        let notifs = tr.translate(&BackendEvent::ToolStarting {
            session_id: sid(),
            request_id: 1,
            tool_call: tc,
        });
        // Only ToolCall + ToolCallUpdate, no Plan
        assert_eq!(notifs.len(), 2);
    }

    #[test]
    fn tool_starting_non_todowrite_no_plan() {
        let mut tr = make_translator();
        let tc = make_tc("read", r#"{"path":"x"}"#);
        let notifs = tr.translate(&BackendEvent::ToolStarting {
            session_id: sid(),
            request_id: 1,
            tool_call: tc,
        });
        assert_eq!(notifs.len(), 2, "non-todowrite should not emit Plan");
    }

    // ── ToolCompleted ────────────────────────────────────────────────────
    #[test]
    fn tool_completed_sends_content_and_completed_status() {
        let mut tr = make_translator();
        let tc = make_tc("read", r#"{"path":"Cargo.toml"}"#);
        let result = tidev_types::message::ToolExecutionResult::new("file content");
        let notifs = tr.translate(&BackendEvent::ToolCompleted {
            session_id: sid(),
            request_id: 1,
            tool_call: tc,
            result: Box::new(result),
        });
        // Should emit content + completed status with raw_output
        assert_eq!(notifs.len(), 2);
        match &notifs[0].update {
            acp::SessionUpdate::ToolCallUpdate(update) => {
                assert!(update.fields.content.is_some());
            }
            _ => panic!("expected ToolCallUpdate with content"),
        }
        match &notifs[1].update {
            acp::SessionUpdate::ToolCallUpdate(update) => {
                assert_eq!(update.tool_call_id.to_string(), "tc-1");
                assert!(update.fields.raw_output.is_some(), "expected raw_output");
            }
            _ => panic!("expected completed ToolCallUpdate"),
        }
    }

    // ── ShellOutput ──────────────────────────────────────────────────────
    #[test]
    fn shell_output_deferred_to_completion() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::ShellOutput {
            session_id: sid(),
            tool_call_id: "tc-1".into(),
            content: "building...".into(),
            finished: false,
            exit_code: None,
        });
        // Shell streaming is deferred to ToolCompleted in v1.
        assert!(notifs.is_empty());
    }

    // ── UsageStats ───────────────────────────────────────────────────────
    #[test]
    fn usage_stats_sends_usage_update() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::UsageStats {
            session_id: sid(),
            request_id: 1,
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: 30,
            cache_write_tokens: 10,
            model_id: "gpt-4".into(),
            duration_ms: Some(1234),
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::UsageUpdate(usage) => {
                // used = input + output (current context fill)
                assert_eq!(usage.used, 150);
                // size = context_window from make_translator
                assert_eq!(usage.size, 200000);
            }
            _ => panic!("expected UsageUpdate"),
        }
    }

    // ── UserMessageCreated ───────────────────────────────────────────────
    #[test]
    fn user_message_created_sends_user_message_chunk() {
        let mut tr = make_translator();
        let mut msg =
            tidev_types::message::Message::new(tidev_types::message::MessageRole::User, "hello");
        msg.id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let notifs = tr.translate(&BackendEvent::UserMessageCreated {
            session_id: sid(),
            message: Box::new(msg),
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::UserMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => assert_eq!(t.text, "hello"),
                _ => panic!("expected Text"),
            },
            _ => panic!("expected UserMessageChunk"),
        }
    }

    // ── Failed ───────────────────────────────────────────────────────────
    #[test]
    fn failed_sends_error_as_agent_message_chunk() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::Failed {
            session_id: sid(),
            request_id: 1,
            error: "API error".into(),
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => {
                    assert!(t.text.contains("API error"));
                }
                _ => panic!("expected Text"),
            },
            _ => panic!("expected AgentMessageChunk"),
        }
    }

    // ── Retrying ─────────────────────────────────────────────────────────
    #[test]
    fn retrying_sends_agent_message_chunk() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::Retrying {
            session_id: sid(),
            request_id: 1,
            attempt: 2,
            max_attempts: 3,
            reason: "rate limited".into(),
            retry_after_secs: Some(5),
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => {
                    assert!(t.text.contains("[Retry 2/3]"));
                    assert!(t.text.contains("rate limited"));
                    assert!(t.text.contains("5s"));
                }
                _ => panic!("expected Text"),
            },
            _ => panic!("expected AgentMessageChunk"),
        }
    }

    #[test]
    fn retrying_without_delay() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::Retrying {
            session_id: sid(),
            request_id: 1,
            attempt: 1,
            max_attempts: 5,
            reason: "timeout".into(),
            retry_after_secs: None,
        });
        assert_eq!(notifs.len(), 1);
        match &notifs[0].update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(t) => {
                    assert!(t.text.contains("[Retry 1/5]"));
                    assert!(!t.text.contains("retrying after"));
                }
                _ => panic!("expected Text"),
            },
            _ => panic!("expected AgentMessageChunk"),
        }
    }

    // ── Ignored events ───────────────────────────────────────────────────
    #[test]
    fn finished_is_ignored_by_translator() {
        let mut tr = make_translator();
        let turn = tidev_types::message::AssistantTurn {
            content: "done".into(),
            ..Default::default()
        };
        let notifs = tr.translate(&BackendEvent::Finished {
            session_id: sid(),
            request_id: 1,
            turn: Box::new(turn),
        });
        assert!(
            notifs.is_empty(),
            "Finished should be ignored by translator"
        );
    }

    #[test]
    fn stream_end_is_ignored() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::StreamEnd {
            session_id: sid(),
            request_id: 1,
            reasoning_started_at: None,
            reasoning_completed_at: None,
        });
        assert!(notifs.is_empty());
    }

    #[test]
    fn instructions_loaded_is_ignored() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::InstructionsLoaded {
            session_id: sid(),
            sources: vec!["AGENTS.md".into()],
        });
        assert!(notifs.is_empty());
    }

    #[test]
    fn context_compacted_is_ignored() {
        let mut tr = make_translator();
        let notifs = tr.translate(&BackendEvent::ContextCompacted {
            session_id: sid(),
            compacted: true,
            manual: false,
            summary: Some("compressed".into()),
            retained_from: 10,
            model_id: None,
            completed_at: None,
            error: None,
        });
        assert!(notifs.is_empty());
    }

    // ── todo_args_to_plan ────────────────────────────────────────────────
    #[test]
    fn todo_args_invalid_json_returns_none() {
        assert!(todo_args_to_plan("not json").is_none());
    }

    #[test]
    fn todo_args_missing_todos_returns_none() {
        assert!(todo_args_to_plan(r#"{"foo":"bar"}"#).is_none());
    }

    #[test]
    fn todo_args_empty_todos_returns_none() {
        assert!(todo_args_to_plan(r#"{"todos":[]}"#).is_none());
    }

    #[test]
    fn todo_args_parses_items() {
        let plan = todo_args_to_plan(
            r#"{"todos":[
                {"content":"Task 1","status":"pending"},
                {"content":"Task 2","status":"in_progress"},
                {"content":"Task 3","status":"completed"}
            ]}"#,
        )
        .expect("should parse");
        assert_eq!(plan.entries.len(), 3);
        assert_eq!(plan.entries[0].content, "Task 1");
        assert_eq!(plan.entries[1].content, "Task 2");
        assert_eq!(plan.entries[2].content, "Task 3");
    }

    #[test]
    fn todo_args_unknown_status_defaults_to_pending() {
        let plan = todo_args_to_plan(r#"{"todos":[{"content":"X","status":"unknown"}]}"#)
            .expect("should parse");
        assert_eq!(plan.entries.len(), 1);
        // Unknown status becomes Pending
        match &plan.entries[0].status {
            acp::PlanEntryStatus::Pending => {} // OK
            _ => panic!("expected Pending"),
        }
    }
}
