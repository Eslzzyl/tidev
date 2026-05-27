use super::*;
use crate::panel_launcher::PanelLauncherState;
use chrono::Utc;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, path::Path, sync::RwLock, time::Duration};
use tidev_storage::database::Database;
use tokio::runtime::Runtime;

/// Find the git worktree root by looking for a .git directory,
/// starting from the given path and walking up to the ancestors.
fn find_git_worktree(start: &Path) -> Option<std::path::PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").is_dir() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

impl App {
    pub(crate) fn new() -> Result<Self> {
        let paths = ConfigPaths::discover()?;
        Self::new_with_paths(paths)
    }

    pub(crate) fn new_with_paths(paths: ConfigPaths) -> Result<Self> {
        let _t0 = std::time::Instant::now();
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let config = AppConfig::load_with_project_overlay(&paths, &workspace_root)?;
        // Initialize shell detection (Windows: auto-detect bash, Unix: sh).
        tidev_engine::shell::init(config.shell.windows_shell.clone(), Some(&paths));
        let _ = tidev_engine::logging::init(&paths.data_dir, config.logging.clone());
        log::info!("App initializing, workspace={}", workspace_root.display());
        log::info!("startup: config loaded in {:?}", _t0.elapsed());
        let _t1 = std::time::Instant::now();
        let auth = AuthStore::load_or_create(&paths)?;
        log::info!("startup: auth loaded in {:?}", _t1.elapsed());
        let _t2 = std::time::Instant::now();
        let db = Database::open(paths.default_database_path())?;
        log::info!(
            "startup: Database::open (schema init) in {:?}",
            _t2.elapsed()
        );
        let _t3 = std::time::Instant::now();
        let store = db.create_session_store()?;
        let memory_store = Arc::new(tidev_engine::memory::MemoryStore::open(
            paths.default_database_path(),
        )?);
        log::info!("startup: stores created in {:?}", _t3.elapsed());
        let _t4 = std::time::Instant::now();
        let llm = LlmClient::new(
            config.logging.save_request_body,
            config.logging.max_request_files,
        )?;
        log::info!("startup: LlmClient::new in {:?}", _t4.elapsed());
        let http_client = Arc::new(llm.http().clone());
        let _t5 = std::time::Instant::now();
        let theme = ThemeManager::new(&config.theme);
        let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        let file_read_tracker = Arc::new(FileReadTracker::new());
        // Find git worktree root to limit skill discovery scope
        let worktree = find_git_worktree(&workspace_root);
        log::info!("startup: theme/mcp created in {:?}", _t5.elapsed());
        let _t6 = std::time::Instant::now();
        let mut tools = ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            config.skills.clone(),
            mcp,
            config.permissions.clone(),
            file_read_tracker.clone(),
            memory_store.clone(),
            config.rtk.enabled,
            worktree,
            config.websearch.clone(),
            Arc::new(auth.clone()),
        );
        log::info!("startup: ToolRegistry created in {:?}", _t6.elapsed());
        #[allow(unused_variables)]
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

        let mut active_model = fallback_model;
        // Load saved thinking level preference for the default model across restarts
        if let Ok(Some(level_str)) =
            store.load_model_thinking_level(&active_model.provider_id, &active_model.model_id)
        {
            active_model.thinking_level =
                tidev_engine::config::reasoning::ThinkingLevelType::from_string(&level_str);
        }
        tools.set_active_model(active_model.clone());
        // Attach LLM to memory store with model overrides
        let mut consolidation_override: Option<tidev_engine::config::ActiveModel> = config
            .memory
            .consolidation_model
            .as_deref()
            .and_then(|s| config.resolve_model(&auth, Some(s)).ok());
        if let Some(ref mut model) = consolidation_override
            && let Some(tl_str) = config.memory.thinking_levels.get("consolidation")
        {
            model.thinking_level =
                tidev_engine::config::reasoning::ThinkingLevelType::from_string(tl_str);
        }

        // Provide a model resolver so summarization can reuse the last
        // assistant message's model (for prompt-cache reuse).
        {
            let config = config.clone();
            let auth = auth.clone();
            memory_store.set_model_resolver(std::sync::Arc::new(move |model_id: &str| {
                config.resolve_model(&auth, Some(model_id))
            }));
        }

        // Provide a tool filter so background summarization produces the same
        // tool list as normal conversation turns (preserving prefix cache).
        {
            let t = tools.clone();
            memory_store.set_tool_filter(std::sync::Arc::new(move |model: &ActiveModel| {
                t.definitions_for_model(model)
            }));
        }

