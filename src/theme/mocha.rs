use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn mocha() -> Self {
        Self {
            name: ThemeName::Mocha,
            background: Color::Rgb(26, 32, 44),
            panel: Color::Rgb(39, 46, 62),
            panel_alt: Color::Rgb(49, 57, 76),
            panel_light: Color::Rgb(65, 75, 95),
            text: Color::Rgb(223, 226, 247),
            muted: Color::Rgb(170, 182, 211),
            border: Color::Rgb(67, 74, 101),
            accent: Color::Rgb(159, 147, 255),
            accent_soft: Color::Rgb(120, 109, 186),
            success: Color::Rgb(162, 190, 140),
            warning: Color::Rgb(239, 184, 102),
            error: Color::Rgb(241, 124, 151),
            selection_bg: Color::Rgb(67, 74, 101),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(159, 147, 255),
            mode_plan: Color::Rgb(120, 109, 186),
        }
    }
}
