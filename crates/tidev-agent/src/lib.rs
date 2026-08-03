//! tidev-agent: prompts and execution loop.
//!
//! This crate defines the thin agent layer — the prompts and loop skeleton
//! that are independent of any concrete runtime implementation.
//! tidev-core provides the real [`AgentContext`] and drives the loop.

pub mod context;
pub mod event;
pub mod loop_;
pub mod prompts;

// Re-export types from tidev-llm (defined there as shared protocol types).
pub use context::{
    AgentContext, AgentLoopConfig, ApprovedTool, ToolCallWithViolations, TuiRequest,
    TuiRequestKind, TuiResponse,
};
pub use event::{AgentEvent, llm_event_to_agent_event};
pub use loop_::run_agent_loop;