        let _t_mem = std::time::Instant::now();
        memory_store.set_models(llm.clone(), active_model.clone(), consolidation_override);
        log::info!("startup: memory set_models in {:?}", _t_mem.elapsed());
        // Set sandbox policy based on session mode and config
        let sandbox_policy = config.sandbox.to_policy();
        tools.set_sandbox_policy(Some(sandbox_policy));
        // Build shared AgentRuntime from the same resources
        let agent = AgentRuntime {
            workspace_root: workspace_root.clone(),
            config_dir: paths.config_dir.clone(),
            config_paths: paths.clone(),
            config: config.clone(),
            auth: auth.clone(),
            store: Arc::new(tokio::sync::Mutex::new(store.clone())),
            llm_client: llm.clone(),
            tools: tools.clone(),
            instructions: config.instructions.clone(),
            instruction_content_cache: std::collections::HashMap::new(),
            queued_messages: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            auto_approve_permissions: true, // TUI handles permissions via channel
            hooks: tidev_engine::hooks::HookEngine::new(
                config.hooks.clone(),
                workspace_root.clone(),
            )
            .with_memory_store(memory_store.clone()),
        };
        // Share current session ID for the background inactivity check.
        let current_session_id: Arc<RwLock<Uuid>> = Arc::new(RwLock::new(session_id));
        let inactivity_check_cancel = CancellationToken::new();

        let last_notice = None;
        let retrying_hint = None;

        let snapshot = SnapshotService::new(&workspace_root, &paths)?;
        let cleanup_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let notifications = notifications::NotificationManager::new(config.notifications.clone());

        // Background cleanup of old tool outputs (runs every hour).
        store.start_output_cleanup(7, std::time::Duration::from_secs(3600));

        let app = Self {
            should_quit: false,
            screen: Screen::Welcome,
            workspace_root: workspace_root.clone(),
            paths: paths.clone(),
            config: config.clone(),
            auth,
            store,
            llm,
            http_client,
            theme,
            mode,
            pending_mode: None,
            active_model: active_model.clone(),
            conversation,
            context_manager: ContextManager::new(),
            tools,
            agent,
            file_read_tracker,
            current_goal: None,
            commands,
            command_palette,
            panel_launcher: PanelLauncherState::default(),
            current_session_id,
            inactivity_check_cancel,
            connect_dialog: None,
            theme_panel: None,
            model_panel: None,
            message_panel: None,
            session_panel: None,
            settings_panel: None,
            rename_dialog: None,
            mcp_panel: None,
            agents_panel: None,
            skills_panel: None,
            sandbox_panel: None,
            sync_panel: None,
            search_panel: None,
            at_mention: AtMentionState::default(),
            snippet_state: SnippetState::default(),
            shell_completion: ShellCompletionState::default(),
            pending_tool_execution: None,
            permission_dialog: None,
            sandbox_elevation: None,
            workspace_boundary_dialog: None,
            workspace_boundary_confirm_dialog: None,
            sensitive_file_dialog: None,
            sensitive_file_permissions: std::collections::HashMap::new(),
            sensitive_file_approved: std::collections::HashMap::new(),
            workspace_boundary_permissions: std::collections::HashMap::new(),
            workspace_boundary_approved: std::collections::HashMap::new(),
            question_dialog: None,
            fork_confirm_dialog: None,
            undo_confirm_dialog: None,
            running_tool_executions: Vec::new(),
            running_subagent_executions: Vec::new(),
            pending_assistant_turns: std::collections::HashSet::new(),
            cached_sessions: std::collections::HashMap::new(),
            compacting_sessions: std::collections::HashSet::new(),
            leader_key_pending: false,
            composer,
            draft_attachments: Vec::new(),
            pending_prompt_queue: std::collections::VecDeque::new(),
            pending_request: false,
            active_request_id: 0,
            request_cancel_token: None,
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
            message_layout_index: RefCell::new(MessageLayoutIndex::default()),
            message_content_area: None,
            message_scrollbar_area: None,
            scrollbar_drag_state: None,
            scrollbar_hovered: false,
            sidebar_area: None,
            sidebar_scroll_offset: 0,
            sidebar_total_lines: 0,
            input_area: Cell::new(None),
            memory_panel_overlay: Cell::new(None),
            theme_panel_overlay: Cell::new(None),
            model_panel_overlay: Cell::new(None),
            session_panel_overlay: Cell::new(None),
            message_panel_overlay: Cell::new(None),
            skills_panel_overlay: Cell::new(None),
            settings_panel_overlay: Cell::new(None),
            agents_panel_overlay: Cell::new(None),
            mcp_panel_overlay: Cell::new(None),
            balance_panel_overlay: Cell::new(None),
            stats_panel_overlay: Cell::new(None),
            input_scroll_offset: 0,
            input_dragging: false,
            selection_clipboard_lease: None,
            last_render_time: Instant::now(),
            render_throttled: false,
            dirty: true,
            backend_tx,
            backend_rx,
            spinner_start: Instant::now(),
            last_spinner_frame: 0,
            context_usage: None,
            snapshot,
            cleanup_cancel,
            loaded_instruction_sources: Vec::new(),
            instruction_content_cache: std::collections::HashMap::new(),
            expanded_tool_results: std::collections::HashSet::new(),
            tool_result_card_bounds: Vec::new(),
            hovered_card: None,
            user_card_bounds: Vec::new(),
            queued_card_bounds: Vec::new(),
            hovered_queued_index: None,
            subagent_task_map: std::collections::HashMap::new(),
            running_subagent_card_bounds: Vec::new(),
            pending_permission_rx: None,
            pending_permission_response: None,
            pending_rejected_tools: Vec::new(),
            selectable_regions: Vec::new(),
            message_scroll_target: None,
            todos: Vec::new(),
            step_snapshot_hashes: Vec::new(),
            step_cached_file_lists: Vec::new(),
            step_cached_file_diffs: None,
            step_prev_hash: None,
            stats_panel: None,
            balance_panel: Arc::new(Mutex::new(None)),
            notifications,
            shell_mode: false,
            thinking_level: active_model.thinking_level.clone(),
            memory_store,
            memory_panel: None,
            terminal_session: None,
            force_full_redraw: false,
            processing_child_session: false,
        };

