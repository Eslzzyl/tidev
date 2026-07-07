//! tidev-tui — Terminal UI for tidev.
//!
//! The TUI is built around [`App`] which holds a [`tidev_core::Runtime`] (all
//! backend resources) and a [`UiState`] (all presentation/interaction state).
//!
//! # Architecture
//!
//! ```text
//! App
//!  ├── runtime: tidev_core::Runtime  ← config, session, LLM, tools
//!  └── ui: UiState                   ← panels, dialogs, input, render cache
//! ```
//!
//! The event loop in [`App::run`] does three things each iteration:
//! 1. Drain `BackendEvent`s from Runtime → update `ui.chat_context`, tokens, etc.
//! 2. Drain `PendingToolApproval`s from Runtime → show permission dialogs
//! 3. Poll crossterm input → dispatch to keyboard/mouse/scroll handlers

pub mod chat_context;
pub mod commands;
pub mod panel_launcher;
pub mod state;
pub mod theme;
pub mod utils;

mod ansi;
mod input;
mod markdown;
mod render;
mod ui;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use crossterm::{execute, terminal};
use ratatui::layout::{Position, Rect};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use uuid::Uuid;

use tidev_types::message::BackendEvent;
use tidev_types::message::{Message, MessageRole, ToolCall};
use tidev_types::prompts::SessionMode;
use tidev_types::tools::TodoItem;

use tidev_core::{ApprovedTool, PendingToolApproval};

pub use commands::{CommandAction, CommandPaletteState, CommandRegistry};
pub use input::Composer;
pub use input::at_mention;
pub(crate) use panel_launcher::{PanelAction, PanelLauncherState};
pub use render::chat_dialog;
pub use render::chat_render;
pub use render::diff_render;
pub(crate) use state::Screen;
pub use ui::connect;
pub use ui::message_panel;
pub use ui::model_panel;
pub use ui::permission;
pub use ui::question;
pub use ui::session_panel;
pub use ui::settings_panel;
pub use ui::theme_panel;

use crate::chat_context::ChatContext;
use crate::input::at_mention::AtMentionState;
use crate::input::mouse_selection::MouseSelectionState;
use crate::input::snippet::SnippetState;
use crate::state::{
    CachedSessionRuntime, ContextUsage, MESSAGE_RENDER_CACHE_MAX_ENTRIES, MessageLayoutIndex,
    MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheValue, NotificationState,
    QueuedPrompt,
    ScrollbarDragState,
};
use crate::theme::ThemeManager;
use crate::ui::agents_panel::AgentsPanelState;
use crate::ui::connect::ConnectDialog;
use crate::ui::fork_confirm::ForkConfirmDialogState;
use crate::ui::image_viewer::ImageViewerState;
use crate::ui::message_panel::MessagePanelState;
use crate::ui::model_panel::ModelPanelState;
use crate::ui::permission::{
    PendingToolExecution, PermissionDialogState, RunningSubagentExecution, RunningToolExecution,
};
use crate::ui::question::QuestionDialogState;
use crate::ui::rename::RenameSessionDialogState;
use crate::ui::search_panel::SearchPanelState;
use crate::ui::sensitive::SensitiveFileConfirmDialogState;
use crate::ui::sensitive::SensitiveFileDialogState;
use crate::ui::session_panel::SessionPanelState;
use crate::ui::settings_panel::SettingsPanelState;
use crate::ui::skills_panel::SkillsPanelState;
use crate::ui::theme_panel::ThemePanelState;
use crate::ui::undo_confirm::UndoConfirmDialogState;
use crate::ui::workspace_boundary::WorkspaceBoundaryConfirmDialogState;
use crate::ui::workspace_boundary::WorkspaceBoundaryDialogState;

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// The top-level TUI application.
///
/// Created with [`App::new`] from a pre-built [`tidev_core::Runtime`], then run
/// with [`App::run`] which enters the terminal event loop.
pub struct App {
    /// Shared backend runtime (config, session, LLM, tools).
    pub runtime: tidev_core::Runtime,
    /// All TUI-specific state (display, input, panels, dialogs).
    pub ui: UiState,
}

// ---------------------------------------------------------------------------
// UiState
// ---------------------------------------------------------------------------

