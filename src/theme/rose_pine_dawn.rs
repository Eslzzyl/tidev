use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn rose_pine_dawn() -> Self {
        Self {
            name: ThemeName::RosePineDawn,
            background: Color::Rgb(250, 244, 237),
            panel: Color::Rgb(242, 233, 225),
            panel_alt: Color::Rgb(229, 222, 216),
            panel_light: Color::Rgb(215, 205, 196),
            text: Color::Rgb(87, 82, 121),
            muted: Color::Rgb(152, 147, 165),
            border: Color::Rgb(215, 205, 196),
            accent: Color::Rgb(235, 111, 146),
            accent_soft: Color::Rgb(196, 167, 231),
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
