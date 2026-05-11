use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn gruvbox_light() -> Self {
        Self {
            name: ThemeName::GruvboxLight,
            background: Color::Rgb(251, 241, 199),
            panel: Color::Rgb(235, 219, 178),
            panel_alt: Color::Rgb(213, 196, 161),
            panel_light: Color::Rgb(189, 174, 147),
            text: Color::Rgb(60, 56, 54),
            muted: Color::Rgb(146, 131, 116),
            border: Color::Rgb(189, 174, 147),
            accent: Color::Rgb(69, 133, 136),
            accent_soft: Color::Rgb(104, 157, 106),
            success: Color::Rgb(152, 151, 26),
            warning: Color::Rgb(215, 153, 33),
            error: Color::Rgb(204, 36, 29),
            selection_bg: Color::Rgb(213, 196, 161),
            selection_fg: Color::Rgb(60, 56, 54),
            mode_build: Color::Rgb(69, 133, 136),
            mode_plan: Color::Rgb(146, 131, 116),
        }
    }
}
