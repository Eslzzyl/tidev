//! Theme name enumeration used by both config and UI rendering.
//!
//! This type lives in `tidev-tui` because it is a UI concept.
//! Config serialisation uses `String` and calls `ThemeName::parse()`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeName {
    Dark,
    Light,
    Nord,
    OneDark,
    Mocha,
    Solarized,
    Orng,
    Github,
    Material,
    Everforest,
    EverforestLight,
    Dusk,
    Gruvbox,
    GruvboxLight,
    TokyoNight,
    RosePine,
    RosePineDawn,
    Contrast,
}

impl ThemeName {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "nord" => Some(Self::Nord),
            "one-dark" | "one_dark" | "onedark" => Some(Self::OneDark),
            "mocha" => Some(Self::Mocha),
            "solarized" => Some(Self::Solarized),
            "orng" => Some(Self::Orng),
            "github" => Some(Self::Github),
            "material" => Some(Self::Material),
            "everforest" => Some(Self::Everforest),
            "everforest-light" | "everforest_light" | "everforestlight" => {
                Some(Self::EverforestLight)
            }
            "dusk" => Some(Self::Dusk),
            "gruvbox" => Some(Self::Gruvbox),
            "gruvbox-light" | "gruvbox_light" | "gruvboxlight" => Some(Self::GruvboxLight),
            "tokyo-night" | "tokyo_night" | "tokyonight" => Some(Self::TokyoNight),
            "rose-pine" | "rose_pine" | "rosepine" => Some(Self::RosePine),
            "rose-pine-dawn" | "rose_pine_dawn" | "rosepinedawn" => Some(Self::RosePineDawn),
            "contrast" => Some(Self::Contrast),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Nord => "nord",
            Self::OneDark => "one-dark",
            Self::Mocha => "mocha",
            Self::Solarized => "solarized",
            Self::Orng => "orng",
            Self::Github => "github",
            Self::Material => "material",
            Self::Everforest => "everforest",
            Self::EverforestLight => "everforest-light",
            Self::Dusk => "dusk",
            Self::Gruvbox => "gruvbox",
            Self::GruvboxLight => "gruvbox-light",
            Self::TokyoNight => "tokyo-night",
            Self::RosePine => "rose-pine",
            Self::RosePineDawn => "rose-pine-dawn",
            Self::Contrast => "contrast",
        }
    }

    pub fn all() -> &'static [ThemeName] {
        &[
            ThemeName::Dark,
            ThemeName::Light,
            ThemeName::Nord,
            ThemeName::OneDark,
            ThemeName::Mocha,
            ThemeName::Solarized,
            ThemeName::Orng,
            ThemeName::Github,
            ThemeName::Material,
            ThemeName::Everforest,
            ThemeName::EverforestLight,
            ThemeName::Dusk,
            ThemeName::Gruvbox,
            ThemeName::GruvboxLight,
            ThemeName::TokyoNight,
            ThemeName::RosePine,
            ThemeName::RosePineDawn,
            ThemeName::Contrast,
        ]
    }

    pub fn toggle(self) -> Self {
        let all = Self::all();
        let index = all.iter().position(|theme| *theme == self).unwrap_or(0);
        all[(index + 1) % all.len()]
    }

    pub fn is_dark(self) -> bool {
        matches!(
            self,
            Self::Dark
                | Self::Nord
                | Self::OneDark
                | Self::Mocha
                | Self::Solarized
                | Self::Everforest
                | Self::Dusk
                | Self::Gruvbox
                | Self::TokyoNight
                | Self::RosePine
                | Self::Contrast
        )
    }
}
