use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::{BackendEvent, MessageAttachment, ToolCall, ToolExecutionResult};
use tidev_storage::SessionStore;
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
    pub mode: Option<SessionMode>,
    pub thinking_level: Option<tidev_config::reasoning::ThinkingLevelType>,
}

/// Configuration for spawning a new agent session.
#[derive(Clone)]
pub struct SessionConfig {
    /// Parent session ID, if this is a sub-agent.
    pub parent_session_id: Option<Uuid>,
    /// The model configuration for this session.
    pub model: tidev_config::ActiveModel,
    /// Tool definitions available to this session.
    pub tools: Vec<tidev_tools::ToolDefinition>,
    /// Shared session store for persistence.
    pub store: Arc<Mutex<SessionStore>>,
    /// Optional workspace root.
    pub workspace_root: Option<std::path::PathBuf>,
}

/// A handle to a running session, obtained from [`SessionManager::spawn`].
pub struct SessionHandle {
    /// The session ID.
    pub session_id: Uuid,
    /// Receive end of the session's event channel.
    pub event_rx: UnboundedReceiver<BackendEvent>,
    /// Cancellation token for this session.
    pub cancel_token: CancellationToken,
}

/// Information about an active session.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub session_id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub agent_type: AgentType,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

/// Agent type classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentType {
    General,
    Explorer,
    Librarian,
    Oracle,
    Designer,
    Fixer,
}

impl AgentType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "general" => Some(Self::General),
            "explorer" => Some(Self::Explorer),
            "librarian" => Some(Self::Librarian),
            "oracle" => Some(Self::Oracle),
            "designer" => Some(Self::Designer),
            "fixer" => Some(Self::Fixer),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Explorer => "explorer",
            Self::Librarian => "librarian",
            Self::Oracle => "oracle",
            Self::Designer => "designer",
            Self::Fixer => "fixer",
        }
    }

    /// Return all variants of AgentType for iteration.
    pub fn variants() -> &'static [Self] {
        &[
            Self::General,
            Self::Explorer,
            Self::Librarian,
            Self::Oracle,
            Self::Designer,
            Self::Fixer,
        ]
    }
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
    pub child_session_id: Option<Uuid>,
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

/// Configuration for the agent loop — groups session identity, model, context,
/// and execution-mode parameters.
pub struct AgentLoopConfig<'a> {
    pub session_id: Uuid,
    pub model: tidev_config::ActiveModel,
    pub context_manager: &'a mut tidev_context::ContextManager,
    pub mode: SessionMode,
    pub thinking_level: tidev_config::reasoning::ThinkingLevelType,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    pub cancel_token: Option<CancellationToken>,
}

/// Configuration for running a sub-agent via the `task` tool.
pub struct SubagentConfig {
    pub parent_session_id: Uuid,
    pub parent_request_id: u64,
    pub tool_call: ToolCall,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    pub cancel_token: Option<CancellationToken>,
    pub parent_model: tidev_config::ActiveModel,
    pub child_session_id: Option<Uuid>,
}
