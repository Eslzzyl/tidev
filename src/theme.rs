use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeName {
    Dark,
    Light,
    Nord,
    OneDark,
    Catppuccin,
    Solarized,
    Orng,
    Github,
    Material,
}

impl ThemeName {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "nord" => Some(Self::Nord),
            "one-dark" | "one_dark" | "onedark" => Some(Self::OneDark),
            "catppuccin" => Some(Self::Catppuccin),
            "solarized" => Some(Self::Solarized),
            "orng" => Some(Self::Orng),
            "github" => Some(Self::Github),
            "material" => Some(Self::Material),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Nord => "nord",
            Self::OneDark => "one-dark",
            Self::Catppuccin => "catppuccin",
            Self::Solarized => "solarized",
            Self::Orng => "orng",
            Self::Github => "github",
            Self::Material => "material",
        }
    }

    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::Dark,
            ThemeName::Light,
            ThemeName::Nord,
            ThemeName::OneDark,
            ThemeName::Catppuccin,
            ThemeName::Solarized,
            ThemeName::Orng,
            ThemeName::Github,
            ThemeName::Material,
        ]
    }

    pub fn toggle(self) -> Self {
        let all = Self::all();
        let index = all.iter().position(|theme| *theme == self).unwrap_or(0);
        all[(index + 1) % all.len()]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub name: ThemeName,
    pub background: Color,
    pub panel: Color,
    pub panel_alt: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
}

impl ThemePalette {
    pub fn dark() -> Self {
        Self {
            name: ThemeName::Dark,
            background: Color::Rgb(12, 16, 23),
            panel: Color::Rgb(19, 25, 36),
            panel_alt: Color::Rgb(25, 32, 44),
            text: Color::Rgb(229, 231, 235),
            muted: Color::Rgb(134, 146, 166),
            border: Color::Rgb(51, 65, 85),
            accent: Color::Rgb(45, 212, 191),
            accent_soft: Color::Rgb(100, 116, 139),
            success: Color::Rgb(34, 197, 94),
            warning: Color::Rgb(251, 191, 36),
            error: Color::Rgb(248, 113, 113),
            selection_bg: Color::Rgb(38, 52, 69),
            selection_fg: Color::Rgb(255, 255, 255),
        }
    }

    pub fn light() -> Self {
        Self {
            name: ThemeName::Light,
            background: Color::Rgb(246, 248, 251),
            panel: Color::Rgb(255, 255, 255),
            panel_alt: Color::Rgb(236, 241, 247),
            text: Color::Rgb(17, 24, 39),
            muted: Color::Rgb(102, 115, 135),
            border: Color::Rgb(203, 213, 225),
            accent: Color::Rgb(13, 148, 136),
            accent_soft: Color::Rgb(71, 85, 105),
            success: Color::Rgb(22, 163, 74),
            warning: Color::Rgb(217, 119, 6),
            error: Color::Rgb(220, 38, 38),
            selection_bg: Color::Rgb(203, 213, 225),
            selection_fg: Color::Rgb(15, 23, 42),
        }
    }

    pub fn orng() -> Self {
        Self {
            name: ThemeName::Orng,
            background: Color::Rgb(255, 245, 235),
            panel: Color::Rgb(255, 250, 242),
            panel_alt: Color::Rgb(255, 235, 214),
            text: Color::Rgb(45, 35, 27),
            muted: Color::Rgb(109, 89, 74),
            border: Color::Rgb(220, 190, 160),
            accent: Color::Rgb(251, 146, 60),
            accent_soft: Color::Rgb(249, 115, 22),
            success: Color::Rgb(34, 197, 94),
            warning: Color::Rgb(234, 179, 8),
            error: Color::Rgb(220, 38, 38),
            selection_bg: Color::Rgb(251, 208, 125),
            selection_fg: Color::Rgb(33, 24, 17),
        }
    }

    pub fn github() -> Self {
        Self {
            name: ThemeName::Github,
            background: Color::Rgb(245, 246, 248),
            panel: Color::Rgb(255, 255, 255),
            panel_alt: Color::Rgb(234, 238, 243),
            text: Color::Rgb(36, 41, 47),
            muted: Color::Rgb(106, 115, 125),
            border: Color::Rgb(208, 215, 222),
            accent: Color::Rgb(9, 105, 218),
            accent_soft: Color::Rgb(31, 111, 235),
            success: Color::Rgb(40, 167, 69),
            warning: Color::Rgb(210, 153, 36),
            error: Color::Rgb(207, 34, 46),
            selection_bg: Color::Rgb(9, 105, 218),
            selection_fg: Color::Rgb(255, 255, 255),
        }
    }

