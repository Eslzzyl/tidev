use ratatui::style::Color;

use tidev_config::{ThemeCatalog, ThemeDefinition};
use tidev_core::Mode as SessionMode;

/// Fully-resolved UI colors for one theme.
#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub is_dark: bool,
    pub background: Color,
    pub panel: Color,
    pub panel_alt: Color,
    pub panel_light: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub diff_add: Color,
    pub diff_delete: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub mode_build: Color,
    pub mode_plan: Color,
}

pub fn mix_colors(fg: Color, bg: Color, weight: f32) -> Color {
    if let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (fg, bg) {
        let r = (r1 as f32 * weight + r2 as f32 * (1.0 - weight)) as u8;
        let g = (g1 as f32 * weight + g2 as f32 * (1.0 - weight)) as u8;
        let b = (b1 as f32 * weight + b2 as f32 * (1.0 - weight)) as u8;
        Color::Rgb(r, g, b)
    } else {
        fg
    }
}

impl ThemePalette {
    /// Build a palette from a theme definition and apply its syntax theme.
    pub fn from_definition(def: &ThemeDefinition) -> Self {
        crate::markdown::set_syntax_theme_by_key(def.syntax_theme_key());
        let color = |c: tidev_config::ThemeColor| Color::Rgb(c.0, c.1, c.2);
        Self {
            is_dark: def.dark,
            background: color(def.background),
            panel: color(def.panel),
            panel_alt: color(def.panel_alt),
            panel_light: color(def.panel_light),
            text: color(def.text),
            muted: color(def.muted),
            border: color(def.border),
            accent: color(def.accent),
            accent_soft: color(def.accent_soft),
            success: color(def.success),
            warning: color(def.warning),
            error: color(def.error),
            diff_add: color(def.diff_add),
            diff_delete: color(def.diff_delete),
            selection_bg: color(def.selection_bg),
            selection_fg: color(def.selection_fg),
            mode_build: color(def.mode_build),
            mode_plan: color(def.mode_plan),
        }
    }

    pub fn border_mode_color(&self, mode: SessionMode) -> Color {
        match mode {
            SessionMode::Build => self.mode_build,
            SessionMode::Plan => self.mode_plan,
        }
    }

    /// Return a subtly highlighted version of `base` for mouse hover effects.
    /// Lightens on dark themes, darkens on light themes, so the card appears
    /// to "lift" on hover without changing hue.
    pub fn hover_bg(&self, base: Color) -> Color {
        if self.is_dark {
            mix_colors(Color::Rgb(255, 255, 255), base, 0.08)
        } else {
            mix_colors(Color::Rgb(0, 0, 0), base, 0.06)
        }
    }
}

/// Resolve a theme id against the catalog, falling back to the bundled
/// `dark` theme when the id is unknown.
pub fn resolve_palette(catalog: &ThemeCatalog, id: &str) -> ThemePalette {
    if catalog.get(id).is_none() {
        log::warn!("unknown theme {id:?}, falling back to \"dark\"");
    }
    let def = catalog
        .get(id)
        .or_else(|| catalog.get("dark"))
        .expect("bundled dark theme must exist");
    ThemePalette::from_definition(def)
}
