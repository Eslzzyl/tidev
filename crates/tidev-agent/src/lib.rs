//! tidev-agent: agent types, prompts, and execution loop.
//!
//! This crate defines the thin agent layer — the types, prompts, and loop
//! skeleton that are independent of any concrete runtime implementation.
//! tidev-core provides the real [`AgentContext`] and drives the loop.

pub mod agent_type;
pub mod context;
pub mod loop_;
pub mod prompts;

// Re-export types from tidev-types (defined there as shared protocol types).
pub use tidev_types::agent_type::{AgentDefinition, AgentOverride, AgentType};
pub use agent_type::{create_agent, create_all_agents, create_sub_agents};
pub use context::{AgentContext, AgentLoopConfig, ApprovedTool, ToolCallWithViolations, TuiRequest, TuiRequestKind, TuiResponse};
pub use loop_::run_agent_loop;
