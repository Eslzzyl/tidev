//! Component context types.

use std::path::Path;

use crate::chat_context::ChatContext;
use crate::theme::ThemePalette;
use tidev_types::prompts::SessionMode;
use tidev_types::reasoning::ThinkingLevelType;

/// Immutable resources passed to every component during initialisation.
pub(crate) struct InitContext<'a> {
    pub config: &'a tidev_config::AppConfig,
    pub auth: &'a tidev_config::AuthStore,
    pub workspace_root: &'a Path,
}

/// Read-only shared data passed to every component each frame during draw.
pub(crate) struct DrawContext<'a> {
    pub palette: ThemePalette,
    pub focused: bool,
    pub chat_context: Option<&'a ChatContext>,
    /// Current session mode (Build/Plan).
    pub mode: SessionMode,
    /// Pending mode switch (shown as "Build → Plan").
    pub pending_mode: Option<SessionMode>,
    /// Active model display name (shown in composer metadata).
    pub model_display: Option<&'a str>,
    /// Active provider display name (shown in composer metadata).
    pub provider_display: Option<&'a str>,
    /// Active thinking level (shown in composer metadata).
    pub thinking_level: Option<&'a ThinkingLevelType>,
}

/// Mutable resources provided during action processing.
pub(crate) struct UpdateContext<'a> {
    pub runtime: &'a mut tidev_core::Runtime,
    pub palette: &'a ThemePalette,
}
