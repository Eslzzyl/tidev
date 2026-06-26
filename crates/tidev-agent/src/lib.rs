//! tidev-agent — the agent runtime with Per-Session Event Bus.
//!
//! This crate provides:
//! - [`AgentLoop`]: the core LLM ↔ tool execution loop
//! - [`SessionManager`]: manages session lifecycle, each with its own event bus
//! - Shared types: [`SessionConfig`], [`SessionHandle`], [`SessionInfo`], etc.

mod agent_loop;
mod session_manager;
pub mod types;

pub use agent_loop::AgentLoop;
pub use session_manager::SessionManager;
pub use types::{
    AgentLoopConfig, AgentType, ApprovedTool, PendingToolApproval, QueuedUserMessage,
    SessionConfig, SessionHandle, SessionInfo, SubagentConfig,
};