/// All presentation and interaction state — none of these fields exist on Runtime.
pub struct UiState {
    // ── Display ──
    pub screen: Screen,
    pub theme: ThemeManager,
    pub chat_context: ChatContext,
    pub context_usage: ContextUsage,
    pub mode: SessionMode,
    pub pending_mode: Option<SessionMode>,
    pub dirty: bool,
    pub should_quit: bool,
    pub force_full_redraw: bool,
    pub last_notice: Option<String>,
    pub notifications: NotificationState,

    // ── Input ──
    pub composer: Composer,
    pub at_mention: AtMentionState,
    pub command_palette: CommandPaletteState,
    pub commands: CommandRegistry,
    pub panel_launcher: PanelLauncherState,
    pub snippet_state: SnippetState,
    pub prompt_history: Vec<String>,
    pub leader_key_pending: bool,
    pub mouse_selection: MouseSelectionState,

    // ── Open panels (None = closed) ──
    pub model_panel: Option<ModelPanelState>,
    pub session_panel: Option<SessionPanelState>,
    pub theme_panel: Option<ThemePanelState>,
    pub settings_panel: Option<SettingsPanelState>,
    pub search_panel: Option<SearchPanelState>,
    pub agents_panel: Option<AgentsPanelState>,
    pub skills_panel: Option<SkillsPanelState>,
    pub message_panel: Option<MessagePanelState>,

    // ── Dialogs ──
    pub connect_dialog: Option<ConnectDialog>,
    pub permission_dialog: Option<PermissionDialogState>,
    pub question_dialog: Option<QuestionDialogState>,
    pub workspace_boundary_dialog: Option<WorkspaceBoundaryDialogState>,
    pub workspace_boundary_confirm_dialog: Option<WorkspaceBoundaryConfirmDialogState>,
    pub sensitive_file_dialog: Option<SensitiveFileDialogState>,
    pub sensitive_file_confirm_dialog: Option<SensitiveFileConfirmDialogState>,
    pub fork_confirm_dialog: Option<ForkConfirmDialogState>,
    pub undo_confirm_dialog: Option<UndoConfirmDialogState>,
    pub rename_session_dialog: Option<RenameSessionDialogState>,

    // ── Image viewer ──
    pub image_viewer: Option<ImageViewerState>,
    pub image_picker: Option<ratatui_image::picker::Picker>,
    pub image_viewer_consume_next_up: bool,

    // ── Tool execution tracking (UI side) ──
    pub pending_tool_execution: Option<PendingToolExecution>,
    pub running_tool_executions: Vec<RunningToolExecution>,
    pub running_subagent_executions: Vec<RunningSubagentExecution>,
    pub pending_prompt_queue: Vec<QueuedPrompt>,
    pub pending_permission_rx: Option<tokio::sync::mpsc::UnboundedReceiver<PendingToolApproval>>,
    pub pending_permission_response: Option<oneshot::Sender<Vec<ApprovedTool>>>,
    pub pending_rejected_tools: Vec<ToolCall>,
    pub pending_permission_tool_calls: Vec<ToolCall>,
    pub subagent_task_map: HashMap<Uuid, Uuid>,
    pub sensitive_file_approved: HashMap<String, bool>,
    pub saved_composer_text: String,
    pub tool_result_card_bounds: Vec<(Uuid, ratatui::layout::Rect)>,
    pub theme_panel_overlay: Cell<Option<Rect>>,
    pub agents_panel_overlay: Cell<Option<Rect>>,
    pub skills_panel_overlay: Cell<Option<Rect>>,
    pub settings_panel_overlay: Cell<Option<Rect>>,
    pub model_panel_overlay: Cell<Option<Rect>>,
    pub message_panel_overlay: Cell<Option<Rect>>,
    pub session_panel_overlay: Cell<Option<Rect>>,
    pub inline_subagent_card_bounds: Vec<(usize, ratatui::layout::Rect)>,
    pub hovered_queued_index: Option<usize>,
    pub hovered_inline_subagent: Option<Uuid>,
    pub message_viewport_lines: usize,
    pub subagent_result_message_map: HashMap<Uuid, Uuid>,
    pub shell_mode: bool,
    pub selection_clipboard_lease: Option<crate::input::mouse_selection::ClipboardLease>,
    pub selectable_regions: Vec<ratatui::layout::Rect>,
    pub rename_dialog: Option<RenameSessionDialogState>,
    pub workspace_boundary_approved: HashMap<String, bool>,
    pub abort_confirmation_deadline: Option<Instant>,

