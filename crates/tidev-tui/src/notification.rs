//! Notification support for tidev.
//!
//! Provides cross-platform desktop notifications using OSC 9 (iTerm2, WezTerm, ghostty, Warp)
//! or BEL (terminal bell) protocols.

use std::env;
use std::fmt;
use std::io;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
        return !val.is_empty();
    }
    // Kitty uses this env var; check that it's actually kitty
    if env::var("KITTY_WINDOW_ID").is_ok() {
        return true;
    }
    false
}

/// OSC 9 notification backend (iTerm2, WezTerm, ghostty, Warp).
#[derive(Debug)]
pub struct Osc9Backend;

impl Osc9Backend {
    fn notify(&self, message: &str) -> io::Result<()> {
        use std::io::Write;
        // OSC 9 escape sequence: ESC ] 9 ; <message> ST
        // where ST is \x1b\ (or BEL \x07 on older terminals)
        write!(stdout(), "\x1b]9;{}\x1b\\", message)
    }
}

/// BEL notification backend (terminal bell).
#[derive(Debug)]
pub struct BelBackend;

impl BelBackend {
    fn notify(&self, _message: &str) -> io::Result<()> {
        use std::io::Write;
        // BEL — just ring the terminal bell
        write!(stdout(), "\x07")
    }
}

/// Notification manager with focus tracking.
pub struct NotificationManager {
    /// The notification backend (OSC 9 or BEL).
    backend: Option<DesktopNotificationBackend>,
    /// Notification condition (unfocused vs always).
    condition: NotificationCondition,
    /// Terminal focus state — shared between the TUI rendering thread
    /// and crossterm focus events.
    focused: Arc<AtomicBool>,
}

impl fmt::Debug for NotificationManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotificationManager")
            .field("backend", &self.backend)
            .field("condition", &self.condition)
            .field("focused", &self.focused)
            .finish()
    }
}

impl NotificationManager {
    pub fn new(config: NotificationConfig) -> Self {
        Self::new_impl(config.enabled, &config.method, &config.condition)
    }

    fn new_impl(enabled: bool, method: &str, condition: &str) -> Self {
        let backend = if enabled {
            Some(DesktopNotificationBackend::for_method(method))
        } else {
            None
        };

        Self {
            backend,
            condition: NotificationCondition::parse(condition),
            focused: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Update the terminal focus state.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused.store(focused, Ordering::Relaxed);
    }

    /// Check whether a notification should be emitted based on the current condition.
    pub fn should_emit(&self) -> bool {
        match self.condition {
            NotificationCondition::Always => true,
            NotificationCondition::Unfocused => !self.focused.load(Ordering::Relaxed),
        }
    }

    /// Emit a desktop notification if conditions allow.
    pub fn notify(&mut self, message: &str) {
        if !self.should_emit() {
            return;
        }
        if let Some(backend) = &mut self.backend
            && let Err(e) = backend.notify(message) {
                log::warn!("notification error: {e}");
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(enabled: bool, condition: &str) -> NotificationConfig {
        match (enabled, condition) {
            (true, c) => NotificationConfig {
                enabled: true,
                method: "bel".to_string(),
                condition: c.to_string(),
            },
            (false, _c) => NotificationConfig {
                enabled: false,
                method: "bel".to_string(),
                condition: "always".to_string(),
            },
        }
    }

    #[test]
    fn test_manager_always_emits_when_focused() {
        let config = make_config(true, "always");
        let mut manager = NotificationManager::new(config);
        manager.set_focused(true);
        assert!(manager.should_emit());
    }

    #[test]
    fn test_manager_always_emits_when_unfocused() {
        let config = make_config(true, "always");
        let mut manager = NotificationManager::new(config);
        manager.set_focused(false);
        assert!(manager.should_emit());
    }

    #[test]
    fn test_manager_unfocused_blocks_when_focused() {
        let config = make_config(true, "unfocused");
        let mut manager = NotificationManager::new(config);
        manager.set_focused(true);
        assert!(!manager.should_emit());
    }

    #[test]
    fn test_manager_unfocused_emits_when_unfocused() {
        let config = make_config(true, "unfocused");
        let mut manager = NotificationManager::new(config);
        manager.set_focused(false);
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
        let backend = DesktopNotificationBackend::for_method("bel");
        assert!(matches!(backend, DesktopNotificationBackend::Bel(_)));

        let backend = DesktopNotificationBackend::for_method("osc9");
        assert!(matches!(backend, DesktopNotificationBackend::Osc9(_)));
    }

    #[test]
    fn test_notify_disabled() {
        let config = make_config(false, "always");
        let mut manager = NotificationManager::new(config);
        // Just verify it doesn't panic
        manager.notify("test");
    }
}
