//! Shared utility functions for tidev.
//!
//! This crate provides path utilities, encoding helpers, and other utility
//! functions shared across multiple tidev crates.
//!
//! ## Modules
//!
//! * [`path`] — path canonicalization, workspace-boundary checking, display helpers
//! * [`encoding`] — command output byte decoding to UTF-8

pub mod encoding;
pub mod path;
pub mod session;
pub mod tmp;
pub mod tool_name;
