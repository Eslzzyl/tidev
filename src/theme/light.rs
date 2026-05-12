use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn light() -> Self {
        Self {
            name: ThemeName::Light,
            background: Color::Rgb(246, 248, 251),
            panel: Color::Rgb(255, 255, 255),
            panel_alt: Color::Rgb(236, 241, 247),
            panel_light: Color::Rgb(239, 246, 252),
            text: Color::Rgb(17, 24, 39),
            muted: Color::Rgb(102, 115, 135),
            border: Color::Rgb(203, 213, 225),
            accent: Color::Rgb(13, 148, 136),
            accent_soft: Color::Rgb(128, 192, 194),
            success: Color::Rgb(22, 163, 74),
            warning: Color::Rgb(217, 119, 6),
            error: Color::Rgb(220, 38, 38),
            selection_bg: Color::Rgb(13, 148, 136),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(13, 148, 136),
            mode_plan: Color::Rgb(71, 85, 105),
        }
    }
}
