use super::*;
use crate::render::chat_render::strip_system_reminder_tags;
use crate::theme::ThemeName;
use tidev_engine::agent::AgentType;

impl App {
    pub(crate) fn apply_theme_command(&mut self, args: &[String]) -> Result<()> {
        let direct_theme = args.first().and_then(|v| ThemeName::parse(v));

        if let Some(theme) = direct_theme {
            self.apply_theme(theme)?;
            Ok(())
        } else {
            self.open_theme_panel();
            Ok(())
        }
    }

    pub(crate) fn open_theme_panel(&mut self) {
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = Some(ThemePanelState::new(self.theme.palette().name));
    }

    pub(crate) fn open_settings_panel(&mut self) {
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = Some(SettingsPanelState::new(&self.config));
        self.sync_panel = None;
    }

    pub(crate) fn open_panel_launcher(&mut self) {
        // Close any open panels so the launcher is clean
        self.command_palette.clear();
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.settings_panel = None;
        self.message_panel = None;
        self.skills_panel = None;
        self.sync_panel = None;
        self.panel_launcher.open();
    }

    pub(crate) fn handle_panel_launcher_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.panel_launcher.clear();
            }
            KeyCode::Up => {
                self.panel_launcher.move_selection(-1);
            }
            KeyCode::Down => {
                self.panel_launcher.move_selection(1);
            }
            KeyCode::Enter => {
                if let Some(action) = self.panel_launcher.take_selected_action() {
                    self.execute_panel_action(action, runtime);
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.panel_launcher.query.push(c);
                self.panel_launcher.sync();
            }
            KeyCode::Backspace => {
                self.panel_launcher.query.pop();
                self.panel_launcher.sync();
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn execute_panel_action(&mut self, action: PanelAction, _runtime: &Runtime) {
        use crate::ui;
        match action {
            PanelAction::Model => {
                self.open_model_panel(String::new());
            }
            PanelAction::Session => {
                if let Err(e) = self.open_session_panel(String::new()) {
                    self.last_notice = Some(format!("{e}"));
                }
            }
            PanelAction::Theme => {
                self.open_theme_panel();
            }
            PanelAction::Settings => {
                self.open_settings_panel();
            }
            PanelAction::Mcp => {
                self.open_mcp_panel(String::new());
            }
            PanelAction::Agents => {
                self.agents_panel = Some(ui::agents_panel::AgentsPanelState::new());
            }
            PanelAction::Skills => {
                self.open_skills_panel();
            }
            PanelAction::Message => {
                if let Err(e) = self.open_message_panel(String::new()) {
                    self.last_notice = Some(format!("{e}"));
                }
            }
        }
    }

    pub(crate) fn open_model_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;

        let mut panel = ModelPanelState::new();
        panel.query.set_text(initial_query);

        // Build tabs: General first, then agent types
        let mut tabs = Vec::new();
        // General tab — main session model
        tabs.push(crate::model_panel::ModelPanelTab::new(
            "general",
            "General",
            &self.active_model.label(),
        ));
        // Agent tabs
        for agent_type in AgentType::all() {
            if *agent_type == AgentType::General {
                continue;
            }
            let ty = agent_type.display_name();
            let label = self.config.read().unwrap().agent_model_display(ty);
            tabs.push(crate::model_panel::ModelPanelTab::new(
                ty,
                agent_type.display_name(),
                &label,
            ));
        }
        panel.tabs = tabs;
        panel.selected_tab_index = 0;

        // Initialize selection for the general tab
        let items = self.model_panel_items(&panel);
        panel.reset_selection(
            &items,
            Some((&self.active_model.provider_id, &self.active_model.model_id)),
        );
        self.model_panel = Some(panel);
    }

    pub(crate) fn close_model_panel(&mut self) {
        self.model_panel = None;
    }

    pub(crate) fn open_search_panel(&mut self) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.settings_panel = None;
        self.session_panel = None;

        let provider = self
            .config
            .read()
            .unwrap()
            .websearch
            .default_provider
            .clone();
        self.search_panel = Some(ui::search_panel::SearchPanelState::new(&provider));
    }

    pub(crate) fn close_search_panel(&mut self) {
        self.search_panel = None;
    }

    /// Switch the active search provider and persist to config.
    pub(crate) fn switch_search_provider(&mut self, provider: &str) -> anyhow::Result<()> {
        // Update ToolRegistry so subsequent websearch calls use this provider
        self.tools.set_active_search_provider(provider);

        // Persist to config
        {
            let mut cfg = self.config.write().unwrap();
            cfg.websearch.default_provider = provider.to_string();
            cfg.save(&self.paths)?;
        }

        Ok(())
    }

    pub(crate) fn open_skills_panel(&mut self) {
        // Close other panels
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = None;

        // Build skill items from the catalog
        let skill_items: Vec<ui::skills_panel::SkillItem> = self
            .tools
            .skills()
            .all()
            .iter()
            .map(|skill| ui::skills_panel::SkillItem {
                name: skill.name.clone(),
                description: skill.description.clone(),
                location: skill.location.clone(),
            })
            .collect();

        self.skills_panel = Some(ui::skills_panel::SkillsPanelState::new(skill_items));
    }

    pub(crate) fn open_message_panel(&mut self, initial_query: String) -> Result<()> {
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search user messages in the current session");
        self.composer.set_text(initial_query);

        let messages = self
            .conversation
            .visible_messages()
            .iter()
            .filter(|message| matches!(message.role, MessageRole::User))
            .map(|message| crate::message_panel::MessagePanelMessage {
                message_id: message.id,
                content: strip_system_reminder_tags(&message.content),
                created_at: message.created_at,
                mode: message.mode,
            })
            .collect();

        self.message_panel = Some(MessagePanelState::new(messages));
        self.reset_message_panel_selection();
        Ok(())
    }

    pub(crate) fn close_message_panel(&mut self) {
        if self.message_panel.take().is_some() {
            self.composer.clear();
            self.composer
                .set_placeholder("Ask tidev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_message_panel_selection(&mut self) {
        if let Some(panel) = &mut self.message_panel {
            panel.reset_selection(self.composer.text());
        }
    }

    pub(crate) fn confirm_fork_session(&mut self, _runtime: &Runtime) -> Result<()> {
        let Some(dialog) = self.fork_confirm_dialog.take() else {
            return Ok(());
        };

        // 获取要复制的消息索引
        let message_index = match self.get_message_index(dialog.selected_message_id) {
            Some(idx) => idx,
            None => {
                self.last_notice = Some("Selected message not found".to_string());
                return Ok(());
            }
        };

        // 保存当前 session 状态
        self.cache_active_session_runtime();

        // 创建新 session（独立的，没有 parent）
        let new_session_id = Uuid::new_v4();

        let _record = self.store.create_session(
            new_session_id,
            self.workspace_root.as_path(),
            &self.active_model.provider_id,
            &self.active_model.provider_display_name,
            &self.active_model.model_id,
            &self.active_model.display_name,
            &format!("Fork of {}", self.conversation.title),
        )?;

        // Copy the parent's static system prompt so the fork shares the same prefix.
        if !self.active_model.system_prompt.is_empty() {
            let parent_prompt = self.active_model.system_prompt.clone();
            if let Err(e) = self
                .store
                .update_session_system_prompt(new_session_id, &parent_prompt)
            {
                log::warn!("failed to persist static system prompt for fork: {}", e);
            }
        }

        // 复制消息（从开头到选中的消息），为每条消息生成新的 ID
        let original_messages: Vec<_> = self.conversation.messages[..=message_index].to_vec();

        let mut id_mapping: std::collections::HashMap<Uuid, Uuid> =
            std::collections::HashMap::new();

        for original in &original_messages {
            let mut new_message = original.clone();
            let new_id = Uuid::new_v4();
            id_mapping.insert(original.id, new_id);
            new_message.id = new_id;

            // 更新 tool_call_id 引用（如果有）
            if let Some(ref tool_call_id) = new_message.tool_call_id
                && let Some(&new_tool_call_id) =
                    id_mapping.get(&Uuid::parse_str(tool_call_id).unwrap_or_else(|_| Uuid::nil()))
            {
                new_message.tool_call_id = Some(new_tool_call_id.to_string());
            }

            self.store.append_message(new_session_id, &new_message)?;
        }

        // 加载新 session
        let conversation = self
            .store
            .load_conversation(new_session_id)?
            .context("Failed to load forked conversation")?;
        self.conversation = conversation;
        *self.current_session_id.write().unwrap() = new_session_id;
        self.reset_active_runtime();

        // 关闭所有面板和状态
        self.message_panel = None;
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask tidev about your code, task, or question...");
        self.scroll_messages_to_bottom();

        self.last_notice = Some(format!(
            "Forked session with {} messages",
            original_messages.len()
        ));

        Ok(())
    }

    pub(crate) fn confirm_undo_to_message(&mut self, runtime: &Runtime) -> Result<()> {
        let Some(dialog) = self.undo_confirm_dialog.take() else {
            return Ok(());
        };

        // 查找选中的消息并克隆完整数据（含 attachments）
        let Some(msg) = self
            .conversation
            .messages
            .iter()
            .find(|m| m.id == dialog.selected_message_id)
            .cloned()
        else {
            self.last_notice = Some("Selected message not found".to_string());
            return Ok(());
        };

        // Save attachments so they can be restored after revert
        self.restored_attachments = msg.attachments.clone();

        // 关闭 message_panel
        self.close_message_panel();

        // 调用 revert_to_message 执行 undo
        self.revert_to_message(msg.id, msg.content.clone(), runtime)?;
        self.last_notice = Some("Undo complete".to_string());

        Ok(())
    }

    pub(crate) fn close_theme_panel(&mut self, apply: bool) -> Result<()> {
        if let Some(panel) = self.theme_panel.take() {
            if apply {
                self.apply_theme(panel.preview_theme)?;
            } else {
                self.theme.set_mode(panel.original_theme);
                self.clear_message_render_cache();
            }
        }
        Ok(())
    }

    pub(crate) fn close_settings_panel(&mut self, _apply: bool) -> Result<()> {
        if let Some(panel) = self.settings_panel.take() {
            let mut cfg = self.config.write().unwrap();
            panel.apply_to_config(&mut cfg);
            cfg.save(&self.paths)?;
        }
        Ok(())
    }

    pub(crate) fn apply_theme(&mut self, theme: ThemeName) -> Result<()> {
        self.theme.set_mode(theme);
        self.clear_message_render_cache();
        {
            let mut cfg = self.config.write().unwrap();
            cfg.set_theme(theme);
            cfg.save(&self.paths)?;
        }
        self.last_notice = Some(format!("Theme switched to {}", self.theme.name()));
        Ok(())
    }

    pub(crate) fn open_rename_session_dialog(&mut self) -> Result<()> {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();

        self.rename_dialog = Some(RenameSessionDialogState::new(
            self.conversation.title.clone(),
        ));
        self.composer.set_text(self.conversation.title.clone());
        self.composer
            .set_placeholder("Type the new session title and press Enter");
        self.last_notice = Some("Rename the current session title".to_string());
        Ok(())
    }

    pub(crate) fn close_rename_session_dialog(&mut self) {
        self.rename_dialog = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Ask tidev about your code, task, or question...");
    }

    pub(crate) fn confirm_rename_session(&mut self) -> Result<()> {
        let mut title = self.composer.text().trim().to_string();
        if title.is_empty() {
            title = "Untitled session".to_string();
        }

        self.conversation.title = title.clone();
        self.store
            .update_session_title(self.conversation.session_id, &title)?;
        self.last_notice = Some("Session title updated".to_string());
        self.close_rename_session_dialog();
        Ok(())
    }

    pub(crate) fn switch_model(&mut self, selector: Option<&str>) -> Result<()> {
        // Preserve the session's static system prompt — it was composed at
        // session creation and must never be recomposed mid-session.
        let saved_system_prompt = self.active_model.system_prompt.clone();
        let model = self
            .config
            .read()
            .unwrap()
            .resolve_model(&self.auth, selector)?;
        self.active_model = model.clone();
        self.active_model.system_prompt = saved_system_prompt;
        self.thinking_level = model.thinking_level.clone();
        // Load saved thinking level preference for this model (overrides auto-detected value)
        if let Ok(Some(level_str)) = self
            .store
            .load_model_thinking_level(&model.provider_id, &model.model_id)
        {
            self.thinking_level =
                tidev_engine::config::reasoning::ThinkingLevelType::from_string(&level_str);
        }
        self.tools.set_active_model(model.clone());
        // Sync the agent's ToolRegistry so per-turn tool filtering
        // (all_definitions → use_apply_patch) uses the correct model.
        self.agent.tools.set_active_model(model.clone());
        self.conversation.set_model(
            model.provider_id.clone(),
            model.provider_display_name.clone(),
            model.model_id.clone(),
            model.display_name.clone(),
        );
        self.store.update_session_model(
            self.conversation.session_id,
            &model.provider_id,
            &model.provider_display_name,
            &model.model_id,
            &model.display_name,
        )?;
        {
            let mut cfg = self.config.write().unwrap();
            cfg.default_provider = model.provider_id.clone();
            cfg.default_model = model.model_id.clone();
            cfg.save(&self.paths)?;
        }
        self.last_notice = Some(format!("Switched to {}", model.label()));
        Ok(())
    }

    pub(crate) fn start_new_session(&mut self) -> Result<()> {
        self.cache_active_session_runtime();

        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            self.active_model.provider_id.clone(),
            self.active_model.provider_display_name.clone(),
            self.active_model.model_id.clone(),
            self.active_model.display_name.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            self.workspace_root.as_path(),
            &self.active_model.provider_id,
            &self.active_model.provider_display_name,
            &self.active_model.model_id,
            &self.active_model.display_name,
            &conversation.title,
        )?;

        self.conversation = conversation;
        *self.current_session_id.write().unwrap() = session_id;
        self.reset_active_runtime();

        // Reset thinking_level to the default auto-detected value for the current model,
        // then apply any saved user preference.
        if let Ok(model) = self.config.read().unwrap().resolve_model_by_ids(
            &self.auth,
            &self.active_model.provider_id,
            &self.active_model.model_id,
        ) {
            self.active_model.thinking_level = model.thinking_level.clone();
            self.thinking_level = model.thinking_level;
        }
        if let Ok(Some(level_str)) = self
            .store
            .load_model_thinking_level(&self.active_model.provider_id, &self.active_model.model_id)
        {
            let level = tidev_engine::config::reasoning::ThinkingLevelType::from_string(&level_str);
            self.active_model.thinking_level = level.clone();
            self.thinking_level = level;
        }

        self.active_request_id = 0;
        self.screen = Screen::Welcome;
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.restored_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask tidev about your code, task, or question...");
        // ── Compose the static system prompt and persist it ──────────────
        // This prompt is frozen for the entire session lifetime. Never change it.
        let static_prompt = self
            .agent
            .compose_static_system_prompt(&self.active_model.system_prompt);
        self.active_model.system_prompt = static_prompt.clone();
        if let Err(e) = self
            .store
            .update_session_system_prompt(session_id, &static_prompt)
        {
            log::warn!("failed to persist static system prompt: {}", e);
        }

        self.scroll_messages_to_bottom();
        self.last_notice = Some("Started a fresh session".to_string());

        Ok(())
    }

    pub(crate) fn submit_prompt(&mut self, prompt: String, runtime: &Runtime) -> Result<()> {
        let prompt = prompt.trim().to_string();
        log::info!(
            "submit_prompt: ENTER prompt={:?}, draft_attachments={}",
            prompt,
            self.draft_attachments.len(),
        );
        if prompt.is_empty() && self.draft_attachments.is_empty() {
            log::info!("submit_prompt: empty prompt and no attachments, returning");
            return Ok(());
        }

        let (attachments, instruction_sources) = self.build_prompt_attachments(&prompt)?;
        log::info!(
            "submit_prompt: build_prompt_attachments returned {} attachments",
            attachments.len(),
        );
        if attachments.iter().any(MessageAttachment::is_image) && !self.active_model.supports_images
        {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        if self.pending_request {
            self.queue_prompt(prompt, attachments, instruction_sources);
            self.draft_attachments.clear();
            self.restored_attachments.clear();
            return Ok(());
        }

        self.submit_prompt_now(prompt, attachments, instruction_sources, runtime)
    }
}
