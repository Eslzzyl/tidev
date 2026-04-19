//! Notification support for tidev.
//!
//! Provides cross-platform desktop notifications using OSC 9 (iTerm2, WezTerm, ghostty, Warp)
//! or BEL (terminal bell) protocols.

use std::env;
use std::fmt;
use std::io;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::Command;
use crossterm::execute;

use crate::config::NotificationConfig;

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
    // iTerm2 sets this
    if env::var_os("ITERM_SESSION_ID").is_some() {
        return true;
    }
    // TERM-based hints
    matches!(
        env::var("TERM").ok().as_deref(),
        Some("xterm-kitty" | "wezterm" | "wezterm-mux")
    )
}

// ============================================================================
// OSC 9 Backend
// ============================================================================

#[derive(Debug, Default)]
pub struct Osc9Backend;

impl Osc9Backend {
    pub fn notify(&mut self, message: &str) -> io::Result<()> {
        execute!(stdout(), PostNotification(message.to_string()))
    }
}

/// Command that emits an OSC 9 desktop notification.
#[derive(Debug, Clone)]
pub struct PostNotification(pub String);

impl Command for PostNotification {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b]9;{}\x07", self.0)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(std::io::Error::other(
            "tried to execute PostNotification using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

// ============================================================================
// BEL Backend
// ============================================================================

#[derive(Debug, Default)]
pub struct BelBackend;

impl BelBackend {
    pub fn notify(&mut self, _message: &str) -> io::Result<()> {
        execute!(stdout(), PostBel)
    }
}

/// Command that emits a BEL character.
#[derive(Debug, Clone)]
pub struct PostBel;

impl Command for PostBel {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x07")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(std::io::Error::other(
            "tried to execute PostBel using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

// ============================================================================
// Notification Manager
// ============================================================================

/// Manages desktop notifications.
pub struct NotificationManager {
    enabled: bool,
    backend: Option<DesktopNotificationBackend>,
    condition: NotificationCondition,
    focused: AtomicBool,
}

impl NotificationManager {
    /// Create a new notification manager from config.
    pub fn new(config: NotificationConfig) -> Self {
        let backend = if config.enabled {
            Some(detect_backend(&config.method))
        } else {
            None
        };

        let condition = NotificationCondition::parse(&config.condition);
        crate::log_info!(
            "NotificationManager: enabled={}, method={}, condition={:?}",
            config.enabled,
            config.method,
            condition
        );

        Self {
            enabled: config.enabled,
            backend,
            condition,
            focused: AtomicBool::new(true),
        }
    }

    /// Update terminal focus state.
    pub fn set_focused(&self, focused: bool) {
        crate::log_debug!("NotificationManager::set_focused({})", focused);
        self.focused.store(focused, Ordering::Relaxed);
    }

    /// Check if notification should be emitted based on condition and focus state.
    fn should_emit(&self) -> bool {
        if !self.enabled {
            crate::log_debug!("NotificationManager::should_emit: disabled");
            return false;
        }

        let focused = self.focused.load(Ordering::Relaxed);
        let result = match self.condition {
            NotificationCondition::Unfocused => !focused,
            NotificationCondition::Always => true,
        };
        crate::log_debug!(
            "NotificationManager::should_emit: focused={}, condition={:?}, result={}",
            focused,
            self.condition,
            result
        );
        result
    }

    /// Send a desktop notification.
    /// Returns true if notification was sent.
    pub fn notify(&mut self, message: &str) -> bool {
        crate::log_info!("NotificationManager::notify({:?})", message);

        if !self.should_emit() {
            crate::log_info!("NotificationManager::notify: skipped (should_emit=false)");
            return false;
        }

        let Some(backend) = self.backend.as_mut() else {
            crate::log_info!("NotificationManager::notify: skipped (no backend)");
            return false;
        };

        match backend.notify(message) {
            Ok(()) => {
                crate::log_info!("NotificationManager::notify: sent successfully");
                true
            }
            Err(err) => {
                crate::log_warn!(
                    "Failed to emit notification: {}, disabling future notifications",
                    err
                );
                self.backend = None;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_condition_parse() {
        assert_eq!(
            NotificationCondition::parse("always"),
            NotificationCondition::Always
        );
        assert_eq!(
            NotificationCondition::parse("unfocused"),
            NotificationCondition::Unfocused
        );
        assert_eq!(
            NotificationCondition::parse("invalid"),
            NotificationCondition::Unfocused
        );
    }

    #[test]
    fn test_notification_manager_disabled() {
        let config = NotificationConfig {
            enabled: false,
            ..Default::default()
        };
        let mut manager = NotificationManager::new(config);
        assert!(!manager.notify("test"));
    }

    #[test]
    fn test_should_emit_unfocused_when_focused() {
        let config = NotificationConfig {
            enabled: true,
            condition: "unfocused".to_string(),
            ..Default::default()
        };
        let manager = NotificationManager::new(config);
        manager.set_focused(true);
        assert!(!manager.should_emit());
    }

    #[test]
    fn test_should_emit_unfocused_when_unfocused() {
        let config = NotificationConfig {
            enabled: true,
            condition: "unfocused".to_string(),
            ..Default::default()
        };
        let manager = NotificationManager::new(config);
        manager.set_focused(false);
        assert!(manager.should_emit());
    }

    #[test]
    fn test_should_emit_always() {
        let config = NotificationConfig {
            enabled: true,
            condition: "always".to_string(),
            ..Default::default()
        };
        let manager = NotificationManager::new(config);
        manager.set_focused(true);
        assert!(manager.should_emit());
    }
}
