//! tidev-agent: execution loop.
//!
//! This crate defines the thin agent layer — the loop skeleton
//! that are independent of any concrete runtime implementation.
//! tidev-core provides the real [`AgentContext`] and drives the loop.

pub mod context;
pub mod event;
pub mod loop_;

// Re-export types from tidev-llm (defined there as shared protocol types).
pub use context::{
    AgentContext, AgentLoopConfig, ApprovedTool, ExecutedTool, ToolCallWithViolations, TuiRequest,
    TuiRequestKind, TuiResponse,
};
pub use event::{AgentEvent, llm_event_to_agent_event};
pub use loop_::run_agent_loop;
