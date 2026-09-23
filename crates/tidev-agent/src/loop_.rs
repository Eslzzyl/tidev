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
    let mut continue_after_compaction_summary = false;
    for request_id in 1_u64.. {
        // ─── 0. Cancellation check ──────────────────────────────────────
        if config.cancel.is_cancelled() {
            log::info!("agent loop cancelled for session {session_id}");
            return Ok(());
        }

        // ─── 1. Materialize and load messages ────────────────────────────
        let preparation = match ctx.prepare_request(session_id).await {
            Ok(preparation) => preparation,
            Err(error) => {
                let error_text = error.to_string();
                ctx.emit_stream_event(AgentEvent::Failed {
                    request_id,
                    error: error_text,
                    retryable: false,
                })
                .await?;
                return Err(error);
            }
        };
        if preparation.auto_compacted {
            continue_after_compaction_summary = !preparation.summary_had_tool_calls;
            if preparation.summary_had_tool_calls {
                ctx.append_compaction_continuation(
                    session_id,
                    compaction_continuation_message(session_id, true),
                )
                .await?;
            }
        }
        let messages = match ctx.load_messages(session_id).await {
            Ok(messages) => messages,
            Err(error) => {
                let error_text = error.to_string();
                ctx.emit_stream_event(AgentEvent::Failed {
                    request_id,
                    error: error_text,
                    retryable: false,
                })
                .await?;
                return Err(error);
            }
        };
        if messages.is_empty() {
            let error = "cannot start LLM turn: context request message list is empty".to_string();
            ctx.emit_stream_event(AgentEvent::Failed {
                request_id,
                error: error.clone(),
                retryable: false,
            })
            .await?;
            return Err(anyhow::anyhow!(error));
        }

        // ─── 2. Notify frontend that a new turn is starting ───────────────
        // New protocol state is durable before the streaming assistant draft
        // is created, so future requests replay the same message bytes.
        let user_message_id = messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User && !message.is_compaction())
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
            .find(|message| message.role == MessageRole::User && !message.is_compaction())
            .and_then(|m| m.thinking_level.clone())
            .unwrap_or_else(|| config.thinking_level.clone());
        let turn = match ctx
            .stream_turn(
                &messages,
                &system_prompt,
                &thinking_level,
                session_id,
                request_id,
            )
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

            if continue_after_compaction_summary {
                continue_after_compaction_summary = false;
                ctx.emit_stream_event(AgentEvent::StreamEnd {
                    request_id,
                    reasoning_started_at: turn.reasoning_started_at,
                    reasoning_completed_at: turn.reasoning_completed_at,
                    status: StreamEndStatus::Completed,
                })
                .await?;
                if let Err(error) = ctx
                    .append_compaction_continuation(
                        session_id,
                        compaction_continuation_message(session_id, false),
                    )
                    .await
                {
                    ctx.emit_stream_event(AgentEvent::Failed {
                        request_id,
                        error: error.to_string(),
                        retryable: false,
                    })
                    .await?;
                    return Err(error);
                }
                continue;
            }

            // Check for user messages steered into this session while the
            // turn was running. The host keeps them pending until the next
            // request boundary, where prepare_request performs compaction
            // before materializing them. The signal only keeps the loop alive
            // so the next iteration can reach that boundary.
            //
            // Queued (non-steered) messages do not set this signal. The host
            // keeps the outer session loop alive when it sees those pending
            // messages after this agent loop returns.
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

