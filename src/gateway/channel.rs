//! Channel trait for gateway platform abstraction.
//!
//! Each platform (Telegram, QQ, etc.) implements this trait to provide
//! a unified interface for the orchestrator.

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

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

    /// Start the channel's main event loop.
    ///
    /// This method should run indefinitely until the channel
    /// encounters a fatal error or receives a shutdown signal.
    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>>;
}
