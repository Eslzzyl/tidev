//! Shared types for the tidev-agent runtime.
//!
//! These types define the configuration structs, permission models,
//! and session handles that frontends use to interact with the agent runtime.

use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::{BackendEvent, MessageAttachment, ToolCall, ToolExecutionResult};
use tidev_types::agent::AgentType;
use tidev_types::prompts::SessionMode;

/// A user message received while the agent loop was already processing a turn.
///
/// After the current turn completes, the agent loop picks up the next
/// queued message, persists it to the database, and continues the loop.
/// This is the shared mechanism for "type-ahead" across all frontends.
#[derive(Clone, Debug)]
pub struct QueuedUserMessage {
    pub content: String,
    pub attachments: Vec<MessageAttachment>,
    pub mode: Option<SessionMode>,
    pub thinking_level: Option<tidev_config::reasoning::ThinkingLevelType>,
}

/// A fully configured agent definition with resolved system prompt and tool settings.
#[derive(Clone, Debug)]
pub struct AgentDefinition {
    /// The agent type.
    pub agent_type: AgentType,
    /// Human-readable display name (e.g. "explorer").
    pub display_name: String,
    /// Short description for tool definitions and UI.
    pub description: String,
    /// The system prompt sent to the LLM.
    pub system_prompt: String,
    /// Optional tool name restrictions. `None` = all tools allowed.
    pub allowed_tools: Option<Vec<String>>,
    /// Optional model override for this agent. `None` = inherit from parent session.
    pub model_override: Option<tidev_config::ActiveModel>,
    /// Temperature override. `None` = use default for agent type.
    pub temperature: Option<f32>,
    /// Whether this agent is read-only.
    pub read_only: bool,
}

/// A tool call with an optional rejection reason.
///
/// Sent by frontends through the permission channel to tell
/// the agent loop which tools are approved and which are rejected.
#[derive(Clone, Debug)]
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

/// Request sent by the agent loop to the frontend for tool permission approval.
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
    /// Workspace root for this session.
    pub workspace_root: std::path::PathBuf,
    /// The composed static system prompt (frozen for session lifetime).
    pub system_prompt: String,
}

/// Configuration for spawning a new agent session.
#[derive(Clone)]
pub struct SessionConfig {
    /// Parent session ID, if this is a sub-agent.
    pub parent_session_id: Option<Uuid>,
    /// The model configuration for this session.
    pub model: tidev_config::ActiveModel,
    /// Shared session store for persistence.
    pub store: Arc<tokio::sync::Mutex<tidev_storage::SessionStore>>,
    /// Optional workspace root.
    pub workspace_root: Option<PathBuf>,
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

/// Events from SessionManager to the frontend.
///
/// Unlike [`BackendEvent`] (which streams per-session LLM/tool events to the
/// active conversation view), `FrontendEvent` carries lifecycle notifications
/// that the frontend uses to manage session subscription and overlay rendering.
///
/// Each subagent session gets its own [`BackendEvent`] channel. The frontend
/// receives the receiver end via `SubagentSpawned` and reads directly from it,
/// eliminating the need for aggregate subagent events.
#[derive(Debug)]
pub enum FrontendEvent {
    /// A child subagent session has started.
    /// The frontend should subscribe to `event_rx` for inline rendering
    /// (e.g. a subagent card overlay in the parent conversation).
    SubagentSpawned {
        child_session_id: Uuid,
        parent_session_id: Uuid,
        agent_type: AgentType,
        description: String,
        /// Receiver for the child session's own BackendEvent stream.
        event_rx: UnboundedReceiver<BackendEvent>,
    },
    /// A child subagent session has completed.
    SubagentFinished {
        child_session_id: Uuid,
        parent_session_id: Uuid,
    },
}

/// Shared mutable state for the agent runtime.
///
/// Holds the queued messages and other runtime-global state
/// that must be accessible from both the frontend and agent loops.
pub struct SharedAgentState {
    /// Queue of user messages received while the agent loop is running.
    /// After each turn completes, the loop processes the next message
    /// automatically. Frontends push through [`queue_user_message`].
    pub queued_messages: Mutex<VecDeque<QueuedUserMessage>>,
}

impl Default for SharedAgentState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedAgentState {
    pub fn new() -> Self {
        Self {
            queued_messages: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueue a user message for processing after the current turn ends.
    pub fn queue_user_message(&self, msg: QueuedUserMessage) {
        if let Ok(mut queue) = self.queued_messages.lock() {
            queue.push_back(msg);
        }
    }

    /// Pop the next queued message, if any.
    pub fn pop_queued_message(&self) -> Option<QueuedUserMessage> {
        self.queued_messages.lock().ok()?.pop_front()
    }
}

/// Compose the static system prompt — called exactly once per session lifetime.
///
/// Content: base prompt + environment info.
/// Result is persisted to the session DB record and never changes.
pub fn compose_static_system_prompt(base_prompt: &str, workspace_root: &std::path::Path) -> String {
    let base_prompt = base_prompt.trim();
    let system_info = tidev_system_info::SystemInfo::detect();
    let working_dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let is_git = tidev_system_info::is_git_repo(workspace_root);

    let mut prompt = String::new();
    if !base_prompt.is_empty() {
        prompt.push_str(base_prompt);
    }
    prompt.push_str("\n\nHere is some useful information about the environment:\n<env>\n  ");
    prompt.push_str(&format!("Working directory: {}\n  ", working_dir));
    prompt.push_str(&format!(
        "Workspace root folder: {}\n  ",
        workspace_root.display()
    ));
    prompt.push_str(&format!(
        "Is directory a git repo: {}\n  ",
        if is_git { "yes" } else { "no" }
    ));
    prompt.push_str(&system_info.format_env());
    prompt.push_str("\n</env>");
    prompt
}
