use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn rose_pine_dawn() -> Self {
        Self {
            name: ThemeName::RosePineDawn,
            background: Color::Rgb(255, 250, 244),
            panel: Color::Rgb(252, 246, 239),
            panel_alt: Color::Rgb(245, 239, 234),
            panel_light: Color::Rgb(248, 242, 238),
            text: Color::Rgb(87, 82, 121),
            muted: Color::Rgb(130, 125, 150),
            border: Color::Rgb(215, 205, 196),
            accent: Color::Rgb(235, 111, 146),
            accent_soft: Color::Rgb(240, 147, 199),
            success: Color::Rgb(40, 105, 131),
            warning: Color::Rgb(234, 157, 52),
            error: Color::Rgb(235, 111, 146),
            selection_bg: Color::Rgb(235, 111, 146),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(235, 111, 146),
            mode_plan: Color::Rgb(152, 147, 165),
        }
    }
}
