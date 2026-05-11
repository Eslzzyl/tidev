use ratatui::style::Color;
use serde::{Deserialize, Serialize};

use crate::prompts::SessionMode;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeName {
    Dark,
    Light,
    Nord,
    OneDark,
    Mocha,
    Solarized,
    Orng,
    Github,
    Material,
    Everforest,
    EverforestLight,
    Dusk,
    Gruvbox,
    GruvboxLight,
    TokyoNight,
    RosePine,
    RosePineDawn,
    Contrast,
}

impl ThemeName {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "nord" => Some(Self::Nord),
            "one-dark" | "one_dark" | "onedark" => Some(Self::OneDark),
            "mocha" => Some(Self::Mocha),
            "solarized" => Some(Self::Solarized),
            "orng" => Some(Self::Orng),
            "github" => Some(Self::Github),
            "material" => Some(Self::Material),
            "everforest" => Some(Self::Everforest),
            "everforest-light" | "everforest_light" | "everforestlight" => {
                Some(Self::EverforestLight)
            }
            "dusk" => Some(Self::Dusk),
            "gruvbox" => Some(Self::Gruvbox),
            "gruvbox-light" | "gruvbox_light" | "gruvboxlight" => Some(Self::GruvboxLight),
            "tokyo-night" | "tokyo_night" | "tokyonight" => Some(Self::TokyoNight),
            "rose-pine" | "rose_pine" | "rosepine" => Some(Self::RosePine),
            "rose-pine-dawn" | "rose_pine_dawn" | "rosepinedawn" => Some(Self::RosePineDawn),
            "contrast" => Some(Self::Contrast),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Nord => "nord",
            Self::OneDark => "one-dark",
            Self::Mocha => "mocha",
            Self::Solarized => "solarized",
            Self::Orng => "orng",
            Self::Github => "github",
            Self::Material => "material",
            Self::Everforest => "everforest",
            Self::EverforestLight => "everforest-light",
            Self::Dusk => "dusk",
            Self::Gruvbox => "gruvbox",
            Self::GruvboxLight => "gruvbox-light",
            Self::TokyoNight => "tokyo-night",
            Self::RosePine => "rose-pine",
            Self::RosePineDawn => "rose-pine-dawn",
            Self::Contrast => "contrast",
        }
    }

    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::Dark,
            ThemeName::Light,
            ThemeName::Nord,
            ThemeName::OneDark,
            ThemeName::Mocha,
            ThemeName::Solarized,
            ThemeName::Orng,
            ThemeName::Github,
            ThemeName::Material,
            ThemeName::Everforest,
            ThemeName::EverforestLight,
            ThemeName::Dusk,
            ThemeName::Gruvbox,
            ThemeName::GruvboxLight,
            ThemeName::TokyoNight,
            ThemeName::RosePine,
            ThemeName::RosePineDawn,
            ThemeName::Contrast,
        ]
    }

    pub fn toggle(self) -> Self {
        let all = Self::all();
        let index = all.iter().position(|theme| *theme == self).unwrap_or(0);
        all[(index + 1) % all.len()]
    }

    pub fn is_dark(self) -> bool {
        matches!(
            self,
            Self::Dark
                | Self::Nord
                | Self::OneDark
                | Self::Mocha
                | Self::Solarized
                | Self::Everforest
                | Self::Dusk
                | Self::Gruvbox
                | Self::TokyoNight
                | Self::RosePine
                | Self::Contrast
        )
    }
}

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
}

#[derive(Clone, Debug)]
pub struct ThemeManager {
    palette: ThemePalette,
}

impl ThemeManager {
    pub fn new(name: &str) -> Self {
        let palette = ThemePalette::from_name(name);
        let theme_name = palette.name;
        crate::markdown_render::spawn_background_load();
        crate::markdown_render::set_syntax_theme_by_name(theme_name);
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
        crate::markdown_render::set_syntax_theme_by_name(name);
    }

    pub fn toggle(&mut self) {
        let next = self.palette.name.toggle();
        self.set_mode(next);
    }

    pub fn name(&self) -> &'static str {
        self.palette.name.as_str()
    }
}
