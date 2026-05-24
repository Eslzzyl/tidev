use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u32,
    #[serde(default = "default_max_files")]
    pub max_files: u32,
    #[serde(default = "default_console")]
    pub console: bool,
    /// Save LLM request bodies to /tmp/tidev-requests/ for debugging.
    #[serde(default)]
    pub save_request_body: bool,
    /// Maximum request body files to keep before rotating (default: 100).
    #[serde(default = "default_max_request_files")]
    pub max_request_files: usize,
}

fn default_enabled() -> bool {
    false
}

fn default_level() -> String {
    "INFO".to_string()
}

fn default_max_size_mb() -> u32 {
    10
}

fn default_max_files() -> u32 {
    5
}

fn default_console() -> bool {
    false
}

fn default_max_request_files() -> usize {
    100
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: "INFO".to_string(),
            max_size_mb: 10,
            max_files: 5,
            console: false,
            save_request_body: false,
            max_request_files: 100,
        }
    }
}
