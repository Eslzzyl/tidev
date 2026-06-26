//! Notification support for tidev.
//!
//! Provides cross-platform desktop notifications using OSC 9 (iTerm2, WezTerm, ghostty, Warp)
//! or BEL (terminal bell) protocols.

use std::env;
use std::fmt;
use std::io;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};

use tidev_config::NotificationConfig;

/// Notification condition - controls when notifications are emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationCondition {
    /// Emit notifications only while the terminal is unfocused.
    #[default]
    Unfocused,
    /// Emit notifications regardless of terminal focus.
    Always,
}

impl NotificationCondition {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "always" => NotificationCondition::Always,
            _ => NotificationCondition::Unfocused,
        }
    }
}

/// Notification method backend.
#[derive(Debug)]
pub enum DesktopNotificationBackend {
    /// OSC 9 protocol (iTerm2, WezTerm, ghostty, Warp).
    Osc9(Osc9Backend),
    /// BEL character (terminal bell).
    Bel(BelBackend),
}

impl DesktopNotificationBackend {
    /// Create backend based on notification method setting.
    pub fn for_method(method: &str) -> Self {
        match method.to_lowercase().as_str() {
            "osc9" => Self::Osc9(Osc9Backend),
            "bel" => Self::Bel(BelBackend),
            _ => {
                if supports_osc9() {
                    Self::Osc9(Osc9Backend)
                } else {
                    Self::Bel(BelBackend)
                }
            }
        }
    }

    /// Send a notification message.
    pub fn notify(&mut self, message: &str) -> io::Result<()> {
        match self {
            DesktopNotificationBackend::Osc9(backend) => backend.notify(message),
            DesktopNotificationBackend::Bel(backend) => backend.notify(message),
        }
    }
}

/// Detect the best available notification backend.
pub fn detect_backend(method: &str) -> DesktopNotificationBackend {
    DesktopNotificationBackend::for_method(method)
}

/// Check if the terminal supports OSC 9 protocol.
fn supports_osc9() -> bool {
    if env::var_os("WT_SESSION").is_some() {
        return false;
    }
    // Prefer TERM_PROGRAM when present
    if matches!(
        env::var("TERM_PROGRAM").ok().as_deref(),
        Some("WezTerm" | "WarpTerminal" | "ghostty")
    ) {
        return true;
    }
    // iTerm2 sets this to a version string
    if let Ok(val) = env::var("ITERM_PROFILE") {
        if !val.is_empty() {
            return true;
        }
    }
    // Apple Terminal does not support OSC 9
    true
}

// ---------------------------------------------------------------------------
// OSC 9 Backend
// ---------------------------------------------------------------------------

/// OSC 9 notification protocol (iTerm2, WezTerm, ghostty, Warp).
#[derive(Debug)]
pub struct Osc9Backend;

impl Osc9Backend {
    pub fn new() -> Self {
        Self
    }

