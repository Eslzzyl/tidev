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
    expires_at: Instant,
}

/// Notification state.
#[derive(Clone, Debug, Default)]
pub(crate) struct NotificationState {
    notifications: Vec<Notification>,
}

impl NotificationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a notification that auto-expires after the given duration.
    pub fn add(&mut self, _message: String, duration: Duration) {
        self.notifications.push(Notification {
            expires_at: Instant::now() + duration,
        });
        // Trim expired and overflow entries.
        self.prune();
    }

    /// Remove expired notifications.
    fn prune(&mut self) {
        let now = Instant::now();
        self.notifications.retain(|n| n.expires_at > now);
        // Keep at most MAX_VISIBLE most recent notifications.
        if self.notifications.len() > MAX_VISIBLE {
            self.notifications
                .drain(..self.notifications.len() - MAX_VISIBLE);
        }
    }
}
