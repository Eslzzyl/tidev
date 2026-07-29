//! Translate tidev backend events into ACP v2 session updates.

use std::collections::HashMap;

use agent_client_protocol::schema::v2 as acp;
use tidev_types::message::BackendEvent;
use uuid::Uuid;

/// Stateful event translator for one ACP v2 session.
pub(crate) struct EventTranslator {
    session_id: acp::SessionId,
    message_counter: u64,
    request_message_ids: HashMap<u64, acp::MessageId>,
    context_window: usize,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
}

impl EventTranslator {
    pub(crate) fn new(session_id: Uuid, context_window: usize) -> Self {
        Self {
            session_id: acp::SessionId::new(session_id.to_string()),
            message_counter: 0,
            request_message_ids: HashMap::new(),
            context_window,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
        }
    }

    pub(crate) fn set_context_window(&mut self, context_window: usize) {
        self.context_window = context_window;
    }

    fn next_message_id(&mut self, request_id: u64) -> acp::MessageId {
        self.message_counter += 1;
        let id = acp::MessageId::new(format!("msg-{}", self.message_counter));
        self.request_message_ids.insert(request_id, id.clone());
        id
    }

    fn message_id(&self, request_id: u64) -> acp::MessageId {
        self.request_message_ids
            .get(&request_id)
            .cloned()
            .unwrap_or_else(|| acp::MessageId::new(format!("msg-{}", self.message_counter)))
    }

    fn update(&self, update: acp::SessionUpdate) -> acp::UpdateSessionNotification {
        acp::UpdateSessionNotification::new(self.session_id.clone(), update)
    }

    fn idle(&self, reason: acp::StopReason) -> acp::UpdateSessionNotification {
        let usage = acp::Usage::new(
            self.input_tokens + self.output_tokens,
            self.input_tokens,
            self.output_tokens,
        )
        .cached_read_tokens(self.cache_read_tokens);
        self.update(acp::SessionUpdate::StateUpdate(acp::StateUpdate::Idle(
            acp::IdleStateUpdate::new().stop_reason(reason).usage(usage),
        )))
    }

