//! Events emitted by the generic agent runtime.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tidev_llm::event::LlmEvent;
use tidev_llm::message::{AssistantTurn, ToolCall, ToolExecutionResult};
use tokio::sync::mpsc::UnboundedSender;

/// Events produced by the agent loop and its runtime components.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentEvent {
    Delta {
        request_id: u64,
        content: String,
    },
    ReasoningDelta {
        request_id: u64,
        content: String,
    },
    ReasoningSummaryDelta {
        request_id: u64,
        content: String,
        summary_index: Option<u32>,
    },
    ToolCallUpdated {
        request_id: u64,
        tool_call: ToolCall,
    },
    Finished {
        request_id: u64,
        turn: Box<AssistantTurn>,
    },
    Failed {
        request_id: u64,
        error: String,
        retryable: bool,
    },
    Retrying {
        request_id: u64,
        attempt: u32,
        max_attempts: u32,
        reason: String,
        retry_after_secs: Option<u32>,
    },
    UsageStats {
        request_id: u64,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
        model_id: String,
        duration_ms: Option<u64>,
    },
    TurnStarting {
        request_id: u64,
        user_message_id: Option<uuid::Uuid>,
    },
    StreamEnd {
        request_id: u64,
        reasoning_started_at: Option<DateTime<Utc>>,
        reasoning_completed_at: Option<DateTime<Utc>>,
    },
    ToolStarting {
        request_id: u64,
        tool_call: ToolCall,
    },
    ToolCompleted {
        request_id: u64,
        tool_call: ToolCall,
        result: Box<ToolExecutionResult>,
    },
    ContextCompacted {
        compacted: bool,
        manual: bool,
        summary: Option<String>,
        retained_from: usize,
        model_id: Option<String>,
        completed_at: Option<DateTime<Utc>>,
        error: Option<String>,
    },
    ShellOutput {
        request_id: u64,
        tool_call_id: String,
        content: String,
        finished: bool,
        exit_code: Option<i32>,
    },
}

/// A host-neutral sink for agent events.
///
/// The default implementation wraps an mpsc sender. Hosts may provide a
/// different sink when agent events must be serialized with product events
/// before reaching a frontend.
pub trait AgentEventSink: Send + Sync {
    fn send_event(&self, event: AgentEvent) -> bool;
}

/// Cloneable handle used by the loop and generic tools to emit agent events.
#[derive(Clone)]
pub struct AgentEventSender(Arc<dyn AgentEventSink>);

impl AgentEventSender {
    pub fn from_sink<S>(sink: S) -> Self
    where
        S: AgentEventSink + 'static,
    {
        Self(Arc::new(sink))
    }

    pub fn send(&self, event: AgentEvent) -> bool {
        self.0.send_event(event)
    }
}

struct ChannelAgentEventSink(UnboundedSender<AgentEvent>);

impl AgentEventSink for ChannelAgentEventSink {
    fn send_event(&self, event: AgentEvent) -> bool {
        self.0.send(event).is_ok()
    }
}

impl From<UnboundedSender<AgentEvent>> for AgentEventSender {
    fn from(sender: UnboundedSender<AgentEvent>) -> Self {
        Self::from_sink(ChannelAgentEventSink(sender))
    }
}

