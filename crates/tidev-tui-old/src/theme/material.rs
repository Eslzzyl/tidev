use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn material() -> Self {
        Self {
            name: ThemeName::Material,
            background: Color::Rgb(255, 255, 255),
            panel: Color::Rgb(246, 243, 252),
            panel_alt: Color::Rgb(233, 230, 245),
            panel_light: Color::Rgb(240, 236, 248),
            text: Color::Rgb(29, 25, 43),
            muted: Color::Rgb(107, 109, 126),
            border: Color::Rgb(215, 212, 240),
            accent: Color::Rgb(124, 58, 237),
            accent_soft: Color::Rgb(139, 92, 246),
            success: Color::Rgb(22, 163, 74),
            warning: Color::Rgb(217, 119, 6),
            error: Color::Rgb(220, 38, 38),
            diff_add: Color::Rgb(58, 132, 55),
            diff_delete: Color::Rgb(237, 72, 49),
            selection_bg: Color::Rgb(124, 58, 237),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(124, 58, 237),
            mode_plan: Color::Rgb(171, 145, 247),
        }
    }
}
