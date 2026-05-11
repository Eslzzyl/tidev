use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn dusk() -> Self {
        Self {
            name: ThemeName::Dusk,
            background: Color::Rgb(40, 42, 54),
            panel: Color::Rgb(49, 50, 68),
            panel_alt: Color::Rgb(54, 56, 72),
            panel_light: Color::Rgb(68, 71, 90),
            text: Color::Rgb(248, 248, 242),
            muted: Color::Rgb(98, 114, 164),
            border: Color::Rgb(68, 71, 90),
            accent: Color::Rgb(189, 147, 249),
            accent_soft: Color::Rgb(149, 128, 255),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(241, 250, 140),
            error: Color::Rgb(255, 85, 85),
            selection_bg: Color::Rgb(68, 71, 90),
            selection_fg: Color::Rgb(248, 248, 242),
            mode_build: Color::Rgb(80, 250, 123),
            mode_plan: Color::Rgb(98, 114, 164),
        }
    }
}
