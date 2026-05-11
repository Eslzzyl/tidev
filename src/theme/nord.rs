use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn nord() -> Self {
        Self {
            name: ThemeName::Nord,
            background: Color::Rgb(46, 52, 64),
            panel: Color::Rgb(59, 66, 82),
            panel_alt: Color::Rgb(67, 76, 94),
            panel_light: Color::Rgb(80, 90, 110),
            text: Color::Rgb(229, 233, 240),
            muted: Color::Rgb(136, 192, 208),
            border: Color::Rgb(81, 93, 106),
            accent: Color::Rgb(163, 190, 140),
            accent_soft: Color::Rgb(116, 145, 159),
            success: Color::Rgb(163, 190, 140),
            warning: Color::Rgb(232, 129, 145),
            error: Color::Rgb(191, 97, 106),
            selection_bg: Color::Rgb(59, 66, 82),
            selection_fg: Color::Rgb(229, 233, 240),
            mode_build: Color::Rgb(163, 190, 140),
            mode_plan: Color::Rgb(116, 145, 159),
        }
    }
}
