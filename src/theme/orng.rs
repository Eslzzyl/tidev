use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn orng() -> Self {
        Self {
            name: ThemeName::Orng,
            background: Color::Rgb(255, 245, 235),
            panel: Color::Rgb(255, 250, 242),
            panel_alt: Color::Rgb(255, 235, 214),
            panel_light: Color::Rgb(255, 248, 245),
            text: Color::Rgb(45, 35, 27),
            muted: Color::Rgb(109, 89, 74),
            border: Color::Rgb(220, 190, 160),
            accent: Color::Rgb(251, 146, 60),
            accent_soft: Color::Rgb(249, 115, 22),
            success: Color::Rgb(34, 197, 94),
            warning: Color::Rgb(234, 179, 8),
            error: Color::Rgb(220, 38, 38),
            selection_bg: Color::Rgb(251, 146, 60),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(251, 146, 60),
            mode_plan: Color::Rgb(234, 179, 8),
        }
    }
}
