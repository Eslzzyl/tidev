use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn everforest_light() -> Self {
        Self {
            name: ThemeName::EverforestLight,
            background: Color::Rgb(253, 246, 227),
            panel: Color::Rgb(240, 232, 208),
            panel_alt: Color::Rgb(229, 221, 197),
            panel_light: Color::Rgb(211, 201, 168),
            text: Color::Rgb(91, 110, 88),
            muted: Color::Rgb(133, 152, 122),
            border: Color::Rgb(211, 201, 168),
            accent: Color::Rgb(122, 158, 107),
            accent_soft: Color::Rgb(100, 140, 150),
            success: Color::Rgb(122, 158, 107),
            warning: Color::Rgb(214, 174, 85),
            error: Color::Rgb(237, 112, 98),
            selection_bg: Color::Rgb(211, 201, 168),
            selection_fg: Color::Rgb(91, 110, 88),
            mode_build: Color::Rgb(122, 158, 107),
            mode_plan: Color::Rgb(133, 152, 122),
        }
    }
}
