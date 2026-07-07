//! tidev-core: core orchestration layer.

pub mod attachment;
pub mod message_buf;
pub mod registry;
pub mod context;
pub mod session;
pub mod agent_ctx;
pub mod runtime;
pub mod undo;

pub use message_buf::MessageBuffer;
pub use registry::ToolRegistry;
pub use context::{CompactionResult, ContextManager};
pub use session::SessionManager;
pub use agent_ctx::CoreContext;
pub use runtime::Runtime;

// Re-export approval types from tidev-agent so tidev-tui can access them
// without depending on tidev-agent directly.
pub use tidev_agent::{ApprovedTool, ToolCallWithViolations, TuiRequest, TuiRequestKind, TuiResponse};

// ── Backward compatibility with tidev-tui (old crate) ──────────────────
// These are deprecated and will be removed when tidev-tui is deleted.

/// Deprecated — kept for tidev-tui compatibility.
/// The new type is [`TuiRequest`].
#[derive(Debug)]
pub struct PendingToolApproval {
    pub tool_calls: Vec<tidev_types::message::ToolCall>,
    pub mode: tidev_types::prompts::SessionMode,
    pub response_tx: tokio::sync::oneshot::Sender<Vec<ApprovedTool>>,
}

// Re-export storage/snapshot types so tidev-tui can use them
// without depending on tidev-storage / tidev-snapshot directly.
pub use tidev_storage::SessionRecord;
pub use tidev_snapshot::FileDiff;
