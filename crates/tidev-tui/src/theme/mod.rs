use ratatui::style::Color;

use tidev_config::{ThemeCatalog, ThemeDefinition};
use tidev_core::Mode as SessionMode;

pub(crate) mod preview;

/// Default background tints for diff added/removed rows, used when a theme
/// does not define `diff_add_bg`/`diff_delete_bg`. Dark themes get the dark
/// set, light themes the light set.
pub(crate) const DARK_ADD_BG: (u8, u8, u8) = (48, 80, 60);
pub(crate) const DARK_DEL_BG: (u8, u8, u8) = (100, 50, 42);
pub(crate) const LIGHT_ADD_BG: (u8, u8, u8) = (218, 251, 225);
pub(crate) const LIGHT_DEL_BG: (u8, u8, u8) = (255, 235, 233);

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
    pub diff_add_bg: Color,
    pub diff_delete_bg: Color,
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

/// Resolve a diff row background tint: the theme's own value when configured,
/// otherwise the built-in default for the theme's brightness.
fn diff_bg_color(
    configured: Option<tidev_config::ThemeColor>,
    dark_default: (u8, u8, u8),
    light_default: (u8, u8, u8),
    is_dark: bool,
) -> Color {
    match configured {
        Some(c) => Color::Rgb(c.0, c.1, c.2),
        None => {
            let (r, g, b) = if is_dark { dark_default } else { light_default };
            Color::Rgb(r, g, b)
        }
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
            diff_add_bg: diff_bg_color(def.diff_add_bg, DARK_ADD_BG, LIGHT_ADD_BG, def.dark),
            diff_delete_bg: diff_bg_color(def.diff_delete_bg, DARK_DEL_BG, LIGHT_DEL_BG, def.dark),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_bg_resolves_configured_value_and_builtin_defaults() {
        let dir = std::env::temp_dir().join(format!("tidev-theme-diff-bg-{}", std::process::id()));
        let themes = dir.join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(
            themes.join("custom.toml"),
            r#"
dark = true
background = "101010"
panel = "202020"
panel_alt = "303030"
panel_light = "404040"
text = "eeeeee"
muted = "aaaaaa"
border = "555555"
accent = "00aaff"
accent_soft = "66ccff"
success = "00cc00"
warning = "cccc00"
error = "cc0000"
diff_add = "00ff00"
diff_delete = "ff0000"
diff_add_bg = "123456"
diff_delete_bg = "654321"
selection_bg = "0055ff"
selection_fg = "ffffff"
mode_build = "00cccc"
mode_plan = "88aaaa"
"#,
        )
        .unwrap();
        let catalog = ThemeCatalog::load(&dir).unwrap();

        // A theme that configures the tints keeps its own values.
        let custom = resolve_palette(&catalog, "custom");
        assert_eq!(custom.diff_add_bg, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(custom.diff_delete_bg, Color::Rgb(0x65, 0x43, 0x21));

        // Bundled themes fall back to the built-in dark/light sets.
        let dark = resolve_palette(&catalog, "dark");
        assert_eq!(
            dark.diff_add_bg,
            Color::Rgb(DARK_ADD_BG.0, DARK_ADD_BG.1, DARK_ADD_BG.2)
        );
        assert_eq!(
            dark.diff_delete_bg,
            Color::Rgb(DARK_DEL_BG.0, DARK_DEL_BG.1, DARK_DEL_BG.2)
        );
        let light = resolve_palette(&catalog, "light");
        assert_eq!(
            light.diff_add_bg,
            Color::Rgb(LIGHT_ADD_BG.0, LIGHT_ADD_BG.1, LIGHT_ADD_BG.2)
        );
        assert_eq!(
            light.diff_delete_bg,
            Color::Rgb(LIGHT_DEL_BG.0, LIGHT_DEL_BG.1, LIGHT_DEL_BG.2)
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
