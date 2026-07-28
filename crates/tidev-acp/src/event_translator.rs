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
}

impl EventTranslator {
    /// Create a new translator for the given session.
    pub fn new(session_id: Uuid) -> Self {
        Self {
            session_id: acp::SessionId::new(session_id.to_string()),
            message_counter: 0,
            request_message_ids: HashMap::new(),
        }
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
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                    acp::TextContent::new(""),
                ))
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
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                    acp::TextContent::new(content),
                ))
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
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                    acp::TextContent::new(content),
                ))
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
                let update = crate::types::tidev_tool_call_to_acp_update(tool_call, None);
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
                let acp_tc = crate::types::tidev_tool_call_to_acp(tool_call);
                let mut notifs = vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCall(acp_tc),
                )];
                // Then send a status update to in_progress.
                let update = crate::types::tool_starting_update(tool_call);
                notifs.push(acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                ));
                notifs
            }

            BackendEvent::ToolCompleted {
                session_id: _,
                request_id: _,
                tool_call,
                result,
            } => {
                let mut notifs = Vec::new();

                // Send tool call content chunks from the result.
                let content = crate::types::tidev_tool_result_to_acp_content(result);
                if !content.is_empty() {
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(
                            acp::ToolCallUpdate::new(
                                tool_call.id.clone(),
                                acp::ToolCallUpdateFields::new().content(Some(content)),
                            ),
                        ),
                    ));
                }

                // Send the completed status update.
                let update = crate::types::tool_completed_update(tool_call);
                notifs.push(acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                ));
                notifs
            }

            BackendEvent::ShellOutput {
                session_id: _,
                tool_call_id,
                content,
                finished: _,
                exit_code: _,
            } => {
                let update = acp::ToolCallUpdate::new(
                    tool_call_id.clone(),
                    acp::ToolCallUpdateFields::new().content(Some(vec![
                        acp::ToolCallContent::Content(acp::Content::new(
                            acp::ContentBlock::Text(acp::TextContent::new(content)),
                        )),
                    ])),
                );
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                )]
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
                        acp::ContentBlock::Text(acp::TextContent::new(
                            format!("[subagent] {}: {}", child_tc.name, status_text),
                        )),
                    ));
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            tool_call_id.clone(),
                            acp::ToolCallUpdateFields::new()
                                .content(Some(vec![child_content])),
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
                let content = crate::types::tidev_tool_result_to_acp_content(result);
                if !content.is_empty() {
                    notifs.push(acp::SessionNotification::new(
                        self.session_id.clone(),
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            tool_call.id.clone(),
                            acp::ToolCallUpdateFields::new().content(Some(content)),
                        )),
                    ));
                }

                // Mark the parent tool as completed.
                let update = crate::types::tool_completed_update(tool_call);
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
                total_tokens,
                ..
            } => {
                let usage = acp::UsageUpdate::new(
                    *total_tokens as u64,
                    (*input_tokens + *output_tokens) as u64,
                );
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::UsageUpdate(usage),
                )]
            }

            BackendEvent::UserMessageCreated {
                session_id: _,
                message,
            } => {
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                    acp::TextContent::new(&message.content),
                ));
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
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                    acp::TextContent::new(format!("Error: {error}")),
                ));
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
                    format!(
                        "[Retry {attempt}/{max_attempts}] {reason} (retrying after {delay}s)"
                    )
                } else {
                    format!("[Retry {attempt}/{max_attempts}] {reason}")
                };
                let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                    acp::TextContent::new(&msg),
                ));
                vec![acp::SessionNotification::new(
                    self.session_id.clone(),
                    acp::SessionUpdate::AgentMessageChunk(chunk),
                )]
            }
        }
    }
}
