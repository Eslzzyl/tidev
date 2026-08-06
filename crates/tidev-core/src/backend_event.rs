//! Events exposed by tidev to frontends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tidev_agent::{AgentEvent, AgentEventSender, AgentEventSink};
use tidev_llm::message::{AssistantTurn, Message, ToolCall, ToolExecutionResult};
use tidev_storage::MessageAppData;
use tokio::sync::mpsc::UnboundedSender;
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

enum CoreEventKind {
    Agent(AgentEvent),
    Backend(BackendEvent),
}

struct CoreEvent {
    session_id: Uuid,
    kind: CoreEventKind,
}

/// Ordered event boundary for a tidev runtime.
///
/// Agent events and product-specific backend events share one queue so a
/// frontend observes the same order in which the host emitted them.
#[derive(Clone)]
pub(crate) struct CoreEventBus {
    tx: UnboundedSender<CoreEvent>,
    session_id: Uuid,
}

impl CoreEventBus {
    pub(crate) fn new(backend_tx: UnboundedSender<BackendEvent>, session_id: Uuid) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
        tokio::spawn(async move {
            while let Some(core_event) = rx.recv().await {
                let event = match core_event.kind {
                    CoreEventKind::Agent(event) => {
                        agent_event_to_backend_event(event, core_event.session_id)
                    }
                    CoreEventKind::Backend(event) => event,
                };
                if backend_tx.send(event).is_err() {
                    break;
                }
            }
        });
        Self { tx, session_id }
    }

    pub(crate) fn for_session(&self, session_id: Uuid) -> Self {
        Self {
            tx: self.tx.clone(),
            session_id,
        }
    }

    pub(crate) fn agent_sender(&self) -> AgentEventSender {
        AgentEventSender::from_sink(self.clone())
    }

    pub(crate) fn send_backend(&self, event: BackendEvent) -> bool {
        self.tx
            .send(CoreEvent {
                session_id: self.session_id,
                kind: CoreEventKind::Backend(event),
            })
            .is_ok()
    }
}

