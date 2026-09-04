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
pub mod profile;
pub mod registry;
pub mod runtime;
pub mod scheduler;
pub mod subagent;
pub mod tool;
pub mod turn;

/// Ensures Reqwest can build Rustls clients with the Ring provider.
pub(crate) fn ensure_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

// Re-export types from tidev-llm (defined there as shared protocol types).
pub use context::{AgentContext, AgentLoopConfig};
pub use context_manager::{CompactionResult, ContextManager, ContextPreparation};
pub use event::{AgentEvent, AgentEventSender, AgentEventSink, llm_event_to_agent_event};
pub use loop_::run_agent_loop;
pub use mcp::{McpConnectionStatus, McpRegistry, McpServerSpec, McpServerSummary, McpToolInfo};
pub use message_buf::MessageBuffer;
pub use profile::{AgentDefinition, AgentOverride};
pub use registry::ToolRegistry;
pub use runtime::{AgentRuntime, MessageStore};
pub use scheduler::{ToolCallExecutor, execute_tool_calls};
pub use subagent::{
    SubagentEventSink, SubagentExecution, SubagentExecutor, execute_subagent_calls,
};
pub use tidev_llm;
pub use tool::{Tool, ToolContext};
pub use turn::{StreamTurnOptions, order_tool_results, stream_turn};
