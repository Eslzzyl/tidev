use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiConfig {
    pub sidebar_width: u16,
    pub welcome_width: u16,
    pub max_input_lines: u16,
    /// Scroll speed multiplier (default: 3)
    #[serde(default = "default_scroll_speed")]
    pub scroll_speed: f32,
    /// GUI external editor command (e.g., "code --wait", "cursor --wait").
    /// Falls back to $VISUAL → $EDITOR → auto-detect among common editors.
    #[serde(default)]
    pub external_editor: Option<String>,
    /// Number of spaces a tab character expands to in diff views (default: 4).
    #[serde(default = "default_tab_width")]
    pub tab_width: usize,
}

fn default_scroll_speed() -> f32 {
    3.0
}

fn default_tab_width() -> usize {
    4
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 40,
            welcome_width: 90,
            max_input_lines: 6,
            scroll_speed: 3.0,
            external_editor: None,
            tab_width: 4,
        }
    }
}
