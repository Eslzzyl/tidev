use crate::config::AppConfig;

#[derive(Clone, Debug)]
pub enum SettingType {
    Toggle(bool),
    Number { value: f32, min: f32, max: f32 },
}

#[derive(Clone, Debug)]
pub struct SettingItem {
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub key: SettingKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingKey {
    NotificationEnabled,
    LoggingEnabled,
    ScrollSpeed,
    RtkEnabled,
    CompressionEnabled,
    LlmCompression,
}

#[derive(Clone, Debug)]
pub struct SettingsPanelState {
    pub selected_index: usize,
    pub items: Vec<SettingItem>,
}

impl SettingsPanelState {
    pub fn new(config: &AppConfig) -> Self {
        let mut items = vec![
            SettingItem {
                name: "Notifications".to_string(),
                description: "Enable system notifications".to_string(),
                setting_type: SettingType::Toggle(config.notifications.enabled),
                key: SettingKey::NotificationEnabled,
            },
            SettingItem {
                name: "Logging".to_string(),
                description: "Enable debug logging to file".to_string(),
                setting_type: SettingType::Toggle(config.logging.enabled),
                key: SettingKey::LoggingEnabled,
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
            },
        ];

        // Add RTK setting if installed, or show as disabled with message
        let rtk_description = if config.rtk.installed {
            "Enable RTK to compress command outputs and save tokens".to_string()
        } else {
            "RTK is not installed (install with: brew install rtk)".to_string()
        };
        items.push(SettingItem {
            name: "RTK".to_string(),
            description: rtk_description,
            setting_type: SettingType::Toggle(config.rtk.enabled && config.rtk.installed),
            key: SettingKey::RtkEnabled,
        });

        items.push(SettingItem {
            name: "Compression".to_string(),
            description: "Enable automatic compression of observations".to_string(),
            setting_type: SettingType::Toggle(config.memory.compression_enabled),
            key: SettingKey::CompressionEnabled,
        });

        items.push(SettingItem {
            name: "LLM Compression".to_string(),
            description: "Use LLM for richer compression (costs tokens; synthetic is default)".to_string(),
            setting_type: SettingType::Toggle(config.memory.llm_compression),
            key: SettingKey::LlmCompression,
        });

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

    /// Toggle for Toggle type only
    pub fn toggle_selected(&mut self, rtk_installed: bool) {
        if let Some(item) = self.items.get_mut(self.selected_index)
            && let SettingType::Toggle(val) = &mut item.setting_type
        {
            // Don't allow toggling RTK if it's not installed
            if item.key == SettingKey::RtkEnabled && !rtk_installed {
                return;
            }
            *val = !*val;
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
                SettingKey::CompressionEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.memory.compression_enabled = val;
                    }
                }
                SettingKey::LlmCompression => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.memory.llm_compression = val;
                    }
                }
            }
        }
    }
}
