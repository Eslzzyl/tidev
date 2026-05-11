use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn latte() -> Self {
        Self {
            name: ThemeName::Latte,
            background: Color::Rgb(239, 241, 245),
            panel: Color::Rgb(230, 233, 239),
            panel_alt: Color::Rgb(220, 224, 232),
            panel_light: Color::Rgb(204, 208, 218),
            text: Color::Rgb(76, 79, 105),
            muted: Color::Rgb(156, 160, 176),
            border: Color::Rgb(204, 208, 218),
            accent: Color::Rgb(136, 57, 239),
            accent_soft: Color::Rgb(114, 135, 253),
            success: Color::Rgb(64, 160, 43),
            warning: Color::Rgb(223, 142, 29),
            error: Color::Rgb(210, 15, 57),
            selection_bg: Color::Rgb(204, 208, 218),
            selection_fg: Color::Rgb(76, 79, 105),
            mode_build: Color::Rgb(136, 57, 239),
            mode_plan: Color::Rgb(156, 160, 176),
        }
    }
}
