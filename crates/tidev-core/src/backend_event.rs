//! Events exposed by tidev to frontends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tidev_agent::AgentEvent;
use tidev_storage::MessageAppData;
use tidev_llm::message::{AssistantTurn, Message, ToolCall, ToolExecutionResult};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

/// Product-facing events. Session identity is added at this boundary so the
/// protocol and agent layers remain independent of tidev sessions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BackendEvent {
    Delta {
        session_id: Uuid,
        request_id: u64,
        content: String,
    },
    ReasoningDelta {
        session_id: Uuid,
        request_id: u64,
        content: String,
    },
    ToolCallUpdated {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
    },
    Finished {
        session_id: Uuid,
        request_id: u64,
        turn: Box<AssistantTurn>,
    },
    Failed {
        session_id: Uuid,
        request_id: u64,
        error: String,
    },
    Retrying {
        session_id: Uuid,
        request_id: u64,
        attempt: u32,
        max_attempts: u32,
        reason: String,
        retry_after_secs: Option<u32>,
    },
    InstructionsLoaded {
        session_id: Uuid,
        sources: Vec<String>,
    },
    ToolStarting {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
    },
    ToolCompleted {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
        result: Box<ToolExecutionResult>,
        child_session_id: Option<Uuid>,
    },
    SubagentStatus {
        session_id: Uuid,
        request_id: u64,
        tool_call_id: String,
        child_session_id: Uuid,
        status_text: String,
        current_tool_call: Option<ToolCall>,
        assistant_message: Box<Option<Message>>,
        content_delta: Option<String>,
        reasoning_delta: Option<String>,
    },
    SubagentCompleted {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
        child_session_id: Uuid,
        result: Box<ToolExecutionResult>,
    },
    UsageStats {
        session_id: Uuid,
        request_id: u64,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
        model_id: String,
        duration_ms: Option<u64>,
    },
    ContextCompacted {
        session_id: Uuid,
        compacted: bool,
        manual: bool,
        summary: Option<String>,
        retained_from: usize,
        model_id: Option<String>,
        completed_at: Option<DateTime<Utc>>,
        error: Option<String>,
    },
    UserMessageCreated {
        session_id: Uuid,
        message: Box<Message>,
        app_data: Box<MessageAppData>,
    },
    UndoCompleted {
        session_id: Uuid,
        target_id: Uuid,
        message_content: String,
    },
    SidebarSnapshotReady {
        session_id: Uuid,
        request_id: u64,
        tool_call_id: String,
        file_diffs_json: String,
    },
    ShellOutput {
        session_id: Uuid,
        tool_call_id: String,
        content: String,
        finished: bool,
        exit_code: Option<i32>,
    },
    TurnStarting {
        session_id: Uuid,
        request_id: u64,
    },
    StreamEnd {
        session_id: Uuid,
        request_id: u64,
        reasoning_started_at: Option<DateTime<Utc>>,
        reasoning_completed_at: Option<DateTime<Utc>>,
    },
    MessagesTruncated {
        session_id: Uuid,
        kept_count: usize,
    },
}

impl BackendEvent {
    pub fn session_id(&self) -> Uuid {
        match self {
            Self::Delta { session_id, .. }
            | Self::ReasoningDelta { session_id, .. }
            | Self::ToolCallUpdated { session_id, .. }
            | Self::Finished { session_id, .. }
            | Self::Failed { session_id, .. }
            | Self::Retrying { session_id, .. }
            | Self::ToolStarting { session_id, .. }
            | Self::ToolCompleted { session_id, .. }
            | Self::SubagentStatus { session_id, .. }
            | Self::SubagentCompleted { session_id, .. }
            | Self::UsageStats { session_id, .. }
            | Self::InstructionsLoaded { session_id, .. }
            | Self::ContextCompacted { session_id, .. }
            | Self::UndoCompleted { session_id, .. }
            | Self::UserMessageCreated { session_id, .. }
            | Self::SidebarSnapshotReady { session_id, .. }
            | Self::ShellOutput { session_id, .. }
            | Self::TurnStarting { session_id, .. }
            | Self::StreamEnd { session_id, .. }
            | Self::MessagesTruncated { session_id, .. } => *session_id,
        }
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Delta { request_id, .. }
            | Self::ReasoningDelta { request_id, .. }
            | Self::ToolCallUpdated { request_id, .. }
            | Self::Finished { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Retrying { request_id, .. }
            | Self::ToolStarting { request_id, .. }
            | Self::ToolCompleted { request_id, .. }
            | Self::SubagentStatus { request_id, .. }
            | Self::SubagentCompleted { request_id, .. }
            | Self::UsageStats { request_id, .. }
            | Self::SidebarSnapshotReady { request_id, .. }
            | Self::TurnStarting { request_id, .. }
            | Self::StreamEnd { request_id, .. } => Some(*request_id),
            Self::InstructionsLoaded { .. }
            | Self::ContextCompacted { .. }
            | Self::UndoCompleted { .. }
            | Self::UserMessageCreated { .. }
            | Self::ShellOutput { .. }
            | Self::MessagesTruncated { .. } => None,
        }
    }
}

