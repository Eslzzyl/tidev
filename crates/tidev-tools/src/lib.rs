//! tidev-tools: built-in tool implementations for the tidev agent.
//!
//! This crate provides:
//!
//! - All builtin tool implementations (file read/write/edit, shell, glob/grep,
//!   web search/fetch, todo, task delegation, apply_patch, question, skill)
//! - [`execute_tool_call`] dispatch routing tool names to implementations
//! - [`ToolContext`] carrying shared configuration into every tool invocation
//! - [`SkillCatalog`] for discovering and serving skill files
//! - [`TodoPersistence`] trait — a 2-method abstraction that lets tidev-core
//!   bridge todo storage without tidev-tools depending on tidev-storage
//! - [`shell`] module — shell detection for command execution

pub mod builtin;
pub mod shell;
pub mod skills;
pub mod todo_persistence;
pub mod types;

mod bundled_skills;

/// Ensures Reqwest can build Rustls clients with the Ring provider.
pub(crate) fn ensure_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

// Re-export key public types.
pub use builtin::definitions as tool_definitions;
pub use builtin::execute_tool_call;
pub use builtin::kill_all_children;
pub use builtin::{ShellOutput, ToolContext};
pub use skills::{SkillCatalog, SkillInfo};
pub use todo_persistence::TodoPersistence;
