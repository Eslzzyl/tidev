use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::BackendEvent;
use tidev_storage::SessionStore;

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
}

/// A tool call awaiting user approval.
#[derive(Clone, Debug)]
pub struct PendingToolApproval {
    pub session_id: Uuid,
    pub request_id: u64,
    pub tool_call: tidev_session::session::ToolCall,
    pub tool_definition: tidev_tools::ToolDefinition,
}

/// A tool call that has been approved by the user.
#[derive(Clone, Debug)]
pub struct ApprovedTool {
    pub tool_call: tidev_session::session::ToolCall,
    pub tool_definition: tidev_tools::ToolDefinition,
}