fn compaction_continuation_message(
    session_id: uuid::Uuid,
    summary_had_tool_calls: bool,
) -> Message {
    let content = if summary_had_tool_calls {
        format!(
            "The context compaction summary is empty because the compaction response requested a tool call. Use the session-history skill to inspect this session and recover the unfinished task, then continue it. Session ID: {session_id}."
        )
    } else {
        format!(
            "The preceding assistant response is an intermediate context summary, not a final answer. Continue the unfinished task from this session. Use the session-history skill to inspect the original conversation when details are needed. Session ID: {session_id}."
        )
    };
    Message::new(MessageRole::User, content)
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::{Arc, atomic::AtomicBool};

    use async_trait::async_trait;
    use tidev_llm::message::{ToolCall, ToolExecutionResult};
    use tidev_llm::reasoning::ThinkingLevelType;
    use tokio::sync::{Mutex, mpsc};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::context::RequestPreparation;
    use crate::event::AgentEventSender;

    #[derive(Default)]
    struct MockState {
        messages: Vec<Message>,
        preparations: VecDeque<RequestPreparation>,
        turns: VecDeque<AssistantTurn>,
        requests: Vec<Vec<Message>>,
        continuations: Vec<Message>,
    }

    struct MockContext {
        state: Mutex<MockState>,
        event_tx: AgentEventSender,
    }

    impl MockContext {
        fn new(
            messages: Vec<Message>,
            preparations: Vec<RequestPreparation>,
            turns: Vec<AssistantTurn>,
        ) -> (Self, mpsc::UnboundedReceiver<AgentEvent>) {
            let (event_tx, event_rx) = mpsc::unbounded_channel();
            (
                Self {
                    state: Mutex::new(MockState {
                        messages,
                        preparations: preparations.into(),
                        turns: turns.into(),
                        ..Default::default()
                    }),
                    event_tx: event_tx.into(),
                },
                event_rx,
            )
        }
    }

    #[async_trait]
    impl AgentContext for MockContext {
        fn tools(&self) -> Vec<tidev_llm::ToolDefinition> {
            Vec::new()
        }

        fn event_tx(&self) -> AgentEventSender {
            self.event_tx.clone()
        }

        async fn stream_turn(
            &self,
            messages: &[Message],
            _system_prompt: &str,
            _thinking_level: &ThinkingLevelType,
            _session_id: uuid::Uuid,
            _request_id: u64,
        ) -> anyhow::Result<AssistantTurn> {
            let mut state = self.state.lock().await;
            state.requests.push(messages.to_vec());
            state
                .turns
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no test turn available"))
        }

        async fn execute_tools(
            &self,
            tool_calls: &[ToolCall],
            _session_id: uuid::Uuid,
            _request_id: u64,
        ) -> anyhow::Result<Vec<(ToolCall, ToolExecutionResult)>> {
            Ok(tool_calls
                .iter()
                .cloned()
                .map(|call| (call, ToolExecutionResult::new("tool result")))
                .collect())
        }

        async fn save_messages(
            &self,
            _session_id: uuid::Uuid,
            messages: &[Message],
        ) -> anyhow::Result<()> {
            self.state.lock().await.messages.extend_from_slice(messages);
            Ok(())
        }

        fn workspace_root(&self) -> &Path {
            Path::new(".")
        }

        async fn prepare_request(
            &self,
            _session_id: uuid::Uuid,
        ) -> anyhow::Result<RequestPreparation> {
            Ok(self
                .state
                .lock()
                .await
                .preparations
                .pop_front()
                .unwrap_or_default())
        }

        async fn append_compaction_continuation(
            &self,
            _session_id: uuid::Uuid,
            message: Message,
        ) -> anyhow::Result<()> {
            let mut state = self.state.lock().await;
            state.continuations.push(message.clone());
            state.messages.push(message);
            Ok(())
        }

        async fn load_messages(&self, _session_id: uuid::Uuid) -> anyhow::Result<Vec<Message>> {
            Ok(self.state.lock().await.messages.clone())
        }
    }

    fn config(session_id: uuid::Uuid) -> AgentLoopConfig {
        AgentLoopConfig {
            session_id,
            system_prompt: "system".into(),
            thinking_level: ThinkingLevelType::None,
            event_tx: mpsc::unbounded_channel().0.into(),
            cancel: CancellationToken::new(),
            steer_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    #[tokio::test]
    async fn successful_compaction_summary_schedules_one_continuation_request() {
        let session_id = uuid::Uuid::new_v4();
        let context = Message::compaction("task summary");
        let (ctx, _events) = MockContext::new(
            vec![context],
            vec![
                RequestPreparation {
                    auto_compacted: true,
                    summary_had_tool_calls: false,
                },
                RequestPreparation::default(),
            ],
            vec![
                AssistantTurn {
                    content: "summary response".into(),
                    ..Default::default()
                },
                AssistantTurn {
                    content: "task completed".into(),
                    ..Default::default()
                },
            ],
        );

        run_agent_loop(&ctx, config(session_id)).await.unwrap();

        let state = ctx.state.lock().await;
        assert_eq!(state.requests.len(), 2);
        assert_eq!(state.continuations.len(), 1);
        assert!(
            state.continuations[0]
                .content
                .contains(&session_id.to_string())
        );
        assert!(
            state.continuations[0]
                .content
                .contains("intermediate context summary")
        );
        assert_eq!(
            state.requests[1].last().unwrap().content,
            state.continuations[0].content
        );
        assert!(
            state.requests[1]
                .iter()
                .any(|message| message.content == "summary response")
        );
    }

    #[tokio::test]
    async fn successful_compaction_waits_for_text_only_response_after_tool_calls() {
        let session_id = uuid::Uuid::new_v4();
        let context = Message::compaction("task summary");
        let (ctx, _events) = MockContext::new(
            vec![context],
            vec![
                RequestPreparation {
                    auto_compacted: true,
                    summary_had_tool_calls: false,
                },
                RequestPreparation::default(),
                RequestPreparation::default(),
            ],
            vec![
                AssistantTurn {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "read_file".into(),
                        arguments: "{}".into(),
                        thought_signature: None,
                    }],
                    ..Default::default()
                },
                AssistantTurn {
                    content: "tool results processed".into(),
                    ..Default::default()
                },
                AssistantTurn {
                    content: "task completed".into(),
                    ..Default::default()
                },
            ],
        );

        run_agent_loop(&ctx, config(session_id)).await.unwrap();

        let state = ctx.state.lock().await;
        assert_eq!(state.requests.len(), 3);
        assert_eq!(state.continuations.len(), 1);
        assert!(state.requests[1].iter().any(|message| {
            message.role == MessageRole::Tool && message.tool_call_id.as_deref() == Some("call-1")
        }));
        assert_eq!(
            state.requests[2].last().unwrap().content,
            state.continuations[0].content
        );
        assert!(
            state.continuations[0]
                .content
                .contains(&session_id.to_string())
        );
    }

    #[tokio::test]
    async fn compaction_tool_call_injects_session_id_before_task_request() {
        let session_id = uuid::Uuid::new_v4();
        let initial = Message::new(MessageRole::User, "unfinished task");
        let initial_id = initial.id;
        let (ctx, _events) = MockContext::new(
            vec![initial],
            vec![
                RequestPreparation {
                    auto_compacted: true,
                    summary_had_tool_calls: true,
                },
                RequestPreparation::default(),
            ],
            vec![
                AssistantTurn {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "session_history".into(),
                        arguments: "{}".into(),
                        thought_signature: None,
                    }],
                    ..Default::default()
                },
                AssistantTurn {
                    content: "task completed".into(),
                    ..Default::default()
                },
            ],
        );

        run_agent_loop(&ctx, config(session_id)).await.unwrap();

        let state = ctx.state.lock().await;
        assert_eq!(state.requests.len(), 2);
        assert_eq!(state.continuations.len(), 1);
        assert!(
            state.continuations[0]
                .content
                .contains(&session_id.to_string())
        );
        assert!(state.continuations[0].content.contains("summary is empty"));
        assert_eq!(state.messages[0].id, initial_id);
        assert_eq!(
            state.requests[0].last().unwrap().content,
            state.continuations[0].content
        );
    }
}
