use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn solarized() -> Self {
        Self {
            name: ThemeName::Solarized,
            background: Color::Rgb(0, 43, 54),
            panel: Color::Rgb(7, 54, 66),
            panel_alt: Color::Rgb(20, 62, 74),
            panel_light: Color::Rgb(35, 72, 84),
            text: Color::Rgb(131, 148, 150),
            muted: Color::Rgb(147, 161, 161),
            border: Color::Rgb(38, 139, 210),
            accent: Color::Rgb(38, 139, 210),
            accent_soft: Color::Rgb(88, 110, 117),
            success: Color::Rgb(42, 161, 152),
            warning: Color::Rgb(203, 75, 22),
            error: Color::Rgb(220, 50, 47),
            selection_bg: Color::Rgb(38, 139, 210),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(38, 139, 210),
            mode_plan: Color::Rgb(88, 110, 117),
        }
    }
}
