use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn rose_pine() -> Self {
        Self {
            name: ThemeName::RosePine,
            background: Color::Rgb(25, 23, 36),
            panel: Color::Rgb(31, 29, 46),
            panel_alt: Color::Rgb(38, 35, 58),
            panel_light: Color::Rgb(49, 47, 68),
            text: Color::Rgb(224, 222, 244),
            muted: Color::Rgb(144, 140, 170),
            border: Color::Rgb(38, 35, 58),
            accent: Color::Rgb(235, 111, 146),
            accent_soft: Color::Rgb(196, 167, 231),
            success: Color::Rgb(156, 207, 216),
            warning: Color::Rgb(246, 193, 119),
            error: Color::Rgb(235, 111, 146),
            selection_bg: Color::Rgb(235, 111, 146),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(235, 111, 146),
            mode_plan: Color::Rgb(144, 140, 170),
        }
    }
}
