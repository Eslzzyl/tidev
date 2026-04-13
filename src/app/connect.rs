use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::ProviderSource;
use crate::provider_setup::{ConnectDialog, NewProviderDraft, NewProviderStep};

use super::App;

#[derive(Clone, Debug)]
pub(crate) enum ProviderPickerItem {
    Provider {
        provider_id: String,
        display_name: String,
        source: ProviderSource,
        connected: bool,
    },
    AddNew {
        query: String,
    },
}

impl App {
    pub(crate) fn open_connect_dialog(&mut self) -> Result<()> {
        self.command_palette.clear();

        self.composer.clear();
        self.composer
            .set_placeholder("Search providers by id or display name");
        self.connect_dialog = Some(ConnectDialog::provider_picker());

        Ok(())
    }

    pub(crate) fn provider_picker_items(&self) -> Vec<ProviderPickerItem> {
        let query = self.composer.text().trim().to_ascii_lowercase();

        let mut items = self
            .config
            .provider_ids()
            .into_iter()
            .filter_map(|provider_id| {
                let display_name = self
                    .config
                    .provider_display_name(&provider_id)
                    .unwrap_or(&provider_id)
                    .to_string();
                let source = self
                    .config
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

        items.push(ProviderPickerItem::AddNew {
            query: self.composer.text().trim().to_string(),
        });

        items
    }

    fn begin_connect_key_entry(&mut self, provider_id: String) {
        let label = self
            .config
            .provider_display_name(&provider_id)
            .map(str::to_string)
            .unwrap_or_else(|| provider_id.clone());
        self.composer.clear();
        self.composer
            .set_placeholder(format!("Enter API key for {label}"));
        self.connect_dialog = Some(ConnectDialog::ApiKey { provider_id });
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
            .resolve_provider_default_model(&self.auth, &provider_id)?;
        self.active_model = model.clone();
        self.conversation.provider_id = model.provider_id.clone();
        self.conversation.model_id = model.model_id.clone();
        self.store.update_session_model(
            self.conversation.session_id,
            &model.provider_id,
            &model.model_id,
        )?;

        self.cancel_connect_dialog();
        self.last_notice = Some(format!("Connected to {}", model.provider_display_name));
        self.push_system_message(format!("Connected to {}", model.provider_display_name))?;
        Ok(())
    }

    fn finish_new_provider_setup(&mut self, draft: NewProviderDraft) -> Result<()> {
        let provider_id = draft.provider_id.clone();

        if self.config.provider_exists(&provider_id) {
            self.last_notice = Some(format!("Provider '{provider_id}' already exists"));
            self.show_new_provider_step(NewProviderStep::ProviderId, draft);
            return Ok(());
        }

        let (provider_id, provider_config, api_key) = draft.into_provider_config()?;

        self.config
            .providers
            .insert(provider_id.clone(), provider_config);
        self.config.save(&self.paths)?;

        self.auth.set_api_key(provider_id.clone(), api_key);
        self.auth.save(&self.paths)?;

        let model = self
            .config
            .resolve_provider_default_model(&self.auth, &provider_id)?;
        self.active_model = model.clone();
        self.conversation.provider_id = model.provider_id.clone();
        self.conversation.model_id = model.model_id.clone();
        self.store.update_session_model(
            self.conversation.session_id,
            &model.provider_id,
            &model.model_id,
        )?;

        self.cancel_connect_dialog();
        self.last_notice = Some(format!(
            "Created provider '{provider_id}' and connected to {}",
            model.provider_display_name
        ));
        self.push_system_message(format!("Connected to {}", model.provider_display_name))?;
        Ok(())
    }

    fn begin_new_provider_setup(&mut self) {
        self.show_new_provider_step(NewProviderStep::ProviderId, NewProviderDraft::default());
    }

    fn show_new_provider_step(&mut self, step: NewProviderStep, draft: NewProviderDraft) {
        self.composer.clear();
        self.composer
            .set_placeholder(format!("{} · {}", step.label(), step.help()));
        self.composer.set_text(draft.current_value(step));
        self.connect_dialog = Some(ConnectDialog::NewProvider { step, draft });
    }

    fn cancel_connect_dialog(&mut self) {
        self.connect_dialog = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
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
                        Some(ProviderPickerItem::AddNew { .. }) => {
                            self.begin_new_provider_setup();
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
            ConnectDialog::NewProvider { step, mut draft } => {
                if matches!(key.code, KeyCode::Esc) {
                    self.cancel_connect_dialog();
                    return Ok(());
                }

                if step == NewProviderStep::AddAnotherModel {
                    if let Some(submission) = self.composer.handle_key_with_history(key, false) {
                        match parse_add_another_model_answer(&submission) {
                            Some(true) => {
                                self.show_new_provider_step(NewProviderStep::ModelId, draft);
                            }
                            Some(false) => {
                                self.finish_new_provider_setup(draft)?;
                            }
                            None => {
                                self.last_notice = Some("Enter y or n".to_string());
                                self.show_new_provider_step(step, draft);
                            }
                        }
                    } else if matches!(key.code, KeyCode::Enter)
                        && !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        self.finish_new_provider_setup(draft)?;
                    }

                    return Ok(());
                }

                if let Some(submission) = self.composer.handle_key_with_history(key, false) {
                    if let Err(error) = draft.apply_step(step, &submission) {
                        self.last_notice = Some(error.to_string());
                        self.show_new_provider_step(step, draft);
                        return Ok(());
                    }

                    if step == NewProviderStep::ProviderId
                        && self.config.providers.contains_key(&draft.provider_id)
                    {
                        self.last_notice =
                            Some(format!("Provider '{}' already exists", draft.provider_id));
                        self.show_new_provider_step(NewProviderStep::ProviderId, draft);
                        return Ok(());
                    }

                    if step == NewProviderStep::ModelId
                        && draft.models.contains_key(&draft.model.model_id)
                    {
                        self.last_notice =
                            Some(format!("Model '{}' already exists", draft.model.model_id));
                        self.show_new_provider_step(NewProviderStep::ModelId, draft);
                        return Ok(());
                    }

                    if step == NewProviderStep::Temperature {
                        if let Err(error) = draft.finish_current_model() {
                            self.last_notice = Some(error.to_string());
                            self.show_new_provider_step(NewProviderStep::ModelId, draft);
                            return Ok(());
                        }

                        self.show_new_provider_step(NewProviderStep::AddAnotherModel, draft);
                        return Ok(());
                    }

                    if let Some(next_step) = step.next() {
                        self.show_new_provider_step(next_step, draft);
                    } else {
                        self.finish_new_provider_setup(draft)?;
                    }
                }
            }
        }

        Ok(())
    }
}

fn parse_add_another_model_answer(input: &str) -> Option<bool> {
    let answer = input.trim().to_ascii_lowercase();

    match answer.as_str() {
        "y" | "yes" | "true" | "1" => Some(true),
        "n" | "no" | "false" | "0" => Some(false),
        _ if answer.is_empty() => Some(false),
        _ => None,
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
