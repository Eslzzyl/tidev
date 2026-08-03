//! Host-owned approval messages and decisions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use tidev_llm::message::{ToolCall, ToolExecutionResult};

use crate::Mode;

/// A tool call with an optional rejection reason.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovedTool {
    pub tool_call: ToolCall,
    /// If `Some`, this result is returned to the model instead of executing
    /// the tool.
    pub rejection: Option<ToolExecutionResult>,
    /// Pre-generated child session ID for a subagent tool.
    pub child_session_id: Option<uuid::Uuid>,
    /// Whether this tool call may access paths outside the workspace.
    pub allow_outside: bool,
    /// Whether this tool call may read sensitive files.
    pub sensitive_file_approved: bool,
    /// Optional user-supplied explanation for the decision.
    pub user_reason: Option<String>,
}

/// A tool call augmented with pre-computed violation information for an
/// approval frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallWithViolations {
    pub tool_call: ToolCall,
    pub workspace_boundary_violation: Option<PathBuf>,
    pub sensitive_file_violation: Option<PathBuf>,
}

/// Request sent by the host to a frontend that can approve tools.
#[derive(Clone, Debug)]
pub struct TuiRequest {
    pub session_id: uuid::Uuid,
    pub kind: TuiRequestKind,
    pub response_tx: UnboundedSender<TuiResponse>,
}

/// Host approval request variants.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TuiRequestKind {
    ToolApproval(Vec<ToolCallWithViolations>),
}

/// Response sent by an approval frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TuiResponse {
    ToolApproval(Vec<ApprovedTool>),
}

/// Compatibility carrier for the legacy TUI permission flow.
#[derive(Debug)]
pub struct PendingToolApproval {
    pub tool_calls: Vec<ToolCall>,
    pub mode: Mode,
    pub response_tx: tokio::sync::oneshot::Sender<Vec<ApprovedTool>>,
}
