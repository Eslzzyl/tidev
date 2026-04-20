use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiConfig {
    pub sidebar_width: u16,
    pub welcome_width: u16,
    pub max_input_lines: u16,
    /// Scroll speed multiplier (default: 3)
    #[serde(default = "default_scroll_speed")]
    pub scroll_speed: f32,
}

fn default_scroll_speed() -> f32 {
    3.0
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 35,
            welcome_width: 72,
            max_input_lines: 6,
            scroll_speed: 3.0,
        }
    }
}
