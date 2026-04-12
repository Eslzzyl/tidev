use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::provider_setup::{ConnectDialog, NewProviderDraft, NewProviderStep};

use super::App;

impl App {
    pub(crate) fn open_connect_dialog(&mut self, provider_hint: Option<&str>) -> Result<()> {
        self.command_palette.clear();

        if let Some(provider_hint) = provider_hint
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if matches!(provider_hint, "new" | "add" | "create") {
                self.begin_new_provider_setup();
                return Ok(());
            }

            if !self.config.providers.contains_key(provider_hint) {
                self.last_notice = Some(format!("Unknown provider '{provider_hint}'"));
                return Ok(());
            }

            self.begin_connect_key_entry(provider_hint.to_string());
            return Ok(());
        }

        let providers = self.config.provider_ids();
        match providers.len() {
            0 => self.begin_new_provider_setup(),
            1 => self.begin_connect_key_entry(providers[0].clone()),
            _ => {
                self.composer.clear();
                self.composer.set_placeholder(
                    "Select a provider with the arrow keys, or create a new one with Enter",
                );
                self.connect_dialog = Some(ConnectDialog::provider_picker());
            }
        }

        Ok(())
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
        let (provider_id, provider_config) = draft.into_provider_config();

        if self.config.providers.contains_key(&provider_id) {
            self.last_notice = Some(format!("Provider '{provider_id}' already exists"));
            self.begin_connect_key_entry(provider_id);
            return Ok(());
        }

        self.config
            .providers
            .insert(provider_id.clone(), provider_config);
        self.config.save(&self.paths)?;

        self.last_notice = Some(format!("Created provider '{provider_id}'"));
        self.begin_connect_key_entry(provider_id);
        Ok(())
    }

    fn begin_new_provider_setup(&mut self) {
        let dialog = ConnectDialog::new_provider();
        if let ConnectDialog::NewProvider { step, draft } = dialog {
            self.show_new_provider_step(step, draft);
        }
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
                    code: KeyCode::Up, ..
                } => {
                    let providers = self.config.provider_ids();
                    let len = providers.len().saturating_add(1);
                    let next = if selected == 0 {
                        len.saturating_sub(1)
                    } else {
                        selected - 1
                    };
                    self.connect_dialog = Some(ConnectDialog::ProviderPicker { selected: next });
                }
                KeyEvent {
                    code: KeyCode::Down,
                    ..
                } => {
                    let providers = self.config.provider_ids();
                    let len = providers.len().saturating_add(1);
                    let next = if len == 0 { 0 } else { (selected + 1) % len };
                    self.connect_dialog = Some(ConnectDialog::ProviderPicker { selected: next });
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    modifiers,
                    ..
                } if !modifiers.contains(KeyModifiers::SHIFT)
                    && !modifiers.contains(KeyModifiers::ALT) =>
                {
                    if selected == 0 {
                        self.begin_new_provider_setup();
                    } else if let Some(provider_id) =
                        self.config.provider_ids().get(selected - 1).cloned()
                    {
                        self.begin_connect_key_entry(provider_id);
                    }
                }
                _ => {}
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

                if let Some(submission) = self.composer.handle_key_with_history(key, false) {
                    if let Err(error) = draft.apply_step(step, &submission) {
                        self.last_notice = Some(error.to_string());
                        self.show_new_provider_step(step, draft);
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
