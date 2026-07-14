//! The [`AgentContext`] trait — the interface between the agent loop and the
//! concrete runtime environment provided by tidev-core.
//!
//! tidev-agent defines the loop skeleton; tidev-core implements [`AgentContext`].

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use tokio_util::sync::CancellationToken;

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
    /// Cancellation token for cooperative termination.
    pub cancel: CancellationToken,
}

// ---------------------------------------------------------------------------
// ApprovedTool, TuiRequest & TuiResponse
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

/// A tool call augmented with pre-computed violation info for the UI.
#[derive(Debug)]
pub struct ToolCallWithViolations {
    pub tool_call: ToolCall,
    /// If the tool targets a path outside the workspace, the resolved path.
    pub workspace_boundary_violation: Option<PathBuf>,
    /// If the tool targets a sensitive file, the resolved path.
    pub sensitive_file_violation: Option<PathBuf>,
    /// Stable permission key for for DB lookups.
    pub permission_key: String,
    /// Human-readable label.
    pub permission_label: String,
}

/// Request sent by the agent loop to the frontend, requiring user interaction.
///
/// The frontend processes the request and sends back a [`TuiResponse`] through
/// `response_tx`. If the sender is dropped without sending, the agent loop
/// treats it as a rejection of all pending tools.
pub struct TuiRequest {
    pub kind: TuiRequestKind,
    pub response_tx: oneshot::Sender<TuiResponse>,
}

/// Variants of [`TuiRequest`].
#[derive(Debug)]
pub enum TuiRequestKind {
    /// Ask the user to approve or reject tool calls.
    ToolApproval(Vec<ToolCallWithViolations>),
}

/// Response sent by the frontend to the agent loop after user interaction.
pub enum TuiResponse {
    /// User decisions for a [`TuiRequestKind::ToolApproval`] request.
    ToolApproval(Vec<ApprovedTool>),
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

    /// Update the content of an existing message in-place (both buffer and store).
    async fn update_message_content(
        &self,
        session_id: uuid::Uuid,
        message_id: uuid::Uuid,
        content: String,
    ) -> Result<()>;
}
