//! tidev-acp — ACP (Agent Client Protocol) adapter for tidev.
//!
//! Bridges tidev's Runtime with ACP-compatible clients (Zed, VS Code, etc.)
//! over stdio JSON-RPC transport.

mod common;
mod v1;
#[cfg(feature = "acp-v2")]
mod v2;

pub use v1::handler::run_acp_agent;
