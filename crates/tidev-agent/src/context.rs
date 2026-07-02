//! The [`AgentContext`] trait — the interface between the agent loop and the
//! concrete runtime environment provided by tidev-core.
//!
//! tidev-agent defines the loop skeleton; tidev-core implements [`AgentContext`].

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use tidev_types::message::{
    AssistantTurn, BackendEvent, Message, ToolCall, ToolExecutionResult,
};
use tidev_types::prompts::SessionMode;
use tidev_types::reasoning::ThinkingLevelType;
use tidev_types::tools::ToolDefinition;

// ---------------------------------------------------------------------------
// AgentLoopConfig
// ---------------------------------------------------------------------------

/// Configuration for a single invocation of [`run_agent_loop`].
pub struct AgentLoopConfig {
    /// The session this agent loop is running in.
    pub session_id: uuid::Uuid,
    /// The initial agent definition (type, system prompt, tool restrictions).
    pub definition: crate::AgentDefinition,
    /// The session mode (Plan / Build).
    pub mode: SessionMode,
    /// Thinking / reasoning level.
    pub thinking_level: ThinkingLevelType,
    /// Channel for sending real-time events to the frontend.
    pub event_tx: UnboundedSender<BackendEvent>,
}

// ---------------------------------------------------------------------------
// ApprovedTool & PendingToolApproval
// ---------------------------------------------------------------------------

/// A tool call with an optional rejection reason.
///
/// Sent by the frontend through the permission channel to tell the agent loop
/// which tools are approved and which are rejected.
#[derive(Debug)]
pub struct ApprovedTool {
    pub tool_call: ToolCall,
    /// If `Some`, the tool is rejected; this [`ToolExecutionResult`] will be
    /// persisted as the tool's output. If `None`, the tool is approved.
    pub rejection: Option<ToolExecutionResult>,
    /// Pre-generated child session ID for subagent (task) tools.
    pub child_session_id: Option<uuid::Uuid>,
    /// Whether this tool call is allowed to access paths outside the workspace.
    pub allow_outside: bool,
    /// Whether this tool call is allowed to read sensitive files.
    pub sensitive_file_approved: bool,
}

/// Request sent by the agent loop to the frontend for tool permission approval.
///
/// The frontend sends back a `Vec<ApprovedTool>` through `response_tx`.
pub struct PendingToolApproval {
    pub tool_calls: Vec<ToolCall>,
    pub mode: SessionMode,
    pub response_tx: oneshot::Sender<Vec<ApprovedTool>>,
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
    fn event_tx(&self) -> UnboundedSender<BackendEvent>;

    /// Stream a single LLM turn and return the completed [`AssistantTurn`].
    ///
    /// The implementation should forward streaming [`BackendEvent`]s (deltas,
    /// tool call updates, usage stats, etc.) through [`Self::event_tx`] in
    /// real time.
    async fn stream_turn(
        &self,
        messages: &[Message],
        system_prompt: &str,
        thinking_level: &ThinkingLevelType,
    ) -> Result<AssistantTurn>;

    /// Request frontend approval for a batch of tool calls.
    ///
    /// Returns a list of [`ApprovedTool`]s where each entry either carries a
    /// rejection reason or is approved for execution.
    async fn request_tool_approval(
        &self,
        tool_calls: &[ToolCall],
        mode: SessionMode,
    ) -> Result<Vec<ApprovedTool>>;

    /// Execute a batch of approved tool calls and return their results.
    ///
    /// The implementation is responsible for:
    /// - Separating read-only vs write tools (parallel vs serial execution)
    /// - Handling `task` tools (subagent delegation)
    /// - Emitting [`BackendEvent::ToolCompleted`] events
    async fn execute_tools(
        &self,
        approved_tools: &[ApprovedTool],
        session_id: uuid::Uuid,
        request_id: u64,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>>;

    /// Persist one or more messages to the session store.
    async fn save_messages(&self, session_id: uuid::Uuid, messages: &[Message]) -> Result<()>;

    /// Load all messages for the current session.
    async fn load_messages(&self, session_id: uuid::Uuid) -> Result<Vec<Message>>;
}
