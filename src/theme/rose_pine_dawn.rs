use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn rose_pine_dawn() -> Self {
        Self {
            name: ThemeName::RosePineDawn,
            background: Color::Rgb(252, 246, 239),
            panel: Color::Rgb(252, 246, 240),
            panel_alt: Color::Rgb(250, 244, 238),
            panel_light: Color::Rgb(240, 234, 228),
            text: Color::Rgb(87, 82, 121),
            muted: Color::Rgb(130, 125, 150),
            border: Color::Rgb(215, 205, 196),
            accent: Color::Rgb(235, 111, 146),
            accent_soft: Color::Rgb(240, 147, 199),
            success: Color::Rgb(40, 105, 131),
            warning: Color::Rgb(234, 157, 52),
            error: Color::Rgb(235, 111, 146),
            selection_bg: Color::Rgb(215, 205, 196),
            selection_fg: Color::Rgb(87, 82, 121),
            mode_build: Color::Rgb(235, 111, 146),
            mode_plan: Color::Rgb(152, 147, 165),
        }
    }
}
