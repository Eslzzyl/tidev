use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiConfig {
    pub sidebar_width: u16,
    pub welcome_width: u16,
    pub max_input_lines: u16,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 30,
            welcome_width: 72,
            max_input_lines: 6,
        }
    }
}