    /// Send an OSC 9 notification.
    ///
    /// The message is wrapped in an OSC 9 escape sequence:
    /// `\e]9;{message}\e\` (iTerm2, WezTerm, ghostty, Warp).
    ///
    /// crossterm implements this with the `SetTitle` command which
    /// conveniently uses the same escape sequence.
    pub fn notify(&mut self, message: &str) -> io::Result<()> {
        // Use crossterm's SetTitle command which outputs \e]0;{title}\a
        // We need \e]9;{message}\a for OSC 9 notifications.
        // crossterm doesn't have a direct API, so we write the sequence.
        use std::io::Write;
        let mut stdout = stdout();
        write!(stdout, "\x1b]9;{}\x07", message)?;
        stdout.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BEL Backend
// ---------------------------------------------------------------------------

/// BEL (terminal bell) notification backend.
#[derive(Debug)]
pub struct BelBackend;

impl BelBackend {
    pub fn new() -> Self {
        Self
    }

    /// Send a BEL notification (terminal bell).
    pub fn notify(&mut self, _message: &str) -> io::Result<()> {
        use std::io::Write;
        let mut stdout = stdout();
        write!(stdout, "\x07")?;
        stdout.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NotificationManager
// ---------------------------------------------------------------------------

/// Manages desktop notifications.
///
/// Wraps a config and a backend, and provides a simple `notify()` method
/// that checks the enabled/disabled state and focus condition before
/// dispatching to the backend.
pub struct NotificationManager {
    enabled: bool,
    condition: NotificationCondition,
    method: String,
    focused: AtomicBool,
}

impl fmt::Debug for NotificationManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotificationManager")
            .field("enabled", &self.enabled)
            .field("condition", &self.condition)
            .field("method", &self.method)
            .finish()
    }
}

impl NotificationManager {
    pub fn new(config: NotificationConfig) -> Self {
        log::info!(
            "NotificationManager: enabled={}, method={}, condition={:?}",
            config.enabled,
            config.method,
            config.condition
        );

        Self {
            enabled: config.enabled,
            condition: NotificationCondition::parse(&config.condition),
            method: config.method,
            focused: AtomicBool::new(true),
        }
    }

    /// Mark the terminal as focused (so notifications may be suppressed).
    pub fn set_focused(&self, focused: bool) {
        log::debug!("NotificationManager::set_focused({})", focused);
        self.focused.store(focused, Ordering::SeqCst);
    }

    /// Check whether a notification should be emitted right now,
    /// based on the enabled flag and focus condition.
    fn should_emit(&self) -> bool {
        if !self.enabled {
            log::debug!("NotificationManager::should_emit: disabled");
            return false;
        }

        let focused = self.focused.load(Ordering::SeqCst);
        let result = match self.condition {
            NotificationCondition::Unfocused => !focused,
            NotificationCondition::Always => true,
        };

        log::debug!(
            "NotificationManager::should_emit: focused={}, condition={:?}, result={}",
            focused,
            self.condition,
            result
        );

        result
    }

    /// Send a notification.
    ///
    /// No-op if notifications are disabled or if the terminal is focused
    /// (when condition is `Unfocused`).
    pub fn notify(&self, message: &str) {
        log::info!("NotificationManager::notify({:?})", message);

        if !self.should_emit() {
            log::info!("NotificationManager::notify: skipped (should_emit=false)");
            return;
        }

        let mut backend = detect_backend(&self.method);
        if let Err(e) = backend.notify(message) {
            log::warn!("NotificationManager::notify: error: {e}");
        } else {
            log::info!("NotificationManager::notify: sent successfully");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(enabled: bool, condition: &str) -> NotificationConfig {
        NotificationConfig {
            enabled,
            method: "bel".to_string(),
            condition: condition.to_string(),
        }
    }

    #[test]
    fn test_disabled_never_emits() {
        let config = make_config(false, "unfocused");
        let manager = NotificationManager::new(config);
        manager.set_focused(false);
        assert!(!manager.should_emit());
    }

    #[test]
    fn test_focused_unfocused_condition() {
        let config = make_config(true, "unfocused");
        let manager = NotificationManager::new(config);
        manager.set_focused(true);
        assert!(!manager.should_emit());
    }

    #[test]
    fn test_should_emit_unfocused_when_unfocused() {
        let config = make_config(true, "unfocused");
        let manager = NotificationManager::new(config);
        manager.set_focused(false);
        assert!(manager.should_emit());
    }

    #[test]
    fn test_should_emit_always() {
        let config = make_config(true, "always");
        let manager = NotificationManager::new(config);
        manager.set_focused(true);
        assert!(manager.should_emit());
    }

    #[test]
    fn test_osc9_supported_terminals() {
        // We can't test actual detection without setting env vars,
        // but we can verify the function doesn't panic
        let _ = supports_osc9();
    }

    #[test]
    fn test_backend_selection() {
        let backend = detect_backend("bel");
        assert!(matches!(backend, DesktopNotificationBackend::Bel(_)));

        let backend = detect_backend("osc9");
        assert!(matches!(backend, DesktopNotificationBackend::Osc9(_)));
    }

    #[test]
    fn test_notify_disabled() {
        let config = make_config(false, "always");
        let manager = NotificationManager::new(config);
        // Just verify it doesn't panic
        manager.notify("test");
    }
}
