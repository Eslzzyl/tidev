//! Events emitted by LLM providers.

use serde::{Deserialize, Serialize};

use crate::message::{AssistantTurn, ToolCall};

/// Provider-level events. These events contain only protocol data and do not
/// carry session or agent-runtime identifiers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LlmEvent {
    Delta {
        content: String,
    },
    ReasoningDelta {
        content: String,
    },
    ReasoningSummaryDelta {
        content: String,
        summary_index: Option<u32>,
    },
    ToolCallUpdated {
        tool_call: ToolCall,
    },
    Finished {
        turn: Box<AssistantTurn>,
    },
    Failed {
        error: String,
        retryable: bool,
    },
    Retrying {
        attempt: u32,
        max_attempts: u32,
        reason: String,
        retry_after_secs: Option<u32>,
    },
    UsageStats {
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
        model_id: String,
        duration_ms: Option<u64>,
    },
}
