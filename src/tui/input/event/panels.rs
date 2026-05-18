use super::*;
use crate::tui::model_panel::{ModelPanelItem, thinking_options_for_model};

impl App {
    pub(crate) fn handle_sandbox_elevation_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.sandbox_elevation.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    // User approved: retry with full access
                    if let Some(dialog) = self.sandbox_elevation.take() {
                        if let Some(tx) = dialog.response_tx.lock().unwrap().take() {
                            let _ = tx.send(true);
                        }
                        self.tools.set_sandbox_policy(Some(
                            crate::sandbox::SandboxPolicy::DangerFullAccess,
                        ));
                        // Also sync to the agent's ToolRegistry (separate copy at init)
                        self.agent.tools.set_sandbox_policy(Some(
                            crate::sandbox::SandboxPolicy::DangerFullAccess,
                        ));
                        self.last_notice =
                            Some("Sandbox policy elevated to full access for retry".to_string());
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                    // User cancelled: pass the denial through
                    if let Some(dialog) = self.sandbox_elevation.take()
                        && let Some(tx) = dialog.response_tx.lock().unwrap().take()
                    {
                        let _ = tx.send(false);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_theme_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.theme_panel {
            match key.code {
                // Navigation
                KeyCode::Up | KeyCode::Char('k') => {
                    let previous_theme = panel.preview_theme;
                    panel.move_up();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                        self.clear_message_render_cache();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let previous_theme = panel.preview_theme;
                    panel.move_down();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                        self.clear_message_render_cache();
                    }
                }
                // Search: backspace removes char
                KeyCode::Backspace => {
                    panel.backspace_query();
                    self.theme.set_mode(panel.preview_theme);
                    self.clear_message_render_cache();
                }
                // Search: any printable char filters
                KeyCode::Char(ch) if !ch.is_control() => {
                    panel.append_query(ch);
                    self.theme.set_mode(panel.preview_theme);
                    self.clear_message_render_cache();
                }
                // Confirm
                KeyCode::Enter => {
                    let _ = self.close_theme_panel(true);
                }
                // Cancel
                KeyCode::Esc | KeyCode::Char('q') => {
                    let _ = self.close_theme_panel(false);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_agents_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.agents_panel {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.agents_panel = None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    panel.scroll_up(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    panel.scroll_down(1);
                }
                KeyCode::PageUp => {
                    panel.scroll_up(10);
                }
                KeyCode::PageDown => {
                    panel.scroll_down(10);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_skills_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        // Handle close actions first to avoid borrowing issues
        let should_close = matches!(key.code, KeyCode::Esc | KeyCode::Char('q'));
        if should_close {
            self.skills_panel = None;
            return Ok(());
        }

        if let Some(panel) = &mut self.skills_panel {
            // When query is active, handle text input first
            if panel.query_active {
                match key.code {
                    KeyCode::Esc => {
                        panel.query_active = false;
                    }
                    KeyCode::Enter => {
                        panel.query_active = false;
                    }
                    KeyCode::Backspace => {
                        panel.backspace_query();
                    }
                    KeyCode::Char(c) => {
                        panel.append_to_query(c);
                    }
                    _ => {}
                }
                return Ok(());
            }

            // Normal navigation mode
            match key.code {
                KeyCode::Char('/') | KeyCode::Char('s') => {
                    panel.query_active = true;
                }
                KeyCode::Char('c') => {
                    // Copy selected skill name to composer and close
                    if let Some(name) = panel.selected_skill_name() {
                        let name = name.to_string();
                        self.composer.set_text(format!("/skill {}", name));
                        self.skills_panel = None;
                        self.last_notice = Some(format!("Skill '{}' selected", name));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    panel.move_up(10);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    panel.move_down(10);
                }
                KeyCode::PageUp => {
                    panel.page_up(10);
                }
                KeyCode::PageDown => {
                    panel.page_down(10);
                }
                KeyCode::Home => {
                    panel.selected_index = 0;
                    panel.list_scroll = 0;
                }
                KeyCode::End if !panel.filtered_indices.is_empty() => {
                    panel.selected_index = panel.filtered_indices.len() - 1;
                }
                KeyCode::Left => {
                    panel.scroll_preview_up(5);
                }
                KeyCode::Right => {
                    panel.scroll_preview_down(5);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_settings_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.settings_panel {
            match key.code {
                KeyCode::Up => {
                    panel.move_up();
                }
                KeyCode::Down => {
                    panel.move_down();
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    panel.toggle_selected(self.config.rtk.installed);
                }
                KeyCode::Left => {
                    panel.decrease_selected();
                }
                KeyCode::Right => {
                    panel.increase_selected();
                }
                KeyCode::Esc => {
                    self.close_settings_panel(true)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_model_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut panel) = self.model_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                if panel.is_memory_tab()
                    && panel.memory_focus == crate::tui::model_panel::MemoryFocus::Sidebar
                {
                    let mut next_panel = panel;
                    next_panel.move_memory_sub_selection(-1);
                    let items = self.model_panel_items(&next_panel);
                    next_panel.reset_selection(&items, None);
                    self.model_panel = Some(next_panel);
                } else {
                    let items = self.model_panel_items(&panel);
                    let mut next_panel = panel;
                    next_panel.move_selection(&items, -1);
                    self.model_panel = Some(next_panel);
                }
            }
            KeyCode::Down => {
                if panel.is_memory_tab()
                    && panel.memory_focus == crate::tui::model_panel::MemoryFocus::Sidebar
                {
                    let mut next_panel = panel;
                    next_panel.move_memory_sub_selection(1);
                    let items = self.model_panel_items(&next_panel);
                    next_panel.reset_selection(&items, None);
                    self.model_panel = Some(next_panel);
                } else {
                    let items = self.model_panel_items(&panel);
                    let mut next_panel = panel;
                    next_panel.move_selection(&items, 1);
                    self.model_panel = Some(next_panel);
                }
            }
            KeyCode::Enter => {
                // In Memory tab sidebar: switch focus to the model list
                if panel.is_memory_tab()
                    && panel.memory_focus == crate::tui::model_panel::MemoryFocus::Sidebar
                {
                    let mut next_panel = panel;
                    next_panel.toggle_memory_focus();
                    let items = self.model_panel_items(&next_panel);
                    next_panel.reset_selection(&items, None);
                    self.model_panel = Some(next_panel);
                    return Ok(());
                }

                let items = self.model_panel_items(&panel);

                // Check if thinking level is currently expanded
                let is_expanded = panel
                    .current_tab()
                    .is_some_and(|t| t.thinking_level_expanded);

                if is_expanded {
                    // Confirm the thinking level selection
                    if let Some(summary) = panel.selected_model(&items).cloned() {
                        let tl_options = thinking_options_for_model(&items, items.iter().position(|item| {
                            matches!(item, ModelPanelItem::Model { summary: s, .. }
                                if s.provider_id == summary.provider_id && s.model_id == summary.model_id)
                        }).unwrap_or(0));
                        let tl_index = panel
                            .current_tab()
                            .map(|t| t.thinking_level_index)
                            .unwrap_or(0);
                        let tl = if tl_options.is_empty() {
                            String::new()
                        } else {
                            tl_options[tl_index % tl_options.len()].to_string()
                        };
                        let mut next_panel = panel;

                        if next_panel.is_general_tab() {
                            // Save thinking level preference and switch model
                            if !tl.is_empty() {
                                let _ = self.store.save_model_thinking_level(
                                    &summary.provider_id,
                                    &summary.model_id,
                                    &tl,
                                );
                            }
                            self.switch_model(Some(&summary.label()))?;
                            if let Some(t) = next_panel.current_tab_mut() {
                                t.current_label = summary.label();
                                t.thinking_level_expanded = false;
                            }
                        } else if next_panel.is_memory_tab() {
                            // Memory tab: save model + thinking level
                            let role = next_panel.active_memory_role();
                            let model_str = summary.label();
                            self.config.set_memory_model_and_thinking(
                                &self.paths,
                                role,
                                &model_str,
                                &tl,
                            )?;
                            if let Some(t) = next_panel.current_tab_mut() {
                                t.current_label = model_str.clone();
                                t.thinking_level_expanded = false;
                            }
                            self.last_notice = Some(format!(
                                "Memory {} model set to {} ({})",
                                role,
                                model_str,
                                if tl.is_empty() { "auto" } else { &tl },
                            ));
                        } else {
                            // Agent tab: save model + thinking level
                            let agent_type_str = next_panel
                                .current_tab()
                                .map(|t| t.agent_type_str.clone())
                                .unwrap_or_default();
                            let model_str = summary.label();
                            self.config.set_agent_model_and_thinking(
                                &self.paths,
                                &agent_type_str,
                                &model_str,
                                &tl,
                            )?;
                            if let Some(t) = next_panel.current_tab_mut() {
                                t.current_label = model_str.clone();
                                t.thinking_level_expanded = false;
                            }
                            self.last_notice = Some(format!(
                                "Agent '{}' model set to {} ({})",
                                agent_type_str,
                                model_str,
                                if tl.is_empty() { "auto" } else { &tl },
                            ));
                        }
                        self.model_panel = Some(next_panel);
                    }
                } else {
                    // Expand to show thinking level options
                    if let Some(summary) = panel.selected_model(&items).cloned() {
                        let tl_options = thinking_options_for_model(
                            &items,
                            panel.current_tab().map(|t| t.selected_index).unwrap_or(0),
                        );
                        if tl_options.is_empty() {
                            // Model doesn't support thinking: act as before (immediate apply)
                            if panel.is_general_tab() {
                                self.switch_model(Some(&summary.label()))?;
                                let mut next_panel = panel;
                                if let Some(t) = next_panel.current_tab_mut() {
                                    t.current_label = summary.label();
                                }
                                self.model_panel = Some(next_panel);
                            } else if panel.is_memory_tab() {
                                let role = panel.active_memory_role();
                                let model_str = summary.label();
                                self.config
                                    .set_memory_model(&self.paths, role, &model_str)?;
                                let mut next_panel = panel;
                                if let Some(t) = next_panel.current_tab_mut() {
                                    t.current_label = model_str.clone();
                                }
                                self.model_panel = Some(next_panel);
                                self.last_notice =
                                    Some(format!("Memory {} model set to {}", role, model_str));
                            } else {
                                let agent_type_str = panel
                                    .current_tab()
                                    .map(|t| t.agent_type_str.clone())
                                    .unwrap_or_default();
                                let model_str = summary.label();
                                self.config.set_agent_model(
                                    &self.paths,
                                    &agent_type_str,
                                    &model_str,
                                )?;
                                let mut next_panel = panel;
                                if let Some(t) = next_panel.current_tab_mut() {
                                    t.current_label = model_str.clone();
                                }
                                self.model_panel = Some(next_panel);
                                self.last_notice = Some(format!(
                                    "Agent '{}' model set to {}",
                                    agent_type_str, model_str
                                ));
                            }
                        } else {
                            // Expand to show thinking level options
                            let mut next_panel = panel;
                            if let Some(t) = next_panel.current_tab_mut() {
                                t.thinking_level_expanded = true;
                                // Calculate the index matching the current thinking level
                                let tl_options =
                                    thinking_options_for_model(&items, t.selected_index);
                                let current_tl = self.thinking_level.to_string();
                                t.thinking_level_index = tl_options
                                    .iter()
                                    .position(|opt| opt.to_ascii_lowercase() == current_tl)
                                    .unwrap_or(0);
                            }
                            self.model_panel = Some(next_panel);
                        }
                    }
                }
            }
            KeyCode::Esc => {
                // If thinking level is expanded, collapse first; only close on second Esc
                let is_expanded = panel
                    .current_tab()
                    .is_some_and(|t| t.thinking_level_expanded);
                if is_expanded {
                    let mut next_panel = panel;
                    if let Some(t) = next_panel.current_tab_mut() {
                        t.thinking_level_expanded = false;
                    }
                    self.model_panel = Some(next_panel);
                } else {
                    self.close_model_panel();
                }
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let items = self.model_panel_items(&panel);
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.close_model_panel();
                    self.begin_provider_edit_for_model(summary.provider_id, summary.model_id)?;
                }
            }
            KeyCode::Left if panel.is_memory_tab() => {
                let mut next_panel = panel;
                next_panel.toggle_memory_focus();
                self.model_panel = Some(next_panel);
            }
            KeyCode::Right if panel.is_memory_tab() => {
                let mut next_panel = panel;
                next_panel.toggle_memory_focus();
                self.model_panel = Some(next_panel);
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                let mut next_panel = panel;
                next_panel.next_tab();
                let items = self.model_panel_items(&next_panel);
                let is_general = next_panel.is_general_tab();
                if is_general {
                    // General tab: use self.active_model directly (authoritative source)
                    next_panel.reset_selection(
                        &items,
                        Some((&self.active_model.provider_id, &self.active_model.model_id)),
                    );
                } else if next_panel.is_memory_tab() {
                    next_panel.reset_selection(&items, None);
                } else {
                    let active = agent_tab_active_model(&next_panel, &self.active_model);
                    if let Some((p, m)) = active {
                        next_panel.reset_selection(&items, Some((&p, &m)));
                    } else {
                        next_panel.reset_selection(&items, None);
                    }
                }
                self.model_panel = Some(next_panel);
            }
            KeyCode::BackTab | KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                let mut next_panel = panel;
                next_panel.prev_tab();
                let items = self.model_panel_items(&next_panel);
                let is_general = next_panel.is_general_tab();
                if is_general {
                    next_panel.reset_selection(
                        &items,
                        Some((&self.active_model.provider_id, &self.active_model.model_id)),
                    );
                } else if next_panel.is_memory_tab() {
                    next_panel.reset_selection(&items, None);
                } else {
                    let active = agent_tab_active_model(&next_panel, &self.active_model);
                    if let Some((p, m)) = active {
                        next_panel.reset_selection(&items, Some((&p, &m)));
                    } else {
                        next_panel.reset_selection(&items, None);
                    }
                }
                self.model_panel = Some(next_panel);
            }
            _ => {
                let previous_query = panel.query.text().to_string();
                let _ = panel.query.handle_key_with_history(key, false);
                if panel.query.text() != previous_query {
                    let items = self.model_panel_items(&panel);
                    let mut next_panel = panel;
                    // On query change, reset the current tab's selection to its configured model
                    if next_panel.is_general_tab() {
                        next_panel.reset_selection(
                            &items,
                            Some((&self.active_model.provider_id, &self.active_model.model_id)),
                        );
                    } else {
                        let active = agent_tab_active_model(&next_panel, &self.active_model);
                        if let Some((p, m)) = active {
                            next_panel.reset_selection(&items, Some((&p, &m)));
                        } else {
                            next_panel.reset_selection(&items, None);
                        }
                    }
                    self.model_panel = Some(next_panel);
                } else {
                    self.model_panel = Some(panel);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn handle_search_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut panel) = self.search_panel.clone() else {
            return Ok(());
        };

        // If editing an API key, route input to the buffer
        if panel.editing_api_key.is_some() {
            match key.code {
                KeyCode::Enter => {
                    // Save the API key
                    let input = panel.input_buffer.text().trim().to_string();
                    if !input.is_empty() {
                        let provider = panel.editing_api_key.clone().unwrap_or_default();
                        if panel.editing_cx {
                            self.auth.web.google_cx = Some(input);
                        } else {
                            self.auth
                                .web
                                .search_api_keys
                                .insert(provider.clone(), input);
                        }
                        self.auth.save(&self.paths)?;
                    }
                    // Clear editing state
                    panel.editing_api_key = None;
                    panel.editing_cx = false;
                    self.search_panel = Some(panel);
                }
                KeyCode::Esc => {
                    panel.editing_api_key = None;
                    panel.editing_cx = false;
                    self.search_panel = Some(panel);
                }
                KeyCode::Char(c) => {
                    panel
                        .input_buffer
                        .handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
                    self.search_panel = Some(panel);
                }
                KeyCode::Backspace => {
                    panel
                        .input_buffer
                        .handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
                    self.search_panel = Some(panel);
                }
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Up => {
                panel.move_selection(-1);
                self.search_panel = Some(panel);
            }
            KeyCode::Down => {
                panel.move_selection(1);
                self.search_panel = Some(panel);
            }
            KeyCode::Enter => {
                let auth = &self.auth;

                // Case 1: provider needs API key but none set → enter key edit mode
                if panel.selected_provider_missing_key(auth) {
                    panel.start_editing_api_key();
                    self.search_panel = Some(panel);
                    return Ok(());
                }

                // Case 2: provider needs Google CX but none set → enter cx edit mode
                if panel.selected_provider_missing_cx(auth) {
                    panel.start_editing_cx();
                    self.search_panel = Some(panel);
                    return Ok(());
                }

                // Case 3: switch to this provider
                if let Some(info) = panel
                    .selected_index
                    .checked_sub(0)
                    .and_then(|i| ui::search_panel::BUILTIN_PROVIDERS.get(i))
                {
                    self.switch_search_provider(info.id)?;
                    panel.active_provider = info.id.to_string();
                    self.search_panel = Some(panel);
                }
            }
            KeyCode::Esc => {
                if panel.editing_api_key.is_some() {
                    panel.editing_api_key = None;
                    panel.editing_cx = false;
                    self.search_panel = Some(panel);
                } else {
                    self.close_search_panel();
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub(crate) fn handle_model_panel_paste(&mut self, text: &str) -> Result<()> {
        let Some(mut panel) = self.model_panel.clone() else {
            return Ok(());
        };

        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let previous_query = panel.query.text().to_string();
        panel.query.insert_str(&normalized);

        if panel.query.text() != previous_query {
            let items = self.model_panel_items(&panel);
            let active = agent_tab_active_model(&panel, &self.active_model);
            if let Some((p, m)) = active {
                panel.reset_selection(&items, Some((&p, &m)));
            } else {
                panel.reset_selection(&items, None);
            }
        }

        self.model_panel = Some(panel);
        Ok(())
    }

    pub(crate) fn handle_message_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(panel) = self.message_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.message_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.message_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let query = self.composer.text().to_string();
                if let Some(message) = panel.selected_message(&query) {
                    self.scroll_messages_to_message(message.message_id);
                    self.close_message_panel();
                }
            }
            KeyCode::Esc => {
                self.close_message_panel();
            }
            KeyCode::Char('f') => {
                let query = self.composer.text().to_string();
                if let Some(message) = panel.selected_message(&query) {
                    // 计算要复制的消息数量
                    let message_count = self
                        .get_message_index(message.message_id)
                        .map(|idx| idx + 1)
                        .unwrap_or(1);

                    self.fork_confirm_dialog =
                        Some(crate::tui::ui::fork_confirm::ForkConfirmDialogState::new(
                            message.message_id,
                            message_count,
                        ));
                }
            }
            KeyCode::Char('u') => {
                let query = self.composer.text().to_string();
                if let Some(message) = panel.selected_message(&query) {
                    self.undo_confirm_dialog =
                        Some(crate::tui::ui::undo_confirm::UndoConfirmDialogState::new(
                            message.message_id,
                            message.content.clone(),
                        ));
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.message_panel = Some(next_panel);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.message_panel = Some(next_panel);
            }
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    self.reset_message_panel_selection();
                }
            }
        }

        Ok(())
    }

    /// 获取消息在 conversation.messages 中的索引
    pub(crate) fn handle_fork_confirm_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                self.confirm_fork_session(runtime)?;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.fork_confirm_dialog = None;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_undo_confirm_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                self.confirm_undo_to_message(runtime)?;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.undo_confirm_dialog = None;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_rename_session_dialog_key(
        &mut self,
        key: KeyEvent,
        _runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.close_rename_session_dialog();
                Ok(())
            }
            KeyCode::Enter
                if !key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.confirm_rename_session()
            }
            _ => {
                let _ = self.composer.handle_key_with_history(key, false);
                Ok(())
            }
        }
    }

    pub(crate) fn handle_stats_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.active = false;
                }
            }
            KeyCode::Tab => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.next_chart();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::BackTab => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.prev_chart();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.prev_granularity();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.next_granularity();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('1') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Hour;
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('2') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Day;
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('3') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Week;
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('4') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Month;
                    self.refresh_stats_panel();
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_balance_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Ok(mut guard) = self.balance_panel.lock()
                    && let Some(panel) = &mut *guard
                {
                    panel.close();
                }
            }
            KeyCode::Char('r') => {
                self.refresh_balance_panel(runtime);
            }
            KeyCode::Tab => {
                if let Ok(mut guard) = self.balance_panel.lock()
                    && let Some(panel) = &mut *guard
                {
                    panel.next_provider();
                    // Query balance for the new provider
                    let provider_id = match panel.selected_provider {
                        crate::tui::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
                        crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
                    };
                    if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
                        panel.set_loading(true);
                        let http = self.http_client.clone();
                        let panel_ptr = self.balance_panel.clone();
                        let panel_ptr_clone = panel_ptr.clone();
                        let api_key_clone = api_key.clone();
                        let selected_provider = panel.selected_provider;

                        runtime.spawn(async move {
                            match selected_provider {
                                crate::tui::ui::balance_panel::ProviderTab::DeepSeek => {
                                    match crate::balance::query_deepseek_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                                crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => {
                                    match crate::balance::query_siliconflow_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_siliconflow_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    } else {
                        let error_msg = match panel.selected_provider {
                            crate::tui::ui::balance_panel::ProviderTab::DeepSeek => {
                                "DeepSeek API key not configured"
                            }
                            crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => {
                                "SiliconFlow API key not configured"
                            }
                        };
                        panel.set_error(error_msg.to_string());
                    }
                }
            }
            KeyCode::BackTab => {
                if let Ok(mut guard) = self.balance_panel.lock()
                    && let Some(panel) = &mut *guard
                {
                    panel.prev_provider();
                    // Query balance for the new provider
                    let provider_id = match panel.selected_provider {
                        crate::tui::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
                        crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
                    };
                    if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
                        panel.set_loading(true);
                        let http = self.http_client.clone();
                        let panel_ptr = self.balance_panel.clone();
                        let panel_ptr_clone = panel_ptr.clone();
                        let api_key_clone = api_key.clone();
                        let selected_provider = panel.selected_provider;

                        runtime.spawn(async move {
                            match selected_provider {
                                crate::tui::ui::balance_panel::ProviderTab::DeepSeek => {
                                    match crate::balance::query_deepseek_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                                crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => {
                                    match crate::balance::query_siliconflow_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_siliconflow_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    } else {
                        let error_msg = match panel.selected_provider {
                            crate::tui::ui::balance_panel::ProviderTab::DeepSeek => {
                                "DeepSeek API key not configured"
                            }
                            crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => {
                                "SiliconFlow API key not configured"
                            }
                        };
                        panel.set_error(error_msg.to_string());
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_sandbox_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.sandbox_panel.is_some() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(ref mut panel) = self.sandbox_panel {
                        panel.move_selection(-1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(ref mut panel) = self.sandbox_panel {
                        panel.move_selection(1);
                    }
                }
                KeyCode::Enter => {
                    self.apply_sandbox_policy();
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.sandbox_panel = None;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

pub(super) fn agent_tab_active_model(
    panel: &crate::tui::model_panel::ModelPanelState,
    default: &crate::config::ActiveModel,
) -> Option<(String, String)> {
    let tab = panel.current_tab()?;
    let label = &tab.current_label;
    if label == "<inherit>" || label.is_empty() {
        Some((default.provider_id.clone(), default.model_id.clone()))
    } else if let Some(slash_pos) = label.find('/') {
        let p = &label[..slash_pos];
        let m = &label[slash_pos + 1..];
        Some((p.to_string(), m.to_string()))
    } else {
        None
    }
}
