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
    AssistantTurn, Message, MessageRole, ToolCall, ToolExecutionResult,
};

use crate::context::{AgentContext, AgentLoopConfig};
use crate::event::AgentEvent;

/// Run the full agent loop until the model produces a text-only response.
///
/// # Flow
///
/// ```text
///  load messages → inject instructions → inject mode reminder → notify turn starting
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
        let mut messages = ctx.load_messages(session_id).await?;
        // CoreContext performs injection while loading; retain the returned
        // source list for the tool-result replay notification below.
        let already_injected = ctx.inject_instructions(session_id, &mut messages).await?;

        // ─── 2. Notify frontend that a new turn is starting ───────────────
        // Placed after instruction/mode injection so the TUI creates the
        // streaming assistant message AFTER any system notification messages
        // emitted by inject_instructions, keeping the correct visual order.
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
            ctx.save_messages(session_id, &[msg]).await?;

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
        ctx.save_messages(session_id, &[assistant_msg]).await?;

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
            ctx.save_messages(session_id, &rejected_msgs).await?;
        }

        // ─── 10. Execute tools ───────────────────────────────────────────
        let mut all_results: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();

        if !other_calls.is_empty() || !task_calls.is_empty() {
            let results = ctx.execute_tools(&approved, session_id, request_id).await?;
            all_results = results;
        }

        // ─── 11. Persist tool results ─────────────────────────────────────
        if !all_results.is_empty() {
            let result_msgs: Vec<Message> = all_results
                .iter()
                .map(|(tc, r)| Message::tool_result(&tc.id, &tc.name, r.clone()))
                .collect();
            ctx.save_messages(session_id, &result_msgs).await?;

            // Persist nearby instruction sources discovered during tool
            // execution so the TUI can restore dedup tracking across
            // session switches without writing to the DB itself.
            for (_, result) in &all_results {
                if !result.instruction_sources.is_empty() {
                    ctx.append_instruction_sources(session_id, &result.instruction_sources)
                        .await?;
                }
            }

            // Also persist a "Loaded instructions from" System message for
            // correct cross-session replay in the conversation history.
            // Only create it for sources NOT already known before this turn
            // (already_injected — captured from step 2's DB snapshot).
            let system_sources: Vec<String> = all_results
                .iter()
                .flat_map(|(_, r)| r.instruction_sources.iter().cloned())
                .collect();
            if !system_sources.is_empty() {
                let mut unique = system_sources;
                unique.sort();
                unique.dedup();

                let new_sources: Vec<String> = unique
                    .into_iter()
                    .filter(|s| !already_injected.contains(s))
                    .collect();
                if !new_sources.is_empty() {
                    let ws_root = ctx.workspace_root();
                    let display: Vec<String> = new_sources
                        .iter()
                        .map(|s| {
                            Path::new(s)
                                .strip_prefix(ws_root)
                                .unwrap_or(Path::new(s))
                                .display()
                                .to_string()
                        })
                        .collect();
                    let content = if display.len() == 1 {
                        format!("Loaded instructions from {}", display[0])
                    } else {
                        format!(
                            "Loaded {} instruction files: {}",
                            display.len(),
                            display.join(", ")
                        )
                    };
                    ctx.save_messages(session_id, &[Message::new(MessageRole::System, &content)])
                        .await?;
                }
            }
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
use std::path::Path;
