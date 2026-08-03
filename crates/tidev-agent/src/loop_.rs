//! The agent loop — the core LLM ↔ tool execution loop skeleton.
//!
//! This module contains the [`run_agent_loop`] function that drives the main
//! interaction loop: stream an LLM turn, execute any tool calls, persist
//! results, and repeat until the model produces a final response.
//!
//! The loop is generic over [`AgentContext`], which provides the concrete
//! implementations for LLM calls, tool execution, and persistence.

use anyhow::Result;
use chrono::Utc;

use tidev_llm::message::{
    AssistantTurn, Message, MessageRole, ToolCall,
};

use crate::context::{AgentContext, AgentLoopConfig};
use crate::event::AgentEvent;

/// Run the full agent loop until the model produces a text-only response.
///
/// # Flow
///
/// ```text
///  load prepared messages → notify turn starting
///       → compose system prompt → stream LLM turn
///       ↑                                                            │
///       │                                                  tool calls? ──no──→ persist → exit
///       │                                                            │
///       └── persist results ←─ execute tools ←───────────────←───────┘
/// ```
pub async fn run_agent_loop(ctx: &dyn AgentContext, config: AgentLoopConfig) -> Result<()> {
    let session_id = config.session_id;
    let event_tx = &config.event_tx;

    for request_id in 1_u64.. {
        // ─── 0. Cancellation check ──────────────────────────────────────
        if config.cancel.is_cancelled() {
            log::info!("agent loop cancelled for session {session_id}");
            return Ok(());
        }

        // ─── 1. Load messages ────────────────────────────────────────────
        let messages = ctx.load_messages(session_id).await?;

        // ─── 2. Notify frontend that a new turn is starting ───────────────
        // CoreContext has already performed injection while loading, so the
        // streaming assistant message is created after any system notices.
        let _ = event_tx.send(AgentEvent::TurnStarting {
            request_id,
        });

        // ─── 5. Compose system prompt ─────────────────────────────────────
        let system_prompt = config.system_prompt.clone();

        // ─── 6. Stream LLM turn ──────────────────────────────────────────
        let turn = match ctx
            .stream_turn(
                &messages,
                &system_prompt,
                &config.thinking_level,
                request_id,
            )
            .await
        {
            Ok(turn) => turn,
            Err(_e) if config.cancel.is_cancelled() => {
                // Cancellation is expected — exit cleanly.
                // Already-streamed content was forwarded to the TUI via events;
                // the TUI will mark the message as done and append a cancel note.
                let _ = event_tx.send(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: None,
                    reasoning_completed_at: None,
                });
                return Ok(());
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: None,
                    reasoning_completed_at: None,
                });
                return Err(e);
            }
        };

        // ─── 7. No tool calls → check for queued messages ────────────────
        if turn.tool_calls.is_empty() {
            let msg = build_assistant_message(&turn);
            ctx.save_messages(session_id, &[msg], &[]).await?;

            // Check for user messages queued while this turn was running.
            // The messages themselves are already persisted in the buffer
            // by submit_prompt_with_attachments — the queue entries serve
            // as a signal: "there's new work, keep the loop alive".
            //
            // We drain ALL queued entries at once since load_messages()
            // will include every persisted message regardless.  A single
            // extra iteration suffices.
            let has_queued = {
                let mut queue = config.queued_messages.lock().unwrap();
                if queue.is_empty() {
                    false
                } else {
                    queue.clear();
                    true
                }
            };

            if has_queued {
                // Finalise the current turn before starting a new one.
                let _ = event_tx.send(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: turn.reasoning_started_at,
                    reasoning_completed_at: turn.reasoning_completed_at,
                });
                // TurnStarting for the next iteration is emitted by
                // step 4 inside the loop body.
                continue;
            }

            let _ = event_tx.send(AgentEvent::StreamEnd {
                request_id,
                reasoning_started_at: turn.reasoning_started_at,
                reasoning_completed_at: turn.reasoning_completed_at,
            });
            return Ok(());
        }

        // ─── 8. Persist assistant message (with tool calls) ──────────────
        let assistant_msg = build_assistant_message(&turn);
        ctx.save_messages(session_id, &[assistant_msg], &[]).await?;

        // ─── 9. Permission approval ──────────────────────────────────────
        let approved = ctx
            .request_tool_approval(&turn.tool_calls, config.read_only)
            .await?;

        let mut task_calls: Vec<(ToolCall, Option<uuid::Uuid>)> = Vec::new();
        let mut other_calls: Vec<(ToolCall, bool, bool)> = Vec::new();
        let mut rejected_msgs: Vec<Message> = Vec::new();

        for approved_tool in &approved {
            if let Some(rejection) = &approved_tool.rejection {
                // Rejected tool: persist rejection as tool result
                let result_msg = Message::tool_result(
                    &approved_tool.tool_call.id,
                    &approved_tool.tool_call.name,
                    rejection.clone(),
                );
                rejected_msgs.push(result_msg);
                let _ = event_tx.send(AgentEvent::ToolCompleted {
                    request_id,
                    tool_call: approved_tool.tool_call.clone(),
                    result: Box::new(rejection.clone()),
                    child_session_id: None,
                });
            } else if approved_tool.tool_call.name == "task" {
                task_calls.push((
                    approved_tool.tool_call.clone(),
                    approved_tool.child_session_id,
                ));
            } else {
                other_calls.push((
                    approved_tool.tool_call.clone(),
                    approved_tool.allow_outside,
                    approved_tool.sensitive_file_approved,
                ));
            }
        }

        if !rejected_msgs.is_empty() {
            ctx.save_messages(session_id, &rejected_msgs, &[]).await?;
        }

        // ─── 10. Execute tools ───────────────────────────────────────────
        let mut all_results = Vec::new();

        if !other_calls.is_empty() || !task_calls.is_empty() {
            let results = ctx.execute_tools(&approved, session_id, request_id).await?;
            all_results = results;
        }

        // ─── 11. Persist tool results ─────────────────────────────────────
        if !all_results.is_empty() {
            let result_msgs: Vec<Message> = all_results
                .iter()
                .map(|execution| {
                    Message::tool_result(
                        &execution.tool_call.id,
                        &execution.tool_call.name,
                        execution.result.clone(),
                    )
                })
                .collect();
            let child_session_ids: Vec<(uuid::Uuid, uuid::Uuid)> = result_msgs
                .iter()
                .zip(&all_results)
                .filter_map(|(message, execution)| {
                    execution
                        .child_session_id
                        .map(|child_id| (message.id, child_id))
                })
                .collect();
            ctx.save_messages(session_id, &result_msgs, &child_session_ids)
                .await?;

        }

        // ─── 12. Prepare for next turn ────────────────────────────────────
        let _ = event_tx.send(AgentEvent::StreamEnd {
            request_id,
            reasoning_started_at: turn.reasoning_started_at,
            reasoning_completed_at: turn.reasoning_completed_at,
        });
        // TurnStarting for the next iteration is emitted by
        // step 4 inside the loop body.
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a [`Message`] from an [`AssistantTurn`].
fn build_assistant_message(turn: &AssistantTurn) -> Message {
    let created_at = turn.created_at.unwrap_or_else(Utc::now);
    let completed_at = turn.completed_at.unwrap_or_else(Utc::now);
    let mut msg = Message::persisted(
        uuid::Uuid::new_v4(),
        MessageRole::Assistant,
        &turn.content,
        created_at,
        false,
    );
    msg.completed_at = Some(completed_at);
    msg.reasoning = turn.reasoning.clone();
    msg.tool_calls = turn.tool_calls.clone();
    msg.metadata.responses_output_items = turn.responses_output_items.clone();
    msg.input_tokens = turn.input_tokens;
    msg.output_tokens = turn.output_tokens;
    msg.total_tokens = turn.total_tokens;
    msg.cache_read_tokens = turn.cache_read_tokens;
    msg.cache_write_tokens = turn.cache_write_tokens;
    msg.model_id = turn.model_id.clone();
    msg.tokens_per_second = turn.tokens_per_second;
    msg.reasoning_started_at = turn.reasoning_started_at;
    msg.reasoning_completed_at = turn.reasoning_completed_at;
    msg
}
