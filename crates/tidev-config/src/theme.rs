//! Theme definition loading for tidev.
//!
//! Themes are TOML files: bundled presets embedded at compile time plus
//! optional user themes in `<config_dir>/themes/`. A theme's id is its file
//! stem (kebab-case); user themes override bundled themes with the same id.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Deserializer};

/// Bundled theme preset directory embedded at compile time.
static THEMES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../themes");

/// An RGB color parsed from a hex string such as `"rrggbb"` or `"#rrggbb"`.
/// The `#` prefix is optional because double-click selection of a color code
/// usually omits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColor(pub u8, pub u8, pub u8);

impl<'de> Deserialize<'de> for ThemeColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let invalid = || {
            serde::de::Error::custom(format!(
                "invalid color {s:?}: expected 6 hex digits, optionally prefixed with \"#\""
            ))
        };
        let hex = s.strip_prefix('#').unwrap_or(&s);
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(invalid());
        }
        let bytes = hex.as_bytes();
        let parse = |range: std::ops::Range<usize>| {
            u8::from_str_radix(std::str::from_utf8(&bytes[range]).unwrap_or(""), 16).ok()
        };
        match (parse(0..2), parse(2..4), parse(4..6)) {
            (Some(r), Some(g), Some(b)) => Ok(Self(r, g, b)),
            _ => Err(invalid()),
        }
    }
}

/// Serializable definition of one theme, parsed from a single TOML file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeDefinition {
    /// Whether this is a dark theme. Controls theme panel grouping, hover
    /// lift direction and diff background shades.
    pub dark: bool,
    /// Optional syntect theme key for markdown/diff syntax highlighting.
    /// When absent, a default is derived from `dark`.
    #[serde(default)]
    pub syntax_theme: Option<String>,
    pub background: ThemeColor,
    pub panel: ThemeColor,
    pub panel_alt: ThemeColor,
    pub panel_light: ThemeColor,
    pub text: ThemeColor,
    pub muted: ThemeColor,
    pub border: ThemeColor,
    pub accent: ThemeColor,
    pub accent_soft: ThemeColor,
    pub success: ThemeColor,
    pub warning: ThemeColor,
    pub error: ThemeColor,
    /// Diff added-line foreground: body text of insert rows, +/- markers,
    /// sidebar change counts and the theme panel's diff swatch. Syntax-
    /// highlighted diff content keeps its highlighting colors instead.
    pub diff_add: ThemeColor,
    /// Diff deleted-line foreground: body text of delete rows, +/- markers,
    /// sidebar change counts and the theme panel's diff swatch. Syntax-
    /// highlighted diff content keeps its highlighting colors instead.
    pub diff_delete: ThemeColor,
    /// Optional background tint for diff added rows. When absent, a built-in
    /// default derived from `dark` is used (dark themes get the dark set,
    /// light themes the light set).
    #[serde(default)]
    pub diff_add_bg: Option<ThemeColor>,
    /// Optional background tint for diff deleted rows. When absent, a
    /// built-in default derived from `dark` is used (dark themes get the
    /// dark set, light themes the light set).
    #[serde(default)]
    pub diff_delete_bg: Option<ThemeColor>,
    pub selection_bg: ThemeColor,
    pub selection_fg: ThemeColor,
    pub mode_build: ThemeColor,
    pub mode_plan: ThemeColor,
}

impl ThemeDefinition {
    /// The syntax highlighting theme key to use for this theme.
    pub fn syntax_theme_key(&self) -> &str {
        self.syntax_theme.as_deref().unwrap_or(if self.dark {
            "base16-ocean.dark"
        } else {
            "InspiredGitHub"
        })
    }
}

/// Catalog of all available themes: bundled presets plus user themes.
#[derive(Clone, Debug)]
pub struct ThemeCatalog {
    themes: BTreeMap<String, ThemeDefinition>,
}

impl ThemeCatalog {
    /// Load bundled themes and user themes from `<config_dir>/themes/`.
    ///
    /// User themes override bundled themes with the same id. A user theme
    /// file that fails to parse is skipped with a warning; a malformed
    /// bundled theme is a hard error because it ships with the binary.
    pub fn load(config_dir: &Path) -> Result<Self> {
        let mut themes = Self::load_bundled()?;

        let user_dir = config_dir.join("themes");
        if user_dir.is_dir() {
            let mut entries: Vec<_> = match fs::read_dir(&user_dir) {
                Ok(entries) => entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
                    .collect(),
                Err(err) => {
                    log::warn!(
                        "failed to read user theme directory {}: {err}",
                        user_dir.display()
                    );
                    Vec::new()
                }
            };
            entries.sort();
            for path in entries {
                let id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default()
                    .to_string();
                match Self::parse_file(&path) {
                    Ok(def) => {
                        themes.insert(id, def);
                    }
                    Err(err) => {
                        log::warn!("skipping theme {}: {err:#}", path.display());
                    }
                }
            }
        }

        Ok(Self { themes })
    }

