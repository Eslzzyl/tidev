//! tidev-acp — ACP (Agent Client Protocol) adapter for tidev.
//!
//! Bridges tidev's Runtime with ACP-compatible clients (Zed, VS Code, etc.)
//! over stdio JSON-RPC transport.

mod event_translator;
mod handler;
mod permission_bridge;
mod types;

pub use handler::run_acp_agent;