    // ── Card bounds (recalculated each frame) ──
    pub user_card_bounds: Vec<(Uuid, ratatui::layout::Rect)>,
    pub queued_card_bounds: Vec<(usize, ratatui::layout::Rect)>,
    pub user_image_badge_bounds: Vec<(Uuid, ratatui::layout::Rect, String)>,

    // ── Scroll / layout ──
    pub message_scroll_offset: usize,
    pub input_scroll_offset: usize,
    pub sidebar_scroll_offset: usize,
    pub message_follow_tail: bool,
    pub message_content_area: Option<Rect>,
    pub message_scrollbar_area: Option<Rect>,
    pub sidebar_area: Option<Rect>,
    pub message_layout_index: RefCell<MessageLayoutIndex>,
    pub scrollbar_drag: Option<ScrollbarDragState>,
    pub scrollbar_hovered: bool,
    pub input_dragging: bool,
    pub hovered_card: Option<Uuid>,
    pub cached_sessions: HashMap<Uuid, CachedSessionRuntime>,

    // ── Message render cache ──
    pub message_render_cache:
        RefCell<lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>>,
    pub message_render_cache_tick: Cell<u64>,
    pub message_render_cache_hits: Cell<u64>,
    pub message_render_cache_misses: Cell<u64>,

    // ── Draft attachments (pasted images, file references) ──
    pub draft_attachments: Vec<tidev_types::message::MessageAttachment>,
    pub restored_attachments: Vec<tidev_types::message::MessageAttachment>,

    // ── Request tracking ──
    pub pending_request: bool,
    pub active_request_id: u64,
    pub thinking_level: tidev_config::reasoning::ThinkingLevelType,
    pub pending_assistant_turns: HashSet<Uuid>,

    // ── Input area ──
    pub input_area: Cell<Option<Rect>>,

    // ── Toast notifications ──
    pub toast: Option<(String, Instant)>,

    // ── Expanded tool results ──
    pub expanded_tool_results: HashSet<Uuid>,
    pub loaded_tool_outputs: HashMap<Uuid, String>,

    // ── Todos ──
    pub todos: Vec<TodoItem>,

    // ── Animation / rendering state ──
    pub spinner_start: std::time::Instant,
    pub message_total_lines: usize,
    pub sidebar_total_lines: usize,
    pub message_scroll_target: Option<Uuid>,

    // ── Permission state ──
    pub sensitive_file_permissions: HashMap<String, bool>,
    pub workspace_boundary_permissions: HashMap<String, bool>,

    // ── Retry hint ──
    pub retrying_hint: Option<(u32, u32, String, std::time::Instant)>,
}

