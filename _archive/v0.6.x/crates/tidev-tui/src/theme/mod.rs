use ratatui::style::Color;

use tidev_types::prompts::SessionMode;

pub use tidev_types::theme::ThemeName;

mod contrast;
mod dark;
mod dusk;
mod everforest;
mod everforest_light;
mod github;
mod gruvbox;
mod gruvbox_light;
mod light;
mod material;
mod mocha;
mod nord;
mod one_dark;
mod orng;
mod rose_pine;
mod rose_pine_dawn;
mod solarized;
mod tokyo_night;

#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub name: ThemeName,
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
    pub fn from_name(value: &str) -> Self {
        match ThemeName::parse(value).unwrap_or(ThemeName::Dark) {
            ThemeName::Dark => Self::dark(),
            ThemeName::Light => Self::light(),
            ThemeName::Nord => Self::nord(),
            ThemeName::OneDark => Self::one_dark(),
            ThemeName::Mocha => Self::mocha(),
            ThemeName::Solarized => Self::solarized(),
            ThemeName::Orng => Self::orng(),
            ThemeName::Github => Self::github(),
            ThemeName::Material => Self::material(),
            ThemeName::Everforest => Self::everforest(),
            ThemeName::EverforestLight => Self::everforest_light(),
            ThemeName::Dusk => Self::dusk(),
            ThemeName::Gruvbox => Self::gruvbox(),
            ThemeName::GruvboxLight => Self::gruvbox_light(),
            ThemeName::TokyoNight => Self::tokyo_night(),
            ThemeName::RosePine => Self::rose_pine(),
            ThemeName::RosePineDawn => Self::rose_pine_dawn(),
            ThemeName::Contrast => Self::contrast(),
        }
    }

    pub fn border_active(&self) -> Color {
        self.accent
    }

    pub fn border_idle(&self) -> Color {
        self.border
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
        if self.name.is_dark() {
            mix_colors(Color::Rgb(255, 255, 255), base, 0.08)
        } else {
            mix_colors(Color::Rgb(0, 0, 0), base, 0.06)
        }
    }
}

#[derive(Clone, Debug)]
pub struct ThemeManager {
    palette: ThemePalette,
}

impl ThemeManager {
    pub fn new(name: &str) -> Self {
        let palette = ThemePalette::from_name(name);
        let theme_name = palette.name;
        crate::markdown::spawn_background_load();
        crate::markdown::set_syntax_theme_by_name(theme_name);
        Self { palette }
    }

    pub fn palette(&self) -> ThemePalette {
        self.palette
    }

    pub fn set_mode(&mut self, name: ThemeName) {
        self.palette = match name {
            ThemeName::Dark => ThemePalette::dark(),
            ThemeName::Light => ThemePalette::light(),
            ThemeName::Nord => ThemePalette::nord(),
            ThemeName::OneDark => ThemePalette::one_dark(),
            ThemeName::Mocha => ThemePalette::mocha(),
            ThemeName::Solarized => ThemePalette::solarized(),
            ThemeName::Orng => ThemePalette::orng(),
            ThemeName::Github => ThemePalette::github(),
            ThemeName::Material => ThemePalette::material(),
            ThemeName::Everforest => ThemePalette::everforest(),
            ThemeName::EverforestLight => ThemePalette::everforest_light(),
            ThemeName::Dusk => ThemePalette::dusk(),
            ThemeName::Gruvbox => ThemePalette::gruvbox(),
            ThemeName::GruvboxLight => ThemePalette::gruvbox_light(),
            ThemeName::TokyoNight => ThemePalette::tokyo_night(),
            ThemeName::RosePine => ThemePalette::rose_pine(),
            ThemeName::RosePineDawn => ThemePalette::rose_pine_dawn(),
            ThemeName::Contrast => ThemePalette::contrast(),
        };
        crate::markdown::set_syntax_theme_by_name(name);
    }

    pub fn toggle(&mut self) {
        let next = self.palette.name.toggle();
        self.set_mode(next);
    }

    pub fn name(&self) -> &'static str {
        self.palette.name.as_str()
    }
}