/// Add the tidev session identifier to an agent event.
pub fn agent_event_to_backend_event(event: AgentEvent, session_id: Uuid) -> BackendEvent {
    match event {
        AgentEvent::Delta {
            request_id,
            content,
        } => BackendEvent::Delta {
            session_id,
            request_id,
            content,
        },
        AgentEvent::ReasoningDelta {
            request_id,
            content,
        } => BackendEvent::ReasoningDelta {
            session_id,
            request_id,
            content,
        },
        AgentEvent::ToolCallUpdated {
            request_id,
            tool_call,
        } => BackendEvent::ToolCallUpdated {
            session_id,
            request_id,
            tool_call,
        },
        AgentEvent::Finished { request_id, turn } => BackendEvent::Finished {
            session_id,
            request_id,
            turn,
        },
        AgentEvent::Failed { request_id, error } => BackendEvent::Failed {
            session_id,
            request_id,
            error,
        },
        AgentEvent::Retrying {
            request_id,
            attempt,
            max_attempts,
            reason,
            retry_after_secs,
        } => BackendEvent::Retrying {
            session_id,
            request_id,
            attempt,
            max_attempts,
            reason,
            retry_after_secs,
        },
        AgentEvent::UsageStats {
            request_id,
            input_tokens,
            output_tokens,
            total_tokens,
            cache_read_tokens,
            cache_write_tokens,
            model_id,
            duration_ms,
        } => BackendEvent::UsageStats {
            session_id,
            request_id,
            input_tokens,
            output_tokens,
            total_tokens,
            cache_read_tokens,
            cache_write_tokens,
            model_id,
            duration_ms,
        },
        AgentEvent::TurnStarting { request_id } => BackendEvent::TurnStarting {
            session_id,
            request_id,
        },
        AgentEvent::StreamEnd {
            request_id,
            reasoning_started_at,
            reasoning_completed_at,
        } => BackendEvent::StreamEnd {
            session_id,
            request_id,
            reasoning_started_at,
            reasoning_completed_at,
        },
        AgentEvent::ToolStarting {
            request_id,
            tool_call,
        } => BackendEvent::ToolStarting {
            session_id,
            request_id,
            tool_call,
        },
        AgentEvent::ToolCompleted {
            request_id,
            tool_call,
            result,
        } => BackendEvent::ToolCompleted {
            session_id,
            request_id,
            tool_call,
            result,
            child_session_id: None,
        },
        AgentEvent::ContextCompacted {
            compacted,
            manual,
            summary,
            retained_from,
            model_id,
            completed_at,
            error,
        } => BackendEvent::ContextCompacted {
            session_id,
            compacted,
            manual,
            summary,
            retained_from,
            model_id,
            completed_at,
            error,
        },
        AgentEvent::ShellOutput {
            request_id: _,
            tool_call_id,
            content,
            finished,
            exit_code,
        } => BackendEvent::ShellOutput {
            session_id,
            tool_call_id,
            content,
            finished,
            exit_code,
        },
    }
}

/// Create a core boundary channel that enriches agent events with a session ID
/// before forwarding them to the product-facing backend event stream.
pub fn agent_event_channel(
    session_id: Uuid,
    backend_tx: UnboundedSender<BackendEvent>,
) -> UnboundedSender<AgentEvent> {
    let (agent_tx, mut agent_rx): (UnboundedSender<AgentEvent>, UnboundedReceiver<AgentEvent>) =
        tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            if backend_tx
                .send(agent_event_to_backend_event(event, session_id))
                .is_err()
            {
                break;
            }
        }
    });
    agent_tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_event_maps_to_one_backend_event() {
        let session_id = Uuid::new_v4();
        let events = vec![
            AgentEvent::Delta {
                request_id: 1,
                content: "d".into(),
            },
            AgentEvent::ReasoningDelta {
                request_id: 1,
                content: "r".into(),
            },
            AgentEvent::ToolCallUpdated {
                request_id: 1,
                tool_call: ToolCall::default(),
            },
            AgentEvent::Finished {
                request_id: 1,
                turn: Box::new(AssistantTurn::default()),
            },
            AgentEvent::Failed {
                request_id: 1,
                error: "e".into(),
            },
            AgentEvent::Retrying {
                request_id: 1,
                attempt: 1,
                max_attempts: 2,
                reason: "retry".into(),
                retry_after_secs: None,
            },
            AgentEvent::UsageStats {
                request_id: 1,
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
                cache_read_tokens: 4,
                cache_write_tokens: 5,
                model_id: "model".into(),
                duration_ms: None,
            },
            AgentEvent::TurnStarting { request_id: 1 },
            AgentEvent::StreamEnd {
                request_id: 1,
                reasoning_started_at: None,
                reasoning_completed_at: None,
            },
            AgentEvent::ToolStarting {
                request_id: 1,
                tool_call: ToolCall::default(),
            },
            AgentEvent::ToolCompleted {
                request_id: 1,
                tool_call: ToolCall::default(),
                result: Box::new(ToolExecutionResult::new("ok")),
            },
            AgentEvent::ContextCompacted {
                compacted: true,
                manual: false,
                summary: Some("summary".into()),
                retained_from: 2,
                model_id: Some("model".into()),
                completed_at: None,
                error: None,
            },
            AgentEvent::ShellOutput {
                request_id: 1,
                tool_call_id: "tool".into(),
                content: "output".into(),
                finished: true,
                exit_code: Some(0),
            },
        ];

        assert_eq!(events.len(), 13);
        for event in events {
            assert_eq!(
                agent_event_to_backend_event(event, session_id).session_id(),
                session_id
            );
        }

    }
}
