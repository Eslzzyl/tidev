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

use tidev_types::message::{
    AssistantTurn, BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
};

use crate::context::{AgentContext, AgentLoopConfig};
use crate::prompts;

/// Maximum number of LLM turns before the loop terminates (safety limit).
const MAX_TURNS: u64 = 50;

/// Run the full agent loop until the model produces a text-only response or
/// the maximum number of turns is reached.
///
/// # Flow
///
/// ```text
///  load messages → inject mode reminder → compose system prompt → stream LLM turn
///       ↑                                                            │
///       │                                                  tool calls? ──no──→ persist → exit
///       │                                                            │
///       └── persist results ←─ execute tools ←───────────────←───────┘
/// ```
pub async fn run_agent_loop(ctx: &dyn AgentContext, config: AgentLoopConfig) -> Result<()> {
    let session_id = config.session_id;
    let event_tx = &config.event_tx;
    let mut request_id: u64 = 1;

    // Notify frontend that a new turn is starting.
    let _ = event_tx.send(BackendEvent::TurnStarting {
        session_id,
        request_id,
    });

    for _turn_index in 0..MAX_TURNS {
        // ─── 0. Cancellation check ──────────────────────────────────────
        if config.cancel.is_cancelled() {
            log::info!("agent loop cancelled for session {session_id}");
            return Ok(());
        }

        // ─── 1. Load messages ────────────────────────────────────────────
        let mut messages = ctx.load_messages(session_id).await?;

        // ─── 2. Inject mode reminder into the last user message ───────────
        inject_mode_reminder(ctx, session_id, &mut messages, config.mode).await?;

        // ─── 3. Compose system prompt (no mode reminder — injected above) ─
        let system_prompt = config.definition.system_prompt.clone();

        // ─── 4. Stream LLM turn ──────────────────────────────────────────
        let turn = match ctx
            .stream_turn(&messages, &system_prompt, &config.thinking_level)
            .await
        {
            Ok(turn) => turn,
            Err(_e) if config.cancel.is_cancelled() => {
                // Cancellation is expected — exit cleanly.
                // Already-streamed content was forwarded to the TUI via events;
                // the TUI will mark the message as done and append a cancel note.
                let _ = event_tx.send(BackendEvent::StreamEnd {
                    session_id,
                    request_id,
                });
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        // ─── 5. No tool calls → done ─────────────────────────────────────
        if turn.tool_calls.is_empty() {
            let msg = build_assistant_message(&turn);
            ctx.save_messages(session_id, &[msg]).await?;

            let _ = event_tx.send(BackendEvent::StreamEnd {
                session_id,
                request_id,
            });
            return Ok(());
        }

        // ─── 6. Persist assistant message (with tool calls) ──────────────
        let assistant_msg = build_assistant_message(&turn);
        ctx.save_messages(session_id, &[assistant_msg]).await?;

        // ─── 7. Permission approval ──────────────────────────────────────
        let approved = ctx
            .request_tool_approval(&turn.tool_calls, config.mode)
            .await?;

        let mut task_calls: Vec<(ToolCall, Option<uuid::Uuid>)> = Vec::new();
        let mut other_calls: Vec<(ToolCall, bool, bool)> = Vec::new();

        for approved_tool in &approved {
            if let Some(rejection) = &approved_tool.rejection {
                // Rejected tool: persist rejection as tool result
                let result_msg = Message::tool_result(
                    &approved_tool.tool_call.id,
                    &approved_tool.tool_call.name,
                    rejection.clone(),
                );
                ctx.save_messages(session_id, &[result_msg]).await?;
                let _ = event_tx.send(BackendEvent::ToolCompleted {
                    session_id,
                    request_id,
                    tool_call: approved_tool.tool_call.clone(),
                    result: rejection.clone(),
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

        // ─── 8. Execute tools ────────────────────────────────────────────
        let mut all_results: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();

        if !other_calls.is_empty() || !task_calls.is_empty() {
            let results = ctx
                .execute_tools(
                    &approved,
                    session_id,
                    request_id,
                )
                .await?;
            all_results = results;
        }

        // ─── 9. Persist tool results ──────────────────────────────────────
        for (tool_call, result) in &all_results {
            let result_msg = Message::tool_result(
                &tool_call.id,
                &tool_call.name,
                result.clone(),
            );
            ctx.save_messages(session_id, &[result_msg]).await?;
        }

        // ─── 10. Prepare for next turn ────────────────────────────────────
        request_id += 1;
        let _ = event_tx.send(BackendEvent::TurnStarting {
            session_id,
            request_id,
        });
    }

    // Safety limit reached — log and exit gracefully.
    log::warn!("run_agent_loop: reached MAX_TURNS ({})", MAX_TURNS);
    Ok(())
}

// ---------------------------------------------------------------------------
// Mode reminder injection
// ---------------------------------------------------------------------------

/// Inject a mode reminder into the last user message if needed.
///
/// Mirrors the old v0.6.x `inject_mode_reminder` logic:
/// - First user message → inject `mode_reminder(current_mode)`
/// - Mode changed from previous user message → inject `plan_switch_reminder()`
///   or `build_switch_reminder()`
/// - Same mode → no injection
///
/// The reminder is prepended to the message content and persisted to both
/// the in-memory buffer and the store so subsequent turns see it.
async fn inject_mode_reminder(
    ctx: &dyn AgentContext,
    session_id: uuid::Uuid,
    messages: &mut Vec<Message>,
    current_mode: tidev_types::prompts::SessionMode,
) -> Result<()> {
    // Find the last user message index.
    let last_user_idx = match messages.iter().rposition(|m| m.role == MessageRole::User) {
        Some(idx) => idx,
        None => return Ok(()),
    };

    // Find the mode of the most recent *previous* user message.
    let prev_mode = messages[..last_user_idx]
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::User)
        .and_then(|m| m.mode);

    let is_first_user = prev_mode.is_none();

    let reminder: Option<String> = match (is_first_user, prev_mode) {
        (true, _) => Some(prompts::mode_reminder(current_mode).to_string()),
        (false, Some(prev)) if prev != current_mode => Some(match current_mode {
            tidev_types::prompts::SessionMode::Plan => prompts::plan_switch_reminder(),
            tidev_types::prompts::SessionMode::Build => prompts::build_switch_reminder(),
        }),
        _ => None,
    };

    let Some(text) = reminder else {
        return Ok(());
    };

    // De-duplicate: skip if the content already starts with this reminder.
    if messages[last_user_idx].content.starts_with(&text) {
        return Ok(());
    }

    // Prepend the reminder.
    let new_content = format!("{text}\n\n{}", messages[last_user_idx].content);

    // Update in-memory message.
    let msg_id = messages[last_user_idx].id;
    messages[last_user_idx].content = new_content.clone();

    // Persist to buffer + store.
    ctx.update_message_content(session_id, msg_id, new_content).await?;

    log::info!(
        "injected mode reminder into user message {} (mode={:?}, is_first={})",
        msg_id,
        current_mode,
        is_first_user,
    );

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
    msg.input_tokens = turn.input_tokens;
    msg.output_tokens = turn.output_tokens;
    msg.total_tokens = turn.total_tokens;
    msg.cache_read_tokens = turn.cache_read_tokens;
    msg.cache_write_tokens = turn.cache_write_tokens;
    msg.model_id = turn.model_id.clone();
    msg.tokens_per_second = turn.tokens_per_second;
    msg
}