impl UiState {
    /// Create a new UI state with default values.
    pub fn new(runtime: &tidev_core::Runtime) -> Self {
        let active_provider = runtime.active_provider_id();
        let active_model = runtime.active_model_id();
        let theme = ThemeManager::new(&runtime.config().theme);

        let at_mention = AtMentionState::new();
        at_mention.start_background_indexing(runtime.workspace_root());

        Self {
            screen: Screen::Welcome,
            theme,
            chat_context: ChatContext::default(),
            context_usage: ContextUsage::default(),
            mode: SessionMode::Build,
            pending_mode: None,
            dirty: true,
            should_quit: false,
            force_full_redraw: false,
            last_notice: None,
            notifications: NotificationState::default(),

            composer: Composer::new("Ask tidev about your code, task, or question..."),
            at_mention,
            command_palette: CommandPaletteState::default(),
            commands: CommandRegistry::new(),
            panel_launcher: PanelLauncherState::default(),
            snippet_state: SnippetState::default(),
            prompt_history: Vec::new(),
            leader_key_pending: false,
            mouse_selection: MouseSelectionState::default(),

            model_panel: None,
            session_panel: None,
            theme_panel: None,
            settings_panel: None,
            search_panel: None,
            agents_panel: None,
            skills_panel: None,
            message_panel: None,

            connect_dialog: None,
            permission_dialog: None,
            question_dialog: None,
            workspace_boundary_dialog: None,
            workspace_boundary_confirm_dialog: None,
            sensitive_file_dialog: None,
            sensitive_file_confirm_dialog: None,
            fork_confirm_dialog: None,
            undo_confirm_dialog: None,
            rename_session_dialog: None,

            image_viewer: None,
            image_picker: None,
            image_viewer_consume_next_up: false,

            pending_tool_execution: None,
            running_tool_executions: Vec::new(),
            running_subagent_executions: Vec::new(),
            pending_prompt_queue: Vec::new(),
            pending_permission_rx: None,
            pending_permission_response: None,
            pending_rejected_tools: Vec::new(),
            pending_permission_tool_calls: Vec::new(),
            subagent_task_map: HashMap::new(),
            sensitive_file_approved: HashMap::new(),
            saved_composer_text: String::new(),
            tool_result_card_bounds: Vec::new(),
            theme_panel_overlay: Cell::new(None),
            agents_panel_overlay: Cell::new(None),
            skills_panel_overlay: Cell::new(None),
            settings_panel_overlay: Cell::new(None),
            model_panel_overlay: Cell::new(None),
            message_panel_overlay: Cell::new(None),
            session_panel_overlay: Cell::new(None),
            inline_subagent_card_bounds: Vec::new(),
            hovered_queued_index: None,
            hovered_inline_subagent: None,
            message_viewport_lines: 0,
            subagent_result_message_map: HashMap::new(),
            shell_mode: false,
            selection_clipboard_lease: None,
            selectable_regions: Vec::new(),
            rename_dialog: None,
            workspace_boundary_approved: HashMap::new(),
            abort_confirmation_deadline: None,

            message_scroll_offset: 0,
            input_scroll_offset: 0,
            sidebar_scroll_offset: 0,
            message_follow_tail: true,
            message_content_area: None,
            message_scrollbar_area: None,
            sidebar_area: None,
            message_layout_index: RefCell::new(MessageLayoutIndex::default()),
            scrollbar_drag: None,
            scrollbar_hovered: false,
            input_dragging: false,
            hovered_card: None,

            message_render_cache: RefCell::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(MESSAGE_RENDER_CACHE_MAX_ENTRIES).unwrap(),
            )),
            message_render_cache_tick: Cell::new(0),
            message_render_cache_hits: Cell::new(0),
            message_render_cache_misses: Cell::new(0),

