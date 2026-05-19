use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn everforest() -> Self {
        Self {
            name: ThemeName::Everforest,
            background: Color::Rgb(45, 53, 44),
            panel: Color::Rgb(55, 65, 54),
            panel_alt: Color::Rgb(64, 74, 63),
            panel_light: Color::Rgb(77, 88, 76),
            text: Color::Rgb(211, 198, 170),
            muted: Color::Rgb(108, 121, 99),
            border: Color::Rgb(72, 84, 70),
            accent: Color::Rgb(131, 165, 104),
            accent_soft: Color::Rgb(100, 130, 80),
            success: Color::Rgb(131, 165, 104),
            warning: Color::Rgb(214, 174, 85),
            error: Color::Rgb(230, 126, 110),
            diff_add: Color::Rgb(155, 205, 151),
            diff_delete: Color::Rgb(252, 83, 58),
            selection_bg: Color::Rgb(131, 165, 104),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(131, 165, 104),
            mode_plan: Color::Rgb(108, 121, 99),
        }
    }
}