        // Start background file indexing so the @-mention panel is ready
        // when the user first presses the @ key.
        app.at_mention.start_background_indexing(&workspace_root);
        log::info!("startup: App::new_with_paths total in {:?}", _t0.elapsed());

        Ok(app)
    }

    pub(crate) fn run(&mut self, runtime: &Runtime) -> Result<()> {
        let _t_run = std::time::Instant::now();
        let mcp_manager = self.tools.mcp_manager();
        runtime.spawn(async move {
            if let Err(e) = mcp_manager.refresh_all().await {
                log::warn!("MCP refresh failed: {}", e);
            }
        });
        log::info!("startup: MCP refresh spawned in {:?}", _t_run.elapsed());
        self.terminal_session = Some(super::TerminalSession::enter()?);
        log::info!("startup: TerminalSession::enter in {:?}", _t_run.elapsed());
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;
        log::info!("startup: terminal created in {:?}", _t_run.elapsed());

        let snapshot = self.snapshot.clone();
        let cleanup_cancel = self.cleanup_cancel.clone();

        runtime.spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            loop {
                if cleanup_cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                if let Err(e) = snapshot.cleanup().await {
                    log::warn!("snapshot cleanup failed: {}", e);
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });

        // Start memory background tasks: eviction, consolidation, reflection.
        log::info!("memory: starting background tasks");
        tidev_engine::memory::start_background_tasks(
            self.memory_store.clone(),
            runtime.handle(),
            &self.workspace_root.to_string_lossy(),
            &self.config.memory,
        );
        log::info!(
            "startup: memory background tasks spawned in {:?}",
            _t_run.elapsed()
        );

        // Schedule periodic session inactivity check (every 60 seconds).
        let check_store = self.store.clone();
        let check_mem_store = self.memory_store.clone();
        let check_ws = self.workspace_root.to_string_lossy().to_string();
        let cancel_token = self.inactivity_check_cancel.clone();
        let sid_ref = self.current_session_id.clone();
        let memory_auto_learn = self.config.memory.enabled && self.config.memory.auto_learn;
        runtime.spawn(async move {
            const INACTIVITY_TIMEOUT_SECS: i64 = 300;
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let now = Utc::now();
                        let cutoff = (now - chrono::Duration::seconds(INACTIVITY_TIMEOUT_SECS)).to_rfc3339();
                        let current = *sid_ref.read().unwrap();
                        let ids = match check_store.find_inactive_sessions(&cutoff, current) {
                            Ok(ids) => ids,
                            Err(e) => {
                                log::warn!("inactivity check failed: {}", e);
                                continue;
                            }
                        };
                        log::debug!("inactivity check: found {} inactive sessions", ids.len());
                        for id in ids {
                            log::info!("marking inactive session completed: {}", id);
                            if let Err(e) = check_store.set_session_status(id, "completed") {
                                log::warn!("failed to mark session completed: {}", e);
                                continue;
                            }
                            if memory_auto_learn
                                && let Err(e) = check_mem_store.summarize_session(id, &check_ws).await
                            {
                                log::warn!("session summarisation failed: {}", e);
                            }
                        }
                    }
                    _ = cancel_token.cancelled() => break,
                }
            }
        });
        log::info!(
            "startup: inactivity check spawned in {:?}",
            _t_run.elapsed()
        );

        log::info!(
            "startup: entering main event loop — total startup {:?}",
            _t_run.elapsed()
        );

        loop {
            self.process_backend_events(runtime)?;
            self.update_mouse_selection_auto_scroll();

            // Throttle rendering during streaming to preserve responsiveness for input events
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_render_time);
            let frame_budget = Duration::from_millis(16); // 60fps

            // Lazy spinner: during active requests, only wake the renderer
            // when the spinner frame actually changes (every 100ms).
            // This avoids fixed-rate polling during quiet tool execution.
            let spinner_frame = (self.spinner_start.elapsed().as_millis() / 100) as u64;
            if !self.dirty && self.pending_request && spinner_frame != self.last_spinner_frame {
                self.dirty = true;
            }

            // After TUI suspend/resume (external editor), ratatui's frame
            // buffer is stale — force a full clear + redraw.
            if self.force_full_redraw {
                terminal.clear().context("failed to clear terminal")?;
                self.force_full_redraw = false;
            }

            if self.dirty && (elapsed >= frame_budget || !self.render_throttled) {
                terminal
                    .draw(|frame| self.render(frame))
                    .context("failed to render frame")?;
                self.last_render_time = now;
                self.render_throttled = true;
                self.dirty = false;
                self.last_spinner_frame = spinner_frame;
            }

            if self.should_quit {
                break;
            }

            // Batch process all pending input events to avoid lag
            // during rapid scrolling or other high-frequency input
            let mut events_processed = 0;
            const MAX_EVENTS_PER_FRAME: usize = 32;

            while events_processed < MAX_EVENTS_PER_FRAME {
                match crossterm::event::poll(Duration::from_millis(0)) {
                    Ok(true) => {
                        if let Ok(event) = crossterm::event::read() {
                            self.handle_event(event, runtime)?;
                            events_processed += 1;
                            self.render_throttled = false; // Reset throttle on input
                            if self.should_quit {
                                break;
                            }
                        }
                    }
                    Ok(false) => break,
                    Err(e) => {
                        return Err(anyhow::anyhow!("failed to poll terminal events: {}", e));
                    }
                }
            }

            // If no events were processed, wait a bit before next frame
            if events_processed == 0
                && crossterm::event::poll(Duration::from_millis(16))
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

        // Cancel the inactivity check and agent loop so they exit
        // promptly instead of blocking the tokio runtime shutdown.
        self.inactivity_check_cancel.cancel();
        if let Some(token) = self.request_cancel_token.take() {
            token.cancel();
        }

        // Give tasks a brief window to notice cancellation and clean up
        // (e.g. bash execution loop checks cancel flag every 100ms).
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Force-kill any remaining child processes (e.g. bash subprocesses
        // whose tokio task was dropped before it could clean up).
        tidev_engine::tooling::builtin::kill_all_children();

        self.pending_permission_rx = None;
        self.pending_permission_response = None;

        terminal.show_cursor().ok();
        Ok(())
    }

    pub(crate) fn cache_active_session_runtime(&mut self) {
        let session_id = self.conversation.session_id;
        let cached = CachedSessionRuntime {
            conversation: self.conversation.clone(),
            active_model: self.active_model.clone(),
            mode: self.mode,
            context_manager: self.context_manager.clone(),
            pending_tool_execution: self.pending_tool_execution.clone(),
            permission_dialog: self.permission_dialog.clone(),
            workspace_boundary_dialog: self.workspace_boundary_dialog.clone(),
            workspace_boundary_confirm_dialog: self.workspace_boundary_confirm_dialog.clone(),
            sensitive_file_dialog: self.sensitive_file_dialog.clone(),
            workspace_boundary_permissions: self.workspace_boundary_permissions.clone(),
            sensitive_file_permissions: self.sensitive_file_permissions.clone(),
            question_dialog: self.question_dialog.clone(),
            running_tool_executions: self.running_tool_executions.clone(),
            running_subagent_executions: self.running_subagent_executions.clone(),
            pending_request: self.pending_request,
            pending_prompt_queue: self.pending_prompt_queue.clone(),
            active_request_id: self.active_request_id,
            abort_confirmation_deadline: self.abort_confirmation_deadline,
            retrying_hint: self.retrying_hint.clone(),
            message_scroll_offset: self.message_scroll_offset,
            message_follow_tail: self.message_follow_tail,
            message_viewport_lines: self.message_viewport_lines,
            message_total_lines: self.message_total_lines,
            context_usage: self.context_usage.clone(),
            todos: self.todos.clone(),
            file_reads: self.file_read_tracker.extract_session_reads(session_id),
            loaded_instruction_sources: self.loaded_instruction_sources.clone(),
            instruction_content_cache: self.instruction_content_cache.clone(),
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
            rename_dialog: self.rename_dialog.clone(),
            mcp_panel: self.mcp_panel.clone(),
            agents_panel: self.agents_panel.clone(),
            skills_panel: self.skills_panel.clone(),
            sandbox_panel: self.sandbox_panel.clone(),
            sync_panel: self.sync_panel.clone(),
            search_panel: self.search_panel.clone(),
            memory_panel: self.memory_panel.clone(),
            message_panel: self.message_panel.clone(),
            at_mention: self.at_mention.clone(),
            snippet_state: self.snippet_state.clone(),
            command_palette: self.command_palette.clone(),
            panel_launcher: self.panel_launcher.clone(),
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
        self.message_panel = snapshot.message_panel;
        self.session_panel = snapshot.session_panel;
        self.memory_panel = snapshot.memory_panel;
        self.rename_dialog = snapshot.rename_dialog;
        self.mcp_panel = snapshot.mcp_panel;
        self.agents_panel = snapshot.agents_panel;
        self.skills_panel = snapshot.skills_panel;
        self.sandbox_panel = snapshot.sandbox_panel;
        self.sync_panel = snapshot.sync_panel;
        self.search_panel = snapshot.search_panel;
        self.at_mention = snapshot.at_mention;
        self.snippet_state = snapshot.snippet_state;
        self.command_palette = snapshot.command_palette;
        self.panel_launcher = snapshot.panel_launcher;
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
        let _session_id = cached.conversation.session_id;
        self.conversation = cached.conversation;
        let thinking_level = cached.active_model.thinking_level.clone();
        self.active_model = cached.active_model;
        self.thinking_level = thinking_level;
        self.context_manager = cached.context_manager;
        self.pending_tool_execution = cached.pending_tool_execution;
        self.permission_dialog = cached.permission_dialog;
        self.workspace_boundary_dialog = cached.workspace_boundary_dialog;
        self.workspace_boundary_confirm_dialog = cached.workspace_boundary_confirm_dialog;
        self.sensitive_file_dialog = cached.sensitive_file_dialog;
        self.workspace_boundary_permissions = cached.workspace_boundary_permissions;
        self.sensitive_file_permissions = cached.sensitive_file_permissions;
        self.question_dialog = cached.question_dialog;
        self.running_tool_executions = cached.running_tool_executions;
        self.running_subagent_executions = cached.running_subagent_executions;
        self.mode = cached.mode;
        self.pending_request = cached.pending_request;
        self.pending_prompt_queue = cached.pending_prompt_queue;
        self.active_request_id = cached.active_request_id;
        self.abort_confirmation_deadline = cached.abort_confirmation_deadline;
        self.retrying_hint = cached.retrying_hint;
        self.message_scroll_offset = cached.message_scroll_offset;
        self.message_follow_tail = cached.message_follow_tail;
        self.message_viewport_lines = cached.message_viewport_lines;
        self.message_total_lines = cached.message_total_lines;
        self.context_usage = cached.context_usage;
        self.todos = cached.todos.clone();
        self.loaded_instruction_sources = cached.loaded_instruction_sources.clone();

        // Restore instruction content cache from loaded sources.
        // This prevents instruction files from being reloaded on the next user message,
        // avoiding duplicate "Loaded instructions from ..." messages.
        self.instruction_content_cache.clear();
        for source in &cached.loaded_instruction_sources {
            if source.starts_with("http://") || source.starts_with("https://") {
                // Skip URLs - they are fetched each time
                continue;
            }

            // Build the full path and canonicalize it to match the format
            // used by system_paths() in system_prompt_and_sources_with_cache().
            // This ensures cache lookups will find the cached content.
            let path = if std::path::Path::new(source).is_absolute() {
                std::path::PathBuf::from(source)
            } else {
                self.workspace_root.join(source)
            };
            let canonical_path = tidev_engine::tooling::builtin::utils::canonicalize_display(&path);

            if let Ok(content) = std::fs::read_to_string(&canonical_path) {
                self.instruction_content_cache
                    .insert(canonical_path.display().to_string(), content);
            }
        }
        log::info!(
            "restore_cached_session_runtime: loaded_instruction_sources={:?} cache_keys={:?}",
            self.loaded_instruction_sources,
            self.instruction_content_cache.keys().collect::<Vec<_>>(),
        );

        // Restore cached file read records
        if let Some(reads) = cached.file_reads {
            self.file_read_tracker
                .restore_session_reads(self.conversation.session_id, reads);
        }
    }

    pub(crate) fn clear_message_render_cache(&self) {
        self.message_render_cache.borrow_mut().clear();
        self.message_render_cache_tick.set(0);
        // Invalidate layout index when cache is cleared
        self.message_layout_index.borrow_mut().valid = false;
    }

    pub(crate) fn invalidate_message_render_cache_for(&self, session_id: Uuid, message_id: Uuid) {
        self.message_render_cache
            .borrow_mut()
            .retain(|key, _| !(key.session_id == session_id && key.message_id == message_id));

        if session_id == self.conversation.session_id {
            // Track dirty message for incremental layout update
            // instead of invalidating the entire layout index
            let mut index = self.message_layout_index.borrow_mut();
            if !index.dirty_messages.contains(&message_id) {
                index.dirty_messages.push(message_id);
            }
        }
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
            .map(|(key, entry)| (key.clone(), entry.last_used_tick))
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
        self.fork_confirm_dialog = None;
        self.running_tool_executions.clear();
        self.running_subagent_executions.clear();
        self.pending_request = false;
        self.pending_mode = None;
        self.pending_prompt_queue.clear();
        self.abort_confirmation_deadline = None;
        self.retrying_hint = None;
        self.context_usage = None;
        self.scroll_messages_to_bottom();
        self.loaded_instruction_sources.clear();
        self.instruction_content_cache.clear();
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

        let mut active_model =
            Self::resolve_conversation_model(&self.config, &self.auth, &conversation)
                .unwrap_or_else(|_| fallback_model.clone());

        // Restore the session's immutable static system prompt.
        // If the session has no stored prompt (legacy session), compose it now.
        let stored_system_prompt = self.store.load_session_system_prompt(session_id)?;
        if !stored_system_prompt.is_empty() {
            active_model.system_prompt = stored_system_prompt;
        } else {
            let composed = self
                .agent
                .compose_static_system_prompt(&active_model.system_prompt);
            if let Err(e) = self
                .store
                .update_session_system_prompt(session_id, &composed)
            {
                log::warn!("failed to persist static system prompt: {}", e);
            }
            active_model.system_prompt = composed;
        }

        let context_manager = ContextManager::from_state(
            conversation.context_summary.clone(),
            conversation.context_retained_from,
        );

        let mut loaded_instruction_sources = self.store.load_instruction_sources(session_id)?;

        // Normalise legacy DB entries (may contain relative paths saved by
        // earlier versions) to canonical absolute form so the in-memory list
        // is consistent regardless of what the DB holds.
        for source in loaded_instruction_sources.iter_mut() {
            if source.starts_with("http://") || source.starts_with("https://") {
                continue;
            }
            let path = if std::path::Path::new(source.as_str()).is_absolute() {
                std::path::PathBuf::from(source.as_str())
            } else {
                self.workspace_root.join(source.as_str())
            };
            let canonical = tidev_engine::tooling::builtin::utils::canonicalize_display(&path);
            *source = canonical.display().to_string();
        }

        // Pre-populate cache from loaded instruction sources so the next
        // user message doesn't re-read all files and avoid redundant
        // "Loaded instructions from ..." messages across restarts.
        let mut instruction_content_cache = std::collections::HashMap::new();
        for source in &loaded_instruction_sources {
            if source.starts_with("http://") || source.starts_with("https://") {
                continue;
            }
            // source is now guaranteed to be canonical absolute.
            if let Ok(content) = std::fs::read_to_string(source) {
                instruction_content_cache.insert(source.clone(), content);
            }
        }
        log::info!(
            "load_session_runtime_from_store: session={} loaded_instruction_sources={:?} cache_keys={:?}",
            session_id,
            loaded_instruction_sources,
            instruction_content_cache.keys().collect::<Vec<_>>(),
        );

        let mut runtime = CachedSessionRuntime {
            conversation,
            active_model,
            mode: SessionMode::Build,
            context_manager,
            pending_tool_execution: None,
            permission_dialog: None,
            workspace_boundary_dialog: None,
            workspace_boundary_confirm_dialog: None,
            sensitive_file_dialog: None,
            workspace_boundary_permissions: std::collections::HashMap::new(),
            sensitive_file_permissions: std::collections::HashMap::new(),
            question_dialog: None,
            running_tool_executions: Vec::new(),
            running_subagent_executions: Vec::new(),
            pending_request: false,
            pending_prompt_queue: std::collections::VecDeque::new(),
            active_request_id: 0,
            abort_confirmation_deadline: None,
            retrying_hint: None,
            message_scroll_offset: 0,
            message_follow_tail: true,
            message_viewport_lines: 0,
            message_total_lines: 0,
            context_usage: None,
            todos: self.store.load_todos(session_id)?,
            file_reads: None,
            loaded_instruction_sources,
            instruction_content_cache,
        };

        // Load file reads from database into the tracker
        if let Err(e) = self
            .file_read_tracker
            .load_from_store(&self.store, session_id)
        {
            log::warn!(
                "Failed to load file reads for session {}: {}",
                session_id,
                e
            );
        }

        if !runtime.conversation.visible_messages().is_empty() {
            let last_token_usage = runtime
                .conversation
                .visible_messages()
                .iter()
                .rev()
                .find_map(|message| {
                    message
                        .total_tokens
                        .map(|total| super::state::ContextUsage {
                            input_tokens: message.input_tokens.unwrap_or(0),
                            output_tokens: message.output_tokens.unwrap_or(0),
                            total_tokens: total,
                            cache_read_tokens: message.cache_read_tokens.unwrap_or(0),
                            cache_write_tokens: message.cache_write_tokens.unwrap_or(0),
                            model_id: message.model_id.clone().unwrap_or_default(),
                            tokens_per_second: message.tokens_per_second,
                        })
                });
            if let Some(usage) = last_token_usage {
                runtime.context_usage = Some(usage);
            }
        }

        // Restore thinking level: preference first, then last message overrides
        if let Ok(Some(level_str)) = self.store.load_model_thinking_level(
            &runtime.active_model.provider_id,
            &runtime.active_model.model_id,
        ) {
            runtime.active_model.thinking_level =
                tidev_engine::config::reasoning::ThinkingLevelType::from_string(&level_str);
        }
        if let Some(last_level) = runtime
            .conversation
            .messages
            .iter()
            .rev()
            .find_map(|m| m.thinking_level.as_ref())
        {
            runtime.active_model.thinking_level = last_level.clone();
        }

        // Restore mode from last user message
        if let Some(last_mode) = runtime.conversation.messages.iter().rev().find_map(|m| {
            if matches!(m.role, tidev_session::session::MessageRole::User) {
                m.mode
            } else {
                None
            }
        }) {
            runtime.mode = last_mode;
        }

        Ok(Some(runtime))
    }

    pub(crate) fn schedule_context_compaction_for_session(
        &mut self,
        session_id: Uuid,
        runtime: &Runtime,
        stream_request_id: Option<u64>,
    ) {
        if self.compacting_sessions.contains(&session_id) {
            return;
        }

        let is_active = self.conversation.session_id == session_id;
        let Some((mut conversation, mut context_manager, mut model)) = (if is_active {
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
        }) else {
            return;
        };

        // Reload messages from DB so they reflect any DC persistence
        // done by the agent loop (which updates DB but not the in-memory
        // conversation copy used above).
        if let Ok(db_messages) = self.store.load_messages(session_id) {
            conversation.messages = db_messages;
        }

        // Use the session's immutable static system prompt from DB.
        // For the active session, model.system_prompt is already correct
        // (loaded in restore_or_load_session), but cached sessions need
        // the stored prompt too. Re-composing would re-capture SystemInfo
        // (date, etc.) and break prefix caching.
        if let Ok(stored) = self.store.load_session_system_prompt(session_id)
            && !stored.is_empty()
        {
            model.system_prompt = stored;
        }
        let mode = if is_active {
            self.mode
        } else {
            tidev_types::prompts::SessionMode::Build
        };

        self.compacting_sessions.insert(session_id);
        let llm = self.llm.clone();
        let tx = self.backend_tx.clone();
        let manual = stream_request_id.is_some();
        // Sync the tool registry's active model so the tool list is
        // byte-for-byte identical to normal requests (preserving prefix cache).
        self.tools.set_active_model(model.clone());
        let tools = self.tools.all_definitions();

        runtime.spawn(async move {
            let result = if let Some(request_id) = stream_request_id {
                context_manager
                    .compact(tidev_engine::context::CompactionConfig {
                        llm: &llm,
                        model: &model,
                        conversation: &conversation,
                        manual: true,
                        stream_ctx: Some((request_id, tx.clone())),
                        tools: &tools,
                        mode,
                    })
                    .await
            } else {
                context_manager
                    .compact_if_needed(tidev_engine::context::CompactionConfig {
                        llm: &llm,
                        model: &model,
                        conversation: &conversation,
                        manual: false,
                        stream_ctx: None,
                        tools: &tools,
                        mode,
                    })
                    .await
            };

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
                manual,
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
        manual: bool,
        summary: Option<String>,
        retained_from: usize,
        error: Option<String>,
    ) {
        self.compacting_sessions.remove(&session_id);

        if self.conversation.session_id == session_id {
            if compacted {
                // Capture prior context state for undo before overwriting.
                let prior_summary = self.context_manager.summary.clone();
                let prior_retained_from = self.context_manager.retained_from;

                self.context_manager.summary = summary.clone();
                self.context_manager.retained_from = retained_from;
                self.conversation
                    .set_context_state(summary.clone(), retained_from);
                if let Err(error) = self.store.update_session_context_state(
                    session_id,
                    summary.as_deref(),
                    retained_from,
                ) {
                    log::warn!("failed to persist compacted context state: {}", error);
                }
                if let Some(summary) = summary.as_ref() {
                    let mut updated_existing = false;
                    if manual
                        && let Some(last_msg) = self.conversation.messages.last_mut()
                        && last_msg.streaming
                        && last_msg.role == tidev_session::session::MessageRole::System
                    {
                        // Don't replace message content — Delta events during
                        // streaming have already accumulated the full summary
                        // text (via BackendEvent::Delta → push_str).  The
                        // `summary` parameter here is truncated to
                        // `maximum_summary_chars` and would cut off the full
                        // output that the user already saw streaming in.
                        last_msg.streaming = false;
                        last_msg.metadata.prior_summary = prior_summary.clone();
                        last_msg.metadata.prior_retained_from = Some(prior_retained_from);
                        updated_existing = true;

                        if let Err(error) = self
                            .store
                            .append_message(self.conversation.session_id, last_msg)
                        {
                            log::warn!("failed to persist compaction message: {}", error);
                        }
                    }
                    if !updated_existing {
                        let mut compaction_message =
                            tidev_session::session::Message::compaction(summary.clone());
                        compaction_message.metadata.prior_summary = prior_summary;
                        compaction_message.metadata.prior_retained_from = Some(prior_retained_from);
                        self.conversation.push(compaction_message.clone());
                        if let Err(error) = self
                            .store
                            .append_message(self.conversation.session_id, &compaction_message)
                        {
                            log::warn!("failed to persist compaction message: {}", error);
                        }
                    }
                    self.scroll_messages_to_bottom();
                    self.clear_message_render_cache();
                }
                self.last_notice = Some("Context compacted".to_string());
            } else if let Some(error) = error {
                self.last_notice = Some(error);
            }
            return;
        }

        if let Some(cached) = self.cached_sessions.get_mut(&session_id)
            && compacted
        {
            cached.context_manager.summary = summary.clone();
            cached.context_manager.retained_from = retained_from;
            cached
                .conversation
                .set_context_state(summary.clone(), retained_from);
            if let Err(error) = self.store.update_session_context_state(
                session_id,
                summary.as_deref(),
                retained_from,
            ) {
                log::warn!("failed to persist compacted context state: {}", error);
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
