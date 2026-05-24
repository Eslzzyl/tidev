//! Channel trait for gateway platform abstraction.
//!
//! Each platform (Telegram, QQ, etc.) implements this trait to provide
//! a unified interface for the orchestrator.

use anyhow::Result;
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;

use tidev_storage::SessionStore;

/// Message to send through a channel.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SendMessage {
    /// Message content.
    pub content: String,
    /// Recipient identifier (e.g., chat_id for Telegram, channel_id for QQ).
    pub recipient: String,
    /// Optional subject (for platforms that support it).
    pub subject: Option<String>,
    /// Platform thread identifier for threaded replies.
    pub thread_ts: Option<String>,
}

impl SendMessage {
    /// Create a new message with content and recipient.
    #[allow(dead_code)]
    pub fn new(content: impl Into<String>, recipient: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            recipient: recipient.into(),
            subject: None,
            thread_ts: None,
        }
    }

    /// Set the thread/thread ID for this message.
    #[allow(dead_code)]
    pub fn in_thread(mut self, thread_ts: impl Into<String>) -> Self {
        self.thread_ts = Some(thread_ts.into());
        self
    }
}

/// A gateway channel that can receive and respond to messages.
///
/// Implementations handle platform-specific connection logic,
/// message parsing, and response delivery.
///
/// Note: This trait does not require `Send` because channels run
/// within a `LocalSet` and share non-thread-safe resources like
/// `rusqlite::Connection`.
#[async_trait]
#[allow(dead_code)]
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

    // ── Draft API ─────────────────────────────────────────────────────
    /// Whether this channel supports progressive message updates via draft edits.
    fn supports_draft_updates(&self) -> bool {
        false
    }

    /// Whether this channel supports multi-message streaming delivery.
    fn supports_multi_message_streaming(&self) -> bool {
        false
    }

    /// Minimum delay (ms) between sending each paragraph in multi-message mode.
    fn multi_message_delay_ms(&self) -> u64 {
        800
    }

    /// Send an initial draft message. Returns a platform-specific message ID for later edits.
    ///
    /// Returns `Ok(None)` if the channel doesn't support drafts (will use fallback behavior).
    async fn send_draft(&mut self, _message: &SendMessage) -> Result<Option<String>> {
        Ok(None)
    }

    /// Update a previously sent draft message with new accumulated content.
    async fn update_draft(
        &mut self,
        _recipient: &str,
        _message_id: &str,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Show a progress/status update (e.g. tool execution status).
    ///
    /// This is separate from `update_draft` to allow channels to distinguish
    /// between content updates and status updates.
    async fn update_draft_progress(
        &mut self,
        _recipient: &str,
        _message_id: &str,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Finalize a draft with the complete response (e.g. apply Markdown formatting).
    ///
    /// This is called when the agent has completed its response. Channels can
    /// use this to apply final formatting or handle multi-chunk messages.
    async fn finalize_draft(
        &mut self,
        _recipient: &str,
        _message_id: &str,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Cancel and remove a previously sent draft message.
    ///
    /// Called when the task is cancelled or failed.
    async fn cancel_draft(&mut self, _recipient: &str, _message_id: &str) -> Result<()> {
        Ok(())
    }
}
