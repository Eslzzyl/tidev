use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TmpConfig {
    /// Automatically clean up known tidev temp files on startup.
    #[serde(default)]
    pub auto_cleanup: bool,
    /// Maximum age (in hours) for temp files before they are removed.
    /// Files newer than this are kept.
    #[serde(default = "default_max_age_hours")]
    pub max_age_hours: u64,
}

fn default_max_age_hours() -> u64 {
    24
}

impl Default for TmpConfig {
    fn default() -> Self {
        Self {
            auto_cleanup: false,
            max_age_hours: 24,
        }
    }
}
