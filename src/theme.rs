use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    Dark,
    Light,
}

impl ThemeName {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub name: ThemeName,
    pub background: Color,
    pub panel: Color,
    pub panel_alt: Color,
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
}

impl ThemePalette {
    pub fn dark() -> Self {
        Self {
            name: ThemeName::Dark,
            background: Color::Rgb(12, 16, 23),
            panel: Color::Rgb(19, 25, 36),
            panel_alt: Color::Rgb(25, 32, 44),
            text: Color::Rgb(229, 231, 235),
            muted: Color::Rgb(134, 146, 166),
            border: Color::Rgb(51, 65, 85),
            accent: Color::Rgb(45, 212, 191),
            accent_soft: Color::Rgb(100, 116, 139),
            success: Color::Rgb(34, 197, 94),
            warning: Color::Rgb(251, 191, 36),
            error: Color::Rgb(248, 113, 113),
            selection_bg: Color::Rgb(38, 52, 69),
            selection_fg: Color::Rgb(255, 255, 255),
        }
    }

    pub fn light() -> Self {
        Self {
            name: ThemeName::Light,
            background: Color::Rgb(246, 248, 251),
            panel: Color::Rgb(255, 255, 255),
            panel_alt: Color::Rgb(236, 241, 247),
            text: Color::Rgb(17, 24, 39),
            muted: Color::Rgb(102, 115, 135),
            border: Color::Rgb(203, 213, 225),
            accent: Color::Rgb(13, 148, 136),
            accent_soft: Color::Rgb(71, 85, 105),
            success: Color::Rgb(22, 163, 74),
            warning: Color::Rgb(217, 119, 6),
            error: Color::Rgb(220, 38, 38),
            selection_bg: Color::Rgb(203, 213, 225),
            selection_fg: Color::Rgb(15, 23, 42),
        }
    }

    pub fn from_name(value: &str) -> Self {
        match ThemeName::parse(value).unwrap_or(ThemeName::Dark) {
            ThemeName::Dark => Self::dark(),
            ThemeName::Light => Self::light(),
        }
    }

    pub fn border_active(&self) -> Color {
        self.accent
    }

    pub fn border_idle(&self) -> Color {
        self.border
    }
}

#[derive(Clone, Debug)]
pub struct ThemeManager {
    palette: ThemePalette,
}

impl ThemeManager {
    pub fn new(name: &str) -> Self {
        Self {
            palette: ThemePalette::from_name(name),
        }
    }

    pub fn palette(&self) -> ThemePalette {
        self.palette
    }

    pub fn set_mode(&mut self, name: ThemeName) {
        self.palette = match name {
            ThemeName::Dark => ThemePalette::dark(),
            ThemeName::Light => ThemePalette::light(),
        };
    }

    pub fn toggle(&mut self) {
        let next = self.palette.name.toggle();
        self.set_mode(next);
    }

    pub fn name(&self) -> &'static str {
        self.palette.name.as_str()
    }
}
