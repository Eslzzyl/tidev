use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn tokyo_night() -> Self {
        Self {
            name: ThemeName::TokyoNight,
            background: Color::Rgb(26, 27, 38),
            panel: Color::Rgb(36, 40, 59),
            panel_alt: Color::Rgb(42, 46, 63),
            panel_light: Color::Rgb(54, 59, 84),
            text: Color::Rgb(169, 177, 214),
            muted: Color::Rgb(86, 95, 137),
            border: Color::Rgb(54, 59, 84),
            accent: Color::Rgb(122, 162, 247),
            accent_soft: Color::Rgb(47, 63, 95),
            success: Color::Rgb(158, 206, 106),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            selection_bg: Color::Rgb(122, 162, 247),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(122, 162, 247),
            mode_plan: Color::Rgb(86, 95, 137),
        }
    }
}
