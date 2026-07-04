use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use tidev_config::provider::ProviderSource;

use super::App;

#[derive(Clone, Debug)]
pub(crate) enum ConnectDialog {
    ProviderPicker { selected: usize },
    ApiKey { provider_id: String },
}

impl ConnectDialog {
    pub(crate) fn provider_picker() -> Self {
        Self::ProviderPicker { selected: 0 }
    }

    pub(crate) fn api_key(provider_id: impl Into<String>) -> Self {
        Self::ApiKey {
            provider_id: provider_id.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ProviderPickerItem {
    Provider {
        provider_id: String,
        display_name: String,
        source: ProviderSource,
        connected: bool,
    },
}

impl App {
    pub(crate) fn open_connect_dialog(&mut self) -> Result<()> {
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();
        self.mcp_panel = None;

        self.composer.clear();
        self.composer
            .set_placeholder("Search providers by id or display name");
        self.connect_dialog = Some(ConnectDialog::provider_picker());

        Ok(())
    }

    pub(crate) fn provider_picker_items(&self) -> Vec<ProviderPickerItem> {
        let query = self.composer.text().trim().to_ascii_lowercase();

        let config = self.config.read().unwrap();
        let items = config
            .provider_ids()
            .into_iter()
            .filter_map(|provider_id| {
                let display_name = config
                    .provider_display_name(&provider_id)
                    .unwrap_or(&provider_id)
                    .to_string();
                let source = config
                    .provider_source(&provider_id)
                    .unwrap_or(ProviderSource::User);
                let connected = self.auth.api_key(&provider_id).is_some();

                if provider_picker_matches(&query, &provider_id, &display_name) {
                    Some(ProviderPickerItem::Provider {
                        provider_id,
                        display_name,
                        source,
                        connected,
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        drop(config);

        items
    }

    fn begin_connect_key_entry(&mut self, provider_id: String) {
        let label = self
            .config
            .read()
            .unwrap()
            .provider_display_name(&provider_id)
            .map(str::to_string)
            .unwrap_or_else(|| provider_id.clone());
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder(format!("Enter API key for {label}"));
        self.connect_dialog = Some(ConnectDialog::api_key(provider_id));
    }

    fn finish_connect_api_key(&mut self, provider_id: String, api_key: String) -> Result<()> {
        if api_key.trim().is_empty() {
            self.cancel_connect_dialog();
            self.last_notice = Some("API key was empty".to_string());
            return Ok(());
        }

        self.auth.set_api_key(provider_id.clone(), api_key);
        self.auth.save(&self.paths)?;

        let model = self
            .config
            .read()
            .unwrap()
            .resolve_provider_default_model(&self.auth, &provider_id)?;

        self.active_model = model.clone();
        self.conversation.set_model(
            model.provider_id.clone(),
            model.provider_display_name.clone(),
            model.model_id.clone(),
            model.display_name.clone(),
        );

        if self
            .store
            .load_session_record(self.conversation.session_id)?
            .is_some()
        {
            self.store.update_session_model(
                self.conversation.session_id,
                &model.provider_id,
                &model.provider_display_name,
                &model.model_id,
                &model.display_name,
            )?;
        }

        self.cancel_connect_dialog();
        self.last_notice = Some(format!("Connected to {}", model.provider_display_name));
        Ok(())
    }

    fn cancel_connect_dialog(&mut self) {
        self.connect_dialog = None;
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask tidev about your code, task, or question...");
    }

    pub(crate) fn handle_connect_dialog_key(
        &mut self,
        key: KeyEvent,
        dialog: ConnectDialog,
    ) -> Result<()> {
        match dialog {
            ConnectDialog::ProviderPicker { selected } => match key {
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => {
                    self.cancel_connect_dialog();
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    modifiers,
                    ..
                } if !modifiers.contains(KeyModifiers::SHIFT)
                    && !modifiers.contains(KeyModifiers::ALT) =>
                {
                    let items = self.provider_picker_items();
                    if items.is_empty() {
                        return Ok(());
                    }

                    let selected = selected.min(items.len().saturating_sub(1));
                    match items.get(selected) {
                        Some(ProviderPickerItem::Provider { provider_id, .. }) => {
                            self.begin_connect_key_entry(provider_id.clone());
                        }
                        None => {}
                    }
                }
                KeyEvent {
                    code: KeyCode::Up, ..
                } => {
                    let item_count = self.provider_picker_items().len();
                    let current = selected.min(item_count.saturating_sub(1));
                    let next = if current == 0 {
                        item_count.saturating_sub(1)
                    } else {
                        current - 1
                    };
                    self.connect_dialog = Some(ConnectDialog::ProviderPicker { selected: next });
                }
                KeyEvent {
                    code: KeyCode::Down,
                    ..
                } => {
                    let item_count = self.provider_picker_items().len();
                    let current = selected.min(item_count.saturating_sub(1));
                    let next = if item_count == 0 {
                        0
                    } else {
                        (current + 1) % item_count
                    };
                    self.connect_dialog = Some(ConnectDialog::ProviderPicker { selected: next });
                }
                KeyEvent {
                    code: KeyCode::Tab, ..
                } => {}
                KeyEvent {
                    code: KeyCode::Char('p'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    let known_ids = self.config.read().unwrap().provider_ids();
                    let pruned = self.auth.prune_orphan_providers(&known_ids);
                    if pruned > 0 {
                        self.auth.save(&self.paths)?;
                        self.last_notice =
                            Some(format!("Pruned {pruned} orphan provider(s) from auth file"));
                    } else {
                        self.last_notice = Some("No orphan auth entries to prune".to_string());
                    }
                }
                _ => {
                    let previous_query = self.composer.text().to_string();
                    let _ = self.composer.handle_key_with_history(key, false);

                    if self.composer.text() != previous_query {
                        self.connect_dialog = Some(ConnectDialog::ProviderPicker { selected: 0 });
                    }
                }
            },
            ConnectDialog::ApiKey { provider_id } => {
                if matches!(key.code, KeyCode::Esc) {
                    self.cancel_connect_dialog();
                    return Ok(());
                }

                if let Some(submission) = self.composer.handle_key_with_history(key, false) {
                    self.finish_connect_api_key(provider_id, submission)?;
                }
            }
        }

        Ok(())
    }
}

fn provider_picker_matches(query: &str, provider_id: &str, display_name: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let provider_id = provider_id.to_ascii_lowercase();
    let display_name = display_name.to_ascii_lowercase();
    provider_id.contains(query) || display_name.contains(query)
}
