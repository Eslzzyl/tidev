use crate::theme::{ThemeName, ThemePalette};
use ratatui::style::Color;

impl ThemePalette {
    pub fn gruvbox_light() -> Self {
        Self {
            name: ThemeName::GruvboxLight,
            background: Color::Rgb(255, 249, 227),
            panel: Color::Rgb(245, 233, 203),
            panel_alt: Color::Rgb(240, 231, 214),
            panel_light: Color::Rgb(240, 231, 214),
            text: Color::Rgb(60, 56, 54),
            muted: Color::Rgb(146, 131, 116),
            border: Color::Rgb(189, 174, 147),
            accent: Color::Rgb(69, 133, 136),
            accent_soft: Color::Rgb(104, 157, 106),
            success: Color::Rgb(152, 151, 26),
            warning: Color::Rgb(215, 153, 33),
            error: Color::Rgb(204, 36, 29),
            diff_add: Color::Rgb(58, 132, 55),
            diff_delete: Color::Rgb(237, 72, 49),
            selection_bg: Color::Rgb(69, 133, 136),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(69, 133, 136),
            mode_plan: Color::Rgb(146, 131, 116),
        }
    }
}
