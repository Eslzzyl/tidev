//! Generic streaming turn execution for agent hosts.

use anyhow::Result;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use tidev_llm::message::{AssistantTurn, Message, ToolCall};
use tidev_llm::reasoning::ThinkingLevelType;
use tidev_llm::{LlmClient, LlmProviderConfig, ToolDefinition};

use crate::event::{AgentEvent, AgentEventSender, llm_event_to_agent_event};

/// Options controlling host-visible behavior while a turn is cancelled.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamTurnOptions {
    /// Emit a synthetic `StreamEnd` event before returning the cancellation
    /// error. Hosts that already emit this event in their loop can disable it.
    pub emit_stream_end_on_cancel: bool,
}

/// Stream one LLM turn and aggregate provider events into an [`AssistantTurn`].
///
/// The provider event-to-agent event mapping and turn reconstruction live here
/// so product hosts do not need to duplicate protocol handling. The returned
/// turn contains the same fields that were accumulated from the provider
/// stream, including opaque Responses API output items.
#[allow(clippy::too_many_arguments)]
pub async fn stream_turn(
    llm: &LlmClient,
    mut model: LlmProviderConfig,
    messages: &[Message],
    tools: &[ToolDefinition],
    system_prompt: &str,
    thinking_level: &ThinkingLevelType,
    request_id: u64,
    event_tx: &AgentEventSender,
    cancel: &CancellationToken,
    options: StreamTurnOptions,
) -> Result<AssistantTurn> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let llm = llm.clone();
    model.system_prompt = Some(system_prompt.to_string());
    let messages = messages.to_vec();
    let tools = tools.to_vec();
    let thinking_level = thinking_level.clone();

    let handle = tokio::spawn(async move {
        llm.stream_chat(model, messages, tools, tx, thinking_level)
            .await;
    });

    let mut turn = AssistantTurn {
        created_at: Some(Utc::now()),
        ..Default::default()
    };

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                handle.abort();
                if options.emit_stream_end_on_cancel {
                    let _ = event_tx.send(AgentEvent::StreamEnd {
                        request_id,
                        reasoning_started_at: None,
                        reasoning_completed_at: None,
                    });
                }
                return Err(anyhow::anyhow!("Stream cancelled by user"));
            }
            event = rx.recv() => {
                let Some(event) = event else { break };
                let event = llm_event_to_agent_event(event, request_id);
                let _ = event_tx.send(event.clone());
                match event {
                    AgentEvent::Delta { content, .. } => {
                        turn.content.push_str(&content);
                        if turn.reasoning_started_at.is_some()
                            && turn.reasoning_completed_at.is_none()
                        {
                            turn.reasoning_completed_at = Some(Utc::now());
                        }
                    }
                    AgentEvent::ReasoningDelta { content, .. } => {
                        if turn.reasoning_started_at.is_none() {
                            turn.reasoning_started_at = Some(Utc::now());
                        }
                        turn.reasoning.push_str(&content);
                    }
                    AgentEvent::ToolCallUpdated { tool_call, .. } => {
                        turn.upsert_tool_call(tool_call);
                        if turn.reasoning_started_at.is_some()
                            && turn.reasoning_completed_at.is_none()
                        {
                            turn.reasoning_completed_at = Some(Utc::now());
                        }
                    }
                    AgentEvent::UsageStats {
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        model_id,
                        duration_ms,
                        ..
                    } => {
                        turn.input_tokens = Some(input_tokens);
                        turn.output_tokens = Some(output_tokens);
                        turn.total_tokens = Some(total_tokens);
                        turn.cache_read_tokens = Some(cache_read_tokens);
                        turn.cache_write_tokens = Some(cache_write_tokens);
                        turn.model_id = Some(model_id);
                        if let Some(ms) = duration_ms.filter(|ms| *ms > 0) {
                            turn.tokens_per_second =
                                Some(output_tokens as f32 / (ms as f32 / 1000.0));
                        }
                    }
                    AgentEvent::Finished { turn: finished_turn, .. } => {
                        turn.responses_output_items = finished_turn.responses_output_items.clone();
                        break;
                    }
                    AgentEvent::Failed { error, .. } => {
                        return Err(anyhow::anyhow!("LLM error: {error}"));
                    }
                    _ => {}
                }
            }
        }
    }

    turn.completed_at = Some(Utc::now());
    Ok(turn)
}

/// Restore the assistant tool-call order after concurrent execution.
///
/// Completion events may arrive in execution order, but protocol messages
/// must follow the order in the assistant turn that requested them.
pub fn order_tool_results(
    tool_calls: &[ToolCall],
    results: Vec<(ToolCall, tidev_llm::message::ToolExecutionResult)>,
) -> Vec<(ToolCall, tidev_llm::message::ToolExecutionResult)> {
    let mut pending: Vec<Option<(ToolCall, tidev_llm::message::ToolExecutionResult)>> =
        results.into_iter().map(Some).collect();
    let mut ordered = Vec::with_capacity(pending.len());

    for tool_call in tool_calls {
        if let Some(index) = pending.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|(result_call, _)| result_call.id == tool_call.id)
        }) {
            ordered.push(pending[index].take().expect("matched result must exist"));
        }
    }

    ordered.extend(pending.into_iter().flatten());
    ordered
}
