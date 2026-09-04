//! The [`AgentContext`] trait — the interface between the agent loop and the
//! concrete runtime environment provided by tidev-core.
//!
//! tidev-agent defines the loop skeleton; tidev-core implements [`AgentContext`].

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use tidev_llm::ToolDefinition;
use tidev_llm::message::{AssistantTurn, Message, ToolCall, ToolExecutionResult};
use tidev_llm::reasoning::ThinkingLevelType;

use crate::event::{AgentEvent, AgentEventSender};

// ---------------------------------------------------------------------------
// AgentLoopConfig
// ---------------------------------------------------------------------------

/// Configuration for a single invocation of [`run_agent_loop`].
#[derive(Clone)]
pub struct AgentLoopConfig {
    /// The session this agent loop is running in.
    pub session_id: uuid::Uuid,
    /// The system prompt for this agent loop run.
    pub system_prompt: String,
    /// Thinking / reasoning level.
    pub thinking_level: ThinkingLevelType,
    /// Channel for sending real-time events to the frontend.
    pub event_tx: AgentEventSender,
    /// Cancellation token for cooperative termination.
    pub cancel: CancellationToken,
    /// Steering signal for user messages that arrived while the loop was busy.
    ///
    /// Steering messages are persisted to the message buffer immediately by
    /// the host; this signal only tells [`run_agent_loop`] that such a message
    /// exists, so the loop must keep running instead of exiting after a turn
    /// without tool calls. The next iteration's `load_messages` picks the
    /// message up naturally.
    pub steer_signal: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// AgentContext trait
// ---------------------------------------------------------------------------

/// The interface that the agent loop needs from its runtime environment.
///
/// tidev-core implements this trait with concrete LLM clients, tool executors,
/// storage backends, and permission channels.
#[async_trait]
pub trait AgentContext: Send + Sync {
    /// Return the current list of tool definitions available to this session.
    fn tools(&self) -> Vec<ToolDefinition>;

    /// Return the event channel for sending real-time events to the frontend.
    fn event_tx(&self) -> AgentEventSender;

    /// Persist or transform a stream event before exposing it to a frontend.
    ///
    /// The default remains suitable for generic hosts. Product hosts that
    /// support recovery can override this to durably journal each delta before
    /// the corresponding UI event is sent.
    async fn emit_stream_event(&self, event: AgentEvent) -> Result<()> {
        // A disconnected UI must not abort the provider request. Durable hosts
        // have already recorded the event before this best-effort delivery.
        let _ = self.event_tx().send(event);
        Ok(())
    }

    /// Stream a single LLM turn and return the completed [`AssistantTurn`].
    ///
    /// The implementation should forward streaming [`AgentEvent`]s (deltas,
    /// tool call updates, usage stats, etc.) through
    /// [`Self::emit_stream_event`] in real time.
    ///
    /// `request_id` is the per-turn sequence number from the agent loop,
    /// embedded in forwarded events so the frontend can reject stale events
    /// after cancellation.
    async fn stream_turn(
        &self,
        messages: &[Message],
        system_prompt: &str,
        thinking_level: &ThinkingLevelType,
        request_id: u64,
    ) -> Result<AssistantTurn>;

    /// Execute a batch of tool calls and return their results.
    ///
    /// The implementation is responsible for:
    /// - Applying any host-specific approval policy
    /// - Separating read-only vs write tools (parallel vs serial execution)
    /// - Handling `task` tools (subagent delegation)
    /// - Emitting [`AgentEvent::ToolCompleted`] events
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCall],
        session_id: uuid::Uuid,
        request_id: u64,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>>;

    /// Persist one or more messages to the session store.
    async fn save_messages(&self, session_id: uuid::Uuid, messages: &[Message]) -> Result<()>;

    /// Return the workspace root path.
    fn workspace_root(&self) -> &Path;

    /// Load all messages for the current session.
    async fn load_messages(&self, session_id: uuid::Uuid) -> Result<Vec<Message>>;
}
