use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use image::ImageEncoder;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Position, Rect},
};
use std::{
    env, io,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use uuid::Uuid;

mod at_mention;
mod connect;
mod diff_render;
mod mcp_panel;
mod model_panel;
mod mouse_selection;
mod permission;
mod question;
mod render;
mod render_chat;
mod render_dialog;
mod session_panel;
mod subagent;
mod theme_panel;
mod undo;

use crate::{
    app::at_mention::{AtMentionKind, AtMentionState, current_at_fragment},
    app::mcp_panel::McpPanelState,
    app::model_panel::ModelPanelState,
    app::mouse_selection::{ClipboardLease, MouseSelectionState},
    app::permission::{
        PendingToolExecution, PermissionDialogState, RunningSubagentExecution, RunningToolExecution,
    },
    app::question::QuestionDialogState,
    app::session_panel::SessionPanelState,
    app::theme_panel::ThemePanelState,
    commands::{CommandAction, CommandPaletteState, CommandRegistry},
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    input::Composer,
    instructions,
    llm::LlmClient,
    mcp::McpManager,
    prompts::SessionMode,
    provider_setup::ConnectDialog,
    session::{AssistantTurn, BackendEvent, Conversation, Message, MessageAttachment, MessageRole},
    storage::SessionStore,
    theme::{ThemeManager, ThemeName},
    tooling::ToolRegistry,
};

const INIT_COMMAND: &str = r#"Create or update `AGENTS.md` for this repository.

The goal is a compact instruction file that helps future OpenCode sessions avoid mistakes and ramp up quickly. Every line should answer: "Would an agent likely miss this without help?" If not, leave it out.

User-provided focus or constraints (honor these):
$ARGUMENTS

## How to investigate

Read the highest-value sources first:
- `README*`, root manifests, workspace config, lockfiles
- build, test, lint, formatter, typecheck, and codegen config
- CI workflows and pre-commit / task runner config
- existing instruction files (`AGENTS.md`, `CLAUDE.md`, `.cursor/rules/`, `.cursorrules`, `.github/copilot-instructions.md`)
- repo-local OpenCode config such as `opencode.json`

If architecture is still unclear after reading config and docs, inspect a small number of representative code files to find the real entrypoints, package boundaries, and execution flow. Prefer reading the files that explain how the system is wired together over random leaf files.

Prefer executable sources of truth over prose. If docs conflict with config or scripts, trust the executable source and only keep what you can verify.

## What to extract

Look for the highest-signal facts for an agent working in this repo:
- exact developer commands, especially non-obvious ones
- how to run a single test, a single package, or a focused verification step
- required command order when it matters, such as `lint -> typecheck -> test`
- monorepo or multi-package boundaries, ownership of major directories, and the real app/library entrypoints
- framework or toolchain quirks: generated code, migrations, codegen, build artifacts, special env loading, dev servers, infra deploy flow
- repo-specific style or workflow conventions that differ from defaults
- testing quirks: fixtures, integration test prerequisites, snapshot workflows, required services, flaky or expensive suites
- important constraints from existing instruction files worth preserving

Good `AGENTS.md` content is usually hard-earned context that took reading multiple files to infer.

## Questions

Only ask the user questions if the repo cannot answer something important. Use the `question` tool for one short batch at most.

Good questions:
- undocumented team conventions
- branch / PR / release expectations
- missing setup or test prerequisites that are known but not written down

Do not ask about anything the repo already makes clear.

## Writing rules

Include only high-signal, repo-specific guidance such as:
- exact commands and shortcuts the agent would otherwise guess wrong
- architecture notes that are not obvious from filenames alone
- conventions that differ from language or framework defaults
- setup requirements, environment quirks, and operational gotchas
- references to existing instruction sources that matter

Exclude:
- generic software advice
- long tutorials or exhaustive file trees
- obvious language conventions
- speculative claims or anything you could not verify
- content better stored in another file referenced via `opencode.json` `instructions`

When in doubt, omit.

Prefer short sections and bullets. If the repo is simple, keep the file simple. If the repo is large, summarize the few structural facts that actually change how an agent should work.

If `AGENTS.md` already exists at `${path}`, improve it in place rather than rewriting blindly. Preserve verified useful guidance, delete fluff or stale claims, and reconcile it with the current codebase."#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Welcome,
    Chat,
}

#[derive(Clone, Debug)]
struct CachedSessionRuntime {
    conversation: Conversation,
    active_model: ActiveModel,
    context_manager: ContextManager,
    pending_tool_execution: Option<PendingToolExecution>,
    permission_dialog: Option<PermissionDialogState>,
    question_dialog: Option<QuestionDialogState>,
    running_tool_execution: Option<RunningToolExecution>,
    running_subagent_executions: Vec<RunningSubagentExecution>,
    pending_request: bool,
    active_request_id: u64,
    abort_confirmation_deadline: Option<Instant>,
    retrying_hint: Option<(u32, u32, String, Option<u32>)>,
    message_scroll_offset: usize,
    message_follow_tail: bool,
    message_viewport_lines: usize,
    message_total_lines: usize,
    context_usage: Option<(u32, u32, u32)>,
}

#[derive(Clone, Debug)]
struct UiStateSnapshot {
    screen: Screen,
    connect_dialog: Option<ConnectDialog>,
    theme_panel: Option<ThemePanelState>,
    model_panel: Option<ModelPanelState>,
    session_panel: Option<SessionPanelState>,
    mcp_panel: Option<McpPanelState>,
    at_mention: AtMentionState,
    command_palette: CommandPaletteState,
    leader_key_pending: bool,
    composer: Composer,
    draft_attachments: Vec<MessageAttachment>,
    last_notice: Option<String>,
    mouse_selection: MouseSelectionState,
}

