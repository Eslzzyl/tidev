use std::env;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::Command;
use crossterm::execute;
use tidev_config::NotificationConfig;

// ---------------------------------------------------------------------------
// NotificationCondition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum NotificationCondition {
    #[default]
    Unfocused,
    Always,
}

impl NotificationCondition {
    fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "always" => NotificationCondition::Always,
            _ => NotificationCondition::Unfocused,
        }
    }
}

// ---------------------------------------------------------------------------
// DesktopNotificationBackend
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum DesktopNotificationBackend {
    Osc9(Osc9Backend),
    Bel(BelBackend),
}

impl DesktopNotificationBackend {
    fn for_method(method: &str) -> Self {
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

    fn notify(&mut self, message: &str) -> io::Result<()> {
        match self {
            DesktopNotificationBackend::Osc9(backend) => backend.notify(message),
            DesktopNotificationBackend::Bel(backend) => backend.notify(message),
        }
    }
}

fn supports_osc9() -> bool {
    if env::var_os("WT_SESSION").is_some() {
        return false;
    }
    if matches!(
        env::var("TERM_PROGRAM").ok().as_deref(),
        Some("WezTerm" | "WarpTerminal" | "ghostty")
    ) {
        return true;
    }
    if env::var_os("ITERM_SESSION_ID").is_some() {
        return true;
    }
    matches!(
        env::var("TERM").ok().as_deref(),
        Some("xterm-kitty" | "wezterm" | "wezterm-mux")
    )
}

// ---------------------------------------------------------------------------
// OSC 9 Backend
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct Osc9Backend;

impl Osc9Backend {
    fn notify(&mut self, message: &str) -> io::Result<()> {
        execute!(io::stdout(), PostNotification(message.to_string()))
    }
}

#[derive(Debug, Clone)]
struct PostNotification(String);

impl Command for PostNotification {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b]9;{}\x07", self.0)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::other(
            "tried to execute PostNotification using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// BEL Backend
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct BelBackend;

impl BelBackend {
    fn notify(&mut self, _message: &str) -> io::Result<()> {
        execute!(io::stdout(), PostBel)
    }
}

#[derive(Debug, Clone)]
struct PostBel;

impl Command for PostBel {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x07")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::other(
            "tried to execute PostBel using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// NotificationManager
// ---------------------------------------------------------------------------

pub(crate) struct NotificationManager {
    enabled: bool,
    backend: Option<DesktopNotificationBackend>,
    condition: NotificationCondition,
    focused: AtomicBool,
}

impl NotificationManager {
    pub fn new(config: &NotificationConfig) -> Self {
        let backend = if config.enabled {
            Some(DesktopNotificationBackend::for_method(&config.method))
        } else {
            None
        };

        let condition = NotificationCondition::parse(&config.condition);
        log::info!(
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

    pub fn set_focused(&self, focused: bool) {
        log::debug!("NotificationManager::set_focused({})", focused);
        self.focused.store(focused, Ordering::Relaxed);
    }

    fn should_emit(&self) -> bool {
        if !self.enabled {
            log::debug!("NotificationManager::should_emit: disabled");
            return false;
        }
        let focused = self.focused.load(Ordering::Relaxed);
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

    pub fn notify(&mut self, message: &str) -> bool {
        log::info!("NotificationManager::notify({:?})", message);

        if !self.should_emit() {
            log::info!("NotificationManager::notify: skipped (should_emit=false)");
            return false;
        }

        let Some(backend) = self.backend.as_mut() else {
            log::info!("NotificationManager::notify: skipped (no backend)");
            return false;
        };

        match backend.notify(message) {
            Ok(()) => {
                log::info!("NotificationManager::notify: sent successfully");
                true
            }
            Err(err) => {
                log::warn!(
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
        let mut manager = NotificationManager::new(&config);
        assert!(!manager.notify("test"));
    }

    #[test]
    fn test_should_emit_unfocused_when_focused() {
        let config = NotificationConfig {
            enabled: true,
            condition: "unfocused".to_string(),
            ..Default::default()
        };
        let manager = NotificationManager::new(&config);
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
        let manager = NotificationManager::new(&config);
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
        let manager = NotificationManager::new(&config);
        manager.set_focused(true);
        assert!(manager.should_emit());
    }
}