impl AgentEventSink for CoreEventBus {
    fn send_event(&self, event: AgentEvent) -> bool {
        self.tx
            .send(CoreEvent {
                session_id: self.session_id,
                kind: CoreEventKind::Agent(event),
            })
            .is_ok()
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

    fn sample_result() -> ToolExecutionResult {
        let mut result = ToolExecutionResult::new("result payload");
        result.metadata.preserve_full_output = true;
        result.metadata.diff = Some("diff payload".into());
        result
    }

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

    #[test]
    fn agent_to_backend_preserves_payload_fields() {
        let session_id = Uuid::new_v4();
        let request_id = 73;
        let tool_call = sample_tool_call();
        let turn = sample_turn();
        let result = sample_result();
        let reasoning_started_at = Some(Utc::now());
        let reasoning_completed_at = Some(Utc::now());
        let completed_at = Some(Utc::now());

        match agent_event_to_backend_event(
            AgentEvent::Delta {
                request_id,
                content: "delta payload".into(),
            },
            session_id,
        ) {
            BackendEvent::Delta {
                session_id: received_session_id,
                request_id: received_request_id,
                content,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(content, "delta payload");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::ReasoningDelta {
                request_id,
                content: "reasoning payload".into(),
            },
            session_id,
        ) {
            BackendEvent::ReasoningDelta {
                session_id: received_session_id,
                request_id: received_request_id,
                content,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(content, "reasoning payload");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::ToolCallUpdated {
                request_id,
                tool_call: tool_call.clone(),
            },
            session_id,
        ) {
            BackendEvent::ToolCallUpdated {
                session_id: received_session_id,
                request_id: received_request_id,
                tool_call: received_tool_call,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(received_tool_call, tool_call);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::Finished {
                request_id,
                turn: Box::new(turn.clone()),
            },
            session_id,
        ) {
            BackendEvent::Finished {
                session_id: received_session_id,
                request_id: received_request_id,
                turn: received_turn,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_turn_payload(&turn, &received_turn);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::Failed {
                request_id,
                error: "failure payload".into(),
            },
            session_id,
        ) {
            BackendEvent::Failed {
                session_id: received_session_id,
                request_id: received_request_id,
                error,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(error, "failure payload");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::Retrying {
                request_id,
                attempt: 4,
                max_attempts: 9,
                reason: "retry payload".into(),
                retry_after_secs: Some(17),
            },
            session_id,
        ) {
            BackendEvent::Retrying {
                session_id: received_session_id,
                request_id: received_request_id,
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(attempt, 4);
                assert_eq!(max_attempts, 9);
                assert_eq!(reason, "retry payload");
                assert_eq!(retry_after_secs, Some(17));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::UsageStats {
                request_id,
                input_tokens: 21,
                output_tokens: 22,
                total_tokens: 43,
                cache_read_tokens: 23,
                cache_write_tokens: 24,
                model_id: "usage-model".into(),
                duration_ms: Some(25),
            },
            session_id,
        ) {
            BackendEvent::UsageStats {
                session_id: received_session_id,
                request_id: received_request_id,
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                duration_ms,
            } => {
                assert_eq!(received_session_id, session_id);
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

        match agent_event_to_backend_event(AgentEvent::TurnStarting { request_id }, session_id) {
            BackendEvent::TurnStarting {
                session_id: received_session_id,
                request_id: received_request_id,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::StreamEnd {
                request_id,
                reasoning_started_at,
                reasoning_completed_at,
            },
            session_id,
        ) {
            BackendEvent::StreamEnd {
                session_id: received_session_id,
                request_id: received_request_id,
                reasoning_started_at: received_started_at,
                reasoning_completed_at: received_completed_at,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(received_started_at, reasoning_started_at);
                assert_eq!(received_completed_at, reasoning_completed_at);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::ToolStarting {
                request_id,
                tool_call: tool_call.clone(),
            },
            session_id,
        ) {
            BackendEvent::ToolStarting {
                session_id: received_session_id,
                request_id: received_request_id,
                tool_call: received_tool_call,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(received_tool_call, tool_call);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::ToolCompleted {
                request_id,
                tool_call: tool_call.clone(),
                result: Box::new(result.clone()),
            },
            session_id,
        ) {
            BackendEvent::ToolCompleted {
                session_id: received_session_id,
                request_id: received_request_id,
                tool_call: received_tool_call,
                result: received_result,
                child_session_id,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(received_request_id, request_id);
                assert_eq!(received_tool_call, tool_call);
                assert_eq!(*received_result, result);
                assert_eq!(child_session_id, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::ContextCompacted {
                compacted: true,
                manual: true,
                summary: Some("summary payload".into()),
                retained_from: 31,
                model_id: Some("compact-model".into()),
                completed_at,
                error: Some("compact error".into()),
            },
            session_id,
        ) {
            BackendEvent::ContextCompacted {
                session_id: received_session_id,
                compacted,
                manual,
                summary,
                retained_from,
                model_id,
                completed_at: received_completed_at,
                error,
            } => {
                assert_eq!(received_session_id, session_id);
                assert!(compacted);
                assert!(manual);
                assert_eq!(summary.as_deref(), Some("summary payload"));
                assert_eq!(retained_from, 31);
                assert_eq!(model_id.as_deref(), Some("compact-model"));
                assert_eq!(received_completed_at, completed_at);
                assert_eq!(error.as_deref(), Some("compact error"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match agent_event_to_backend_event(
            AgentEvent::ShellOutput {
                request_id,
                tool_call_id: "shell-call".into(),
                content: "shell payload".into(),
                finished: true,
                exit_code: Some(42),
            },
            session_id,
        ) {
            BackendEvent::ShellOutput {
                session_id: received_session_id,
                tool_call_id,
                content,
                finished,
                exit_code,
            } => {
                assert_eq!(received_session_id, session_id);
                assert_eq!(tool_call_id, "shell-call");
                assert_eq!(content, "shell payload");
                assert!(finished);
                assert_eq!(exit_code, Some(42));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn core_event_bus_preserves_mixed_event_order() {
        let session_id = Uuid::new_v4();
        let (backend_tx, mut backend_rx) = tokio::sync::mpsc::unbounded_channel();
        let bus = CoreEventBus::new(backend_tx, session_id);

        bus.agent_sender()
            .send(AgentEvent::TurnStarting { request_id: 7 });
        bus.send_backend(BackendEvent::StreamEnd {
            session_id,
            request_id: 7,
            reasoning_started_at: None,
            reasoning_completed_at: None,
        });
        bus.agent_sender().send(AgentEvent::Delta {
            request_id: 7,
            content: "delta".into(),
        });

        assert!(matches!(
            backend_rx.recv().await,
            Some(BackendEvent::TurnStarting {
                session_id: received_session_id,
                request_id: 7,
            }) if received_session_id == session_id
        ));
        assert!(matches!(
            backend_rx.recv().await,
            Some(BackendEvent::StreamEnd {
                session_id: received_session_id,
                request_id: 7,
                ..
            }) if received_session_id == session_id
        ));
        assert!(matches!(
            backend_rx.recv().await,
            Some(BackendEvent::Delta {
                session_id: received_session_id,
                request_id: 7,
                content,
            }) if received_session_id == session_id && content == "delta"
        ));
    }
}
