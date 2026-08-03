//! tidev-core: core orchestration layer.

pub mod agent_ctx;
pub mod agent_type;
pub mod attachment;
pub mod backend_event;
pub mod context;
pub mod mcp;
pub mod message_buf;
pub mod prompts;
pub mod registry;
pub mod runtime;
pub mod session;
pub mod system_info;
pub mod undo;

pub use agent_ctx::CoreContext;
pub use backend_event::{BackendEvent, agent_event_to_backend_event};
pub use context::{CompactionResult, ContextManager};
pub use message_buf::MessageBuffer;
pub use registry::ToolRegistry;
pub use runtime::Runtime;
pub use session::SessionManager;

// Re-export approval types from tidev-agent so tidev-tui can access them
// without depending on tidev-agent directly.
pub use tidev_agent::{
    ApprovedTool, ToolCallWithViolations, TuiRequest, TuiRequestKind, TuiResponse,
};

// ── Backward compatibility with tidev-tui (old crate) ──────────────────
// These are deprecated and will be removed when tidev-tui is deleted.

/// Deprecated — kept for tidev-tui compatibility.
/// The new type is [`TuiRequest`].
#[derive(Debug)]
pub struct PendingToolApproval {
    pub tool_calls: Vec<tidev_llm::message::ToolCall>,
    pub mode: tidev_llm::mode::SessionMode,
    pub response_tx: tokio::sync::oneshot::Sender<Vec<ApprovedTool>>,
}

// Re-export storage/snapshot types so tidev-tui can use them
// without depending on tidev-storage / tidev-snapshot directly.
pub use tidev_snapshot::FileDiff;
pub use tidev_storage::SessionRecord;
