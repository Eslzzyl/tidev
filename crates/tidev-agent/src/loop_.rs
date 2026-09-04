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

use tidev_llm::message::{AssistantTurn, Message, MessageRole};

use crate::context::{AgentContext, AgentLoopConfig};
use crate::event::{AgentEvent, StreamEndStatus};

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
        let user_message_id = messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.id);
        ctx.emit_stream_event(AgentEvent::TurnStarting {
            request_id,
            user_message_id,
            assistant_message_id: None,
        })
        .await?;

        // ─── 5. Compose system prompt ─────────────────────────────────────
        let system_prompt = config.system_prompt.clone();

        // ─── 6. Stream LLM turn ──────────────────────────────────────────
        // Per-turn thinking level: prefer the last user message's level so
        // that “high” sent with a message is both used for the request and
        // shown in the footer. Falls back to the session's default.
        let thinking_level = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.thinking_level.clone())
            .unwrap_or_else(|| config.thinking_level.clone());
        let turn = match ctx
            .stream_turn(&messages, &system_prompt, &thinking_level, request_id)
            .await
        {
            Ok(turn) => turn,
            Err(_e) if config.cancel.is_cancelled() => {
                ctx.emit_stream_event(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: None,
                    reasoning_completed_at: None,
                    status: StreamEndStatus::Cancelled,
                })
                .await?;
                return Ok(());
            }
            Err(e) => {
                ctx.emit_stream_event(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: None,
                    reasoning_completed_at: None,
                    status: StreamEndStatus::Failed,
                })
                .await?;
                return Err(e);
            }
        };

        // ─── 7. No tool calls → check for steered messages ───────────────
        if turn.tool_calls.is_empty() {
            let msg = build_assistant_message(&turn);
            ctx.save_messages(session_id, &[msg]).await?;

            // Check for user messages steered into this session while the
            // turn was running. Steering messages are persisted to the
            // buffer by the host immediately — the signal only keeps the
            // loop alive so the next load_messages() picks them up.
            //
            // Queued (non-steered) messages do NOT set this signal: the
            // host drains them after the loop exits and starts a fresh
            // loop for the next turn.
            if config
                .steer_signal
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                // Finalise the current turn before starting a new one.
                ctx.emit_stream_event(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: turn.reasoning_started_at,
                    reasoning_completed_at: turn.reasoning_completed_at,
                    status: StreamEndStatus::Completed,
                })
                .await?;
                // TurnStarting for the next iteration is emitted by
                // step 4 inside the loop body.
                continue;
            }

            ctx.emit_stream_event(AgentEvent::StreamEnd {
                request_id,
                reasoning_started_at: turn.reasoning_started_at,
                reasoning_completed_at: turn.reasoning_completed_at,
                status: StreamEndStatus::Completed,
            })
            .await?;
            return Ok(());
        }

        // ─── 8. Persist assistant message (with tool calls) ──────────────
        let assistant_msg = build_assistant_message(&turn);
        ctx.save_messages(session_id, &[assistant_msg]).await?;

        // ─── 9. Approve and execute tools ─────────────────────────────────
        // Approval is host policy. The generic loop receives one ordered
        // result stream so rejected results remain before executed results.
        let all_results = ctx
            .execute_tools(&turn.tool_calls, session_id, request_id)
            .await?;

        // ─── 11. Persist tool results ─────────────────────────────────────
        if !all_results.is_empty() {
            let result_msgs: Vec<Message> = all_results
                .iter()
                .map(|(tool_call, result)| {
                    Message::tool_result(&tool_call.id, &tool_call.name, result.clone())
                })
                .collect();
            ctx.save_messages(session_id, &result_msgs).await?;
        }

        // ─── 12. Prepare for next turn ────────────────────────────────────
        let status = if config.cancel.is_cancelled() {
            StreamEndStatus::Cancelled
        } else {
            StreamEndStatus::Completed
        };
        ctx.emit_stream_event(AgentEvent::StreamEnd {
            request_id,
            reasoning_started_at: turn.reasoning_started_at,
            reasoning_completed_at: turn.reasoning_completed_at,
            status,
        })
        .await?;
        if status == StreamEndStatus::Cancelled {
            return Ok(());
        }
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
    msg.thinking_level = turn.thinking_level.clone();
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
