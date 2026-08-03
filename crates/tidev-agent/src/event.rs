//! Events emitted by the generic agent runtime.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tidev_llm::event::LlmEvent;
use tidev_llm::message::{AssistantTurn, ToolCall, ToolExecutionResult};

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
        child_session_id: Option<uuid::Uuid>,
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
        LlmEvent::ToolCallUpdated { tool_call } => AgentEvent::ToolCallUpdated {
            request_id,
            tool_call,
        },
        LlmEvent::Finished { turn } => AgentEvent::Finished { request_id, turn },
        LlmEvent::Failed { error } => AgentEvent::Failed { request_id, error },
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
            LlmEvent::Failed { error: "e".into() },
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
}
