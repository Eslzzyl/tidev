//! Git workspace snapshot service for tidev.
//!
//! Provides [`SnapshotService`] for capturing and reverting workspace state
//! using a dedicated Git repository.

pub mod git;
mod service;

pub use service::*;