    /// Load bundled themes embedded in the binary.
    fn load_bundled() -> Result<BTreeMap<String, ThemeDefinition>> {
        let mut themes = BTreeMap::new();
        let mut files: Vec<_> = THEMES_DIR.files().collect();
        files.sort_by_key(|file| file.path());
        for file in files {
            if file.path().extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let id = file
                .path()
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            let content = file.contents_utf8().context("non-utf8 theme preset file")?;
            let def: ThemeDefinition = toml::from_str(content)
                .with_context(|| format!("failed to parse bundled theme {id}"))?;
            themes.insert(id, def);
        }
        Ok(themes)
    }

    fn parse_file(path: &Path) -> Result<ThemeDefinition> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Look up a theme by id.
    pub fn get(&self, id: &str) -> Option<&ThemeDefinition> {
        self.themes.get(id)
    }

    /// Iterate over all themes `(id, definition)` sorted by id.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ThemeDefinition)> {
        self.themes.iter().map(|(id, def)| (id.as_str(), def))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_COLOR_FIELDS: [&str; 18] = [
        "background",
        "panel",
        "panel_alt",
        "panel_light",
        "text",
        "muted",
        "border",
        "accent",
        "accent_soft",
        "success",
        "warning",
        "error",
        "diff_add",
        "diff_delete",
        "selection_bg",
        "selection_fg",
        "mode_build",
        "mode_plan",
    ];

    fn theme_toml(dark: bool) -> String {
        let mut toml = format!("dark = {dark}\n");
        for field in ALL_COLOR_FIELDS {
            toml.push_str(&format!("{field} = \"#010203\"\n"));
        }
        toml
    }

    fn load_from(dir: &Path) -> ThemeCatalog {
        ThemeCatalog::load(dir).expect("catalog should load")
    }

    #[test]
    fn bundled_catalog_contains_all_builtin_themes() {
        let catalog = load_from(Path::new("/nonexistent"));
        for id in [
            "dark",
            "light",
            "nord",
            "one-dark",
            "mocha",
            "solarized",
            "orng",
            "github",
            "material",
            "everforest",
            "everforest-light",
            "dusk",
            "gruvbox",
            "gruvbox-light",
            "tokyo-night",
            "rose-pine",
            "rose-pine-dawn",
            "contrast",
        ] {
            assert!(catalog.get(id).is_some(), "missing bundled theme {id}");
        }
    }

    #[test]
    fn bundled_dark_theme_matches_source() {
        let catalog = load_from(Path::new("/nonexistent"));
        let dark = catalog.get("dark").unwrap();
        assert!(dark.dark);
        assert_eq!(dark.background, ThemeColor(12, 16, 23));
        assert_eq!(dark.text, ThemeColor(229, 231, 235));
        assert_eq!(dark.syntax_theme_key(), "base16-ocean.dark");
        let light = catalog.get("light").unwrap();
        assert!(!light.dark);
        assert_eq!(light.background, ThemeColor(255, 255, 255));
        assert_eq!(light.syntax_theme_key(), "InspiredGitHub");
    }

    #[test]
    fn theme_color_parses_hex() {
        #[derive(Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let parsed: Wrapper = toml::from_str("color = \"#0C1017\"").unwrap();
        assert_eq!(parsed.color, ThemeColor(12, 16, 23));
        // The `#` prefix is optional (double-click selection omits it).
        let parsed: Wrapper = toml::from_str("color = \"0c1017\"").unwrap();
        assert_eq!(parsed.color, ThemeColor(12, 16, 23));
    }

    #[test]
    fn theme_color_rejects_bad_hex() {
        #[derive(Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        for value in [
            "#12345", "#gggggg", "#1234567", "#", "12345", "abcdef0", "##123456",
        ] {
            assert!(
                toml::from_str::<Wrapper>(&format!("color = \"{value}\"")).is_err(),
                "color {value} should be rejected"
            );
        }
    }

    #[test]
    fn theme_definition_rejects_unknown_fields() {
        let toml = format!("{}\nbackgroundd = \"#010203\"\n", theme_toml(true));
        assert!(toml::from_str::<ThemeDefinition>(&toml).is_err());
    }

    #[test]
    fn user_themes_override_bundled() {
        let dir = std::env::temp_dir().join(format!("tidev-theme-test-{}", std::process::id()));
        let themes = dir.join("themes");
        fs::create_dir_all(&themes).unwrap();
        fs::write(themes.join("dark.toml"), theme_toml(false)).unwrap();
        let catalog = load_from(&dir);
        assert!(!catalog.get("dark").unwrap().dark);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_user_theme_is_skipped() {
        let dir = std::env::temp_dir().join(format!("tidev-theme-bad-{}", std::process::id()));
        let themes = dir.join("themes");
        fs::create_dir_all(&themes).unwrap();
        fs::write(themes.join("broken.toml"), "dark = \"not-a-bool\"\n").unwrap();
        let catalog = load_from(&dir);
        assert!(catalog.get("broken").is_none());
        assert!(catalog.get("dark").is_some());
        fs::remove_dir_all(&dir).unwrap();
    }
}
