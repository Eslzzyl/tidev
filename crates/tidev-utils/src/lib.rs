//! Shared utility functions for tidev.
//!
//! This crate provides path utilities, encoding helpers, and other utility
//! functions shared across multiple tidev crates.
//!
//! ## Modules
//!
//! * [`path`] — path canonicalization, workspace-boundary checking, display helpers
//! * [`encoding`] — text and command output decoding to UTF-8 with legacy
//!   encoding detection and source-encoding preservation

pub mod encoding;
pub mod path;
pub mod session;
pub mod tmp;
pub mod tool_name;
