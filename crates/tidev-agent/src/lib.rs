//! tidev-agent — the agent runtime with Per-Session Event Bus.
//!
//! This crate provides:
//! - [`AgentLoop`]: the core LLM ↔ tool execution loop
//! - [`SessionManager`]: manages session lifecycle, each with its own event bus
//! - Shared types: [`AgentDefinition`], [`ApprovedTool`], [`PendingToolApproval`], etc.
//! - System prompts and factory functions for all built-in agent types
//!
//! ## Architecture
//!
//! Each session runs its own [`AgentLoop`] with an independent event channel
//! (Per-Session Event Bus). The [`SessionManager`] holds only shared resources
//! (store, LLM client) and is responsible for spawning/cancelling sessions.
//!
//! Frontend-specific state (workspace root, config, tools, hooks) is NOT
//! stored in SessionManager — it lives in the frontend and is passed to
//! [`AgentLoop`] at construction time.

mod agent_loop;
pub mod persistence;
mod session_manager;
pub mod types;
#[cfg(test)]
mod tests;

pub mod factories;
pub mod prompts;

pub use agent_loop::{AgentLoop, ToolExecResult};
pub use session_manager::SessionManager;
pub use types::{
    AgentDefinition, AgentLoopConfig, ApprovedTool, ControlEvent, PendingToolApproval,
    QueuedUserMessage, SessionConfig, SessionHandle, SessionInfo, SharedAgentState,
    SubagentConfig, compose_static_system_prompt,
};
