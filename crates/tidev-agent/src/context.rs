//! The [`AgentContext`] trait — the interface between the agent loop and the
//! concrete runtime environment provided by tidev-core.
//!
//! tidev-agent defines the loop skeleton; tidev-core implements [`AgentContext`].

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use tidev_llm::message::{
    AssistantTurn, Message, QueuedUserMessage, ToolCall, ToolExecutionResult,
};
use tidev_llm::reasoning::ThinkingLevelType;
use tidev_llm::ToolDefinition;

use crate::event::AgentEvent;

// ---------------------------------------------------------------------------
// AgentLoopConfig
// ---------------------------------------------------------------------------

/// Configuration for a single invocation of [`run_agent_loop`].
pub struct AgentLoopConfig {
    /// The session this agent loop is running in.
    pub session_id: uuid::Uuid,
    /// The system prompt for this agent loop run.
    pub system_prompt: String,
    /// Thinking / reasoning level.
    pub thinking_level: ThinkingLevelType,
    /// Channel for sending real-time events to the frontend.
    pub event_tx: UnboundedSender<AgentEvent>,
    /// Cancellation token for cooperative termination.
    pub cancel: CancellationToken,
    /// Queue of user messages that arrived while the loop was busy.
    ///
    /// Populated by [`Runtime::submit_prompt_with_attachments`] when the loop
    /// is running; checked by [`run_agent_loop`] after turns without tool
    /// calls. Each queued message triggers one additional loop iteration.
    pub queued_messages: Arc<Mutex<VecDeque<QueuedUserMessage>>>,
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
    fn event_tx(&self) -> UnboundedSender<AgentEvent>;

    /// Stream a single LLM turn and return the completed [`AssistantTurn`].
    ///
    /// The implementation should forward streaming [`AgentEvent`]s (deltas,
    /// tool call updates, usage stats, etc.) through [`Self::event_tx`] in
    /// real time.
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