            cached_sessions: HashMap::new(),
            draft_attachments: Vec::new(),
            restored_attachments: Vec::new(),
            pending_request: false,
            active_request_id: 0,
            thinking_level: tidev_config::reasoning::ThinkingLevelType::default(),
            pending_assistant_turns: HashSet::new(),
            input_area: Cell::new(None),
            toast: None,
            expanded_tool_results: HashSet::new(),
            loaded_tool_outputs: HashMap::new(),
            todos: Vec::new(),
            user_card_bounds: Vec::new(),
            queued_card_bounds: Vec::new(),
            user_image_badge_bounds: Vec::new(),
            spinner_start: std::time::Instant::now(),
            message_total_lines: 0,
            sidebar_total_lines: 0,
            message_scroll_target: None,
            sensitive_file_permissions: HashMap::new(),
            workspace_boundary_permissions: HashMap::new(),
            retrying_hint: None,
        }
    }

    /// Returns (hits, misses, entries) for the message render cache.
    pub(crate) fn message_render_cache_stats(&self) -> (u64, u64, usize) {
        (
            self.message_render_cache_hits.get(),
            self.message_render_cache_misses.get(),
            self.message_render_cache.borrow().len(),
        )
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

impl App {
    /// Create a new TUI application from a pre-built Runtime.
    pub fn new(runtime: tidev_core::Runtime) -> Self {
        let ui = UiState::new(&runtime);
        Self { runtime, ui }
    }

    /// Palette shorthand for render code.
    pub(crate) fn palette(&self) -> crate::theme::ThemePalette {
        self.ui.theme.palette()
    }

    /// Clear the message render cache (called when theme, messages, or width change).
    pub(crate) fn clear_message_render_cache(&mut self) {
        self.ui.message_render_cache.borrow_mut().clear();
        if let Some(ref mut index) = self.ui.message_layout_index.try_borrow_mut().ok() {
            index.valid = false;
        }
    }

    /// Enter the terminal event loop (blocking, runs on current tokio runtime).
    pub async fn run(&mut self) -> Result<()> {
        // ── 1. Initialise terminal ──
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableLineWrap,
            EnableBracketedPaste,
            EnableFocusChange,
            EnableMouseCapture,
        )
        .context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("failed to create terminal backend")?;
        terminal.clear()?;

        // ── 2. Consume runtime channels ──
        let mut event_rx = self
            .runtime
            .event_rx()
            .await
            .context("event_rx already consumed")?;
        let mut perm_rx = self
            .runtime
            .perm_rx()
            .await
            .context("perm_rx already consumed")?;

        // Register the permission receiver on UiState so the request handler
        // can await on it outside the main loop's try_recv.
        self.ui.pending_permission_rx = Some(perm_rx);

        // ── 3. Initialise image picker ──
        if let Ok(picker) = ratatui_image::picker::Picker::from_query_stdio() {
            self.ui.image_picker = Some(picker);
        }

        // ── 4. Main event loop ──
        self.ui.dirty = true;
        let poll_timeout = Duration::from_millis(50);

        loop {
            // Render if dirty
            if self.ui.dirty {
                terminal.draw(|frame| self.render(frame))?;
                self.ui.dirty = false;
            }

            // Drain backend events (non-blocking)
            while let Ok(event) = event_rx.try_recv() {
                self.handle_backend_event(event).await;
            }

            // Check for queued prompts (submitted while agent was busy)
            self.drain_queued_prompts().await?;

            // Drain permission requests (non-blocking)
            if let Some(ref mut rx) = self.ui.pending_permission_rx {
                let approvals: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
                for approval in approvals {
                    self.handle_permission_request(approval);
                }
            }

            // Poll user input (with timeout so backend events get processed)
            if crossterm::event::poll(poll_timeout)? {
                let event = crossterm::event::read()?;
                self.handle_event(event)?;
            }

            if self.ui.should_quit {
                break;
            }
        }

        // ── 5. Cleanup terminal ──
        let _ = execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableLineWrap,
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
        );
        disable_raw_mode()?;
        terminal.show_cursor()?;
        println!();

        // ── 6. Shutdown background tasks ──
        self.runtime.shutdown().await;

        Ok(())
    }

    /// Process a single backend event from the Runtime.
    ///
    /// Keeps the TUI's local [`ChatContext`] in sync with the Runtime's
    /// [`MessageBuffer`] by forwarding streaming deltas, finalising turns,
    /// and reloading after bulk mutations (undo/redo/compaction).
    async fn handle_backend_event(&mut self, event: BackendEvent) {
        let session_id = event.session_id();

        // Only process events for the currently visible session.
        if session_id != self.ui.chat_context.session_id {
            log::debug!("handle_backend_event: ignoring event for session {session_id}");
            return;
        }

        self.ui.dirty = true;

        match event {
            BackendEvent::Delta { content, .. } => {
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    if msg.streaming {
                        msg.content.push_str(&content);
                        let mid = msg.id;
                        self.invalidate_active_message_render_cache_for(mid);
                    }
                }
            }
            BackendEvent::ReasoningDelta { content, .. } => {
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    if msg.streaming && msg.role == MessageRole::Assistant {
                        msg.reasoning.push_str(&content);
                        let mid = msg.id;
                        self.invalidate_active_message_render_cache_for(mid);
                    }
                }
            }
            BackendEvent::ToolCallUpdated { tool_call, .. } => {
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    if msg.streaming {
                        msg.upsert_tool_call(tool_call);
                        let mid = msg.id;
                        self.invalidate_active_message_render_cache_for(mid);
                    }
                }
            }
            BackendEvent::Finished { turn, .. } => {
                self.ui.retrying_hint = None;

                // Finalise the streaming assistant message.
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    if msg.streaming && msg.role == MessageRole::Assistant {
                        msg.content = turn.content.clone();
                        msg.reasoning = turn.reasoning.clone();
                        msg.tool_calls = turn.tool_calls.clone();
                        msg.streaming = false;
                        msg.completed_at = Some(Utc::now());
                        msg.input_tokens = turn.input_tokens;
                        msg.output_tokens = turn.output_tokens;
                        msg.total_tokens = turn.total_tokens;
                        msg.model_id = turn.model_id.clone();
                        let mid = msg.id;
                        self.invalidate_active_message_render_cache_for(mid);
                    }
                }

                if turn.tool_calls.is_empty() {
                    self.ui.pending_request = false;
                    self.ui.last_notice = Some(match turn.finish_reason.as_deref() {
                        Some(r) if r != "stop" => format!("Response finished ({r})"),
                        _ => "Response complete".to_string(),
                    });
                } else {
                    self.ui.last_notice = Some(format!(
                        "Processing {} tool call(s)...",
                        turn.tool_calls.len()
                    ));
                }
            }
            BackendEvent::ToolCompleted {
                tool_call, result, ..
            } => {
                let result_msg = Message::tool_result(
                    &tool_call.id,
                    &tool_call.name,
                    result,
                );
                let msg_id = result_msg.id;
                self.ui.chat_context.messages.push(result_msg);
                self.invalidate_active_message_render_cache_for(msg_id);

                // Clean up running_tool_executions.
                self.ui.running_tool_executions.retain(|r| {
                    r.request_id != 0 || r.tool_call.id != tool_call.id
                });
            }
            BackendEvent::TurnStarting { .. } => {
                // Create a new streaming assistant message for the next turn.
                let mut msg = Message::streaming(MessageRole::Assistant, "");
                msg.mode = Some(self.ui.mode);
                self.ui.chat_context.messages.push(msg);
            }
            BackendEvent::StreamEnd { .. } => {
                self.ui.pending_request = false;
                self.ui.last_notice = None;
            }
            BackendEvent::Failed { error, .. } => {
                self.ui.pending_request = false;
                self.ui.pending_mode = None;
                self.ui.last_notice = Some(error.clone());

                // Mark the last streaming message as failed.
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    if msg.streaming {
                        msg.streaming = false;
                        msg.content = format!("Request failed: {error}");
                        let mid = msg.id;
                        self.invalidate_active_message_render_cache_for(mid);
                    }
                }
            }
            BackendEvent::ContextCompacted { .. } => {
                // Reload messages from the Runtime's canonical buffer.
                let runtime = self.runtime.clone();
                let sid = self.ui.chat_context.session_id;
                let buf = runtime.message_buffer(sid).await;
                let msgs = buf.read().await.load().to_vec();
                self.ui.chat_context.messages = msgs;
                self.ui.chat_context.visible_count =
                    self.ui.chat_context.messages.len();
                self.clear_message_render_cache();
            }
            BackendEvent::UsageStats {
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                duration_ms,
                ..
            } => {
                self.ui.context_usage = crate::state::ContextUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    model_id: model_id.clone(),
                    tokens_per_second: duration_ms.map(|ms| {
                        total_tokens as f32 / (ms as f32 / 1000.0)
                    }),
                };

                // Also copy onto the last message for persistence.
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    msg.input_tokens = Some(input_tokens);
                    msg.output_tokens = Some(output_tokens);
                    msg.total_tokens = Some(total_tokens);
                }
            }
            BackendEvent::ShellOutput {
                content, finished, ..
            } => {
                // Real-time bash output — update the last streaming Tool message.
                if let Some(msg) = self.ui.chat_context.messages.last_mut() {
                    if msg.role == MessageRole::Tool && msg.streaming {
                        msg.content = content;
                        if finished {
                            msg.streaming = false;
                        }
                        let mid = msg.id;
                        self.invalidate_active_message_render_cache_for(mid);
                    }
                }
            }
            BackendEvent::Retrying {
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
                ..
            } => {
                let deadline = Instant::now()
                    + Duration::from_secs(retry_after_secs.unwrap_or(0) as u64);
                self.ui.retrying_hint = Some((attempt, max_attempts, reason, deadline));
            }
            BackendEvent::SubagentStatus {
                child_session_id,
                status_text,
                current_tool_call,
                ..
            } => {
                if let Some(exec) = self
                    .ui
                    .running_subagent_executions
                    .iter_mut()
                    .find(|e| e.child_session_id == child_session_id)
                {
                    exec.status = crate::ui::permission::SubagentStatus::from_status_text(&status_text);
                    exec.current_tool_call = current_tool_call;
                }
            }
            BackendEvent::SubagentCompleted {
                child_session_id, ..
            } => {
                self.ui
                    .running_subagent_executions
                    .retain(|e| e.child_session_id != child_session_id);
            }
            // InstructionsLoaded and SidebarSnapshotReady are not yet constructed.
            _ => {}
        }
    }

    /// Process a permission request from the Runtime.
    fn handle_permission_request(&mut self, approval: PendingToolApproval) {
        log::debug!("Permission request for {} tools", approval.tool_calls.len());
    }

    /// If there are queued prompts and no pending request, submit one.
    async fn drain_queued_prompts(&mut self) -> Result<()> {
        Ok(())
    }

    /// Schedule context compaction for a session (manual trigger, e.g. /compact).
    pub(crate) fn schedule_context_compaction_for_session(
        &mut self,
        session_id: Uuid,
        stream_request_id: Option<u64>,
    ) {
        let runtime = self.runtime.clone();
        let mode = self.ui.mode;
        tokio::spawn(async move {
            if let Err(e) = runtime
                .compact_session(session_id, mode, stream_request_id)
                .await
            {
                log::error!("Compaction failed for session {session_id}: {e}");
            }
        });
    }

    // ── Message render cache helpers ─────────────────────────────────

    pub(crate) fn next_message_render_cache_tick(&self) -> u64 {
        let tick = self.ui.message_render_cache_tick.get().wrapping_add(1);
        self.ui.message_render_cache_tick.set(tick);
        tick
    }

    pub(crate) fn record_message_render_cache_hit(&self) {
        self.ui
            .message_render_cache_hits
            .set(self.ui.message_render_cache_hits.get().saturating_add(1));
    }

    pub(crate) fn record_message_render_cache_miss(&self) {
        self.ui
            .message_render_cache_misses
            .set(self.ui.message_render_cache_misses.get().saturating_add(1));
    }

    pub(crate) fn prune_message_render_cache_if_needed(&self) {
        let cache_len = self.ui.message_render_cache.borrow().len();
        if cache_len <= crate::state::MESSAGE_RENDER_CACHE_MAX_ENTRIES {
            return;
        }

        let remove_count = cache_len - crate::state::MESSAGE_RENDER_CACHE_MAX_ENTRIES;
        let mut evict_candidates = self
            .ui
            .message_render_cache
            .borrow()
            .iter()
            .map(|(key, entry)| (key.clone(), entry.last_used_tick))
            .collect::<Vec<_>>();
        evict_candidates.sort_by_key(|(_, tick)| *tick);

        let mut cache = self.ui.message_render_cache.borrow_mut();
        for (key, _) in evict_candidates.into_iter().take(remove_count) {
            cache.pop(&key);
        }
    }

    pub(crate) fn invalidate_active_message_render_cache_for(&self, message_id: Uuid) {
        let session_id = self.ui.chat_context.session_id;
        // Collect keys to remove (can't mutate LruCache while iterating).
        let keys_to_remove: Vec<MessageRenderCacheKey> = self
            .ui
            .message_render_cache
            .borrow()
            .iter()
            .filter(|(key, _)| key.session_id == session_id && key.message_id == message_id)
            .map(|(key, _)| key.clone())
            .collect();
        let mut cache = self.ui.message_render_cache.borrow_mut();
        for key in keys_to_remove {
            cache.pop(&key);
        }
        if let Ok(mut index) = self.ui.message_layout_index.try_borrow_mut() {
            if !index.dirty_messages.contains(&message_id) {
                index.dirty_messages.push(message_id);
            }
            index.valid = false;
        }
    }

    // ── Background subagent status (for status bar) ──────────────────

    pub(crate) fn background_running_count(&self) -> usize {
        self.ui.running_subagent_executions.len()
    }

    pub(crate) fn background_waiting_question_count(&self) -> usize {
        // TODO: track background sessions with pending questions
        0
    }

    // ── Tool management ─────────────────────────────────────────────

    /// Refresh tool definitions (called on mode switch).
    /// In the new architecture the tool registry is built once by RuntimeBuilder,
    /// so this is a no-op. If mode-specific tool filtering is needed later,
    /// it should be handled at the Runtime level.
    pub(crate) fn refresh_tools(&mut self) {
        log::debug!("refresh_tools: no-op (tools are static after Runtime build)");
    }

    // ── Instruction sources ─────────────────────────────────────────

    /// Resolve a fallback model for session loading — tries the active model
    /// first, then falls back to the first available model.
    pub(crate) fn resolve_fallback_model(
        config: &tidev_config::AppConfig,
        auth: &tidev_config::AuthStore,
    ) -> anyhow::Result<tidev_config::auth::ActiveModel> {
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
}
