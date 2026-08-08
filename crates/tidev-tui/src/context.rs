//! Component context types.

use crate::theme::ThemePalette;
use std::path::Path;
use tidev_core::Mode as SessionMode;
use tidev_llm::reasoning::ThinkingLevelType;

/// Immutable resources passed to every component during initialisation.
pub(crate) struct InitContext<'a> {
    pub config: &'a tidev_config::AppConfig,
    pub auth: &'a tidev_config::AuthStore,
}

/// Read-only shared data passed to every component each frame during draw.
pub(crate) struct DrawContext<'a> {
    pub palette: ThemePalette,
    pub focused: bool,
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
    /// Whether the subagent (task tool) is currently disabled.
    pub subagent_disabled: bool,
    /// Whether thinking content is collapsed by default (from config).
    pub collapse_thinking: bool,
    /// Whether edit/write/apply_patch diffs are collapsed to per-file
    /// +N/-M summaries by default (from config).
    pub collapse_diffs: bool,
    /// Workspace root path, used for path clipping in tool renders.
    pub workspace_root: &'a Path,
}

/// Mutable resources provided during action processing.
pub(crate) struct UpdateContext<'a> {
    pub runtime: &'a mut tidev_core::Runtime,
}
