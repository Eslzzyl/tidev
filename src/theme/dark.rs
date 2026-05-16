use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn dark() -> Self {
        Self {
            name: ThemeName::Dark,
            background: Color::Rgb(12, 16, 23),
            panel: Color::Rgb(19, 25, 36),
            panel_alt: Color::Rgb(25, 32, 44),
            panel_light: Color::Rgb(35, 45, 60),
            text: Color::Rgb(229, 231, 235),
            muted: Color::Rgb(134, 146, 166),
            border: Color::Rgb(51, 65, 85),
            accent: Color::Rgb(45, 212, 191),
            accent_soft: Color::Rgb(100, 116, 139),
            success: Color::Rgb(34, 197, 94),
            warning: Color::Rgb(251, 191, 36),
            error: Color::Rgb(248, 113, 113),
            selection_bg: Color::Rgb(45, 212, 191),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(45, 212, 191),
            mode_plan: Color::Rgb(100, 116, 139),
        }
    }
}
