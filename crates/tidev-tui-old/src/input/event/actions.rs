use super::*;
use crate::render::chat_render::strip_system_reminder_tags;
use crate::theme::ThemeName;
use tidev_types::agent_type::AgentType;
use tidev_types::message::MessageAttachment;
use tidev_types::message::MessageRole;

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
        self.ui.agents_panel = None;
        self.ui.theme_panel = Some(ThemePanelState::new(self.ui.theme.palette().name));
    }

    pub(crate) fn open_settings_panel(&mut self) {
        self.ui.agents_panel = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;

        let cfg = self.runtime.config();
        self.ui.settings_panel = Some(SettingsPanelState::new(&cfg));
    }

    pub(crate) fn open_panel_launcher(&mut self) {
        // Close any open panels so the launcher is clean
        self.ui.command_palette.clear();
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;
        self.ui.agents_panel = None;
        self.ui.settings_panel = None;
        self.ui.message_panel = None;
        self.ui.skills_panel = None;
        self.ui.panel_launcher.open();
    }

    pub(crate) fn handle_panel_launcher_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.ui.panel_launcher.clear();
            }
            KeyCode::Up => {
                self.ui.panel_launcher.move_selection(-1);
            }
            KeyCode::Down => {
                self.ui.panel_launcher.move_selection(1);
            }
            KeyCode::Enter => {
                if let Some(action) = self.ui.panel_launcher.take_selected_action() {
                    self.execute_panel_action(action);
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.ui.panel_launcher.query.push(c);
                self.ui.panel_launcher.sync();
            }
            KeyCode::Backspace => {
                self.ui.panel_launcher.query.pop();
                self.ui.panel_launcher.sync();
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn execute_panel_action(&mut self, action: PanelAction) {
        use crate::ui;
        match action {
            PanelAction::Model => {
                self.open_model_panel(String::new());
            }
            PanelAction::Session => {
                if let Err(e) = self.open_session_panel(String::new()) {
                    self.ui.last_notice = Some(format!("{e}"));
                }
            }
            PanelAction::Theme => {
                self.open_theme_panel();
            }
            PanelAction::Settings => {
                self.open_settings_panel();
            }
            PanelAction::Agents => {
                self.ui.agents_panel = Some(ui::agents_panel::AgentsPanelState::new());
            }
            PanelAction::Skills => {
                self.open_skills_panel();
            }
            PanelAction::Message => {
                if let Err(e) = self.open_message_panel(String::new()) {
                    self.ui.last_notice = Some(format!("{e}"));
                }
            }
        }
    }

    pub(crate) fn open_model_panel(&mut self, initial_query: String) {
        self.ui.command_palette.clear();
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.agents_panel = None;

        // Resolve the active model from config.
        let config = self.runtime.config();
        let auth = self.runtime.auth();
        let active_model = match config.resolve_active_model(&auth) {
            Ok(m) => m,
            Err(e) => {
                self.ui.last_notice = Some(format!("Failed to resolve active model: {e}"));
                return;
            }
        };

        let mut panel = ModelPanelState::new();
        panel.query.set_text(initial_query);

        // Build tabs: General first, then agent types
        let mut tabs = Vec::new();
        // General tab — main session model
        tabs.push(crate::model_panel::ModelPanelTab::new(
            "general",
            "General",
            &active_model.label(),
        ));
        // Agent tabs
        for agent_type in AgentType::all() {
            if *agent_type == AgentType::General {
                continue;
            }
            let ty = agent_type.display_name();
            let label = config.agent_model_display(ty);
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
            Some((&active_model.provider_id, &active_model.model_id)),
        );
        self.ui.model_panel = Some(panel);
    }

    pub(crate) fn close_model_panel(&mut self) {
        self.ui.model_panel = None;
    }

    pub(crate) fn open_search_panel(&mut self) {
        self.ui.command_palette.clear();
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.agents_panel = None;
        self.ui.settings_panel = None;
        self.ui.session_panel = None;

        let config = self.runtime.config();
        let provider = config.websearch.default_provider.clone();
        self.ui.search_panel = Some(ui::search_panel::SearchPanelState::new(&provider));
    }

    pub(crate) fn close_search_panel(&mut self) {
        self.ui.search_panel = None;
    }

    /// Switch the active search provider and persist to config.
    pub(crate) fn switch_search_provider(&mut self, provider: &str) -> anyhow::Result<()> {
        // Persist to config
        self.runtime.update_config(|cfg| {
            cfg.websearch.default_provider = provider.to_string();
        });
        self.runtime.save_config()?;

        Ok(())
    }

    pub(crate) fn open_skills_panel(&mut self) {
        // Close other panels
        self.ui.agents_panel = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;
        self.ui.settings_panel = None;

        // Build skill items from the catalog
        let skill_items: Vec<ui::skills_panel::SkillItem> = self
            .runtime
            .skills
            .all()
            .iter()
            .map(|skill| ui::skills_panel::SkillItem {
                name: skill.name.clone(),
                description: skill.description.clone(),
                location: skill.location.clone(),
            })
            .collect();

        self.ui.skills_panel = Some(ui::skills_panel::SkillsPanelState::new(skill_items));
    }

    pub(crate) fn open_message_panel(&mut self, initial_query: String) -> Result<()> {
        self.ui.command_palette.clear();
        self.ui.at_mention.clear();
        self.ui.draft_attachments.clear();
        self.ui.restored_attachments.clear();
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;
        self.ui.agents_panel = None;
        self.ui.composer.clear();
        self.ui
            .composer
            .set_placeholder("Search user messages in the current session");
        self.ui.composer.set_text(initial_query);

        let messages = self
            .ui
            .chat_context
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

        self.ui.message_panel = Some(MessagePanelState::new(messages));
        self.reset_message_panel_selection();
        Ok(())
    }

    pub(crate) fn close_message_panel(&mut self) {
        if self.ui.message_panel.take().is_some() {
            self.ui.composer.clear();
            self.ui
                .composer
                .set_placeholder("Ask tidev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_message_panel_selection(&mut self) {
        if let Some(panel) = &mut self.ui.message_panel {
            panel.reset_selection(self.ui.composer.text());
        }
    }

    pub(crate) fn confirm_fork_session(&mut self) -> Result<()> {
        let Some(dialog) = self.ui.fork_confirm_dialog.take() else {
            return Ok(());
        };

        // 获取要复制的消息索引
        let message_index = match self.get_message_index(dialog.selected_message_id) {
            Some(idx) => idx,
            None => {
                self.ui.last_notice = Some("Selected message not found".to_string());
                return Ok(());
            }
        };

        // 保存当前 session 状态
        self.cache_active_session_runtime();

        // 创建新 session（独立的，没有 parent）
        let new_session_id = Uuid::new_v4();

        let workspace_root = self.runtime.workspace_root().to_string_lossy().to_string();
        let config = self.runtime.config();
        let auth = self.runtime.auth();
        let active_model = config.resolve_active_model(&auth)?;

        self.runtime.session_manager().create_session(
            new_session_id,
            &workspace_root,
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            &format!("Fork of {}", self.ui.chat_context.title),
        )?;

        // Copy the parent's static system prompt so the fork shares the same prefix.
        if !active_model.system_prompt.is_empty() {
            let parent_prompt = active_model.system_prompt.clone();
            if let Err(e) = self.runtime.session_manager().store().update_session(
                new_session_id,
                None,
                None,
                None,
                None,
                Some(&parent_prompt),
                None,
                None,
                None,
                None,
            ) {
                log::warn!("failed to persist static system prompt for fork: {}", e);
            }
        }

        // 复制消息（从开头到选中的消息），为每条消息生成新的 ID
        let original_messages: Vec<_> = self.ui.chat_context.messages[..=message_index].to_vec();

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

            self.runtime
                .session_manager()
                .append_message(new_session_id, &new_message)?;
        }

        // 加载新 session 的消息并更新 chat_context
        let messages = self
            .runtime
            .session_manager()
            .load_messages(new_session_id)?;
        self.ui.chat_context = ChatContext::new(
            new_session_id,
            format!("Fork of {}", self.ui.chat_context.title),
            workspace_root,
            messages,
            None,
            active_model.provider_id.clone(),
            active_model.model_id.clone(),
            active_model.display_name.clone(),
            active_model.provider_display_name.clone(),
        );

        // 关闭所有面板和状态
        self.ui.message_panel = None;
        self.ui.command_palette.clear();
        self.ui.at_mention.clear();
        self.ui.draft_attachments.clear();
        self.ui.restored_attachments.clear();
        self.ui.composer.clear();
        self.ui
            .composer
            .set_placeholder("Ask tidev about your code, task, or question...");
        self.scroll_messages_to_bottom();

        self.ui.last_notice = Some(format!(
            "Forked session with {} messages",
            original_messages.len()
        ));

        Ok(())
    }

    pub(crate) fn confirm_undo_to_message(&mut self) -> Result<()> {
        let Some(dialog) = self.ui.undo_confirm_dialog.take() else {
            return Ok(());
        };

        // 查找选中的消息并克隆完整数据（含 attachments）
        let Some(msg) = self
            .ui
            .chat_context
            .messages
            .iter()
            .find(|m| m.id == dialog.selected_message_id)
            .cloned()
        else {
            self.ui.last_notice = Some("Selected message not found".to_string());
            return Ok(());
        };

        // Save attachments so they can be restored after revert
        self.ui.restored_attachments = msg.attachments.clone();

        // 关闭 message_panel
        self.close_message_panel();

        // 调用 revert_to_message 执行 undo
        self.revert_to_message(msg.id, msg.content.clone())?;
        self.ui.last_notice = Some("Undo complete".to_string());

        Ok(())
    }

    pub(crate) fn close_theme_panel(&mut self, apply: bool) -> Result<()> {
        if let Some(panel) = self.ui.theme_panel.take() {
            if apply {
                self.apply_theme(panel.preview_theme)?;
            } else {
                self.ui.theme.set_mode(panel.original_theme);
                self.ui.message_render_cache.borrow_mut().clear();
            }
        }
        Ok(())
    }

    pub(crate) fn close_settings_panel(&mut self, _apply: bool) -> Result<()> {
        if let Some(panel) = self.ui.settings_panel.take() {
            self.runtime.update_config(|cfg| panel.apply_to_config(cfg));
            self.runtime.save_config()?;
        }
        Ok(())
    }

    pub(crate) fn apply_theme(&mut self, theme: ThemeName) -> Result<()> {
        self.ui.theme.set_mode(theme);
        self.ui.message_render_cache.borrow_mut().clear();
        self.runtime
            .update_config(|cfg| cfg.set_theme(theme.as_str()));
        self.runtime.save_config()?;
        self.ui.last_notice = Some(format!("Theme switched to {}", self.ui.theme.name()));
        Ok(())
    }

    pub(crate) fn open_rename_session_dialog(&mut self) -> Result<()> {
        self.ui.command_palette.clear();
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;
        self.ui.agents_panel = None;
        self.ui.at_mention.clear();
        self.ui.draft_attachments.clear();
        self.ui.restored_attachments.clear();

        self.ui.rename_session_dialog = Some(RenameSessionDialogState::new(
            self.ui.chat_context.title.clone(),
        ));
        self.ui
            .composer
            .set_text(self.ui.chat_context.title.clone());
        self.ui
            .composer
            .set_placeholder("Type the new session title and press Enter");
        self.ui.last_notice = Some("Rename the current session title".to_string());
        Ok(())
    }

    pub(crate) fn close_rename_session_dialog(&mut self) {
        self.ui.rename_session_dialog = None;
        self.ui.composer.clear();
        self.ui
            .composer
            .set_placeholder("Ask tidev about your code, task, or question...");
    }

    pub(crate) fn confirm_rename_session(&mut self) -> Result<()> {
        let mut title = self.ui.composer.text().trim().to_string();
        if title.is_empty() {
            title = "Untitled session".to_string();
        }

        self.ui.chat_context.title = title.clone();
        self.runtime.session_manager().update_session(
            self.ui.chat_context.session_id,
            Some(&title),
            None,
        )?;
        self.ui.last_notice = Some("Session title updated".to_string());
        self.close_rename_session_dialog();
        Ok(())
    }

    pub(crate) fn switch_model(&mut self, selector: Option<&str>) -> Result<()> {
        let config = self.runtime.config();
        let auth = self.runtime.auth();

        let mut model = config.resolve_active_model(&auth)?;
        if let Some(sel) = selector {
            model = config.resolve_model(&auth, Some(sel), None)?;
        }

        // Update chat_context
        self.ui.chat_context.provider_id = model.provider_id.clone();
        self.ui.chat_context.model_id = model.model_id.clone();

        // Persist model to session
        self.runtime.session_manager().store().update_session(
            self.ui.chat_context.session_id,
            None,
            None,
            None,
            None,
            None,
            Some(&model.provider_id),
            Some(&model.provider_display_name),
            Some(&model.model_id),
            Some(&model.display_name),
        )?;

        // Update default provider/model in config
        self.runtime.update_config(|cfg| {
            cfg.default_provider = model.provider_id.clone();
            cfg.default_model = model.model_id.clone();
        });
        self.runtime.save_config()?;

        self.ui.last_notice = Some(format!("Switched to {}", model.label()));
        Ok(())
    }

    pub(crate) fn start_new_session(&mut self) -> Result<()> {
        self.cache_active_session_runtime();

        let session_id = Uuid::new_v4();
        let workspace_root = self.runtime.workspace_root().to_string_lossy().to_string();
        let config = self.runtime.config();
        let auth = self.runtime.auth();
        let active_model = config.resolve_active_model(&auth)?;

        // Create session in DB
        self.runtime.session_manager().create_session(
            session_id,
            &workspace_root,
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            "Untitled session",
        )?;

        // Set up the new chat context
        self.ui.chat_context = ChatContext::new(
            session_id,
            "Untitled session".to_string(),
            workspace_root,
            Vec::new(),
            None,
            active_model.provider_id.clone(),
            active_model.model_id.clone(),
            active_model.display_name.clone(),
            active_model.provider_display_name.clone(),
        );

        self.ui.screen = Screen::Welcome;
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;
        self.ui.agents_panel = None;
        self.ui.command_palette.clear();
        self.ui.at_mention.clear();
        self.ui.draft_attachments.clear();
        self.ui.restored_attachments.clear();
        self.ui.composer.clear();
        self.ui
            .composer
            .set_placeholder("Ask tidev about your code, task, or question...");

        // System prompt composition is handled by Runtime during submit_prompt.
        self.scroll_messages_to_bottom();
        self.ui.last_notice = Some("Started a fresh session".to_string());

        Ok(())
    }

    pub(crate) fn submit_prompt(&mut self, prompt: String) -> Result<()> {
        let prompt = prompt.trim().to_string();
        log::info!(
            "submit_prompt: ENTER prompt={:?}, draft_attachments={}",
            prompt,
            self.ui.draft_attachments.len(),
        );
        if prompt.is_empty() && self.ui.draft_attachments.is_empty() {
            log::info!("submit_prompt: empty prompt and no attachments, returning");
            return Ok(());
        }

        let (attachments, instruction_sources) = self.build_prompt_attachments(&prompt)?;
        log::info!(
            "submit_prompt: build_prompt_attachments returned {} attachments",
            attachments.len(),
        );

        let config = self.runtime.config();
        let auth = self.runtime.auth();
        let active_model = config.resolve_active_model(&auth)?;
        if attachments.iter().any(MessageAttachment::is_image) && !active_model.supports_images {
            self.ui.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        if self.ui.pending_request {
            self.queue_prompt(prompt, attachments, instruction_sources);
            self.ui.draft_attachments.clear();
            self.ui.restored_attachments.clear();
            return Ok(());
        }

        self.submit_prompt_now(prompt, attachments, instruction_sources)
    }

    // ---------------------------------------------------------------------------
    // Stub / transitional methods — these will be properly implemented as the
    // new architecture solidifies.
    // ---------------------------------------------------------------------------

    /// Cache the active session's runtime state (for subagent orphan handling).
    pub(crate) fn cache_active_session_runtime(&mut self) {
        let ctx = &self.ui.chat_context;
        self.ui.cached_sessions.insert(
            ctx.session_id,
            CachedSessionRuntime {
                messages: ctx.messages.clone(),
                provider_id: ctx.provider_id.clone(),
                model_id: ctx.model_id.clone(),
                mode: tidev_types::prompts::SessionMode::Build,
            },
        );
    }

    /// Revert the conversation to a specific message (undo).
    pub(crate) fn revert_to_message(&mut self, message_id: Uuid, _content: String) -> Result<()> {
        // Delegate to runtime's undo mechanism.
        // Runtime::undo() undoes to the previous user message.
        // For undo-to-message we'd need a more precise API on Runtime,
        // but for the common "undo last" case this suffices.
        let session_id = self.ui.chat_context.session_id;
        let runtime = self.runtime.clone();
        let ui_session_id = session_id;
        tokio::spawn(async move {
            if let Err(e) = runtime.undo(ui_session_id).await {
                log::error!("revert_to_message failed: {e}");
            }
        });
        self.ui.last_notice = Some("Undo in progress...".to_string());
        Ok(())
    }

    /// Undo the last user message — revert to the previous conversation state.
    pub(crate) fn undo_last_user_message(&mut self) -> Result<()> {
        if self.ui.pending_request {
            // Cancel the current request so undo can proceed cleanly.
            let runtime = self.runtime.clone();
            tokio::spawn(async move {
                runtime.cancel().await;
            });
            self.ui.pending_request = false;
        }

        let session_id = self.ui.chat_context.session_id;
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = runtime.undo(session_id).await {
                log::error!("undo_last_user_message failed: {e}");
            }
        });
        self.ui.last_notice = Some("Undo in progress...".to_string());
        Ok(())
    }

    /// Redo — move forward past the last undo.
    pub(crate) fn redo_last_user_message(&mut self) -> Result<()> {
        if self.ui.pending_request {
            let runtime = self.runtime.clone();
            tokio::spawn(async move {
                runtime.cancel().await;
            });
            self.ui.pending_request = false;
        }

        let session_id = self.ui.chat_context.session_id;
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = runtime.redo(session_id).await {
                log::error!("redo_last_user_message failed: {e}");
            }
        });
        self.ui.last_notice = Some("Redo in progress...".to_string());
        Ok(())
    }

    /// Build prompt attachments from composer state and at-mentions.
    pub(crate) fn build_prompt_attachments(
        &self,
        prompt: &str,
    ) -> Result<(Vec<MessageAttachment>, Vec<String>)> {
        let mut attachments: Vec<MessageAttachment> = Vec::new();
        let mut instruction_sources: Vec<String> = Vec::new();

        // Collect draft attachments (images pasted from clipboard)
        for attachment in &self.ui.draft_attachments {
            attachments.push(attachment.clone());
        }

        // Collect at-mention references (file/directory paths)
        let mention_count = self.ui.at_mention.selected_count();
        for i in 0..mention_count {
            if let Some(suggestion) = self.ui.at_mention.selected_suggestion(i) {
                let path = std::path::Path::new(&suggestion.path);
                if path.is_dir() {
                    attachments.push(MessageAttachment::DirectoryReference {
                        path: suggestion.path.clone(),
                        tree: std::sync::Arc::new(String::new()),
                    });
                } else if path.is_file() {
                    attachments.push(MessageAttachment::FileReference {
                        path: suggestion.path.clone(),
                        content: std::sync::Arc::new(String::new()),
                        tool_output: None,
                        truncated: false,
                    });
                }
                instruction_sources.push(suggestion.path.clone());
            }
        }

        Ok((attachments, instruction_sources))
    }

    /// Queue a prompt when another request is already in flight.
    pub(crate) fn queue_prompt(
        &mut self,
        prompt: String,
        attachments: Vec<MessageAttachment>,
        instruction_sources: Vec<String>,
    ) {
        self.ui.pending_prompt_queue.push(QueuedPrompt::new(
            prompt,
            attachments,
            None,
            instruction_sources,
        ));
    }

    /// Submit a prompt immediately (no pending request).
    pub(crate) fn submit_prompt_now(
        &mut self,
        prompt: String,
        _attachments: Vec<MessageAttachment>,
        _instruction_sources: Vec<String>,
    ) -> Result<()> {
        let session_id = self.ui.chat_context.session_id;
        let runtime = self.runtime.clone();
        let line_count = self.ui.prompt_history.len() + 1;

        // Prepend the prompt to history
        self.ui.prompt_history.push(prompt.clone());
        if self.ui.prompt_history.len() > 100 {
            self.ui.prompt_history.remove(0);
        }

        self.ui.pending_request = true;
        self.ui.composer.clear();
        self.ui.at_mention.clear();
        self.ui.draft_attachments.clear();
        self.ui.restored_attachments.clear();
        self.scroll_messages_to_bottom();

        log::info!("submit_prompt_now: submitting line {line_count}, prompt={prompt:?}");

        tokio::spawn(async move {
            if let Err(e) = runtime.submit_prompt(session_id, prompt).await {
                log::error!("submit_prompt_now: runtime error: {e}");
            }
        });

        Ok(())
    }
}
