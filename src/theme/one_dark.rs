use ratatui::style::Color;
use crate::theme::{ThemeName, ThemePalette};

impl ThemePalette {
    pub fn one_dark() -> Self {
        Self {
            name: ThemeName::OneDark,
            background: Color::Rgb(40, 44, 52),
            panel: Color::Rgb(48, 52, 64),
            panel_alt: Color::Rgb(60, 64, 76),
            panel_light: Color::Rgb(75, 80, 95),
            text: Color::Rgb(171, 178, 191),
            muted: Color::Rgb(98, 114, 164),
            border: Color::Rgb(76, 86, 106),
            accent: Color::Rgb(97, 175, 239),
            accent_soft: Color::Rgb(120, 129, 175),
            success: Color::Rgb(152, 195, 121),
            warning: Color::Rgb(224, 108, 117),
            error: Color::Rgb(231, 76, 60),
            selection_bg: Color::Rgb(69, 76, 89),
            selection_fg: Color::Rgb(223, 230, 255),
            mode_build: Color::Rgb(97, 175, 239),
            mode_plan: Color::Rgb(120, 129, 175),
        }
    }
}
