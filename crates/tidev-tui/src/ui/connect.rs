use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use tidev_engine::config::ProviderSource;
use tidev_engine::provider_setup::{
    ConnectDialog, EditModelStep, EditProviderDraft, EditProviderStep, NewProviderDraft,
    NewProviderStep,
};

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
        self.at_mention.clear();
        self.draft_attachments.clear();
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
        let mut items = config
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

        items.push(ProviderPickerItem::AddNew {
            query: self.composer.text().trim().to_string(),
        });

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
        self.composer.clear();
        self.composer
            .set_placeholder(format!("Enter API key for {label}"));
        self.connect_dialog = Some(ConnectDialog::ApiKey { provider_id });
    }

    fn begin_provider_edit(&mut self, provider_id: String) -> Result<()> {
        self.begin_provider_edit_with_selection(provider_id, None)
    }

    pub(crate) fn begin_provider_edit_for_model(
        &mut self,
        provider_id: String,
        model_id: String,
    ) -> Result<()> {
        self.begin_provider_edit_with_selection(provider_id, Some(model_id))
    }

    fn begin_provider_edit_with_selection(
        &mut self,
        provider_id: String,
        selected_model_id: Option<String>,
    ) -> Result<()> {
        let Some(provider) = self
            .config
            .read()
            .unwrap()
            .providers
            .get(&provider_id)
            .cloned()
        else {
            self.last_notice = Some(format!("Provider '{provider_id}' is not editable"));
            return Ok(());
        };

        let draft = EditProviderDraft::from_provider(
            &provider,
            self.auth.api_key(&provider_id).map(str::to_string),
        );
        if let Some(model_id) = selected_model_id
            && let Some(index) = draft
                .models
                .keys()
                .position(|candidate| candidate == &model_id)
        {
            let mut draft = draft;
            draft.selected_model_index = index;
            self.show_edit_provider_step(provider_id, EditProviderStep::ModelList, None, draft);
            return Ok(());
        }
        self.show_edit_provider_step(provider_id, EditProviderStep::DisplayName, None, draft);
        Ok(())
    }

    fn show_edit_provider_step(
        &mut self,
        provider_id: String,
        step: EditProviderStep,
        model_step: Option<EditModelStep>,
        draft: EditProviderDraft,
    ) {
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        if let Some(model_step) = model_step {
            self.composer.set_placeholder(format!(
                "{} · {}",
                model_step.label(),
                model_step.help()
            ));
            self.composer
                .set_text(draft.model.current_value_for_edit(model_step));
        } else if step == EditProviderStep::ModelList {
            self.composer
                .set_placeholder("Enter to edit · n to add · d to delete · s to save");
        } else if step == EditProviderStep::ConfirmDeleteModel {
            self.composer.set_placeholder("y or n");
        } else {
            self.composer
                .set_placeholder(format!("{} · {}", step.label(), step.help()));
            self.composer.set_text(draft.current_value(step));
        }

        self.connect_dialog = Some(ConnectDialog::EditProvider {
            provider_id,
            step,
            model_step,
            draft,
        });
    }

    fn begin_new_model_in_edit(&mut self, provider_id: String, mut draft: EditProviderDraft) {
        draft.begin_new_model();
        self.show_edit_provider_step(
            provider_id,
            EditProviderStep::ModelList,
            Some(EditModelStep::ModelId),
            draft,
        );
    }

    fn begin_existing_model_edit(
        &mut self,
        provider_id: String,
        model_id: String,
        mut draft: EditProviderDraft,
    ) -> Result<()> {
        draft.begin_edit_model(&model_id)?;
        self.show_edit_provider_step(
            provider_id,
            EditProviderStep::ModelList,
            Some(EditModelStep::ModelDisplayName),
            draft,
        );
        Ok(())
    }

    fn finish_provider_edit(
        &mut self,
        provider_id: String,
        draft: EditProviderDraft,
    ) -> Result<()> {
        let (provider_config, api_key) = draft.into_provider_config()?;
        {
            let mut cfg = self.config.write().unwrap();
            cfg.providers.insert(provider_id.clone(), provider_config);
            cfg.save(&self.paths)?;
        }

        self.auth.set_api_key(provider_id.clone(), api_key);
        self.auth.save(&self.paths)?;

        self.refresh_active_model_after_provider_change(&provider_id)?;

        self.cancel_connect_dialog();
        self.last_notice = Some(format!("Updated provider '{provider_id}'"));
        Ok(())
    }

    fn refresh_active_model_after_provider_change(&mut self, provider_id: &str) -> Result<()> {
        if self.active_model.provider_id != provider_id {
            return Ok(());
        }

        let updated = self
            .config
            .read()
            .unwrap()
            .resolve_model_by_ids(
                &self.auth,
                &self.active_model.provider_id,
                &self.active_model.model_id,
            )
            .or_else(|_| {
                self.config
                    .read()
                    .unwrap()
                    .resolve_provider_default_model(&self.auth, provider_id)
            })?;

        self.active_model = updated.clone();
        self.conversation.set_model(
            updated.provider_id.clone(),
            updated.provider_display_name.clone(),
            updated.model_id.clone(),
            updated.display_name.clone(),
        );
        self.store.update_session_model(
            self.conversation.session_id,
            &updated.provider_id,
            &updated.provider_display_name,
            &updated.model_id,
            &updated.display_name,
        )?;

        Ok(())
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

    fn finish_new_provider_setup(&mut self, draft: NewProviderDraft) -> Result<()> {
        let provider_id = draft.provider_id.clone();

        if self.config.read().unwrap().provider_exists(&provider_id) {
            self.last_notice = Some(format!("Provider '{provider_id}' already exists"));
            self.show_new_provider_step(NewProviderStep::ProviderId, draft);
            return Ok(());
        }

        let (provider_id, provider_config, api_key) = draft.into_provider_config()?;

        {
            let mut cfg = self.config.write().unwrap();
            cfg.providers.insert(provider_id.clone(), provider_config);
            cfg.save(&self.paths)?;
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
        self.last_notice = Some(format!(
            "Created provider '{provider_id}' and connected to {}",
            model.provider_display_name
        ));
        Ok(())
    }

    fn begin_new_provider_setup(&mut self) {
        self.show_new_provider_step(NewProviderStep::ProviderId, NewProviderDraft::default());
    }

    fn show_new_provider_step(&mut self, step: NewProviderStep, draft: NewProviderDraft) {
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder(format!("{} · {}", step.label(), step.help()));
        self.composer.set_text(draft.current_value(step));
        self.connect_dialog = Some(ConnectDialog::NewProvider { step, draft });
    }

    fn cancel_connect_dialog(&mut self) {
        self.connect_dialog = None;
        self.at_mention.clear();
        self.draft_attachments.clear();
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
                KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    let items = self.provider_picker_items();
                    let selected = selected.min(items.len().saturating_sub(1));
                    if let Some(ProviderPickerItem::Provider {
                        provider_id,
                        source,
                        ..
                    }) = items.get(selected)
                    {
                        if matches!(source, ProviderSource::User) {
                            self.begin_provider_edit(provider_id.clone())?;
                        } else {
                            self.last_notice = Some(
                                "Bundled presets are read-only; clone them with the new provider flow".to_string(),
                            );
                        }
                    }
                }
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
                        && self
                            .config
                            .read()
                            .unwrap()
                            .providers
                            .contains_key(&draft.provider_id)
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

                    if step == NewProviderStep::ModelApiType {
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
            ConnectDialog::EditProvider {
                provider_id,
                step,
                model_step,
                mut draft,
            } => {
                if matches!(key.code, KeyCode::Esc) {
                    self.cancel_connect_dialog();
                    return Ok(());
                }

                if let Some(model_step) = model_step {
                    if let Some(submission) = self.composer.handle_key_with_history(key, false) {
                        if let Err(error) = draft.model.apply_edit_step(model_step, &submission) {
                            self.last_notice = Some(error.to_string());
                            self.show_edit_provider_step(
                                provider_id,
                                EditProviderStep::ModelList,
                                Some(model_step),
                                draft,
                            );
                            return Ok(());
                        }

                        if let Some(next_step) = model_step.next(draft.editing_model_id.is_some()) {
                            self.show_edit_provider_step(
                                provider_id,
                                EditProviderStep::ModelList,
                                Some(next_step),
                                draft,
                            );
                        } else {
                            if let Err(error) = draft.finish_current_model() {
                                self.last_notice = Some(error.to_string());
                                self.show_edit_provider_step(
                                    provider_id,
                                    EditProviderStep::ModelList,
                                    Some(EditModelStep::ModelId),
                                    draft,
                                );
                                return Ok(());
                            }

                            self.show_edit_provider_step(
                                provider_id,
                                EditProviderStep::ModelList,
                                None,
                                draft,
                            );
                        }
                    }

                    return Ok(());
                }

                match step {
                    EditProviderStep::DisplayName
                    | EditProviderStep::BaseUrl
                    | EditProviderStep::ApiKey => {
                        if let Some(submission) = self.composer.handle_key_with_history(key, false)
                        {
                            if let Err(error) = draft.apply_step(step, &submission) {
                                self.last_notice = Some(error.to_string());
                                self.show_edit_provider_step(provider_id, step, None, draft);
                                return Ok(());
                            }

                            if let Some(next_step) = step.next() {
                                self.show_edit_provider_step(provider_id, next_step, None, draft);
                            } else {
                                self.show_edit_provider_step(
                                    provider_id,
                                    EditProviderStep::ModelList,
                                    None,
                                    draft,
                                );
                            }
                        }
                    }
                    EditProviderStep::ModelList => match key {
                        KeyEvent {
                            code: KeyCode::Up, ..
                        } => {
                            draft.move_selection_up();
                            self.show_edit_provider_step(
                                provider_id,
                                EditProviderStep::ModelList,
                                None,
                                draft,
                            );
                        }
                        KeyEvent {
                            code: KeyCode::Down,
                            ..
                        } => {
                            draft.move_selection_down();
                            self.show_edit_provider_step(
                                provider_id,
                                EditProviderStep::ModelList,
                                None,
                                draft,
                            );
                        }
                        KeyEvent {
                            code: KeyCode::Enter,
                            modifiers,
                            ..
                        } if !modifiers.contains(KeyModifiers::SHIFT)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                        {
                            if let Some(model_id) = draft.selected_model_id() {
                                self.begin_existing_model_edit(provider_id, model_id, draft)?;
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Char('n'),
                            modifiers,
                            ..
                        } if !modifiers.contains(KeyModifiers::CONTROL)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                        {
                            self.begin_new_model_in_edit(provider_id, draft);
                        }
                        KeyEvent {
                            code: KeyCode::Char('d'),
                            modifiers,
                            ..
                        } if !modifiers.contains(KeyModifiers::CONTROL)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                        {
                            if let Err(error) = draft.request_delete_selected_model() {
                                self.last_notice = Some(error.to_string());
                                self.show_edit_provider_step(
                                    provider_id,
                                    EditProviderStep::ModelList,
                                    None,
                                    draft,
                                );
                            } else {
                                self.show_edit_provider_step(
                                    provider_id,
                                    EditProviderStep::ConfirmDeleteModel,
                                    None,
                                    draft,
                                );
                            }
                        }
                        KeyEvent {
                            code: KeyCode::Char('s'),
                            modifiers,
                            ..
                        } if !modifiers.contains(KeyModifiers::CONTROL)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                        {
                            self.finish_provider_edit(provider_id, draft)?;
                        }
                        _ => {}
                    },
                    EditProviderStep::ConfirmDeleteModel => {
                        if let Some(submission) = self.composer.handle_key_with_history(key, false)
                        {
                            match parse_add_another_model_answer(&submission) {
                                Some(true) => {
                                    if let Err(error) = draft.confirm_delete_selected_model() {
                                        self.last_notice = Some(error.to_string());
                                    }
                                    self.show_edit_provider_step(
                                        provider_id,
                                        EditProviderStep::ModelList,
                                        None,
                                        draft,
                                    );
                                }
                                Some(false) => {
                                    draft.pending_delete_model_id = None;
                                    self.show_edit_provider_step(
                                        provider_id,
                                        EditProviderStep::ModelList,
                                        None,
                                        draft,
                                    );
                                }
                                None => {
                                    self.last_notice = Some("Enter y or n".to_string());
                                    self.show_edit_provider_step(
                                        provider_id,
                                        EditProviderStep::ConfirmDeleteModel,
                                        None,
                                        draft,
                                    );
                                }
                            }
                        } else if matches!(key.code, KeyCode::Enter)
                            && !key.modifiers.contains(KeyModifiers::SHIFT)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            if let Err(error) = draft.confirm_delete_selected_model() {
                                self.last_notice = Some(error.to_string());
                            }
                            self.show_edit_provider_step(
                                provider_id,
                                EditProviderStep::ModelList,
                                None,
                                draft,
                            );
                        }
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
