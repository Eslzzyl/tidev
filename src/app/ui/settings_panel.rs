use crate::config::AppConfig;

#[derive(Clone, Debug)]
pub enum SettingType {
    Toggle(bool),
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
}

#[derive(Clone, Debug)]
pub struct SettingsPanelState {
    pub selected_index: usize,
    pub items: Vec<SettingItem>,
}

impl SettingsPanelState {
    pub fn new(config: &AppConfig) -> Self {
        let items = vec![
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

    pub fn toggle_selected(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected_index) {
            match &mut item.setting_type {
                SettingType::Toggle(val) => *val = !*val,
            }
        }
    }

    pub fn apply_to_config(&self, config: &mut AppConfig) {
        for item in &self.items {
            match item.key {
                SettingKey::NotificationEnabled => {
                    let SettingType::Toggle(val) = item.setting_type;
                    config.notifications.enabled = val;
                }
                SettingKey::LoggingEnabled => {
                    let SettingType::Toggle(val) = item.setting_type;
                    config.logging.enabled = val;
                }
            }
        }
    }
}
