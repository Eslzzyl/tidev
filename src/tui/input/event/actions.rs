use super::*;
use crate::agent::AgentType;

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

    pub(crate) fn open_memory_panel(&mut self) -> Result<()> {
        self.command_palette.clear();
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = None;
        let mut panel = MemoryPanelState::new();
        panel.load(
            &self.memory_store,
            &self.workspace_root.display().to_string(),
        )?;
        self.memory_panel = Some(panel);
        Ok(())
    }

    pub(crate) fn open_settings_panel(&mut self) {
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = Some(SettingsPanelState::new(&self.config));
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
        self.memory_panel = None;
        self.message_panel = None;
        self.skills_panel = None;
        self.sandbox_panel = None;
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
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
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

    pub(crate) fn execute_panel_action(&mut self, action: PanelAction, runtime: &Runtime) {
        use crate::tui::ui;
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
            PanelAction::Memory => {
                if let Err(e) = self.open_memory_panel() {
                    self.last_notice = Some(format!("{e}"));
                }
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
            PanelAction::Sandbox => {
                self.open_sandbox_panel();
            }
            PanelAction::Stats => {
                self.toggle_stats_panel();
            }
            PanelAction::Balance => {
                if let Err(e) = self.open_balance_panel(runtime) {
                    self.last_notice = Some(format!("{e}"));
                }
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

        // Build tabs: General first, then agent types, then Memory
        let mut tabs = Vec::new();
        // General tab — main session model
        tabs.push(crate::tui::model_panel::ModelPanelTab::new(
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
            let label = self.config.agent_model_display(ty);
            tabs.push(crate::tui::model_panel::ModelPanelTab::new(
                ty,
                agent_type.display_name(),
                &label,
            ));
        }
        // Memory tab — compression / summarization / embedding models
        {
            let display = self.config.memory_model_display("compression");
            tabs.push(crate::tui::model_panel::ModelPanelTab::new(
                "memory",
                "Memory",
                &display,
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
        self.memory_panel = None;

        self.search_panel = Some(ui::search_panel::SearchPanelState::new(
            &self.config.websearch.default_provider,
        ));
    }

    pub(crate) fn close_search_panel(&mut self) {
        self.search_panel = None;
    }

    /// Switch the active search provider and persist to config.
    pub(crate) fn switch_search_provider(&mut self, provider: &str) -> anyhow::Result<()> {
        // Update ToolRegistry so subsequent websearch calls use this provider
        self.tools.set_active_search_provider(provider);

        // Persist to config
        self.config.websearch.default_provider = provider.to_string();
        self.config.save(&self.paths)?;

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
        self.memory_panel = None;

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
            .map(|message| crate::tui::message_panel::MessagePanelMessage {
                message_id: message.id,
                content: message.content.clone(),
                created_at: message.created_at,
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
                .set_placeholder("Ask TiDev about your code, task, or question...");
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
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
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

        // 查找选中的消息并克隆所需数据
        let message_data = self
            .conversation
            .messages
            .iter()
            .find(|m| m.id == dialog.selected_message_id)
            .map(|m| (m.id, m.content.clone()));

        let Some((message_id, message_content)) = message_data else {
            self.last_notice = Some("Selected message not found".to_string());
            return Ok(());
        };

        // 关闭 message_panel
        self.close_message_panel();

        // 调用 revert_to_message 执行 undo
        self.revert_to_message(message_id, message_content, runtime)?;
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
            panel.apply_to_config(&mut self.config);
            self.config.save(&self.paths)?;
        }
        Ok(())
    }

    pub(crate) fn apply_theme(&mut self, theme: ThemeName) -> Result<()> {
        self.theme.set_mode(theme);
        self.clear_message_render_cache();
        self.config.set_theme(theme);
        self.config.save(&self.paths)?;
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
            .set_placeholder("Ask TiDev about your code, task, or question...");
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
        let model = self.config.resolve_model(&self.auth, selector)?;
        self.active_model = model.clone();
        self.thinking_level = model.thinking_level.clone();
        // Load saved thinking level preference for this model (overrides auto-detected value)
        if let Ok(Some(level_str)) = self
            .store
            .load_model_thinking_level(&model.provider_id, &model.model_id)
        {
            self.thinking_level =
                crate::config::reasoning::ThinkingLevelType::from_string(&level_str);
        }
        self.tools.set_active_model(model.clone());
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
        self.config.default_provider = model.provider_id.clone();
        self.config.default_model = model.model_id.clone();
        self.config.save(&self.paths)?;
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
        if let Ok(model) = self.config.resolve_model_by_ids(
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
            let level = crate::config::reasoning::ThinkingLevelType::from_string(&level_str);
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
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.scroll_messages_to_bottom();
        self.last_notice = Some("Started a fresh session".to_string());

        Ok(())
    }

    pub(crate) fn submit_prompt(&mut self, prompt: String, runtime: &Runtime) -> Result<()> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() && self.draft_attachments.is_empty() {
            return Ok(());
        }

        let attachments = self.build_prompt_attachments(&prompt)?;
        if attachments.iter().any(MessageAttachment::is_image) && !self.active_model.supports_images
        {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        if self.pending_request || !self.pending_prompt_queue.is_empty() {
            self.queue_prompt(prompt, attachments);
            self.draft_attachments.clear();
            return Ok(());
        }

        self.submit_prompt_now(prompt, attachments, runtime)
    }

    pub(crate) fn toggle_stats_panel(&mut self) {
        if let Some(panel) = &mut self.stats_panel {
            panel.toggle();
            if panel.active {
                self.refresh_stats_panel();
            }
        } else {
            let mut panel = crate::tui::ui::stats_panel::StatsPanelState::new();
            panel.active = true;
            self.stats_panel = Some(panel);
            self.refresh_stats_panel();
        }
    }

    pub(crate) fn refresh_stats_panel(&mut self) {
        if let Some(panel) = &mut self.stats_panel
            && panel.needs_refresh()
        {
            let (start, end) = panel.granularity.default_range();
            match self
                .store
                .get_time_range_stats(panel.granularity, start, end)
            {
                Ok(stats) => {
                    panel.cached_stats = Some(stats);
                    panel.last_refresh = Some(chrono::Utc::now());
                }
                Err(e) => {
                    crate::log_error!("Failed to refresh stats: {}", e);
                }
            }
        }
    }

    pub(crate) fn open_balance_panel(&mut self, runtime: &Runtime) -> Result<()> {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.mcp_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = None;
        self.agents_panel = None;

        let mut panel = crate::tui::ui::balance_panel::BalancePanelState::new();
        let selected_provider = panel.selected_provider;
        panel.open();
        *self.balance_panel.lock().unwrap() = Some(panel);

        // Query balance based on selected provider
        let provider_id = match selected_provider {
            crate::tui::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
            crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
        };

        if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
            // Set loading state
            if let Ok(mut guard) = self.balance_panel.lock()
                && let Some(panel) = &mut *guard
            {
                panel.set_loading(true);
            }

            let http = self.http_client.clone();
            let panel_ptr = self.balance_panel.clone();
            let panel_ptr_clone = panel_ptr.clone();
            let api_key_clone = api_key.clone();

            runtime.spawn(async move {
                match selected_provider {
                    crate::tui::ui::balance_panel::ProviderTab::DeepSeek => {
                        match crate::balance::query_deepseek_balance(&http, &api_key_clone).await {
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
                        match crate::balance::query_siliconflow_balance(&http, &api_key_clone).await
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
            // Set error state
            let error_msg = match selected_provider {
                crate::tui::ui::balance_panel::ProviderTab::DeepSeek => {
                    "DeepSeek API key not configured"
                }
                crate::tui::ui::balance_panel::ProviderTab::SiliconFlow => {
                    "SiliconFlow API key not configured"
                }
            };
            if let Ok(mut guard) = self.balance_panel.lock()
                && let Some(panel) = &mut *guard
            {
                panel.set_error(error_msg.to_string());
            }
        }

        Ok(())
    }

    pub(crate) fn refresh_balance_panel(&mut self, runtime: &Runtime) {
        let mut guard = match self.balance_panel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let panel = match &mut *guard {
            Some(panel) => panel,
            None => return,
        };

        if !panel.active || panel.loading {
            return;
        }

        // Determine provider based on selected tab
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
                        match crate::balance::query_deepseek_balance(&http, &api_key_clone).await {
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
                        match crate::balance::query_siliconflow_balance(&http, &api_key_clone).await
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