struct App {
    should_quit: bool,
    screen: Screen,
    workspace_root: PathBuf,
    paths: ConfigPaths,
    config: AppConfig,
    auth: AuthStore,
    store: SessionStore,
    llm: LlmClient,
    theme: ThemeManager,
    mode: SessionMode,
    active_model: ActiveModel,
    conversation: Conversation,
    context_manager: ContextManager,
    tools: ToolRegistry,
    commands: CommandRegistry,
    command_palette: CommandPaletteState,
    connect_dialog: Option<ConnectDialog>,
    theme_panel: Option<ThemePanelState>,
    model_panel: Option<ModelPanelState>,
    session_panel: Option<SessionPanelState>,
    mcp_panel: Option<McpPanelState>,
    at_mention: AtMentionState,
    pending_tool_execution: Option<PendingToolExecution>,
    permission_dialog: Option<PermissionDialogState>,
    question_dialog: Option<QuestionDialogState>,
    running_tool_execution: Option<RunningToolExecution>,
    running_subagent_executions: Vec<RunningSubagentExecution>,
    pending_assistant_turns: std::collections::HashSet<Uuid>,
    cached_sessions: std::collections::HashMap<Uuid, CachedSessionRuntime>,
    compacting_sessions: std::collections::HashSet<Uuid>,
    leader_key_pending: bool,
    composer: Composer,
    draft_attachments: Vec<MessageAttachment>,
    pending_request: bool,
    active_request_id: u64,
    abort_confirmation_deadline: Option<Instant>,
    last_notice: Option<String>,
    mouse_selection: MouseSelectionState,
    retrying_hint: Option<(u32, u32, String, Option<u32>)>, // (attempt, max, reason, retry_after_secs)
    message_scroll_offset: usize,
    message_follow_tail: bool,
    message_viewport_lines: usize,
    message_total_lines: usize,
    message_content_area: Option<Rect>,
    sidebar_area: Option<Rect>,
    selection_clipboard_lease: Option<ClipboardLease>,
    backend_tx: UnboundedSender<BackendEvent>,
    backend_rx: UnboundedReceiver<BackendEvent>,
    loading_frame: usize,
    context_usage: Option<(u32, u32, u32)>, // (input_tokens, output_tokens, total_tokens)
}

pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    let mut app = App::new()?;
    app.run(&runtime)
}

impl App {
    fn new() -> Result<Self> {
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

        Ok(Self {
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
            running_tool_execution: None,
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
            mouse_selection: MouseSelectionState::default(),
            retrying_hint,
            message_scroll_offset: 0,
            message_follow_tail: true,
            message_viewport_lines: 0,
            message_total_lines: 0,
            message_content_area: None,
            sidebar_area: None,
            selection_clipboard_lease: None,
            backend_tx,
            backend_rx,
            loading_frame: 0,
            context_usage: None,
        })
    }

    fn run(&mut self, runtime: &Runtime) -> Result<()> {
        runtime.block_on(self.refresh_mcp_tools())?;
        let _session = TerminalSession::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;

        loop {
            self.process_backend_events(runtime)?;
            self.update_mouse_selection_auto_scroll();
            terminal
                .draw(|frame| self.render(frame))
                .context("failed to render frame")?;

            if self.should_quit {
                break;
            }

            if event::poll(Duration::from_millis(50)).context("failed to poll terminal events")? {
                let event = event::read().context("failed to read terminal event")?;
                self.handle_event(event, runtime)?;
            }

            if self.should_quit {
                break;
            }
        }

        terminal.show_cursor().ok();
        Ok(())
    }