    pub fn material() -> Self {
        Self {
            name: ThemeName::Material,
            background: Color::Rgb(251, 248, 255),
            panel: Color::Rgb(255, 255, 255),
            panel_alt: Color::Rgb(237, 235, 255),
            text: Color::Rgb(29, 25, 43),
            muted: Color::Rgb(107, 109, 126),
            border: Color::Rgb(215, 212, 240),
            accent: Color::Rgb(124, 58, 237),
            accent_soft: Color::Rgb(139, 92, 246),
            success: Color::Rgb(22, 163, 74),
            warning: Color::Rgb(217, 119, 6),
            error: Color::Rgb(220, 38, 38),
            selection_bg: Color::Rgb(124, 58, 237),
            selection_fg: Color::Rgb(255, 255, 255),
        }
    }

    pub fn nord() -> Self {
        Self {
            name: ThemeName::Nord,
            background: Color::Rgb(46, 52, 64),
            panel: Color::Rgb(59, 66, 82),
            panel_alt: Color::Rgb(67, 76, 94),
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
        }
    }

    pub fn one_dark() -> Self {
        Self {
            name: ThemeName::OneDark,
            background: Color::Rgb(40, 44, 52),
            panel: Color::Rgb(48, 52, 64),
            panel_alt: Color::Rgb(60, 64, 76),
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
        }
    }

    pub fn catppuccin() -> Self {
        Self {
            name: ThemeName::Catppuccin,
            background: Color::Rgb(26, 32, 44),
            panel: Color::Rgb(39, 46, 62),
            panel_alt: Color::Rgb(49, 57, 76),
            text: Color::Rgb(223, 226, 247),
            muted: Color::Rgb(170, 182, 211),
            border: Color::Rgb(67, 74, 101),
            accent: Color::Rgb(159, 147, 255),
            accent_soft: Color::Rgb(120, 109, 186),
            success: Color::Rgb(162, 190, 140),
            warning: Color::Rgb(239, 184, 102),
            error: Color::Rgb(241, 124, 151),
            selection_bg: Color::Rgb(67, 74, 101),
            selection_fg: Color::Rgb(255, 255, 255),
        }
    }

    pub fn solarized() -> Self {
        Self {
            name: ThemeName::Solarized,
            background: Color::Rgb(0, 43, 54),
            panel: Color::Rgb(7, 54, 66),
            panel_alt: Color::Rgb(88, 110, 117),
            text: Color::Rgb(131, 148, 150),
            muted: Color::Rgb(147, 161, 161),
            border: Color::Rgb(38, 139, 210),
            accent: Color::Rgb(38, 139, 210),
            accent_soft: Color::Rgb(88, 110, 117),
            success: Color::Rgb(42, 161, 152),
            warning: Color::Rgb(203, 75, 22),
            error: Color::Rgb(220, 50, 47),
            selection_bg: Color::Rgb(7, 54, 66),
            selection_fg: Color::Rgb(253, 246, 227),
        }
    }

    pub fn from_name(value: &str) -> Self {
        match ThemeName::parse(value).unwrap_or(ThemeName::Dark) {
            ThemeName::Dark => Self::dark(),
            ThemeName::Light => Self::light(),
            ThemeName::Nord => Self::nord(),
            ThemeName::OneDark => Self::one_dark(),
            ThemeName::Catppuccin => Self::catppuccin(),
            ThemeName::Solarized => Self::solarized(),
            ThemeName::Orng => Self::orng(),
            ThemeName::Github => Self::github(),
            ThemeName::Material => Self::material(),
        }
    }

    pub fn border_active(&self) -> Color {
        self.accent
    }

    pub fn border_idle(&self) -> Color {
        self.border
    }
}

#[derive(Clone, Debug)]
pub struct ThemeManager {
    palette: ThemePalette,
}

impl ThemeManager {
    pub fn new(name: &str) -> Self {
        Self {
            palette: ThemePalette::from_name(name),
        }
    }

    pub fn palette(&self) -> ThemePalette {
        self.palette
    }

    pub fn set_mode(&mut self, name: ThemeName) {
        self.palette = match name {
            ThemeName::Dark => ThemePalette::dark(),
            ThemeName::Light => ThemePalette::light(),
            ThemeName::Nord => ThemePalette::nord(),
            ThemeName::OneDark => ThemePalette::one_dark(),
            ThemeName::Catppuccin => ThemePalette::catppuccin(),
            ThemeName::Solarized => ThemePalette::solarized(),
            ThemeName::Orng => ThemePalette::orng(),
            ThemeName::Github => ThemePalette::github(),
            ThemeName::Material => ThemePalette::material(),
        };
    }

    pub fn toggle(&mut self) {
        let next = self.palette.name.toggle();
        self.set_mode(next);
    }

    pub fn name(&self) -> &'static str {
        self.palette.name.as_str()
    }
}
