use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn github() -> Self {
        Self {
            name: ThemeName::Github,
            background: Color::Rgb(255, 255, 255),
            panel: Color::Rgb(246, 248, 250),
            panel_alt: Color::Rgb(234, 238, 243),
            panel_light: Color::Rgb(240, 243, 246),
            text: Color::Rgb(36, 41, 47),
            muted: Color::Rgb(106, 115, 125),
            border: Color::Rgb(208, 215, 222),
            accent: Color::Rgb(9, 105, 218),
            accent_soft: Color::Rgb(31, 111, 235),
            success: Color::Rgb(40, 167, 69),
            warning: Color::Rgb(210, 153, 36),
            error: Color::Rgb(207, 34, 46),
            selection_bg: Color::Rgb(9, 105, 218),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(9, 105, 218),
            mode_plan: Color::Rgb(127, 139, 167),
        }
    }
}
