use tidev_engine::config::{AppConfig, SharedConfig};

#[derive(Clone, Debug)]
pub enum SettingType {
    Toggle(bool),
    Number {
        value: f32,
        min: f32,
        max: f32,
    },
    Cycle {
        options: Vec<String>,
        selected: usize,
    },
}

#[derive(Clone, Debug)]
pub struct SettingItem {
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub key: SettingKey,
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingKey {
    NotificationEnabled,
    LoggingEnabled,
    LogLevel,
    SaveRequestBody,
    SaveResponseBody,
    ScrollSpeed,
    RtkEnabled,
}

#[derive(Clone, Debug)]
pub struct SettingsPanelState {
    pub selected_index: usize,
    pub items: Vec<SettingItem>,
}

impl SettingsPanelState {
    pub fn new(config: &SharedConfig) -> Self {
        let config = config.read().unwrap();
        let log_levels = vec![
            "DEBUG".to_string(),
            "INFO".to_string(),
            "WARN".to_string(),
            "ERROR".to_string(),
        ];
        let log_level_index = log_levels
            .iter()
            .position(|l| l == &config.logging.level.to_uppercase())
            .unwrap_or(1);

        let items = vec![
            SettingItem {
                name: "Notifications".to_string(),
                description: "Enable system notifications".to_string(),
                setting_type: SettingType::Toggle(config.notifications.enabled),
                key: SettingKey::NotificationEnabled,
                disabled: false,
            },
            SettingItem {
                name: "Logging".to_string(),
                description: "Enable debug logging to file".to_string(),
                setting_type: SettingType::Toggle(config.logging.enabled),
                key: SettingKey::LoggingEnabled,
                disabled: false,
            },
            SettingItem {
                name: "Log Level".to_string(),
                description: format!("Log level: {}", log_levels[log_level_index]),
                setting_type: SettingType::Cycle {
                    options: log_levels,
                    selected: log_level_index,
                },
                key: SettingKey::LogLevel,
                disabled: false,
            },
            SettingItem {
                name: "Save Request Body".to_string(),
                description: "Save LLM request bodies to /tmp/tidev-requests/ for debugging"
                    .to_string(),
                setting_type: SettingType::Toggle(config.logging.save_request_body),
                key: SettingKey::SaveRequestBody,
                disabled: false,
            },
            SettingItem {
                name: "Save Response Body".to_string(),
                description:
                    "Save LLM streaming response payloads to /tmp/tidev-responses/ for debugging"
                        .to_string(),
                setting_type: SettingType::Toggle(config.logging.save_response_body),
                key: SettingKey::SaveResponseBody,
                disabled: false,
            },
            SettingItem {
                name: "Scroll Speed".to_string(),
                description: format!("Scroll speed multiplier: {:.1}", config.ui.scroll_speed),
                setting_type: SettingType::Number {
                    value: config.ui.scroll_speed,
                    min: 1.0,
                    max: 10.0,
                },
                key: SettingKey::ScrollSpeed,
                disabled: false,
            },
            // ── RTK ─────────────────────────────────────────────────────
            SettingItem {
                name: "RTK".to_string(),
                description: if config.rtk.installed {
                    "Enable RTK to compress command outputs and save tokens".to_string()
                } else {
                    "RTK is not installed (install with: brew install rtk)".to_string()
                },
                setting_type: SettingType::Toggle(config.rtk.enabled && config.rtk.installed),
                key: SettingKey::RtkEnabled,
                disabled: false,
            },
        ];

        Self {
            selected_index: 0,
            items,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.items.len() - 1 {
            self.selected_index += 1;
        }
    }

    /// Toggle for Toggle / Cycle type
    pub fn toggle_selected(&mut self, rtk_installed: bool) {
        let selected = self.selected_index;
        let Some(item) = self.items.get(selected) else {
            return;
        };

        // Skip disabled items
        if item.disabled {
            return;
        }

        // Non-master toggle / cycle: get a mutable reference now
        if let Some(item) = self.items.get_mut(selected) {
            match &mut item.setting_type {
                SettingType::Toggle(val) => {
                    // Don't allow toggling RTK if it's not installed
                    if item.key == SettingKey::RtkEnabled && !rtk_installed {
                        return;
                    }
                    *val = !*val;
                }
                SettingType::Cycle { options, selected } => {
                    *selected = (*selected + 1) % options.len();
                    item.description = format!("Log level: {}", options[*selected]);
                }
                SettingType::Number { .. } => {}
            }
        }
    }

    /// Increase value for Number type only
    pub fn increase_selected(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected_index)
            && let SettingType::Number { value, min: _, max } = &mut item.setting_type
        {
            *value = (*value + 1.0).min(*max);
            item.description = format!("Scroll speed multiplier: {:.1}", *value);
        }
    }

    /// Decrease value for Number type only
    pub fn decrease_selected(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected_index)
            && let SettingType::Number { value, min, max: _ } = &mut item.setting_type
        {
            *value = (*value - 1.0).max(*min);
            item.description = format!("Scroll speed multiplier: {:.1}", *value);
        }
    }

    pub fn apply_to_config(&self, config: &mut AppConfig) {
        for item in &self.items {
            match item.key {
                SettingKey::NotificationEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.notifications.enabled = val;
                    }
                }
                SettingKey::LoggingEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.logging.enabled = val;
                    }
                }
                SettingKey::LogLevel => {
                    if let SettingType::Cycle { options, selected } = &item.setting_type
                        && *selected < options.len()
                    {
                        config.logging.level = options[*selected].clone();
                    }
                }
                SettingKey::SaveRequestBody => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.logging.save_request_body = val;
                    }
                }
                SettingKey::SaveResponseBody => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.logging.save_response_body = val;
                    }
                }
                SettingKey::ScrollSpeed => {
                    if let SettingType::Number { value, .. } = item.setting_type {
                        config.ui.scroll_speed = value;
                    }
                }
                SettingKey::RtkEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        // Only allow enabling RTK if it's installed
                        config.rtk.enabled = val && config.rtk.installed;
                    }
                }
            }
        }
    }
}
