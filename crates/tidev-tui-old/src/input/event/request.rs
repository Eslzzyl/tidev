use super::*;
use tidev_types::message::{Message, MessageRole, ToolExecutionResult};

impl App {
    pub(crate) fn handle_request_abort_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<bool> {
        if key.code != KeyCode::Esc || !self.ui.pending_request {
            return Ok(false);
        }

        if self.ui.abort_confirmation_deadline
            .is_some_and(|deadline| deadline > Instant::now())
        {
            self.abort_current_request();
            return Ok(true);
        }

        self.ui.abort_confirmation_deadline = Some(Instant::now() + Duration::from_secs(3));
        self.ui.last_notice =
            Some("Press Esc again within 3 seconds to stop the current request".to_string());
        Ok(true)
    }

    pub(crate) fn is_active_request(&self, request_id: u64) -> bool {
        request_id == self.ui.active_request_id
    }

    pub(crate) fn cancel_running_subagents(&mut self) {
        self.ui.running_subagent_executions.clear();
    }

    pub(crate) fn abort_current_request(&mut self) {
        self.ui.active_request_id = self.ui.active_request_id.wrapping_add(1);
        self.ui.abort_confirmation_deadline = None;
        self.ui.pending_request = false;
        self.ui.pending_mode = None;

        // Cancel the agent loop so it stops at its next check point.
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            runtime.cancel().await;
        });

        // Drop the permission channel sender so the agent loop unblocks
        // if it's waiting for a permission approval response.
        self.ui.pending_permission_response = None;

        // Also drop the permission channel receiver.  Any queued
        // PendingToolApproval in the channel is dropped along with its
        // oneshot sender, causing resp_rx.await in the agent loop to
        // return Err and exit.
        self.ui.pending_permission_rx = None;

        // Clear the display queue.  Messages in the runtime queue
        // (agent.queued_messages) survive — the next spawned agent
        // loop picks them up after its first turn.
        self.ui.pending_prompt_queue.clear();
        self.ui.pending_tool_execution = None;
        self.ui.permission_dialog = None;
        self.ui.question_dialog = None;
        self.ui.fork_confirm_dialog = None;

        // Handle subagent cancellations: record "User cancelled" tool results
        // so the parent agent's tool call has a matching tool message.
        // This prevents orphaned tool calls that some providers (e.g. OpenAI) reject.
        if !self.ui.running_subagent_executions.is_empty() {
            let parent_session_id = self.ui.running_subagent_executions[0].parent_session_id;
            let current_session_id = self.ui.chat_context.session_id;
            let is_in_subsession = current_session_id != parent_session_id;
            let cancel_output = "User cancelled the request".to_string();

            for execution in self.ui.running_subagent_executions.drain(..) {
                let result = ToolExecutionResult::new(cancel_output.clone());
                let msg = Message::tool_result(
                    execution.tool_call.id.clone(),
                    execution.tool_call.name.clone(),
                    result,
                );

                // Store in DB for parent session
                let _ = self.runtime.session_manager().append_message(parent_session_id, &msg);

                if is_in_subsession {
                    // Also push to cached parent conversation so it's in-memory when restored
                    if let Some(cached) = self.ui.cached_sessions.get_mut(&parent_session_id) {
                        cached.messages.push(msg.clone());
                    }
                } else {
                    // We're on the parent session: push to in-memory conversation
                    self.ui.chat_context.messages.push(msg.clone());
                }
            }

            if is_in_subsession {
                self.ui.pending_assistant_turns.insert(parent_session_id);
            }
        }

        self.cancel_running_subagents();

        // Clean up per-batch boundary approvals
        self.ui.workspace_boundary_approved.clear();

        // Handle running tool execution cancellations: same orphan prevention
        if !self.ui.running_tool_executions.is_empty() {
            let session_id = self.ui.chat_context.session_id;

            for running in self.ui.running_tool_executions.drain(..) {
                // For bash calls, capture partial output that was already
                // streamed via ShellOutput events before the drain.
                let cancel_output = if running.tool_call.name == "bash" {
                    if let Some(idx) = self.ui.chat_context.messages.iter().rposition(|m| {
                        m.role == MessageRole::Tool
                            && m.streaming
                            && m.tool_call_id.as_deref() == Some(&running.tool_call.id)
                    }) {
                        let partial = &self.ui.chat_context.messages[idx].content;
                        if partial.is_empty() {
                            "User cancelled the request".to_string()
                        } else {
                            format!(
                                "User cancelled\n\nPartial output before termination:\n{}",
                                partial
                            )
                        }
                    } else {
                        "User cancelled the request".to_string()
                    }
                } else {
                    "User cancelled the request".to_string()
                };

                let result = ToolExecutionResult::new(cancel_output);
                let msg = Message::tool_result(
                    running.tool_call.id.clone(),
                    running.tool_call.name.clone(),
                    result,
                );

                let _ = self.runtime.session_manager().append_message(session_id, &msg);
                self.ui.chat_context.messages.push(msg);
            }
        }

        if let Some(message) = self.ui.chat_context.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.role = MessageRole::Error;
            message.streaming = false;
            // Keep original reasoning and content intact to preserve thinking at interruption point
            let persisted = message.clone();
            let message_id = message.id;
            let _ = self.runtime.session_manager().store()
                .append_message(self.ui.chat_context.session_id, &persisted);
            self.invalidate_active_message_render_cache_for(message_id);
        }

        self.ui.last_notice = Some("Request cancelled".to_string());
    }

    /// 获取消息在 conversation.messages 中的索引
    pub(crate) fn get_message_index(&self, message_id: Uuid) -> Option<usize> {
        self.ui.chat_context
            .messages
            .iter()
            .position(|m| m.id == message_id)
    }
}
