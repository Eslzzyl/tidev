use super::*;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::Duration};
use tokio::runtime::Runtime;

impl App {
    pub(crate) fn new() -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let paths = ConfigPaths::discover()?;
        let config = AppConfig::load_or_create(&paths)?;
        crate::logging::init(&paths.data_dir, config.logging.clone());
        crate::log_info!("App initializing, workspace={}", workspace_root.display());
        let auth = AuthStore::load_or_create(&paths)?;
        let store = SessionStore::open(paths.default_database_path())?;
        let llm = LlmClient::new()?;
        let theme = ThemeManager::new(&config.theme);
        let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        let tools = ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            config.skills.clone(),
            mcp,
            config.permissions.clone(),
        );
        let commands = CommandRegistry::new();
        let command_palette = CommandPaletteState::default();
        let composer = Composer::new("Ask TiDev about your code, task, or question...");
        let (backend_tx, backend_rx) = unbounded_channel();
        let mode = SessionMode::Build;

        let fallback_model = Self::resolve_fallback_model(&config, &auth)?;
        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            workspace_root.display().to_string(),
            fallback_model.provider_id.clone(),
            fallback_model.provider_display_name.clone(),
            fallback_model.model_id.clone(),
            fallback_model.display_name.clone(),
            "Untitled session",
        );

        let active_model = fallback_model.clone();
        let last_notice = None;
        let retrying_hint = None;

        let snapshot = SnapshotService::new(&workspace_root, &paths)?;
        let cleanup_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let app = Self {
            should_quit: false,
            screen: Screen::Welcome,
            workspace_root,
            paths,
            config,
            auth,
            store,
            llm,
            theme,
            mode,
            active_model,
            conversation,
            context_manager: ContextManager::new(),
            tools,
            commands,
            command_palette,
            connect_dialog: None,
            theme_panel: None,
            model_panel: None,
            session_panel: None,
            mcp_panel: None,
            at_mention: AtMentionState::default(),
            pending_tool_execution: None,
            permission_dialog: None,
            question_dialog: None,
            running_tool_executions: Vec::new(),
            running_subagent_executions: Vec::new(),
            pending_assistant_turns: std::collections::HashSet::new(),
            cached_sessions: std::collections::HashMap::new(),
            compacting_sessions: std::collections::HashSet::new(),
            leader_key_pending: false,
            composer,
            draft_attachments: Vec::new(),
            pending_request: false,
            active_request_id: 0,
            abort_confirmation_deadline: None,
            last_notice,
            toast: None,
            mouse_selection: MouseSelectionState::default(),
            retrying_hint,
            message_scroll_offset: 0,
            message_follow_tail: true,
            message_viewport_lines: 0,
            message_total_lines: 0,
            message_render_cache: RefCell::new(std::collections::HashMap::new()),
            message_render_cache_tick: Cell::new(0),
            message_render_cache_hits: Cell::new(0),
            message_render_cache_misses: Cell::new(0),
            message_content_area: None,
            sidebar_area: None,
            selection_clipboard_lease: None,
            backend_tx,
            backend_rx,
            loading_frame: 0,
            context_usage: None,
            snapshot,
            cleanup_cancel,
        };

        app.at_mention
            .start_background_indexing(app.workspace_root.as_path());

        Ok(app)
    }

    pub(crate) fn run(&mut self, runtime: &Runtime) -> Result<()> {
        runtime.block_on(self.refresh_mcp_tools())?;
        let _session = super::TerminalSession::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;

        let snapshot = self.snapshot.clone();
        let cleanup_cancel = self.cleanup_cancel.clone();

        runtime.spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            loop {
                if cleanup_cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                if let Err(e) = snapshot.cleanup().await {
                    crate::log_warn!("snapshot cleanup failed: {}", e);
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });

        loop {
            self.process_backend_events(runtime)?;
            self.update_mouse_selection_auto_scroll();
            terminal
                .draw(|frame| self.render(frame))
                .context("failed to render frame")?;

            if self.should_quit {
                break;
            }

            if crossterm::event::poll(Duration::from_millis(50))
                .context("failed to poll terminal events")?
            {
                let event = crossterm::event::read().context("failed to read terminal event")?;
                self.handle_event(event, runtime)?;
            }

            if self.should_quit {
                break;
            }
        }

        self.cleanup_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        terminal.show_cursor().ok();
        Ok(())
    }

    pub(crate) fn cache_active_session_runtime(&mut self) {
        let session_id = self.conversation.session_id;
        let cached = CachedSessionRuntime {
            conversation: self.conversation.clone(),
            active_model: self.active_model.clone(),
            context_manager: self.context_manager.clone(),
            pending_tool_execution: self.pending_tool_execution.clone(),
            permission_dialog: self.permission_dialog.clone(),
            question_dialog: self.question_dialog.clone(),
            running_tool_executions: self.running_tool_executions.clone(),
            running_subagent_executions: self.running_subagent_executions.clone(),
            pending_request: self.pending_request,
            active_request_id: self.active_request_id,
            abort_confirmation_deadline: self.abort_confirmation_deadline,
            retrying_hint: self.retrying_hint.clone(),
            message_scroll_offset: self.message_scroll_offset,
            message_follow_tail: self.message_follow_tail,
            message_viewport_lines: self.message_viewport_lines,
            message_total_lines: self.message_total_lines,
            context_usage: self.context_usage,
        };

        self.cached_sessions.insert(session_id, cached);
    }

    pub(crate) fn capture_ui_snapshot(&self) -> UiStateSnapshot {
        UiStateSnapshot {
            screen: self.screen,
            connect_dialog: self.connect_dialog.clone(),
            theme_panel: self.theme_panel.clone(),
            model_panel: self.model_panel.clone(),
            session_panel: self.session_panel.clone(),
            mcp_panel: self.mcp_panel.clone(),
            at_mention: self.at_mention.clone(),
            command_palette: self.command_palette.clone(),
            leader_key_pending: self.leader_key_pending,
            composer: self.composer.clone(),
            draft_attachments: self.draft_attachments.clone(),
            last_notice: self.last_notice.clone(),
            toast: self.toast.clone(),
            mouse_selection: self.mouse_selection.clone(),
        }
    }

    pub(crate) fn restore_ui_snapshot(&mut self, snapshot: UiStateSnapshot) {
        self.screen = snapshot.screen;
        self.connect_dialog = snapshot.connect_dialog;
        self.theme_panel = snapshot.theme_panel;
        self.model_panel = snapshot.model_panel;
        self.session_panel = snapshot.session_panel;
        self.mcp_panel = snapshot.mcp_panel;
        self.at_mention = snapshot.at_mention;
        self.command_palette = snapshot.command_palette;
        self.leader_key_pending = snapshot.leader_key_pending;
        self.composer = snapshot.composer;
        self.draft_attachments = snapshot.draft_attachments;
        self.last_notice = snapshot.last_notice;
        self.toast = snapshot.toast;
        self.mouse_selection = snapshot.mouse_selection;
    }

    pub(crate) fn with_temporary_session_context<F>(
        &mut self,
        session_id: Uuid,
        operation: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        if self.conversation.session_id == session_id {
            return operation(self);
        }

        let original_session_id = self.conversation.session_id;
        let ui_snapshot = self.capture_ui_snapshot();
        self.cache_active_session_runtime();

        let fallback_model = Self::resolve_fallback_model(&self.config, &self.auth)?;
        let target_runtime = if let Some(cached) = self.cached_sessions.remove(&session_id) {
            cached
        } else {
            match self.load_session_runtime_from_store(session_id, &fallback_model)? {
                Some(runtime) => runtime,
                None => {
                    if let Some(original_runtime) =
                        self.cached_sessions.remove(&original_session_id)
                    {
                        self.restore_cached_session_runtime(original_runtime);
                    }
                    self.restore_ui_snapshot(ui_snapshot);
                    return Ok(());
                }
            }
        };

        self.restore_cached_session_runtime(target_runtime);
        let result = operation(self);
        self.cache_active_session_runtime();

        if let Some(original_runtime) = self.cached_sessions.remove(&original_session_id) {
            self.restore_cached_session_runtime(original_runtime);
        }
        self.restore_ui_snapshot(ui_snapshot);

        result
    }

    pub(crate) fn restore_cached_session_runtime(&mut self, cached: CachedSessionRuntime) {
        self.conversation = cached.conversation;
        self.active_model = cached.active_model;
        self.context_manager = cached.context_manager;
        self.pending_tool_execution = cached.pending_tool_execution;
        self.permission_dialog = cached.permission_dialog;
        self.question_dialog = cached.question_dialog;
        self.running_tool_executions = cached.running_tool_executions;
        self.running_subagent_executions = cached.running_subagent_executions;
        self.pending_request = cached.pending_request;
        self.active_request_id = cached.active_request_id;
        self.abort_confirmation_deadline = cached.abort_confirmation_deadline;
        self.retrying_hint = cached.retrying_hint;
        self.message_scroll_offset = cached.message_scroll_offset;
        self.message_follow_tail = cached.message_follow_tail;
        self.message_viewport_lines = cached.message_viewport_lines;
        self.message_total_lines = cached.message_total_lines;
        self.context_usage = cached.context_usage;
    }

    pub(crate) fn clear_message_render_cache(&self) {
        self.message_render_cache.borrow_mut().clear();
        self.message_render_cache_tick.set(0);
    }

    pub(crate) fn invalidate_message_render_cache_for(&self, session_id: Uuid, message_id: Uuid) {
        self.message_render_cache
            .borrow_mut()
            .retain(|key, _| !(key.session_id == session_id && key.message_id == message_id));
    }

    pub(crate) fn invalidate_active_message_render_cache_for(&self, message_id: Uuid) {
        self.invalidate_message_render_cache_for(self.conversation.session_id, message_id);
    }

    pub(crate) fn next_message_render_cache_tick(&self) -> u64 {
        let tick = self.message_render_cache_tick.get().wrapping_add(1);
        self.message_render_cache_tick.set(tick);
        tick
    }

    pub(crate) fn record_message_render_cache_hit(&self) {
        self.message_render_cache_hits
            .set(self.message_render_cache_hits.get().saturating_add(1));
    }

    pub(crate) fn record_message_render_cache_miss(&self) {
        self.message_render_cache_misses
            .set(self.message_render_cache_misses.get().saturating_add(1));
    }

    pub(crate) fn message_render_cache_stats(&self) -> (u64, u64, usize) {
        (
            self.message_render_cache_hits.get(),
            self.message_render_cache_misses.get(),
            self.message_render_cache.borrow().len(),
        )
    }

    pub(crate) fn prune_message_render_cache_if_needed(&self) {
        let cache_len = self.message_render_cache.borrow().len();
        if cache_len <= MESSAGE_RENDER_CACHE_MAX_ENTRIES {
            return;
        }

        let remove_count = cache_len - MESSAGE_RENDER_CACHE_MAX_ENTRIES;
        let mut evict_candidates = self
            .message_render_cache
            .borrow()
            .iter()
            .map(|(key, entry)| (*key, entry.last_used_tick))
            .collect::<Vec<_>>();
        evict_candidates.sort_by_key(|(_, tick)| *tick);

        let mut cache = self.message_render_cache.borrow_mut();
        for (key, _) in evict_candidates.into_iter().take(remove_count) {
            cache.remove(&key);
        }
    }

    pub(crate) fn reset_active_runtime(&mut self) {
        self.context_manager = ContextManager::new();
        self.pending_tool_execution = None;
        self.permission_dialog = None;
        self.question_dialog = None;
        self.running_tool_executions.clear();
        self.running_subagent_executions.clear();
        self.pending_request = false;
        self.abort_confirmation_deadline = None;
        self.retrying_hint = None;
        self.context_usage = None;
        self.scroll_messages_to_bottom();
    }

    pub(crate) fn restore_or_load_session(
        &mut self,
        session_id: Uuid,
        fallback_model: &ActiveModel,
    ) -> Result<()> {
        if let Some(cached) = self.cached_sessions.remove(&session_id) {
            self.restore_cached_session_runtime(cached);
            return Ok(());
        }

        let Some(runtime) = self.load_session_runtime_from_store(session_id, fallback_model)?
        else {
            anyhow::bail!("session not found");
        };

        self.restore_cached_session_runtime(runtime);
        Ok(())
    }

    pub(crate) fn load_session_runtime_from_store(
        &self,
        session_id: Uuid,
        fallback_model: &ActiveModel,
    ) -> Result<Option<CachedSessionRuntime>> {
        let Some(conversation) = self.store.load_conversation(session_id)? else {
            return Ok(None);
        };

        let active_model =
            Self::resolve_conversation_model(&self.config, &self.auth, &conversation)
                .unwrap_or_else(|_| fallback_model.clone());

        let mut runtime = CachedSessionRuntime {
            conversation,
            active_model,
            context_manager: ContextManager::new(),
            pending_tool_execution: None,
            permission_dialog: None,
            question_dialog: None,
            running_tool_executions: Vec::new(),
            running_subagent_executions: Vec::new(),
            pending_request: false,
            active_request_id: 0,
            abort_confirmation_deadline: None,
            retrying_hint: None,
            message_scroll_offset: 0,
            message_follow_tail: true,
            message_viewport_lines: 0,
            message_total_lines: 0,
            context_usage: None,
        };

        if !runtime.conversation.visible_messages().is_empty() {
            let total_tokens: u32 = runtime
                .conversation
                .messages
                .iter()
                .filter_map(|message| message.total_tokens)
                .sum();
            if total_tokens > 0 {
                runtime.context_usage = Some((0, 0, total_tokens));
            }
        }

        Ok(Some(runtime))
    }

    pub(crate) fn schedule_context_compaction_for_session(
        &mut self,
        session_id: Uuid,
        runtime: &Runtime,
    ) {
        if self.compacting_sessions.contains(&session_id) {
            return;
        }

        let Some((conversation, mut context_manager, model)) =
            (if self.conversation.session_id == session_id {
                Some((
                    self.conversation.clone(),
                    self.context_manager.clone(),
                    self.active_model.clone(),
                ))
            } else {
                self.cached_sessions.get(&session_id).map(|cached| {
                    (
                        cached.conversation.clone(),
                        cached.context_manager.clone(),
                        cached.active_model.clone(),
                    )
                })
            })
        else {
            return;
        };

        self.compacting_sessions.insert(session_id);
        let llm = self.llm.clone();
        let tx = self.backend_tx.clone();

        runtime.spawn(async move {
            let result = context_manager
                .compact_if_needed(&llm, &model, &conversation)
                .await;

            let (compacted, summary, retained_from, error) = match result {
                Ok(compacted) => (
                    compacted,
                    context_manager.summary,
                    context_manager.retained_from,
                    None,
                ),
                Err(error) => (false, None, 0, Some(error.to_string())),
            };

            let _ = tx.send(BackendEvent::ContextCompacted {
                session_id,
                compacted,
                summary,
                retained_from,
                error,
            });
        });
    }

    pub(crate) fn apply_context_compaction(
        &mut self,
        session_id: Uuid,
        compacted: bool,
        summary: Option<String>,
        retained_from: usize,
        error: Option<String>,
    ) {
        self.compacting_sessions.remove(&session_id);

        if self.conversation.session_id == session_id {
            if compacted {
                self.context_manager.summary = summary;
                self.context_manager.retained_from = retained_from;
                self.last_notice = Some("Context compacted".to_string());
            } else if let Some(error) = error {
                self.last_notice = Some(error);
            }
            return;
        }

        if let Some(cached) = self.cached_sessions.get_mut(&session_id) {
            if compacted {
                cached.context_manager.summary = summary;
                cached.context_manager.retained_from = retained_from;
            }
        }
    }

    pub(crate) fn background_running_count(&self) -> usize {
        self.cached_sessions
            .values()
            .filter(|cached| cached.pending_request)
            .count()
    }

    pub(crate) fn background_waiting_question_count(&self) -> usize {
        self.cached_sessions
            .values()
            .filter(|cached| cached.question_dialog.is_some())
            .count()
    }
}
