//! tidev-core: core orchestration layer.

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
pub use tidev_agent::{ApprovedTool, PendingToolApproval};
