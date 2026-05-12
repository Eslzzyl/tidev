use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn contrast() -> Self {
        Self {
            name: ThemeName::Contrast,
            background: Color::Rgb(39, 40, 34),
            panel: Color::Rgb(46, 47, 42),
            panel_alt: Color::Rgb(56, 57, 50),
            panel_light: Color::Rgb(73, 72, 62),
            text: Color::Rgb(248, 248, 242),
            muted: Color::Rgb(117, 113, 94),
            border: Color::Rgb(73, 72, 62),
            accent: Color::Rgb(166, 226, 46),
            accent_soft: Color::Rgb(102, 217, 239),
            success: Color::Rgb(166, 226, 46),
            warning: Color::Rgb(230, 219, 116),
            error: Color::Rgb(249, 38, 114),
            selection_bg: Color::Rgb(166, 226, 46),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(166, 226, 46),
            mode_plan: Color::Rgb(117, 113, 94),
        }
    }
}