    pub(crate) fn translate(
        &mut self,
        event: &BackendEvent,
    ) -> Vec<acp::UpdateSessionNotification> {
        match event {
            BackendEvent::TurnStarting { request_id, .. } => {
                self.next_message_id(*request_id);
                vec![
                    self.update(acp::SessionUpdate::StateUpdate(acp::StateUpdate::Running(
                        acp::RunningStateUpdate::new(),
                    ))),
                ]
            }
            BackendEvent::Delta {
                request_id,
                content,
                ..
            } => vec![self.update(acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new(content)),
                    self.message_id(*request_id),
                ),
            ))],
            BackendEvent::ReasoningDelta {
                request_id,
                content,
                ..
            } => vec![self.update(acp::SessionUpdate::AgentThoughtChunk(
                acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new(content)),
                    self.message_id(*request_id),
                ),
            ))],
            BackendEvent::ToolCallUpdated { tool_call, .. } => vec![self.update(
                acp::SessionUpdate::ToolCallUpdate(crate::v2_types::tool_call_update(
                    tool_call,
                    Some(acp::ToolCallStatus::Pending),
                )),
            )],
            BackendEvent::ToolStarting { tool_call, .. } => {
                let mut updates = vec![self.update(acp::SessionUpdate::ToolCallUpdate(
                    crate::v2_types::tool_call_update(
                        tool_call,
                        Some(acp::ToolCallStatus::InProgress),
                    ),
                ))];
                if matches!(
                    tidev_types::tools::canonical_tool_name(&tool_call.name),
                    Some("shell") | Some("exec")
                ) {
                    let terminal =
                        acp::TerminalUpdate::new(crate::v2_types::terminal_id(&tool_call.id))
                            .command(crate::v2_types::shell_command(tool_call))
                            .cwd(crate::v2_types::absolute_path(
                                std::env::current_dir().unwrap_or_default(),
                            ));
                    updates.push(self.update(acp::SessionUpdate::TerminalUpdate(terminal)));
                }
                if let Some(plan) = crate::v2_types::todo_plan_update(tool_call) {
                    updates.push(self.update(acp::SessionUpdate::PlanUpdate(plan)));
                }
                updates
            }
            BackendEvent::ToolCompleted {
                tool_call, result, ..
            } => {
                let content = crate::v2_types::tool_result_content(tool_call, result);
                let mut update = crate::v2_types::tool_call_update(
                    tool_call,
                    Some(acp::ToolCallStatus::Completed),
                );
                if !content.is_empty() {
                    update = update.content(content);
                }
                update = update.raw_output(serde_json::Value::String(result.output.clone()));
                vec![self.update(acp::SessionUpdate::ToolCallUpdate(update))]
            }
            BackendEvent::ShellOutput {
                tool_call_id,
                content,
                finished,
                exit_code,
                ..
            } => {
                let terminal_id = crate::v2_types::terminal_id(tool_call_id);
                let mut updates = vec![self.update(acp::SessionUpdate::TerminalOutputChunk(
                    acp::TerminalOutputChunk::new(
                        terminal_id.clone(),
                        base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            content.as_bytes(),
                        ),
                    ),
                ))];
                if *finished {
                    updates.push(
                        self.update(acp::SessionUpdate::TerminalUpdate(
                            acp::TerminalUpdate::new(terminal_id).exit_status(
                                acp::TerminalExitStatus::new()
                                    .exit_code(exit_code.map(|code| code as u32)),
                            ),
                        )),
                    );
                }
                updates
            }
            BackendEvent::UsageStats {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                ..
            } => {
                self.input_tokens += *input_tokens as u64;
                self.output_tokens += *output_tokens as u64;
                self.cache_read_tokens += *cache_read_tokens as u64;
                vec![
                    self.update(acp::SessionUpdate::UsageUpdate(acp::UsageUpdate::new(
                        (*input_tokens as u64) + (*output_tokens as u64),
                        self.context_window as u64,
                    ))),
                ]
            }
            BackendEvent::UserMessageCreated { message, .. } => vec![
                self.update(acp::SessionUpdate::UserMessage(
                    acp::UserMessage::new(message.id.to_string())
                        .content(crate::v2_types::message_content(message)),
                )),
            ],
            BackendEvent::Failed { error, .. } => {
                let mut updates = vec![self.update(acp::SessionUpdate::AgentMessageChunk(
                    acp::ContentChunk::new(
                        acp::ContentBlock::Text(acp::TextContent::new(format!("Error: {error}"))),
                        acp::MessageId::new(format!("msg-{}", self.message_counter)),
                    ),
                ))];
                updates.push(self.idle(acp::StopReason::EndTurn));
                updates
            }
            BackendEvent::Finished { turn, .. } => {
                let reason = match turn.finish_reason.as_deref() {
                    Some("max_tokens") | Some("length") => acp::StopReason::MaxTokens,
                    Some("cancelled") | Some("canceled") => acp::StopReason::Cancelled,
                    _ => acp::StopReason::EndTurn,
                };
                vec![self.idle(reason)]
            }
            BackendEvent::Retrying {
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
                ..
            } => {
                let suffix = retry_after_secs
                    .map(|delay| format!(" (retrying after {delay}s)"))
                    .unwrap_or_default();
                vec![self.update(acp::SessionUpdate::AgentMessageChunk(
                    acp::ContentChunk::new(
                        acp::ContentBlock::Text(acp::TextContent::new(format!(
                            "[Retry {attempt}/{max_attempts}] {reason}{suffix}"
                        ))),
                        acp::MessageId::new(format!("msg-{}", self.message_counter)),
                    ),
                ))]
            }
            BackendEvent::SubagentStatus {
                tool_call_id,
                content_delta,
                status_text,
                ..
            } => {
                let text = content_delta.as_deref().unwrap_or(status_text);
                vec![self.update(acp::SessionUpdate::ToolCallContentChunk(
                    acp::ToolCallContentChunk::new(
                        tool_call_id.clone(),
                        acp::ToolCallContent::Content(Box::new(acp::Content::new(
                            acp::ContentBlock::Text(acp::TextContent::new(text)),
                        ))),
                    ),
                ))]
            }
            BackendEvent::SubagentCompleted {
                tool_call, result, ..
            } => {
                let mut update = crate::v2_types::tool_call_update(
                    tool_call,
                    Some(acp::ToolCallStatus::Completed),
                );
                update = update
                    .content(crate::v2_types::tool_result_content(tool_call, result))
                    .raw_output(serde_json::Value::String(result.output.clone()));
                vec![self.update(acp::SessionUpdate::ToolCallUpdate(update))]
            }
            BackendEvent::StreamEnd { .. }
            | BackendEvent::InstructionsLoaded { .. }
            | BackendEvent::ContextCompacted { .. }
            | BackendEvent::UndoCompleted { .. }
            | BackendEvent::SidebarSnapshotReady { .. }
            | BackendEvent::MessagesTruncated { .. } => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_types::message::{AssistantTurn, ToolCall};

    fn translator() -> EventTranslator {
        EventTranslator::new(Uuid::nil(), 128_000)
    }

    #[test]
    fn v2_turn_lifecycle_reports_running_and_idle_state() {
        let mut translator = translator();
        let running = translator.translate(&BackendEvent::TurnStarting {
            session_id: Uuid::nil(),
            request_id: 7,
        });
        assert_eq!(running.len(), 1);
        assert_eq!(
            serde_json::to_value(&running[0]).unwrap()["update"]["sessionUpdate"],
            "state_update"
        );

        let idle = translator.translate(&BackendEvent::Finished {
            session_id: Uuid::nil(),
            request_id: 7,
            turn: Box::new(AssistantTurn::default()),
        });
        assert_eq!(idle.len(), 1);
        assert_eq!(
            serde_json::to_value(&idle[0]).unwrap()["update"]["state"],
            "idle"
        );
    }

    #[test]
    fn v2_shell_start_reports_terminal_metadata() {
        let mut translator = translator();
        let updates = translator.translate(&BackendEvent::ToolStarting {
            session_id: Uuid::nil(),
            request_id: 1,
            tool_call: ToolCall {
                id: "shell-1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"command":"printf hello"}"#.to_string(),
                thought_signature: None,
            },
        });
        assert!(updates.iter().any(|update| {
            serde_json::to_value(update).unwrap()["update"]["sessionUpdate"] == "terminal_update"
        }));
    }
}
