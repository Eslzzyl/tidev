//! Shared data types used by the agent runtime.
//!
//! These types define the configuration structs, message types, and permission
//! models that frontends use to interact with [`super::AgentRuntime`].

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::config::{ActiveModel, reasoning::ThinkingLevelType};
use crate::context::ContextManager;
use tidev_session::session::{BackendEvent, MessageAttachment, ToolCall, ToolExecutionResult};
use tidev_types::prompts::SessionMode;

/// A user message received while the agent loop was already processing a turn.
///
/// After the current turn completes, `run_agent_loop` picks up the next
/// queued message, persists it to the database, and continues the loop.
/// This is the shared mechanism for "type-ahead" across all frontends.
#[derive(Clone, Debug)]
pub struct QueuedUserMessage {
    pub content: String,
    pub attachments: Vec<MessageAttachment>,
    pub mode: Option<tidev_types::prompts::SessionMode>,
    pub thinking_level: Option<ThinkingLevelType>,
}

/// Configuration for the agent loop — groups session identity, model, context,
/// and execution-mode parameters shared by [`run_agent_loop`](super::AgentRuntime::run_agent_loop) and friends.
pub struct AgentLoopConfig<'a> {
    pub session_id: uuid::Uuid,
    pub model: ActiveModel,
    pub context_manager: &'a mut ContextManager,
    pub mode: SessionMode,
    pub thinking_level: ThinkingLevelType,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    pub cancel_token: Option<CancellationToken>,
}

/// Configuration for [`run_subagent`](super::AgentRuntime::run_subagent).
pub struct SubagentConfig {
    pub parent_session_id: uuid::Uuid,
    pub parent_request_id: u64,
    pub tool_call: ToolCall,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    pub cancel_token: Option<CancellationToken>,
    pub parent_model: ActiveModel,
    pub child_session_id: Option<uuid::Uuid>,
}

/// A tool call with an optional rejection reason.
///
/// Sent by frontends through the permission channel to tell
/// `run_agent_loop` which tools are approved and which are rejected.
#[derive(Debug)]
pub struct ApprovedTool {
    pub tool_call: ToolCall,
    /// If `Some`, the tool is rejected; this [`ToolExecutionResult`] will
    /// be persisted as the tool's output.  If `None`, the tool is approved
    /// for execution.
    pub rejection: Option<ToolExecutionResult>,
    /// Pre-generated child session ID for subagent (task) tools.
    /// When set, the runtime will use this ID instead of generating a random one,
    /// allowing the TUI to track and navigate to the child session accurately.
    pub child_session_id: Option<uuid::Uuid>,
    /// Whether this tool call is allowed to access paths outside the workspace.
    /// Set by the TUI frontend when the user approves a workspace boundary violation.
    pub allow_outside: bool,
    /// Whether this tool call is allowed to read sensitive files listed in
    /// `.tidev/sensitive.txt`.  Set by the TUI frontend when the user approves
    /// a sensitive file read.
    pub sensitive_file_approved: bool,
}

/// Request sent by `run_agent_loop` to the frontend for tool permission approval.
///
/// The frontend sends back a `Vec<ApprovedTool>` through `response_tx`.
pub struct PendingToolApproval {
    pub tool_calls: Vec<ToolCall>,
    pub mode: SessionMode,
    pub response_tx: oneshot::Sender<Vec<ApprovedTool>>,
}

impl std::fmt::Debug for PendingToolApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingToolApproval")
            .field("tool_calls", &self.tool_calls)
            .field("mode", &self.mode)
            .field("response_tx", &"oneshot::Sender<...>")
            .finish()
    }
}
