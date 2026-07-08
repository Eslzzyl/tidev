//! Notification state — transient popup messages with auto-expiry.
//!
//! Supports multiple overlapping notifications and terminal focus tracking.
//! Mirrors the old `tidev_tui::state::NotificationState` behaviour.

use std::time::{Duration, Instant};

/// Maximum number of visible notifications at once.
const MAX_VISIBLE: usize = 5;

/// A single notification entry.
#[derive(Clone, Debug)]
struct Notification {
    message: String,
    expires_at: Instant,
}

/// Notification state.
#[derive(Clone, Debug, Default)]
pub(crate) struct NotificationState {
    notifications: Vec<Notification>,
    focused: bool,
}

impl NotificationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the terminal as focused.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Whether the terminal currently has focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Add a notification that auto-expires after the given duration.
    pub fn add(&mut self, message: String, duration: Duration) {
        self.notifications.push(Notification {
            message,
            expires_at: Instant::now() + duration,
        });
        // Trim expired and overflow entries.
        self.prune();
    }

    /// Get all currently visible (non-expired) notifications.
    pub fn visible(&self) -> Vec<&str> {
        let now = Instant::now();
        self.notifications
            .iter()
            .filter(|n| n.expires_at > now)
            .map(|n| n.message.as_str())
            .collect()
    }

    /// Remove expired notifications.
    fn prune(&mut self) {
        let now = Instant::now();
        self.notifications.retain(|n| n.expires_at > now);
        // Keep at most MAX_VISIBLE most recent notifications.
        if self.notifications.len() > MAX_VISIBLE {
            self.notifications.drain(..self.notifications.len() - MAX_VISIBLE);
        }
    }
}
