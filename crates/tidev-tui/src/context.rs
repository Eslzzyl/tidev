//! Component context types.

use std::path::Path;

use tidev_tui_old::chat_context::ChatContext;
use tidev_tui_old::theme::ThemePalette;

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
}

/// Mutable resources provided during action processing.
pub(crate) struct UpdateContext<'a> {
    pub runtime: &'a mut tidev_core::Runtime,
    pub palette: &'a ThemePalette,
}
