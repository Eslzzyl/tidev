use serde::{Deserialize, Serialize};

/// Configuration for the logging subsystem.
///
/// The actual logging runtime (file I/O, rotation, stderr output) lives in
/// the `tidev-logging` crate. This type is purely the configuration struct.
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
    #[serde(default)]
    pub save_request_body: bool,
    #[serde(default = "default_max_request_files")]
    pub max_request_files: usize,
    #[serde(default)]
    pub save_response_body: bool,
    #[serde(default = "default_max_response_files")]
    pub max_response_files: usize,
}

fn default_enabled() -> bool {
    true
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

fn default_max_response_files() -> usize {
    100
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "INFO".to_string(),
            max_size_mb: 10,
            max_files: 5,
            console: false,
            save_request_body: false,
            max_request_files: 100,
            save_response_body: false,
            max_response_files: 100,
        }
    }
}
