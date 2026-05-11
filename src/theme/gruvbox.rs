use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn gruvbox() -> Self {
        Self {
            name: ThemeName::Gruvbox,
            background: Color::Rgb(40, 40, 40),
            panel: Color::Rgb(50, 48, 47),
            panel_alt: Color::Rgb(60, 56, 54),
            panel_light: Color::Rgb(80, 73, 69),
            text: Color::Rgb(235, 219, 178),
            muted: Color::Rgb(168, 152, 132),
            border: Color::Rgb(80, 73, 69),
            accent: Color::Rgb(250, 189, 47),
            accent_soft: Color::Rgb(215, 153, 33),
            success: Color::Rgb(184, 187, 38),
            warning: Color::Rgb(254, 128, 25),
            error: Color::Rgb(251, 73, 52),
            selection_bg: Color::Rgb(250, 189, 47),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(131, 165, 152),
            mode_plan: Color::Rgb(168, 152, 132),
        }
    }
}
