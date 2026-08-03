//! The [`AgentContext`] trait — the interface between the agent loop and the
//! concrete runtime environment provided by tidev-core.
//!
//! tidev-agent defines the loop skeleton; tidev-core implements [`AgentContext`].

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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
    /// Whether this run is read-only.
    pub read_only: bool,
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
// ApprovedTool, TuiRequest & TuiResponse
// ---------------------------------------------------------------------------

/// A tool call with an optional rejection reason.
///
/// Sent by the frontend through the permission channel to tell the agent loop
/// which tools are approved and which are rejected.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// An optional user-supplied reason attached during permission approval.
    /// When set, the reason is incorporated into the rejection output or
    /// passed along as context for why the tool was approved/rejected.
    pub user_reason: Option<String>,
}

/// A completed tool execution and its application-owned session association.
///
/// The child session ID is intentionally carried outside the protocol result
/// so it cannot leak into messages sent to an LLM provider.
#[derive(Clone, Debug)]
pub struct ExecutedTool {
    pub tool_call: ToolCall,
    pub result: ToolExecutionResult,
    pub child_session_id: Option<uuid::Uuid>,
}

/// A tool call augmented with pre-computed violation info for the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallWithViolations {
    pub tool_call: ToolCall,
    /// If the tool targets a path outside the workspace, the resolved path.
    pub workspace_boundary_violation: Option<PathBuf>,
    /// If the tool targets a sensitive file, the resolved path.
    pub sensitive_file_violation: Option<PathBuf>,
}

/// Request sent by the agent loop to the frontend, requiring user interaction.
///
/// The request is broadcast to every registered frontend (see
/// `Runtime::request_rx`). Any frontend may respond by sending a
/// [`TuiResponse`] through `response_tx`; the first response wins. If every
/// frontend drops its `response_tx` without sending, the agent loop treats it
/// as a rejection of all pending tools.
#[derive(Clone, Debug)]
pub struct TuiRequest {
    /// The session that originated this request.
    pub session_id: uuid::Uuid,
    pub kind: TuiRequestKind,
    pub response_tx: UnboundedSender<TuiResponse>,
}

/// Variants of [`TuiRequest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TuiRequestKind {
    /// Ask the user to approve or reject tool calls.
    ToolApproval(Vec<ToolCallWithViolations>),
}

/// Response sent by the frontend to the agent loop after user interaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    /// Request frontend approval for a batch of tool calls.
    ///
    /// Returns a list of [`ApprovedTool`]s where each entry either carries a
    /// rejection reason or is approved for execution.
    async fn request_tool_approval(
        &self,
        tool_calls: &[ToolCall],
        read_only: bool,
    ) -> Result<Vec<ApprovedTool>>;

    /// Execute a batch of approved tool calls and return their results.
    ///
    /// The implementation is responsible for:
    /// - Separating read-only vs write tools (parallel vs serial execution)
    /// - Handling `task` tools (subagent delegation)
    /// - Emitting [`AgentEvent::ToolCompleted`] events
    async fn execute_tools(
        &self,
        approved_tools: &[ApprovedTool],
        session_id: uuid::Uuid,
        request_id: u64,
    ) -> Result<Vec<ExecutedTool>>;

    /// Persist one or more messages to the session store.
    async fn save_messages(
        &self,
        session_id: uuid::Uuid,
        messages: &[Message],
        child_session_ids: &[(uuid::Uuid, uuid::Uuid)],
    ) -> Result<()>;

    /// Return the workspace root path.
    fn workspace_root(&self) -> &Path;

    /// Load all messages for the current session.
    async fn load_messages(&self, session_id: uuid::Uuid) -> Result<Vec<Message>>;

}
