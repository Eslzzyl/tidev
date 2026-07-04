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
///  load messages → compose system prompt → stream LLM turn
///       ↑                                      │
///       │                            tool calls? ──no──→ persist → exit
///       │                                      │
///       └── persist results ←─ execute tools ←─┘
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
        let messages = ctx.load_messages(session_id).await?;

        // ─── 2. Compose system prompt ────────────────────────────────────
        let system_prompt = compose_system_prompt(&config, &messages);

        // ─── 3. Stream LLM turn ──────────────────────────────────────────
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

        // ─── 4. No tool calls → done ─────────────────────────────────────
        if turn.tool_calls.is_empty() {
            let msg = build_assistant_message(&turn);
            ctx.save_messages(session_id, &[msg]).await?;

            let _ = event_tx.send(BackendEvent::StreamEnd {
                session_id,
                request_id,
            });
            return Ok(());
        }

        // ─── 5. Persist assistant message (with tool calls) ──────────────
        let assistant_msg = build_assistant_message(&turn);
        ctx.save_messages(session_id, &[assistant_msg]).await?;

        // ─── 6. Permission approval ──────────────────────────────────────
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

        // ─── 7. Execute tools ────────────────────────���───────────────────
        // Execute all approved non-task tools. The context implementation
        // handles parallel/serial execution internally.
        let mut all_results: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();

        if !other_calls.is_empty() || !task_calls.is_empty() {
            // Convert approved tools back to the format expected by execute_tools.
            // We pass ALL approved tools (including rejected ones already handled).
            let results = ctx
                .execute_tools(
                    &approved,
                    session_id,
                    request_id,
                )
                .await?;
            all_results = results;
        }

        // ─── 8. Persist tool results ──────────────────────────────────────
        for (tool_call, result) in &all_results {
            let result_msg = Message::tool_result(
                &tool_call.id,
                &tool_call.name,
                result.clone(),
            );
            ctx.save_messages(session_id, &[result_msg]).await?;
        }

        // ─── 9. Prepare for next turn ────────────────────────────────────
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
// Helpers
// ---------------------------------------------------------------------------

/// Compose the system prompt from the agent definition, mode reminder, and
/// any additional context.
fn compose_system_prompt(config: &AgentLoopConfig, _messages: &[Message]) -> String {
    let mode_reminder = prompts::mode_reminder(config.mode);

    format!(
        "{}\n\n{}",
        config.definition.system_prompt,
        mode_reminder,
    )
}

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
