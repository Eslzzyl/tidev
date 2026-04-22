//! Channel trait for gateway platform abstraction.
//!
//! Each platform (Telegram, QQ, etc.) implements this trait to provide
//! a unified interface for the orchestrator.

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

use crate::storage::SessionStore;

/// A gateway channel that can receive and respond to messages.
///
/// Implementations handle platform-specific connection logic,
/// message parsing, and response delivery.
///
/// Note: This trait does not require `Send` because channels run
/// within a `LocalSet` and share non-thread-safe resources like
/// `rusqlite::Connection`.
pub trait Channel {
    /// Human-readable channel name (e.g., "telegram", "qq").
    fn name(&self) -> &'static str;

    /// Get the channel's session store.
    ///
    /// Returns None if the channel doesn't have a session store.
    fn store(&self) -> Option<&SessionStore> {
        None
    }

    /// Start the channel's main event loop.
    ///
    /// This method should run indefinitely until the channel
    /// encounters a fatal error or receives a shutdown signal.
    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>>;

    /// Restore sessions from persistent storage.
    ///
    /// Called at startup to hydrate in-memory conversation histories
    /// from previously persisted sessions.
    ///
    /// Returns the number of sessions restored.
    /// Note: Takes ownership of the SessionStore since it's only read.
    fn restore_sessions(&mut self, _store: SessionStore) -> Result<usize> {
        Ok(0)
    }
}