    fn cache_active_session_runtime(&mut self) {
        let session_id = self.conversation.session_id;
        let cached = CachedSessionRuntime {
            conversation: self.conversation.clone(),
            active_model: self.active_model.clone(),
            context_manager: self.context_manager.clone(),
            pending_tool_execution: self.pending_tool_execution.clone(),
            permission_dialog: self.permission_dialog.clone(),
            question_dialog: self.question_dialog.clone(),
            running_tool_execution: self.running_tool_execution.clone(),
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

    fn capture_ui_snapshot(&self) -> UiStateSnapshot {
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
            mouse_selection: self.mouse_selection.clone(),
        }
    }

    fn restore_ui_snapshot(&mut self, snapshot: UiStateSnapshot) {
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
        self.mouse_selection = snapshot.mouse_selection;
    }

    fn with_temporary_session_context<F>(&mut self, session_id: Uuid, operation: F) -> Result<()>
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

    fn restore_cached_session_runtime(&mut self, cached: CachedSessionRuntime) {
        self.conversation = cached.conversation;
        self.active_model = cached.active_model;
        self.context_manager = cached.context_manager;
        self.pending_tool_execution = cached.pending_tool_execution;
        self.permission_dialog = cached.permission_dialog;
        self.question_dialog = cached.question_dialog;
        self.running_tool_execution = cached.running_tool_execution;
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

    fn reset_active_runtime(&mut self) {
        self.context_manager = ContextManager::new();
        self.pending_tool_execution = None;
        self.permission_dialog = None;
        self.question_dialog = None;
        self.running_tool_execution = None;
        self.running_subagent_executions.clear();
        self.pending_request = false;
        self.abort_confirmation_deadline = None;
        self.retrying_hint = None;
        self.context_usage = None;
        self.scroll_messages_to_bottom();
    }

    fn restore_or_load_session(
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

    fn load_session_runtime_from_store(
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
            running_tool_execution: None,
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

    fn schedule_context_compaction_for_session(&mut self, session_id: Uuid, runtime: &Runtime) {
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

    fn apply_context_compaction(
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

    fn background_running_count(&self) -> usize {
        self.cached_sessions
            .values()
            .filter(|cached| cached.pending_request)
            .count()
    }

    fn background_waiting_question_count(&self) -> usize {
        self.cached_sessions
            .values()
            .filter(|cached| cached.question_dialog.is_some())
            .count()
    }

    fn handle_event(&mut self, event: Event, runtime: &Runtime) -> Result<()> {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key_event(key, runtime)?;
            }
            Event::Paste(text) => {
                self.handle_text_paste(&text)?;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
            }
            Event::Resize(_, _) => {
                self.clear_mouse_selection();
                self.message_content_area = None;
                self.sidebar_area = None;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                if let Some(bounds) = self.selection_bounds_for_position(position) {
                    self.mouse_selection
                        .press_with_bounds(position, Some(bounds));
                } else {
                    self.clear_mouse_selection();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.mouse_selection
                    .drag(Position::new(mouse.column, mouse.row));
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.mouse_selection
                    .release(Position::new(mouse.column, mouse.row));
            }
            MouseEventKind::ScrollUp => {
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    self.scroll_messages_up(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    self.scroll_messages_down(3);
                }
            }
            _ => {}
        }
    }

    fn update_mouse_selection_auto_scroll(&mut self) {
        if !self.mouse_selection.is_dragging() || !self.can_scroll_conversation() {
            return;
        }

        let Some(area) = self.message_content_area else {
            return;
        };

        let Some(pointer) = self.mouse_selection.pointer() else {
            return;
        };

        let left = area.x;
        let right = area.x.saturating_add(area.width);
        if pointer.x < left || pointer.x >= right {
            return;
        }

        let top_threshold = area.y.saturating_add(1);
        let bottom_threshold = area.y.saturating_add(area.height.saturating_sub(2));

        if pointer.y <= top_threshold {
            self.scroll_messages_up_internal(3);
        } else if pointer.y >= bottom_threshold {
            self.scroll_messages_down_internal(3);
        }
    }

    fn clear_mouse_selection(&mut self) {
        self.mouse_selection.clear();
    }

    fn selection_bounds_for_position(&self, position: Position) -> Option<Rect> {
        if let Some(area) = self.message_content_area
            && area.contains(position)
        {
            return Some(area);
        }

        if let Some(area) = self.sidebar_area
            && area.contains(position)
        {
            return Some(area);
        }

        None
    }

    pub(crate) fn register_selection_region(&self, _area: Rect) {}

    fn can_scroll_conversation(&self) -> bool {
        self.screen == Screen::Chat
            && self.permission_dialog.is_none()
            && self.question_dialog.is_none()
            && self.connect_dialog.is_none()
            && self.theme_panel.is_none()
            && self.model_panel.is_none()
            && self.mcp_panel.is_none()
            && !self.command_palette.visible
    }

    fn scroll_messages_to_bottom(&mut self) {
        self.clear_mouse_selection();
        self.message_scroll_offset = 0;
        self.message_follow_tail = true;
    }

    fn message_scroll_max(&self) -> usize {
        self.message_total_lines
            .saturating_sub(self.message_viewport_lines)
    }

    fn message_scroll_page(&self) -> usize {
        self.message_viewport_lines.saturating_sub(1).max(1)
    }

    fn scroll_messages_up(&mut self, lines: usize) {
        self.clear_mouse_selection();
        self.scroll_messages_up_internal(lines);
    }

    fn scroll_messages_up_internal(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = current.saturating_sub(lines);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
    }

    fn scroll_messages_down(&mut self, lines: usize) {
        self.clear_mouse_selection();
        self.scroll_messages_down_internal(lines);
    }

    fn scroll_messages_down_internal(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = current.saturating_add(lines).min(max_scroll);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
    }

    fn handle_message_scroll_key(&mut self, key: KeyEvent) -> bool {
        if !self.can_scroll_conversation() {
            return false;
        }

        match key.code {
            KeyCode::PageUp => {
                self.scroll_messages_up(self.message_scroll_page());
                true
            }
            KeyCode::PageDown => {
                self.scroll_messages_down(self.message_scroll_page());
                true
            }
            _ => false,
        }
    }

    fn handle_request_abort_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.code != KeyCode::Esc || !self.pending_request {
            return Ok(false);
        }

        if self
            .abort_confirmation_deadline
            .is_some_and(|deadline| deadline > Instant::now())
        {
            self.abort_current_request();
            return Ok(true);
        }

        self.abort_confirmation_deadline = Some(Instant::now() + Duration::from_secs(3));
        self.last_notice =
            Some("Press Esc again within 3 seconds to stop the current request".to_string());
        Ok(true)
    }

    fn is_active_request(&self, request_id: u64) -> bool {
        request_id == self.active_request_id
    }

    fn cancel_running_subagents(&mut self) {
        for execution in &self.running_subagent_executions {
            execution.cancel_requested.store(true, Ordering::SeqCst);
        }
        self.running_subagent_executions.clear();
    }

    fn abort_current_request(&mut self) {
        self.active_request_id = self.active_request_id.wrapping_add(1);
        self.abort_confirmation_deadline = None;
        self.pending_request = false;
        self.pending_tool_execution = None;
        self.permission_dialog = None;
        self.question_dialog = None;
        self.cancel_running_subagents();

        if let Some(running) = self.running_tool_execution.take() {
            running.cancel_requested.store(true, Ordering::SeqCst);
        }

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.role = MessageRole::Error;
            message.streaming = false;
            message.content = "Request cancelled".to_string();
            let persisted = message.clone();
            let _ = self
                .store
                .append_message(self.conversation.session_id, &persisted);
        }

        self.last_notice = Some("Request cancelled".to_string());
    }

    fn handle_theme_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.theme_panel {
            match key.code {
                KeyCode::Up => {
                    let previous_theme = panel.preview_theme;
                    panel.move_up();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                    }
                }
                KeyCode::Down => {
                    let previous_theme = panel.preview_theme;
                    panel.move_down();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                    }
                }
                KeyCode::Enter => {
                    let _ = self.close_theme_panel(true);
                }
                KeyCode::Esc => {
                    let _ = self.close_theme_panel(false);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_model_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(panel) = self.model_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                let items = self.model_panel_items();
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let items = self.model_panel_items();
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let items = self.model_panel_items();
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.switch_model(Some(&summary.label()))?;
                    self.close_model_panel();
                }
            }
            KeyCode::Esc => {
                self.close_model_panel();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let items = self.model_panel_items();
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.close_model_panel();
                    self.begin_provider_edit_for_model(summary.provider_id, summary.model_id)?;
                }
            }
            KeyCode::Tab => {}
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    let items = self.model_panel_items();
                    let mut next_panel = panel;
                    next_panel.reset_selection(&items);
                    self.model_panel = Some(next_panel);
                }
            }
        }

        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<()> {
        if self.leader_key_pending {
            self.leader_key_pending = false;
            let _ = self.handle_leader_key(key, runtime)?;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('x')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.leader_key_pending = true;
            self.last_notice =
                Some("Leader key active: use arrows to navigate subagents".to_string());
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            if !self.composer.text().is_empty() {
                self.composer.clear();
                self.at_mention.clear();
                self.command_palette
                    .sync(self.composer.text(), &self.commands);
                self.last_notice = Some("Input cleared".to_string());
            }
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('d')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        if self.permission_dialog.is_some() {
            return self.handle_permission_dialog_key(key, runtime);
        }

        if self.question_dialog.is_some() {
            return self.handle_question_dialog_key(key, runtime);
        }

        if let Some(dialog) = self.connect_dialog.clone() {
            self.handle_connect_dialog_key(key, dialog)?;
            return Ok(());
        }

        if self.theme_panel.is_some() {
            return self.handle_theme_panel_key(key);
        }

        if self.mcp_panel.is_some() {
            return self.handle_mcp_panel_key(key, runtime);
        }

        if self.model_panel.is_some() {
            return self.handle_model_panel_key(key);
        }

        if self.session_panel.is_some() {
            return self.handle_session_panel_key(key, runtime);
        }

        if self.handle_request_abort_key(key)? {
            return Ok(());
        }

        if matches!(key.code, KeyCode::Esc) && self.mouse_selection.has_selection() {
            self.clear_mouse_selection();
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('v'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.handle_clipboard_paste()?;
            return Ok(());
        }

        if !self.command_palette.visible && key.code == KeyCode::Tab {
            self.mode = self.mode.toggle();
            self.last_notice = Some(format!("Mode switched to {}", self.mode.as_str()));
            return Ok(());
        }

        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc => {
                    self.command_palette.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.command_palette.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.command_palette.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab => {
                    if let Some(completion) = self.command_palette.completion() {
                        self.composer.set_text(completion);
                    }
                    self.command_palette
                        .sync(self.composer.text(), &self.commands);
                    return Ok(());
                }
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(selected) = self.command_palette.selected() {
                        let command_line = format!("/{}", selected.spec.name);
                        self.composer.remember_submission(&command_line);
                        self.composer.clear();
                        self.command_palette.clear();
                        self.execute_command_line(&command_line, runtime)?;
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        if !self.command_palette.visible && self.handle_message_scroll_key(key) {
            return Ok(());
        }

        if self.at_mention.visible {
            match key.code {
                KeyCode::Esc => {
                    self.at_mention.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.at_mention.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.at_mention.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_at_mention();
                    return Ok(());
                }
                _ => {}
            }
        }

        if let Some(submission) = self.composer.handle_key_with_history(key, true) {
            self.handle_submission(submission, runtime)?;
            self.at_mention.clear();
        } else {
            if key.code == KeyCode::Enter && !self.draft_attachments.is_empty() {
                self.handle_submission(String::new(), runtime)?;
                self.at_mention.clear();
                self.command_palette
                    .sync(self.composer.text(), &self.commands);
                return Ok(());
            }
            self.refresh_at_mention_state();
        }

        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    fn handle_leader_key(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<bool> {
        let current_session_id = self.conversation.session_id;
        let parent_session_id = self
            .conversation
            .parent_session_id
            .unwrap_or(current_session_id);

        match key.code {
            KeyCode::Up => {
                if parent_session_id != current_session_id {
                    self.switch_session(parent_session_id, runtime)?;
                    return Ok(true);
                }
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Left => {
                let children = self.store.load_child_sessions(parent_session_id)?;
                if children.is_empty() {
                    return Ok(false);
                }

                let step = if matches!(key.code, KeyCode::Left) {
                    -1
                } else {
                    1
                };
                let index = children
                    .iter()
                    .position(|session| session.session_id == current_session_id)
                    .unwrap_or(usize::MAX);
                let next_index = if index == usize::MAX {
                    0
                } else {
                    (index as isize + step).rem_euclid(children.len() as isize) as usize
                };

                if let Some(target) = children.get(next_index) {
                    self.switch_session(target.session_id, runtime)?;
                    return Ok(true);
                }
            }
            _ => {}
        }

        Ok(false)
    }

    fn handle_submission(&mut self, submission: String, runtime: &Runtime) -> Result<()> {
        let trimmed = submission.trim();
        if trimmed.starts_with('/') {
            self.execute_command_line(trimmed, runtime)?;
            self.at_mention.clear();
            self.draft_attachments.clear();
        } else {
            self.submit_prompt(submission, runtime)?;
        }

        Ok(())
    }

    fn handle_text_paste(&mut self, text: &str) -> Result<()> {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.composer.insert_str(&normalized);
        self.refresh_at_mention_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    fn handle_clipboard_paste(&mut self) -> Result<()> {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(error) => {
                self.last_notice = Some(format!("Clipboard unavailable: {error}"));
                return Ok(());
            }
        };

        if let Ok(text) = clipboard.get_text()
            && !text.is_empty()
        {
            return self.handle_text_paste(&text);
        }

        let image = match clipboard.get_image() {
            Ok(image) => image,
            Err(_) => {
                self.last_notice =
                    Some("Clipboard does not contain pasteable text or image".to_string());
                return Ok(());
            }
        };

        if !self.active_model.supports_images {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let data_url = match png_data_url_from_clipboard_image(image) {
            Ok(value) => value,
            Err(error) => {
                self.last_notice = Some(format!("Failed to decode clipboard image: {error}"));
                return Ok(());
            }
        };

        self.draft_attachments.push(MessageAttachment::Image {
            filename: format!("pasted-image-{}.png", Uuid::new_v4()),
            mime: "image/png".to_string(),
            data_url,
        });
        self.last_notice = Some("Image pasted into draft".to_string());
        Ok(())
    }

    fn refresh_at_mention_state(&mut self) {
        if self.command_palette.visible
            || self.connect_dialog.is_some()
            || self.theme_panel.is_some()
            || self.model_panel.is_some()
            || self.session_panel.is_some()
            || self.mcp_panel.is_some()
            || self.question_dialog.is_some()
        {
            self.at_mention.clear();
            return;
        }

        let text = self.composer.text();
        let cursor = self.composer.cursor();
        self.at_mention
            .sync(self.workspace_root.as_path(), text, cursor);
    }

    fn accept_at_mention(&mut self) {
        let Some((start, _query)) =
            current_at_fragment(self.composer.text(), self.composer.cursor())
        else {
            self.at_mention.clear();
            return;
        };

        let Some(selection) = self.at_mention.selected().cloned() else {
            self.at_mention.clear();
            return;
        };

        let replacement = match selection.kind {
            AtMentionKind::Directory => format!("@{}/", selection.path.trim_end_matches('/')),
            _ => format!("@{}", selection.path),
        };
        self.composer
            .replace_range(start, self.composer.cursor(), &replacement);
        self.at_mention.clear();
        self.refresh_at_mention_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    fn execute_command_line(&mut self, line: &str, runtime: &Runtime) -> Result<()> {
        let Some((name, args)) = self.commands.parse_invocation(line) else {
            self.last_notice = Some("Invalid command".to_string());
            return Ok(());
        };

        let Some(spec) = self.commands.command(&name).cloned() else {
            self.last_notice = Some(format!("Unknown command '/{name}'"));
            return Ok(());
        };

        self.run_command(spec.name, spec.action, &args, runtime)?;
        self.commands.mark_used(spec.name);
        Ok(())
    }

    fn run_command(
        &mut self,
        _command_name: &str,
        action: CommandAction,
        args: &[String],
        runtime: &Runtime,
    ) -> Result<()> {
        if self.pending_request {
            match action {
                CommandAction::Theme
                | CommandAction::Quit
                | CommandAction::Undo
                | CommandAction::Redo
                | CommandAction::Session
                | CommandAction::Clear => {}
                _ => {
                    self.last_notice = Some(
                        "A response is still streaming. Wait for it to finish before changing sessions.".to_string(),
                    );
                    return Ok(());
                }
            }
        }

        match action {
            CommandAction::Connect => {
                if !args.is_empty() {
                    self.last_notice = Some("Ignoring arguments to /connect".to_string());
                }
                self.open_connect_dialog()?;
            }
            CommandAction::Mcp => match args.first().map(|value| value.as_str()) {
                Some("add") | Some("new") | Some("create") => {
                    self.open_mcp_panel(String::new());
                    self.open_new_mcp_server_editor(String::new());
                }
                Some("edit") => {
                    if let Some(server_name) = args.get(1) {
                        self.open_mcp_panel(server_name.clone());
                        self.open_existing_mcp_server_editor(String::new(), server_name.clone())?;
                    } else {
                        self.last_notice = Some("Usage: /mcp edit <server-name>".to_string());
                    }
                }
                Some("remove") | Some("delete") | Some("rm") => {
                    if let Some(server_name) = args.get(1) {
                        if let Err(error) = self.remove_mcp_server_from_editor(runtime, server_name)
                        {
                            self.last_notice = Some(error.to_string());
                        }
                    } else {
                        self.last_notice = Some("Usage: /mcp remove <server-name>".to_string());
                    }
                }
                _ => {
                    self.open_mcp_panel(args.join(" "));
                }
            },
            CommandAction::Model => {
                self.open_model_panel(args.join(" "));
            }
            CommandAction::Session => {
                self.open_session_panel(args.join(" "))?;
            }
            CommandAction::Clear => {
                self.start_new_session()?;
            }
            CommandAction::Undo => {
                self.undo_last_user_message()?;
            }
            CommandAction::Redo => {
                self.redo_last_user_message()?;
            }
            CommandAction::Theme => {
                self.apply_theme_command(args)?;
            }
            CommandAction::Quit => {
                self.should_quit = true;
            }
            CommandAction::Init => {
                self.composer.set_text(INIT_COMMAND.to_string());
                self.last_notice = Some("Init prompt loaded".to_string());
            }
        }

        Ok(())
    }

    fn apply_theme_command(&mut self, args: &[String]) -> Result<()> {
        let direct_theme = args.first().and_then(|v| ThemeName::parse(v));

        if let Some(theme) = direct_theme {
            self.apply_theme(theme)?;
            Ok(())
        } else {
            self.open_theme_panel();
            Ok(())
        }
    }

    fn open_theme_panel(&mut self) {
        self.mcp_panel = None;
        self.theme_panel = Some(ThemePanelState::new(self.theme.palette().name));
    }

    fn open_model_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.mcp_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search connected models by provider or model name");
        self.composer.set_text(initial_query);

        let mut panel = ModelPanelState::new();
        let items = self.model_panel_items();
        panel.reset_selection(&items);
        self.model_panel = Some(panel);
    }

    fn close_model_panel(&mut self) {
        self.model_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
    }

    async fn refresh_mcp_tools(&self) -> Result<()> {
        self.tools.refresh_mcp_tools().await
    }

    fn close_theme_panel(&mut self, apply: bool) -> Result<()> {
        if let Some(panel) = self.theme_panel.take() {
            if apply {
                self.apply_theme(panel.preview_theme)?;
            } else {
                self.theme.set_mode(panel.original_theme);
            }
        }
        Ok(())
    }

    fn apply_theme(&mut self, theme: ThemeName) -> Result<()> {
        self.theme.set_mode(theme);
        self.config.set_theme(theme);
        self.config.save(&self.paths)?;
        self.last_notice = Some(format!("Theme switched to {}", self.theme.name()));
        Ok(())
    }

    fn switch_model(&mut self, selector: Option<&str>) -> Result<()> {
        let model = self.config.resolve_model(&self.auth, selector)?;
        self.active_model = model.clone();
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

    fn start_new_session(&mut self) -> Result<()> {
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
        self.reset_active_runtime();
        self.active_request_id = 0;
        self.screen = Screen::Welcome;
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
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

    fn submit_prompt(&mut self, prompt: String, runtime: &Runtime) -> Result<()> {
        let prompt = prompt.trim().to_string();
        if self.pending_request {
            self.last_notice = Some("A response is already in progress".to_string());
            return Ok(());
        }

        if prompt.is_empty() && self.draft_attachments.is_empty() {
            return Ok(());
        }

        if self.screen == Screen::Welcome {
            let session_exists = self
                .store
                .load_session_record(self.conversation.session_id)?
                .is_some();

            if !session_exists {
                let session_id = Uuid::new_v4();
                self.conversation.session_id = session_id;
                self.store.create_session(
                    session_id,
                    self.workspace_root.as_path(),
                    &self.active_model.provider_id,
                    &self.active_model.provider_display_name,
                    &self.active_model.model_id,
                    &self.active_model.display_name,
                    "Untitled session",
                )?;
            }
            self.context_manager = ContextManager::new();
            self.pending_tool_execution = None;
            self.permission_dialog = None;
            self.question_dialog = None;
            self.running_tool_execution = None;
            self.abort_confirmation_deadline = None;
            self.active_request_id = self.active_request_id.wrapping_add(1);
        }

        self.screen = Screen::Chat;
        self.command_palette.clear();
        self.connect_dialog = None;

        if self.conversation.is_reverted() {
            self.discard_reverted_branch()?;
            self.context_manager = ContextManager::new();
        }

        let attachments = self.build_prompt_attachments(&prompt)?;
        if attachments.iter().any(MessageAttachment::is_image) && !self.active_model.supports_images
        {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let mut user_message = Message::new(MessageRole::User, prompt.clone());
        user_message.attachments = attachments;
        self.conversation.push(user_message.clone());
        self.store
            .append_message(self.conversation.session_id, &user_message)?;

        self.draft_attachments.clear();

        if let Err(error) = self.capture_prompt_snapshot(user_message.id) {
            self.last_notice = Some(format!("Workspace snapshot unavailable: {error}"));
        }

        if self.conversation.messages.len() == 1 || self.conversation.title == "Untitled session" {
            self.conversation.update_title_from_prompt(&prompt);
            self.store
                .update_session_title(self.conversation.session_id, &self.conversation.title)?;
        }

        self.scroll_messages_to_bottom();

        self.schedule_context_compaction_for_session(self.conversation.session_id, runtime);

        self.start_assistant_turn(runtime)
    }

    fn build_prompt_attachments(&self, prompt: &str) -> Result<Vec<MessageAttachment>> {
        let mut attachments = Vec::new();
        let mut seen_paths = std::collections::BTreeSet::new();

        for path in self.inline_file_references(prompt) {
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            let absolute = self.resolve_workspace_path(&path);
            match self.build_attachment_for_path(&path, &absolute)? {
                Some(attachment) => attachments.push(attachment),
                None => continue,
            }
        }

        attachments.extend(self.draft_attachments.iter().cloned());
        Ok(attachments)
    }

    fn build_attachment_for_path(
        &self,
        path: &str,
        absolute: &Path,
    ) -> Result<Option<MessageAttachment>> {
        let metadata = match std::fs::metadata(absolute) {
            Ok(metadata) => metadata,
            Err(_error) => return Ok(None),
        };

        if metadata.is_dir() {
            let tree = build_directory_tree(absolute, 2, 80)?;
            return Ok(Some(MessageAttachment::DirectoryReference {
                path: path.trim_end_matches(['/', '\\']).to_string(),
                tree,
            }));
        }

        if let Some(mime) = image_mime_from_path(absolute) {
            let bytes = std::fs::read(absolute)?;
            let filename = absolute
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_string();
            let data_url = format!("data:{mime};base64,{}", BASE64_STANDARD.encode(bytes));
            return Ok(Some(MessageAttachment::Image {
                filename,
                mime: mime.to_string(),
                data_url,
            }));
        }

        let content = match std::fs::read_to_string(absolute) {
            Ok(content) => content,
            Err(_error) => return Ok(None),
        };

        Ok(Some(MessageAttachment::FileReference {
            path: path.to_string(),
            content,
        }))
    }

    fn inline_file_references(&self, prompt: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        for token in prompt.split_whitespace() {
            let Some(path) = token.strip_prefix('@') else {
                continue;
            };
            let path = path.trim_matches(|ch: char| {
                matches!(ch, ',' | '.' | ';' | ':' | ')' | ']' | '"' | '\'')
            });
            if path.is_empty() {
                continue;
            }
            if !seen.insert(path.to_string()) {
                continue;
            }
            paths.push(path.to_string());
        }

        paths
    }

    fn resolve_workspace_path(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            return candidate.to_path_buf();
        }
        self.workspace_root.join(path)
    }

    fn start_assistant_turn(&mut self, runtime: &Runtime) -> Result<()> {
        crate::log_info!(
            "start_assistant_turn: session_id={}, message_count={}",
            self.conversation.session_id,
            self.conversation.messages.len()
        );
        self.refresh_tools();
        self.pending_request = true;
        self.abort_confirmation_deadline = None;
        self.active_request_id = self.active_request_id.wrapping_add(1);
        let request_id = self.active_request_id;
        crate::log_info!("start_assistant_turn: new request_id={}", request_id);
        self.last_notice = Some(match self.mode {
            SessionMode::Plan => "Planning...".to_string(),
            SessionMode::Build => "Thinking...".to_string(),
        });

        let llm = self.llm.clone();
        let (system_prompt, instruction_sources) = self.compose_system_prompt();
        let mut model = self.active_model.clone();
        model.system_prompt = system_prompt;

        let _ = self.push_loaded_instruction_sources_message(&instruction_sources);

        let assistant_message = Message::streaming(MessageRole::Assistant, "");
        self.conversation.push(assistant_message);

        let messages = self.conversation.visible_messages().to_vec();
        let tools = self.tools.available_definitions(self.mode);
        let tx = self.backend_tx.clone();
        let session_id = self.conversation.session_id;

        runtime.spawn(async move {
            llm.stream_chat(session_id, request_id, model, messages, tools, tx)
                .await;
        });

        Ok(())
    }

    fn refresh_tools(&mut self) {
        let mcp = self.tools.mcp_manager();
        self.tools = ToolRegistry::new(
            self.workspace_root.clone(),
            self.paths.config_dir.clone(),
            self.config.skills.clone(),
            mcp,
            self.config.permissions.clone(),
        );
    }

    fn compose_system_prompt(&self) -> (String, Vec<String>) {
        let base_prompt = self.active_model.system_prompt.trim();
        let mode_reminder = self.mode.reminder();
        let (instruction_prompt, sources) = instructions::system_prompt_and_sources(
            &self.workspace_root,
            &self.paths.config_dir,
            &self.config.instructions,
        )
        .unwrap_or_default();

        let mut prompt = String::new();
        if !base_prompt.is_empty() {
            prompt.push_str(base_prompt);
        }
        if !instruction_prompt.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&instruction_prompt);
        }
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(mode_reminder);

        (prompt, sources)
    }

    fn push_loaded_instruction_sources_message(&mut self, sources: &[String]) -> Result<()> {
        if sources.is_empty() {
            return Ok(());
        }

        let already_present = self.conversation.messages.iter().any(|message| {
            matches!(message.role, MessageRole::System)
                && message.content.starts_with("Loaded instruction sources:")
        });

        if already_present {
            return Ok(());
        }

        let display_sources: Vec<String> = sources
            .iter()
            .map(|source| self.display_instruction_source(source))
            .collect();
        let content = format!("Loaded instruction sources: {}", display_sources.join(", "));
        self.push_system_message(content)
    }

    fn display_instruction_source(&self, source: &str) -> String {
        if source.starts_with("http://") || source.starts_with("https://") {
            return source.to_string();
        }

        let path = Path::new(source);
        if path.is_absolute()
            && let Ok(rel) = path.strip_prefix(&self.workspace_root)
        {
            return rel.display().to_string();
        }

        source.to_string()
    }

    fn push_system_message(&mut self, content: impl Into<String>) -> Result<()> {
        self.push_message(MessageRole::System, content)
    }

    fn push_message(&mut self, role: MessageRole, content: impl Into<String>) -> Result<()> {
        let message = Message::new(role, content);
        self.conversation.push(message.clone());
        self.store
            .append_message(self.conversation.session_id, &message)?;
        self.screen = Screen::Chat;
        Ok(())
    }

    fn process_backend_events(&mut self, runtime: &Runtime) -> Result<()> {
        while let Ok(event) = self.backend_rx.try_recv() {
            self.handle_backend_event(event, runtime)?;
        }

        Ok(())
    }

    fn handle_backend_event(&mut self, event: BackendEvent, runtime: &Runtime) -> Result<()> {
        let session_id = event.session_id();
        let request_id = event.request_id();
        if session_id != self.conversation.session_id {
            return self.with_temporary_session_context(session_id, |app| {
                if let Some(request_id) = request_id {
                    app.prime_active_request(request_id);
                }
                app.handle_backend_event_for_active(event, runtime)
            });
        }

        if let Some(request_id) = request_id {
            self.prime_active_request(request_id);
        }

        self.handle_backend_event_for_active(event, runtime)
    }

    fn prime_active_request(&mut self, request_id: u64) {
        if self.active_request_id == 0 {
            self.active_request_id = request_id;
        }
    }

    fn handle_backend_event_for_active(
        &mut self,
        event: BackendEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        let event_type = match &event {
            BackendEvent::Delta { .. } => "Delta",
            BackendEvent::ReasoningDelta { .. } => "ReasoningDelta",
            BackendEvent::Finished { request_id, .. } => {
                crate::log_info!("handle_backend_event: Finished request_id={}", request_id);
                "Finished"
            }
            BackendEvent::Retrying { .. } => "Retrying",
            BackendEvent::Failed { request_id, .. } => {
                crate::log_info!("handle_backend_event: Failed request_id={}", request_id);
                "Failed"
            }
            BackendEvent::ToolCompleted { request_id, .. } => {
                crate::log_info!(
                    "handle_backend_event: ToolCompleted request_id={}",
                    request_id
                );
                "ToolCompleted"
            }
            BackendEvent::SubagentStatus { .. } => "SubagentStatus",
            BackendEvent::SubagentToolResult { .. } => "SubagentToolResult",
            BackendEvent::SubagentCompleted { .. } => "SubagentCompleted",
            BackendEvent::UsageStats { .. } => "UsageStats",
            BackendEvent::ContextCompacted { .. } => "ContextCompacted",
        };
        if event_type != "Delta"
            && event_type != "ReasoningDelta"
            && event_type != "UsageStats"
            && event_type != "SubagentStatus"
            && event_type != "SubagentToolResult"
        {
            crate::log_debug!("handle_backend_event: {}", event_type);
        }
        match event {
            BackendEvent::Delta {
                session_id: _,
                request_id,
                content,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.content.push_str(&content);
                }
            }
            BackendEvent::ReasoningDelta {
                session_id: _,
                request_id,
                content,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.reasoning.push_str(&content);
                }
            }
            BackendEvent::Finished {
                session_id: _,
                request_id,
                turn,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                self.finish_assistant_turn(turn, runtime)?;
            }
            BackendEvent::Retrying {
                session_id: _,
                request_id,
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                self.retrying_hint = Some((attempt, max_attempts, reason, retry_after_secs));
            }
            BackendEvent::Failed {
                session_id: _,
                request_id,
                error,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                self.pending_request = false;
                self.pending_tool_execution = None;
                self.permission_dialog = None;
                self.question_dialog = None;
                self.running_tool_execution = None;
                self.cancel_running_subagents();
                self.abort_confirmation_deadline = None;
                self.retrying_hint = None;

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.role = MessageRole::Error;
                    message.streaming = false;
                    message.content = format!("Request failed: {error}");
                    let persisted = message.clone();
                    self.store
                        .append_message(self.conversation.session_id, &persisted)?;
                    self.last_notice = Some(error);
                    return Ok(());
                }

                let message = Message::new(MessageRole::Error, format!("Request failed: {error}"));
                self.conversation.push(message.clone());
                self.store
                    .append_message(self.conversation.session_id, &message)?;
                self.last_notice = Some(error);
            }
            BackendEvent::ToolCompleted {
                session_id: _,
                request_id,
                tool_call,
                result,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                let Some(running) = self.running_tool_execution.take() else {
                    return Ok(());
                };

                if running.request_id != request_id || running.tool_call.id != tool_call.id {
                    self.running_tool_execution = Some(running);
                    return Ok(());
                }

                self.record_tool_result(tool_call, result)?;
                self.advance_pending_tool_execution();
                self.process_pending_tool_execution(runtime)?;
            }
            BackendEvent::SubagentStatus {
                session_id: _,
                request_id,
                child_session_id,
                status_text,
                current_tool_call,
                assistant_message,
                content_delta: _,
                reasoning_delta: _,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                if let Some(execution) = self
                    .running_subagent_executions
                    .iter_mut()
                    .find(|execution| execution.child_session_id == child_session_id)
                {
                    execution.status_text = status_text.clone();
                    execution.current_tool_call = current_tool_call;
                }

                if self.conversation.session_id == child_session_id {
                    let is_completed = status_text == "Completed";
                    self.pending_request = !is_completed;

                    if let Some(message) = assistant_message {
                        let existing_index = self.conversation.messages.iter().position(|m| {
                            matches!(m.role, MessageRole::Assistant) && m.id == message.id
                        });

                        if let Some(index) = existing_index {
                            let existing = &mut self.conversation.messages[index];
                            existing.content = message.content.clone();
                            existing.reasoning = message.reasoning.clone();
                            existing.tool_calls = message.tool_calls.clone();
                            existing.streaming = message.streaming;
                        } else {
                            self.conversation.messages.push(message.clone());
                        }
                    }
                }
            }
            BackendEvent::SubagentToolResult {
                session_id: _,
                request_id,
                child_session_id,
                message,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                if self.conversation.session_id == child_session_id {
                    let tool_call_id = message.tool_call_id.clone();
                    let already_exists = self.conversation.messages.iter().any(|m| {
                        matches!(m.role, MessageRole::Tool) && m.tool_call_id == tool_call_id
                    });

                    if !already_exists {
                        self.conversation.messages.push(message);
                    }
                }
            }
            BackendEvent::SubagentCompleted {
                session_id: _,
                request_id,
                tool_call,
                child_session_id,
                result,
            } => {
                crate::log_info!(
                    "SubagentCompleted: request_id={}, active_request_id={}, child_session_id={}, tool_call_id={}",
                    request_id,
                    self.active_request_id,
                    child_session_id,
                    tool_call.id
                );
                if !self.is_active_request(request_id) {
                    crate::log_warn!(
                        "SubagentCompleted ignored: request_id {} != active_request_id {}",
                        request_id,
                        self.active_request_id
                    );
                    return Ok(());
                }

                let execution_index =
                    self.running_subagent_executions
                        .iter()
                        .position(|execution| {
                            execution.request_id == request_id
                                && execution.child_session_id == child_session_id
                                && execution.tool_call.id == tool_call.id
                        });

                let Some(index) = execution_index else {
                    crate::log_warn!(
                        "SubagentCompleted: no matching running_subagent_execution found"
                    );
                    return Ok(());
                };

                let execution = self.running_subagent_executions.remove(index);
                let parent_session_id = execution.parent_session_id;
                crate::log_info!(
                    "Removed running_subagent_executions[{}], remaining count={}, parent_session_id={}",
                    index,
                    self.running_subagent_executions.len(),
                    parent_session_id
                );

                let is_on_parent_session = self.conversation.session_id == parent_session_id;
                crate::log_info!(
                    "SubagentCompleted: is_on_parent_session={}, current_session_id={}",
                    is_on_parent_session,
                    self.conversation.session_id
                );

                if is_on_parent_session {
                    self.record_tool_result(tool_call, result)?;
                    crate::log_info!(
                        "record_tool_result done, pending_tool_execution={}, running_subagent_executions={}",
                        self.pending_tool_execution.is_some(),
                        self.running_subagent_executions.len()
                    );

                    if self.pending_tool_execution.is_none()
                        && self.running_subagent_executions.is_empty()
                    {
                        crate::log_info!("SubagentCompleted: calling start_assistant_turn");
                        self.start_assistant_turn(runtime)?;
                    } else if !self.running_subagent_executions.is_empty() {
                        self.last_notice = Some(format!(
                            "Waiting for {} subagent(s)...",
                            self.running_subagent_executions.len()
                        ));
                    }
                } else {
                    crate::log_info!(
                        "SubagentCompleted: user switched away from parent session, writing to database directly"
                    );
                    self.store.append_tool_event(
                        parent_session_id,
                        &tool_call.name,
                        &tool_call.arguments,
                        &result.output,
                    )?;
                    let message = Message::tool_result(tool_call.id, tool_call.name, result);
                    self.store.append_message(parent_session_id, &message)?;
                    self.pending_assistant_turns.insert(parent_session_id);
                    crate::log_info!(
                        "SubagentCompleted: marked parent_session_id={} as pending assistant turn",
                        parent_session_id
                    );
                }
            }
            BackendEvent::UsageStats {
                session_id: _,
                request_id,
                input_tokens,
                output_tokens,
                total_tokens,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                self.context_usage = Some((input_tokens, output_tokens, total_tokens));
            }
            BackendEvent::ContextCompacted {
                session_id,
                compacted,
                summary,
                retained_from,
                error,
            } => {
                self.apply_context_compaction(session_id, compacted, summary, retained_from, error);
            }
        }

        Ok(())
    }

    fn finish_assistant_turn(&mut self, turn: AssistantTurn, runtime: &Runtime) -> Result<()> {
        crate::log_info!(
            "finish_assistant_turn: tool_calls_count={}, finish_reason={:?}",
            turn.tool_calls.len(),
            turn.finish_reason
        );
        let mut persisted_message = None;

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.content = turn.content.clone();
            message.reasoning = turn.reasoning.clone();
            message.tool_calls = turn.tool_calls.clone();
            message.streaming = false;

            if let Some((input_tokens, output_tokens, total_tokens)) = self.context_usage {
                message.input_tokens = Some(input_tokens);
                message.output_tokens = Some(output_tokens);
                message.total_tokens = Some(total_tokens);
            }

            persisted_message = Some(message.clone());
        }

        if let Some(message) = persisted_message {
            self.store
                .append_message(self.conversation.session_id, &message)?;
        }

        if !turn.tool_calls.is_empty() {
            let tool_names: Vec<_> = turn.tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            crate::log_info!(
                "finish_assistant_turn: calling begin_tool_execution for {:?}",
                tool_names
            );
            self.last_notice = Some(format!("Running {} tool call(s)...", turn.tool_calls.len()));

            self.begin_tool_execution(turn.tool_calls, runtime)?;
            return Ok(());
        }

        self.pending_request = false;
        self.abort_confirmation_deadline = None;
        self.last_notice = Some(match turn.finish_reason.as_deref() {
            Some(reason) if reason != "stop" => format!("Response finished ({reason})"),
            _ => "Response complete".to_string(),
        });
        self.schedule_context_compaction_for_session(self.conversation.session_id, runtime);

        Ok(())
    }

    fn resolve_fallback_model(config: &AppConfig, auth: &AuthStore) -> Result<ActiveModel> {
        if let Ok(model) = config.resolve_active_model(auth) {
            return Ok(model);
        }

        let summary = config
            .available_models()
            .into_iter()
            .next()
            .context("no models are configured")?;

        config.resolve_model_by_ids(auth, &summary.provider_id, &summary.model_id)
    }

    fn resolve_conversation_model(
        config: &AppConfig,
        auth: &AuthStore,
        conversation: &Conversation,
    ) -> Result<ActiveModel> {
        config.resolve_model_by_ids(auth, &conversation.provider_id, &conversation.model_id)
    }
}

fn png_data_url_from_clipboard_image(image: arboard::ImageData<'_>) -> Result<String> {
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    let width = image.width as u32;
    let height = image.height as u32;
    let rgba = image.bytes.into_owned();
    encoder
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .context("failed to encode clipboard image")?;
    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(png)
    ))
}

fn build_directory_tree(path: &Path, max_depth: usize, max_entries: usize) -> Result<String> {
    let label = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.display().to_string());

    let mut lines = vec![format!("{label}/")];
    let mut entry_count = 0usize;
    append_directory_tree(
        path,
        1,
        max_depth,
        max_entries,
        &mut entry_count,
        &mut lines,
    )?;
    Ok(lines.join("\n"))
}

fn append_directory_tree(
    path: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    entry_count: &mut usize,
    lines: &mut Vec<String>,
) -> Result<()> {
    if depth > max_depth || *entry_count >= max_entries {
        return Ok(());
    }

    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", path.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push((file_type.is_dir(), name, entry.path()));
    }

    entries.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)));

    for (is_dir, name, child_path) in entries {
        if *entry_count >= max_entries {
            lines.push(format!("{}...", "  ".repeat(depth)));
            break;
        }

        let indent = "  ".repeat(depth);
        if is_dir {
            lines.push(format!("{indent}{name}/"));
            *entry_count += 1;
            append_directory_tree(
                &child_path,
                depth + 1,
                max_depth,
                max_entries,
                entry_count,
                lines,
            )?;
        } else {
            lines.push(format!("{indent}{name}"));
            *entry_count += 1;
        }
    }

    Ok(())
}

fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        crossterm::execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            crossterm::cursor::Hide,
        )
        .context("failed to enter alternate screen")?;

        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
            Show,
        );
    }
}
