//! tidev-agent: execution loop.
//!
//! This crate defines the thin agent layer — the loop skeleton
//! that are independent of any concrete runtime implementation.
//! tidev-core provides the real [`AgentContext`] and drives the loop.

pub mod context;
pub mod context_manager;
pub mod event;
pub mod loop_;
pub mod mcp;
pub mod message_buf;
pub mod registry;
pub mod tool;

// Re-export types from tidev-llm (defined there as shared protocol types).
pub use context::{
    AgentContext, AgentLoopConfig,
};
pub use context_manager::{CompactionResult, ContextManager};
pub use event::{AgentEvent, llm_event_to_agent_event};
pub use loop_::run_agent_loop;
pub use mcp::{
    McpConnectionStatus, McpRegistry, McpServerSpec, McpServerSummary, McpToolInfo,
};
pub use message_buf::MessageBuffer;
pub use registry::ToolRegistry;
pub use tool::{Tool, ToolContext};
