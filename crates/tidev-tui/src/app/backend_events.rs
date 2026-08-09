use super::*;

use chrono::Utc;
use std::collections::HashSet;
use tidev_core::BackendEvent;
use uuid::Uuid;

use crate::utils::strip_system_reminder_tags;

impl App {
    /// Handle a backend event from the agent loop (streaming, tool results, etc.).
    pub(crate) fn handle_backend_event(&mut self, event: BackendEvent) {
        // Forward to MessageList for all chat-related events.
        if let Some(ref mut chat) = self.message_list {
            chat.handle_backend_event(&event);
        }

        match event {
            BackendEvent::UsageStats {
                session_id,
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                duration_ms,
                ..
            } if Some(session_id) == self.current_session_id => {
                // Store context usage for display in status bar.
                let usage = ContextUsage {
                    input_tokens,
                    output_tokens,
                    tokens_per_second: if let Some(ms) = duration_ms {
                        if ms > 0 {
                            Some(output_tokens as f32 / (ms as f32 / 1000.0))
                        } else {
                            None
                        }
                    } else {
                        None
                    },
                };
                self.context_usage_cache.insert(session_id, usage.clone());
                self.context_usage = Some(usage);

                // Update the last message's token fields.
                if let Some(ref mut chat) = self.message_list {
                    chat.set_last_message_tokens(
                        Some(input_tokens),
                        Some(output_tokens),
                        Some(total_tokens),
                        Some(cache_read_tokens),
                        Some(cache_write_tokens),
                        self.context_usage
                            .as_ref()
                            .and_then(|u| u.tokens_per_second),
                        Some(model_id.clone()),
                        Some(Utc::now()),
                        Some(self.mode),
                    );
                }

                // Token usage is persisted with the completed assistant message by
                // tidev-core; this event updates the live UI state only.
            }
            BackendEvent::InstructionsLoaded {
                session_id,
                sources,
            } => {
                log::info!("Instructions loaded: {sources:?}");
                if Some(session_id) == self.current_session_id && !sources.is_empty() {
                    self.show_instruction_sources(&sources);
                }
            }
            BackendEvent::Retrying {
                session_id,
                attempt,
                max_attempts,
                reason,
                ..
            } => {
                log::info!("Retrying (attempt {attempt}/{max_attempts}): {reason}");
                if Some(session_id) == self.current_session_id {
                    self.set_toast(
                        format!("Retry {attempt}/{max_attempts}: {reason}"),
                        std::time::Duration::from_secs(5),
                    );
                }
            }
            BackendEvent::Failed {
                session_id, error, ..
            } => {
                log::error!("Request failed for session {session_id}: {error}");
                // Clean up pending state for this session.
                self.pending_approvals.remove(&session_id);
                if self.active_approval_session == Some(session_id) {
                    self.active_approval_session = None;
                }
                self.pending_modes.remove(&session_id);

                // Mark the last streaming message as error.
                if let Some(ref mut chat) = self.message_list {
                    chat.mark_streaming_as_error(&error);
                }

                if Some(session_id) == self.current_session_id {
                    self.set_toast(
                        format!("Request failed: {error}"),
                        std::time::Duration::from_secs(8),
                    );
                }
                self.desktop_notifications
                    .notify(&format!("Request failed: {error}"));
            }
            BackendEvent::Finished {
                session_id, turn, ..
            } => {
                // Apply pending mode switch on final turn (no tool calls).
                if turn.tool_calls.is_empty() {
                    if let Some(new_mode) = self.pending_modes.remove(&session_id)
                        && Some(session_id) == self.current_session_id
                    {
                        self.mode = new_mode;
                        self.set_notice(format!("Mode switched to {}", self.mode.title()));
                    }
                    if Some(session_id) == self.current_session_id {
                        self.desktop_notifications.notify("Response complete");
                    }
                }

                // If a compact was queued and no request is active, run it now.
                if self.pending_compacts.remove(&session_id)
                    && Some(session_id) == self.current_session_id
                    && !self.has_active_request()
                {
                    self.execute_compact();
                }
            }
            BackendEvent::ContextCompacted {
                session_id,
                error: Some(ref msg),
                ..
            } => {
                self.compacting_sessions.remove(&session_id);
                self.set_notice(format!("Compaction failed: {msg}"));
            }
            BackendEvent::ContextCompacted {
                session_id,
                error: None,
                ..
            } => {
                self.compacting_sessions.remove(&session_id);
                self.set_notice("Context compacted");
            }
            BackendEvent::UserMessageCreated {
                session_id,
                message,
                app_data,
                queued,
            } => {
                if queued {
                    // Submitted while the session was busy (queued or
                    // steered). The message is not yet part of any request —
                    // show it as a pending preview above the composer until
                    // the next TurnStarting commits it into history.
                    // The delivery mode is derived from the runtime config
                    // that the submission was made under.
                    let steered = self.runtime.config().ui.send_while_busy
                        == tidev_config::SendWhileBusy::Steer;
                    self.add_pending_input(session_id, *message, *app_data, steered);
                } else if let Some(ref mut chat) = self.message_list {
                    if let Some(ref mut ctx) = chat.active_chat_context_mut()
                        && ctx.session_id == session_id
                    {
                        ctx.push_with_app_data(*message, *app_data);
                    }
                    chat.invalidate_layout();
                }
            }
            BackendEvent::MessagesTruncated { session_id, .. } => {
                if Some(session_id) == self.current_session_id
                    && let Some(ref mut chat) = self.message_list
                {
                    if let Some(ref mut ctx) = chat.active_chat_context_mut()
                        && ctx.session_id == session_id
                    {
                        // Use the locally-held revert_message_id to determine the
                        // truncation point instead of relying on `kept_count` from
                        // the runtime buffer.  The buffer and ctx.messages are
                        // independent Vecs; their positions can diverge (e.g. when
                        // set_message_buffer overwrites the buffer with stale data
                        // or when the agent loop appends messages between undo and
                        // send).  Truncating by revert_message_id is always correct
                        // for the TUI's own message list.
                        if let Some(revert_id) = ctx.revert_message_id
                            && let Some(pos) = ctx.messages.iter().position(|m| m.id == revert_id)
                        {
                            ctx.messages.truncate(pos);
                        }
                        ctx.revert_message_id = None;
                    }
                    chat.invalidate_layout();
                }
            }
            BackendEvent::UndoCompleted {
                target_id,
                message_content,
                ..
            } => {
                if let Some(ref mut chat) = self.message_list {
                    if let Some(ref mut ctx) = chat.active_chat_context_mut() {
                        if target_id == Uuid::nil() {
                            ctx.revert_message_id = None;
                        } else {
                            ctx.revert_message_id = Some(target_id);
                        }
                    }
                    chat.follow_tail = true;
                    chat.invalidate_layout();
                }
                if let Some(ref mut composer) = self.composer {
                    if !message_content.is_empty() {
                        composer.set_text(strip_system_reminder_tags(&message_content));
                    } else {
                        composer.clear();
                    }
                }
                self.set_notice("Undo complete");
            }
            BackendEvent::ToolCompleted {
                session_id,
                ref tool_call,
                ..
            } => {
                // todowrite-specific: reload todos from database.
                // Guard with current_session_id so that a subagent's todowrite
                // in a child session doesn't overwrite the parent's sidebar.
                if tool_call.name == "todowrite"
                    && Some(session_id) == self.current_session_id
                    && let Ok(todos) = self
                        .runtime
                        .session_manager()
                        .store()
                        .load_todos(session_id)
                {
                    self.todos = todos;
                }
            }
            BackendEvent::TurnStarting { session_id, .. } => {
                // Commit queued/steered user messages into the visible
                // history. This request's assistant placeholder was already
                // pushed by the chat component's own TurnStarting handling,
                // so insert the pending messages before it.
                let is_active = self
                    .message_list
                    .as_ref()
                    .and_then(|c| c.active_chat_context())
                    .is_some_and(|ctx| ctx.session_id == session_id);
                if is_active {
                    let pending = self.take_pending_inputs(session_id);
                    if !pending.is_empty()
                        && let Some(ref mut chat) = self.message_list
                        && let Some(ref mut ctx) = chat.active_chat_context_mut()
                        && ctx.session_id == session_id
                    {
                        let existing: HashSet<Uuid> = ctx.messages.iter().map(|m| m.id).collect();
                        let mut insert_at = ctx
                            .messages
                            .iter()
                            .rposition(|m| m.streaming)
                            .unwrap_or(ctx.messages.len());
                        for p in pending {
                            // Skip messages already present (e.g. loaded from
                            // the store after a session switch).
                            if existing.contains(&p.message.id) {
                                continue;
                            }
                            let message_id = p.message.id;
                            ctx.messages.insert(insert_at, p.message);
                            ctx.message_app_data.insert(message_id, p.app_data);
                            insert_at += 1;
                        }
                        chat.invalidate_layout();
                    }
                }
            }
            BackendEvent::StreamEnd { session_id, .. }
                if self.pending_compacts.remove(&session_id) && !self.has_active_request() =>
            {
                // If a compact was queued and no request is active, run it now.
                self.execute_compact();
            }
            BackendEvent::StreamEnd { .. } => {}
            _ => {
                // Events already forwarded to MessageList above:
                //   Delta, ReasoningDelta, ToolCallUpdated, Finished, ToolCompleted,
                //   SubagentStatus, SidebarSnapshotReady,
                //   ShellOutput, ContextCompacted
            }
        }
    }
}