/// Add the loop request identifier to a provider event.
pub fn llm_event_to_agent_event(event: LlmEvent, request_id: u64) -> AgentEvent {
    match event {
        LlmEvent::Delta { content } => AgentEvent::Delta {
            request_id,
            content,
        },
        LlmEvent::ReasoningDelta { content } => AgentEvent::ReasoningDelta {
            request_id,
            content,
        },
        LlmEvent::ReasoningSummaryDelta {
            content,
            summary_index,
        } => AgentEvent::ReasoningSummaryDelta {
            request_id,
            content,
            summary_index,
        },
        LlmEvent::ToolCallUpdated { tool_call } => AgentEvent::ToolCallUpdated {
            request_id,
            tool_call,
        },
        LlmEvent::Finished { turn } => AgentEvent::Finished { request_id, turn },
        LlmEvent::Failed { error, retryable } => AgentEvent::Failed {
            request_id,
            error,
            retryable,
        },
        LlmEvent::Retrying {
            attempt,
            max_attempts,
            reason,
            retry_after_secs,
        } => AgentEvent::Retrying {
            request_id,
            attempt,
            max_attempts,
            reason,
            retry_after_secs,
        },
        LlmEvent::UsageStats {
            input_tokens,
            output_tokens,
            total_tokens,
            cache_read_tokens,
            cache_write_tokens,
            model_id,
            duration_ms,
        } => AgentEvent::UsageStats {
            request_id,
            input_tokens,
            output_tokens,
            total_tokens,
            cache_read_tokens,
            cache_write_tokens,
            model_id,
            duration_ms,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tool_call() -> ToolCall {
        ToolCall {
            id: "call-payload".into(),
            name: "payload_tool".into(),
            arguments: r#"{"value":"payload"}"#.into(),
            thought_signature: Some("signature-payload".into()),
        }
    }

    fn sample_turn() -> AssistantTurn {
        let now = Utc::now();
        AssistantTurn {
            content: "finished content".into(),
            reasoning: "finished reasoning".into(),
            tool_calls: vec![sample_tool_call()],
            finish_reason: Some("tool_calls".into()),
            input_tokens: Some(11),
            output_tokens: Some(12),
            total_tokens: Some(23),
            cache_read_tokens: Some(13),
            cache_write_tokens: Some(14),
            model_id: Some("payload-model".into()),
            tokens_per_second: Some(15.5),
            created_at: Some(now),
            completed_at: Some(now),
            reasoning_started_at: Some(now),
            reasoning_completed_at: Some(now),
            responses_output_items: vec![serde_json::json!({"type": "payload"})],
            thinking_level: None,
        }
    }

    fn assert_turn_payload(expected: &AssistantTurn, actual: &AssistantTurn) {
        assert_eq!(actual.content, expected.content);
        assert_eq!(actual.reasoning, expected.reasoning);
        assert_eq!(actual.tool_calls, expected.tool_calls);
        assert_eq!(actual.finish_reason, expected.finish_reason);
        assert_eq!(actual.input_tokens, expected.input_tokens);
        assert_eq!(actual.output_tokens, expected.output_tokens);
        assert_eq!(actual.total_tokens, expected.total_tokens);
        assert_eq!(actual.cache_read_tokens, expected.cache_read_tokens);
        assert_eq!(actual.cache_write_tokens, expected.cache_write_tokens);
        assert_eq!(actual.model_id, expected.model_id);
        assert_eq!(actual.tokens_per_second, expected.tokens_per_second);
        assert_eq!(actual.created_at, expected.created_at);
        assert_eq!(actual.completed_at, expected.completed_at);
        assert_eq!(actual.reasoning_started_at, expected.reasoning_started_at);
        assert_eq!(
            actual.reasoning_completed_at,
            expected.reasoning_completed_at
        );
        assert_eq!(
            actual.responses_output_items,
            expected.responses_output_items
        );
    }

    fn llm_events() -> Vec<LlmEvent> {
        vec![
            LlmEvent::Delta {
                content: "d".into(),
            },
            LlmEvent::ReasoningDelta {
                content: "r".into(),
            },
            LlmEvent::ToolCallUpdated {
                tool_call: ToolCall::default(),
            },
            LlmEvent::Finished {
                turn: Box::new(AssistantTurn::default()),
            },
            LlmEvent::Failed {
                error: "e".into(),
                retryable: false,
            },
            LlmEvent::Retrying {
                attempt: 1,
                max_attempts: 2,
                reason: "retry".into(),
                retry_after_secs: Some(1),
            },
            LlmEvent::UsageStats {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
                cache_read_tokens: 4,
                cache_write_tokens: 5,
                model_id: "model".into(),
                duration_ms: Some(6),
            },
        ]
    }

    #[test]
    fn every_llm_event_maps_to_one_agent_event() {
        let events = llm_events();
        assert_eq!(events.len(), 7);
        for event in events {
            let mapped = llm_event_to_agent_event(event, 42);
            let request_id = match mapped {
                AgentEvent::Delta { request_id, .. }
                | AgentEvent::ReasoningDelta { request_id, .. }
                | AgentEvent::ToolCallUpdated { request_id, .. }
                | AgentEvent::Finished { request_id, .. }
                | AgentEvent::Failed { request_id, .. }
                | AgentEvent::Retrying { request_id, .. }
                | AgentEvent::UsageStats { request_id, .. } => request_id,
                _ => panic!("provider event mapped to a non-provider event"),
            };
            assert_eq!(request_id, 42);
        }
    }

    #[test]
    fn llm_to_agent_preserves_payload_fields() {
        let request_id = 73;
        let tool_call = sample_tool_call();
        let turn = sample_turn();

        match llm_event_to_agent_event(
            LlmEvent::Delta {
                content: "delta payload".into(),
            },
            request_id,
        ) {
            AgentEvent::Delta {
                request_id: received_request_id,
                content,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_eq!(content, "delta payload");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match llm_event_to_agent_event(
            LlmEvent::ReasoningDelta {
                content: "reasoning payload".into(),
            },
            request_id,
        ) {
            AgentEvent::ReasoningDelta {
                request_id: received_request_id,
                content,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_eq!(content, "reasoning payload");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match llm_event_to_agent_event(
            LlmEvent::ToolCallUpdated {
                tool_call: tool_call.clone(),
            },
            request_id,
        ) {
            AgentEvent::ToolCallUpdated {
                request_id: received_request_id,
                tool_call: received_tool_call,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_eq!(received_tool_call, tool_call);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match llm_event_to_agent_event(
            LlmEvent::Finished {
                turn: Box::new(turn.clone()),
            },
            request_id,
        ) {
            AgentEvent::Finished {
                request_id: received_request_id,
                turn: received_turn,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_turn_payload(&turn, &received_turn);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match llm_event_to_agent_event(
            LlmEvent::Failed {
                error: "failure payload".into(),
                retryable: true,
            },
            request_id,
        ) {
            AgentEvent::Failed {
                request_id: received_request_id,
                error,
                retryable,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_eq!(error, "failure payload");
                assert!(retryable);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match llm_event_to_agent_event(
            LlmEvent::Retrying {
                attempt: 4,
                max_attempts: 9,
                reason: "retry payload".into(),
                retry_after_secs: Some(17),
            },
            request_id,
        ) {
            AgentEvent::Retrying {
                request_id: received_request_id,
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_eq!(attempt, 4);
                assert_eq!(max_attempts, 9);
                assert_eq!(reason, "retry payload");
                assert_eq!(retry_after_secs, Some(17));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match llm_event_to_agent_event(
            LlmEvent::UsageStats {
                input_tokens: 21,
                output_tokens: 22,
                total_tokens: 43,
                cache_read_tokens: 23,
                cache_write_tokens: 24,
                model_id: "usage-model".into(),
                duration_ms: Some(25),
            },
            request_id,
        ) {
            AgentEvent::UsageStats {
                request_id: received_request_id,
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                duration_ms,
            } => {
                assert_eq!(received_request_id, request_id);
                assert_eq!(input_tokens, 21);
                assert_eq!(output_tokens, 22);
                assert_eq!(total_tokens, 43);
                assert_eq!(cache_read_tokens, 23);
                assert_eq!(cache_write_tokens, 24);
                assert_eq!(model_id, "usage-model");
                assert_eq!(duration_ms, Some(25));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
