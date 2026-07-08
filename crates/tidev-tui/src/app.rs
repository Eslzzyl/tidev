//! New-architecture App root component.
//!
//! Owns the Runtime, manages the component tree via OverlayStack,
//! routes Actions, and dispatches async commands.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use tidev_core::{ApprovedTool, ToolCallWithViolations};
use tidev_core::TuiResponse;
use tidev_types::agent_type::AgentType;
use tidev_types::message::{BackendEvent, MessageRole, ToolExecutionResult};
use tidev_types::tools::QuestionArgs;
use tidev_tui_old::theme::{ThemeName, ThemePalette};
use tidev_types::prompts::SessionMode;
use tidev_types::reasoning::ThinkingLevelType;
use tidev_types::tools::{QuestionArgs, TodoItem};
use uuid::Uuid;

use crate::action::{Action, BoundaryDecision, ChatAction, ConnectAction, OverlayAction,
    OverlayKind, PermissionDecision, SearchAction, SensitiveFileDecision, SessionAction,
    ThemeAction};
use crate::component::Component;
use crate::components::overlay_stack::OverlayStack;
use crate::components::overlays::agents::AgentsPanel;
use crate::components::overlays::connect::ConnectDialog;
use crate::components::overlays::fork::ForkConfirmDialog;
use crate::components::overlays::image::ImageViewer;
use crate::components::overlays::message::{MessagePanel, MessagePanelMessage};
use crate::components::overlays::model::ModelPanel;
use crate::components::overlays::panel_launcher::PanelLauncher;
use crate::components::overlays::permission::PermissionDialog;
use crate::components::overlays::question::QuestionDialog;
use crate::components::overlays::rename::RenameDialog;
use crate::components::overlays::search::SearchPanel;
use crate::components::overlays::sensitive::SensitiveFileDialog;
use crate::components::overlays::session::SessionPanel;
use crate::components::overlays::settings::SettingsPanel;
use crate::components::overlays::skills::{SkillItem, SkillsPanel};
use crate::components::overlays::theme::ThemePanel;
use crate::components::overlays::undo::UndoConfirmDialog;
use crate::components::overlays::workspace::WorkspaceBoundaryDialog;
use crate::components::chat::MessageList;
use crate::components::composer::Composer;
use crate::components::sidebar::Sidebar;
use crate::context::{DrawContext, UpdateContext};
use crate::utils::strip_system_reminder_tags;

/// Token usage statistics for the current/last request.
#[derive(Clone, Debug)]
pub(crate) struct ContextUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub model_id: String,
    pub tokens_per_second: Option<f32>,
}

pub struct App {
    pub(crate) runtime: tidev_core::Runtime,
    overlays: OverlayStack,
    current_palette: ThemePalette,
    should_quit: bool,
    /// Pending scroll target set by ChatAction::ScrollTo (consumed by Chat component).
    scroll_target: Option<uuid::Uuid>,
    /// Current active session (set by SessionPanel when switching sessions).
    current_session_id: Option<uuid::Uuid>,
    /// Current session mode (Build / Plan).
    mode: SessionMode,
    /// Pending mode switch (applied on next Finished with no tool calls).
    pending_mode: Option<SessionMode>,
    /// Current thinking level for the active model.
    thinking_level: ThinkingLevelType,
    /// Status notice shown at the bottom of the screen (plain text, no timeout).
    last_notice: Option<(String, Instant)>,
    /// Transient popup notification in top-right corner (auto-expires).
    toast: Option<(String, Instant)>,
    /// Receiver for tool permission requests from the agent loop.
    pub(crate) request_rx: Option<tokio::sync::mpsc::UnboundedReceiver<tidev_core::TuiRequest>>,
    /// Receiver for backend events (streaming deltas, tool results, etc.).
    pub(crate) event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<BackendEvent>>,

    /// Chat message list component.
    pub(crate) message_list: Option<MessageList>,

    /// Text input composer.
    pub(crate) composer: Option<Composer>,

    /// Right-hand info sidebar.
    sidebar: Sidebar,

    /// Cached sidebar area for mouse hit-testing.
    sidebar_area: Option<Rect>,

    /// Current session's todo items (loaded from store).
    todos: Vec<TodoItem>,

    // ── Tool approval pipeline ──

    /// Oneshot sender for responding to a pending TuiRequest.
    pending_response_tx: Option<tokio::sync::oneshot::Sender<TuiResponse>>,
    /// Tools still awaiting user decisions.
    pending_tools: Vec<ToolCallWithViolations>,
    /// Current index into pending_tools.
    tool_index: usize,
    /// Accumulated approved/rejected tools.
    approved_tools: Vec<ApprovedTool>,

    // ── In-memory permission caches ──

    /// allowlist for workspace boundary (canonical path → allowed).
    /// Uses prefix matching so allowing a directory allows all files under it.
    boundary_permissions: HashMap<String, bool>,
    /// allowlist for sensitive file access (canonical path → allowed).
    sensitive_permissions: HashMap<String, bool>,

    /// Token usage statistics from the last request (for status bar display).
    context_usage: Option<ContextUsage>,
}

impl App {
    pub fn new(
        runtime: tidev_core::Runtime,
        request_rx: Option<tokio::sync::mpsc::UnboundedReceiver<tidev_core::TuiRequest>>,
        event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<BackendEvent>>,
    ) -> Self {
        let theme_str = runtime.config().theme;
        let current_palette = ThemePalette::from_name(&theme_str);

        // Capture before runtime is moved into Self.
        let file_index = runtime.file_search_index();
        let ws_root = runtime.workspace_root().clone();
        let cfg_dir = runtime.config_dir().clone();
        let supports_images = runtime.active_model().supports_images;
        let thinking_level = runtime.active_model().thinking_level.clone();

        Self {
            runtime,
            overlays: OverlayStack::new(),
            current_palette,
            should_quit: false,
            scroll_target: None,
            current_session_id: None,
            mode: SessionMode::Build,
            pending_mode: None,
            thinking_level: thinking_level,
            last_notice: None,
            toast: None,
            request_rx,
            event_rx,
            pending_response_tx: None,
            pending_tools: Vec::new(),
            tool_index: 0,
            approved_tools: Vec::new(),
            boundary_permissions: HashMap::new(),
            sensitive_permissions: HashMap::new(),
            context_usage: None,
            message_list: None,
            sidebar: Sidebar::new(),
            sidebar_area: None,
            todos: Vec::new(),
            composer: {
                let mut c = Composer::new("Ask tidev...");
                c.set_file_search_index(file_index);
                c.set_workspace_root(ws_root);
                c.set_config_dir(cfg_dir);
                c.set_model_supports_images(supports_images);
                Some(c)
            },
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    // ── Notifications ──

    /// Set a persistent status notice shown at the bottom of the screen.
    pub(crate) fn set_notice(&mut self, msg: impl Into<String>) {
        self.last_notice = Some((msg.into(), Instant::now()));
    }

    /// Set a transient toast notification (auto-expires after `duration`).
    pub(crate) fn set_toast(&mut self, msg: impl Into<String>, duration: std::time::Duration) {
        self.toast = Some((msg.into(), Instant::now() + duration));
    }

    // ── Event handling ──

    /// Handle a backend event from the agent loop (streaming, tool results, etc.).
    pub(crate) fn handle_backend_event(&mut self, event: BackendEvent) {
        // Forward to MessageList for all chat-related events.
        if let Some(ref mut chat) = self.message_list {
            chat.handle_backend_event(&event);
        }

        match event {
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
                // Store context usage for display in status bar.
                self.context_usage = Some(ContextUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    model_id: model_id.clone(),
                    tokens_per_second: if let Some(ms) = duration_ms {
                        if ms > 0 {
                            Some(output_tokens as f32 / (ms as f32 / 1000.0))
                        } else {
                            None
                        }
                    } else {
                        None
                    },
                });

                // Update the last message's token fields.
                if let Some(ref mut chat) = self.message_list {
                    chat.set_last_message_tokens(
                        Some(input_tokens),
                        Some(output_tokens),
                        Some(total_tokens),
                        Some(cache_read_tokens),
                        Some(cache_write_tokens),
                        self.context_usage.as_ref().and_then(|u| u.tokens_per_second),
                    );
                }

                // Persist to store (record_usage API not yet available in new storage).
                // TODO: add record_usage to tidev-storage if needed later.
            }
            BackendEvent::InstructionsLoaded { sources, .. } => {
                log::info!("Instructions loaded: {sources:?}");
                if !sources.is_empty() {
                    self.set_notice(format!("Loaded {} instruction source(s)", sources.len()));
                }
            }
            BackendEvent::Retrying {
                attempt,
                max_attempts,
                reason,
                ..
            } => {
                log::info!("Retrying (attempt {attempt}/{max_attempts}): {reason}");
                self.set_toast(
                    format!("Retry {attempt}/{max_attempts}: {reason}"),
                    std::time::Duration::from_secs(5),
                );
            }
            BackendEvent::Failed { error, .. } => {
                log::error!("Request failed: {error}");
                // Clean up pending state (mirrors old behaviour).
                self.pending_tools.clear();
                self.tool_index = 0;
                self.approved_tools.clear();
                self.pending_response_tx = None;

                // Mark the last streaming message as error.
                if let Some(ref mut chat) = self.message_list {
                    chat.mark_streaming_as_error(&error);
                }

                self.set_toast(
                    format!("Request failed: {error}"),
                    std::time::Duration::from_secs(8),
                );
            }
            BackendEvent::Finished { turn, .. } => {
                // Apply pending mode switch on final turn (no tool calls).
                if turn.tool_calls.is_empty() {
                    if let Some(new_mode) = self.pending_mode.take() {
                        self.mode = new_mode;
                        self.set_notice(format!("Mode switched to {}", self.mode.title()));
                    }
                }
            }
            BackendEvent::ContextCompacted { .. } => {
                self.set_notice("Context compacted");
            }
            _ => {
                // Events already forwarded to MessageList above:
                //   Delta, ReasoningDelta, ToolCallUpdated, Finished, ToolCompleted,
                //   SubagentStatus, SubagentCompleted, TurnStarting, StreamEnd,
                //   SidebarSnapshotReady, ShellOutput, ContextCompacted
            }
        }
    }

    /// Handle a pending tool approval request from the agent loop.
    pub(crate) fn handle_tui_request(
        &mut self,
        request: tidev_core::TuiRequest,
    ) {
        match request.kind {
            tidev_core::TuiRequestKind::ToolApproval(tools_with_violations) => {
                log::info!(
                    "handle_tui_request: {} tool(s) pending approval",
                    tools_with_violations.len()
                );
                self.pending_response_tx = Some(request.response_tx);
                self.pending_tools = tools_with_violations;
                self.tool_index = 0;
                self.approved_tools = Vec::new();
                self.process_next_tool();
            }
        }
    }

    /// Process the next pending tool in the approval pipeline.
    /// Opens the appropriate dialog (workspace boundary, sensitive file,
    /// question, or permission) for the tool at `tool_index`. When all tools
    /// are processed, sends the approval response back to the runtime.
    fn process_next_tool(&mut self) {
        while self.tool_index < self.pending_tools.len() {
            // Clone data we need before borrowing self for mutations.
            let (boundary_path, sensitive_path, is_question, args, perm_key, perm_label, tc)
                = {
                let twv = &self.pending_tools[self.tool_index];
                let tc = &twv.tool_call;
                (
                    twv.workspace_boundary_violation.clone(),
                    twv.sensitive_file_violation.clone(),
                    tc.name == "question",
                    tc.arguments.clone(),
                    twv.permission_key.clone(),
                    twv.permission_label.clone(),
                    tc.clone(),
                )
            };
            let current_index = self.tool_index + 1;
            let total = self.pending_tools.len();

            // Step 1: Workspace boundary violation check
            if let Some(ref path) = boundary_path {
                let path_str = path.to_string_lossy().to_string();
                match Self::is_path_allowed(&self.boundary_permissions, &path_str) {
                    Some(true) => {
                        log::info!("Boundary path already allowed: {path_str}");
                    }
                    Some(false) => {
                        log::info!("Boundary path previously denied: {path_str}");
                        self.approved_tools.push(ApprovedTool {
                            tool_call: tc,
                            rejection: Some(ToolExecutionResult::new(format!(
                                "Path '{}' was denied by remembered boundary permission.",
                                path_str
                            ))),
                            child_session_id: None,
                            allow_outside: false,
                            sensitive_file_approved: false,
                        });
                        self.tool_index += 1;
                        continue;
                    }
                    None => {
                        log::info!("Opening WorkspaceBoundaryDialog for: {path_str}");
                        self.set_notice("Workspace boundary violation — please make a decision");
                        self.overlays.push(Box::new(
                            WorkspaceBoundaryDialog::new(
                                path.clone(),
                                self.runtime.workspace_root().clone(),
                                current_index,
                                total,
                            ),
                        ));
                        return;
                    }
                }
            }

            // Step 2: Sensitive file violation check
            if let Some(ref path) = sensitive_path {
                let path_str = path.to_string_lossy().to_string();
                match Self::is_path_allowed(&self.sensitive_permissions, &path_str) {
                    Some(true) => {
                        log::info!("Sensitive path already allowed: {path_str}");
                    }
                    Some(false) => {
                        log::info!("Sensitive path previously denied: {path_str}");
                        self.approved_tools.push(ApprovedTool {
                            tool_call: tc,
                            rejection: Some(ToolExecutionResult::new(format!(
                                "Sensitive file '{}' was denied by remembered permission.",
                                path_str
                            ))),
                            child_session_id: None,
                            allow_outside: false,
                            sensitive_file_approved: false,
                        });
                        self.tool_index += 1;
                        continue;
                    }
                    None => {
                        log::info!("Opening SensitiveFileDialog for: {path_str}");
                        self.set_notice("Sensitive file access — please make a decision");
                        self.overlays.push(Box::new(
                            SensitiveFileDialog::new(
                                path.clone(),
                                self.runtime.workspace_root().clone(),
                                current_index,
                                total,
                            ),
                        ));
                        return;
                    }
                }
            }

            // Step 3: Question tool?
            if is_question {
                match serde_json::from_str::<QuestionArgs>(&args) {
                    Ok(qa) if !qa.questions.is_empty() => {
                        log::info!("Opening QuestionDialog ({} questions)", qa.questions.len());
                        self.set_notice("LLM has questions — please provide answers");
                        self.overlays.push(Box::new(QuestionDialog::new(qa.questions)));
                        return;
                    }
                    _ => {
                        log::warn!("Invalid or empty question tool call arguments");
                        self.approved_tools.push(ApprovedTool {
                            tool_call: tc,
                            rejection: Some(ToolExecutionResult::new(
                                "Tool 'question' was rejected: invalid or empty arguments.",
                            )),
                            child_session_id: None,
                            allow_outside: false,
                            sensitive_file_approved: false,
                        });
                        self.tool_index += 1;
                        continue;
                    }
                }
            }

            // Step 4: PermissionDialog — final approve / reject
            log::info!(
                "Opening PermissionDialog for tool: {} ({}/{})",
                perm_label,
                current_index,
                total
            );
            self.set_notice(format!(
                "Approve tool call {} of {}: {}",
                current_index, total, perm_label
            ));
            self.overlays.push(Box::new(PermissionDialog::new(
                perm_key,
                perm_label,
                args,
                current_index,
                total,
            )));
            return;
        }

        // All tools processed — send response
        self.send_approval_response();
    }

    /// Send the accumulated approval response back to the runtime.
    fn send_approval_response(&mut self) {
        let response_tx = match self.pending_response_tx.take() {
            Some(tx) => tx,
            None => {
                log::warn!("send_approval_response: no pending response_tx");
                return;
            }
        };

        let tools = std::mem::take(&mut self.approved_tools);
        log::info!(
            "send_approval_response: {} tool(s) approved/rejected",
            tools.len()
        );

        let _ = response_tx.send(TuiResponse::ToolApproval(tools));
        self.pending_tools.clear();
        self.tool_index = 0;
    }

    /// Check whether a path is in an allowlist, using prefix matching so that
    /// allowing a directory also allows all files under it.
    fn is_path_allowed(cache: &HashMap<String, bool>, path: &str) -> Option<bool> {
        let target = Path::new(path);
        let mut result: Option<bool> = None;
        let mut longest_prefix: usize = 0;
        for (stored, allowed) in cache {
            let stored_path = Path::new(stored);
            if target.starts_with(stored_path) {
                let components = stored_path.components().count();
                if components > longest_prefix {
                    longest_prefix = components;
                    result = Some(*allowed);
                }
            }
        }
        result
    }

    /// Record a workspace boundary decision in the in-memory cache.
    fn record_boundary_decision(&mut self, path: &std::path::PathBuf, decision: &BoundaryDecision) {
        match decision {
            BoundaryDecision::AllowOnce => {}
            BoundaryDecision::AllowUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.boundary_permissions.insert(path_str, true);
            }
            BoundaryDecision::DenyOnce => {}
            BoundaryDecision::DenyUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.boundary_permissions.insert(path_str, false);
            }
        }
    }

    /// Record a sensitive file decision in the in-memory cache.
    fn record_sensitive_decision(&mut self, path: &std::path::PathBuf, decision: &SensitiveFileDecision) {
        match decision {
            SensitiveFileDecision::AllowOnce => {}
            SensitiveFileDecision::AllowUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.sensitive_permissions.insert(path_str, true);
            }
            SensitiveFileDecision::DenyOnce => {}
            SensitiveFileDecision::DenyUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.sensitive_permissions.insert(path_str, false);
            }
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) {
        // 0. Ctrl+C: clear input (overrides quit — Ctrl+D is the quit shortcut).
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            if let Some(ref mut composer) = self.composer {
                if !composer.is_empty() {
                    composer.clear();
                    self.set_notice("Input cleared");
                }
            }
            return;
        }

        // 1. Global shortcuts (unaffected by overlays)
        if let Some(action) = self.handle_global_key(key) {
            self.process_action(action);
            return;
        }

        // 1a. Message scrolling keys work even when overlays are open.
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            if let Some(ref mut chat) = self.message_list {
                if let Some(action) = chat.handle_key_event(key) {
                    self.process_action(action);
                    return;
                }
            }
        }

        // 2. OverlayStack top-first
        if let Some(action) = self.overlays.handle_key_event(key) {
            self.process_action(action);
            return;
        }

        // 2a. Subsession navigation (when parent_session_id is set).
        if let Some(ref chat) = self.message_list {
            if let Some(ref ctx) = chat.chat_context {
                if ctx.parent_session_id.is_some() {
                    match key.code {
                        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                            self.handle_subsession_navigation(key);
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }

        // 2b. Tab: session mode switch (only when no overlay/composer popup is active).
        if key.code == KeyCode::Tab && key.modifiers.is_empty() {
            self.handle_tab_mode_switch();
            return;
        }

        // 2c. Shift+Tab / Ctrl+T: cycle thinking level.
        if (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT))
            || (key.code == KeyCode::Char('t') && key.modifiers == KeyModifiers::CONTROL)
        {
            self.process_action(Action::Session(SessionAction::CycleThinkingLevel));
            return;
        }

        // 3. Composer (when no overlay consumed the event)
        if let Some(ref mut composer) = self.composer {
            if let Some(action) = composer.handle_key_event(key) {
                self.process_action(action);
                return;
            }
        }

        // 4. MessageList (only when no overlay/composer consumed the event)
        if let Some(ref mut chat) = self.message_list {
            if let Some(action) = chat.handle_key_event(key) {
                self.process_action(action);
            }
        }
    }

    /// Handle Tab key for session mode switching.
    fn handle_tab_mode_switch(&mut self) {
        if let Some(ref pending) = self.pending_mode {
            // Cancel pending mode switch.
            self.pending_mode = None;
            self.set_notice("Mode switch cancelled");
        } else {
            let new_mode = self.mode.toggle();
            self.pending_mode = Some(new_mode);
            self.set_notice(format!(
                "Mode will switch to {} on next message",
                new_mode.title()
            ));
        }
    }

    /// Navigate between subsessions.
    fn handle_subsession_navigation(&mut self, key: KeyEvent) {
        let Some(ref chat) = self.message_list else { return };
        let Some(ref ctx) = chat.chat_context else { return };
        let Some(parent_id) = ctx.parent_session_id else { return };
        let current_id = ctx.session_id;

        match key.code {
            KeyCode::Up => {
                // Switch to parent session.
                self.switch_to_session(parent_id);
            }
            KeyCode::Down => {
                // Switch to the last (most recently delegated) child.
                let all = self.runtime.session_manager().store()
                    .list_sessions(1000, 0).unwrap_or_default();
                let children: Vec<_> = all.into_iter()
                    .filter(|s| s.parent_session_id == Some(parent_id))
                    .collect();
                if let Some(target) = children.last() {
                    self.switch_to_session(target.session_id);
                }
            }
            KeyCode::Left | KeyCode::Right => {
                let step = if key.code == KeyCode::Left { -1isize } else { 1 };
                let all = self.runtime.session_manager().store()
                    .list_sessions(1000, 0).unwrap_or_default();
                let children: Vec<_> = all.into_iter()
                    .filter(|s| s.parent_session_id == Some(parent_id))
                    .collect();
                if children.is_empty() { return; }
                let index = children.iter()
                    .position(|s| s.session_id == current_id)
                    .unwrap_or(usize::MAX);
                let next_index = if index == usize::MAX {
                    0
                } else {
                    (index as isize + step).rem_euclid(children.len() as isize) as usize
                };
                if let Some(target) = children.get(next_index) {
                    self.switch_to_session(target.session_id);
                }
            }
            _ => {}
        }
    }

    /// Switch to a different session (via SessionAction::Select).
    fn switch_to_session(&mut self, session_id: Uuid) {
        self.process_action(Action::Session(SessionAction::Select(session_id)));
    }

    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        use crossterm::event::MouseEventKind;
        use crossterm::event::MouseButton;

        // Sidebar scroll (scroll events in the sidebar area)
        if let Some(sidebar_area) = self.sidebar_area {
            let position = ratatui::layout::Position::new(mouse.column, mouse.row);
            if sidebar_area.contains(position) {
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        self.sidebar.scroll_down(3);
                    }
                    MouseEventKind::ScrollUp => {
                        self.sidebar.scroll_up(3);
                    }
                    _ => {}
                }
                return;
            }
        }

        // MessageList click-to-expand or subsession navigation
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            if let Some(ref mut chat) = self.message_list {
                if let Some(action) = chat.handle_mouse_click(mouse.column, mouse.row) {
                    self.process_action(action);
                }
            }
        }
    }

    pub fn handle_resize(&mut self, _w: u16, _h: u16) {
        // Full layout rebuild on resize (width change invalidates all line counts).
        if let Some(ref mut chat) = self.message_list {
            chat.invalidate_layout();
        }
        self.sidebar_area = None;
    }

    /// Global shortcuts that work regardless of overlay state.
    fn handle_global_key(&self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => Some(Action::Quit),
            KeyCode::F(1) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::ThemePanel))),
            KeyCode::F(2) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::AgentsPanel))),
            KeyCode::F(3) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SkillsPanel))),
            KeyCode::F(4) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SettingsPanel))),
            KeyCode::F(5) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SearchPanel))),
            KeyCode::F(6) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::MessagePanel))),
            KeyCode::F(7) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::ModelPanel))),
            KeyCode::F(8) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SessionPanel))),
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                Some(Action::Overlay(OverlayAction::Open(OverlayKind::PanelLauncher)))
            }
            KeyCode::Esc if !self.overlays.is_empty() => {
                Some(Action::Overlay(OverlayAction::CloseTop))
            }
            _ => None,
        }
    }

    // ── Action processing ──

    fn process_action(&mut self, action: Action) {
        let mut queue = vec![action];
        while let Some(action) = queue.pop() {
            match action {
                Action::Quit => {
                    self.should_quit = true;
                }
                Action::Overlay(OverlayAction::Open(kind)) => {
                    self.open_overlay(kind);
                }
                Action::Overlay(OverlayAction::Close(kind)) => {
                    self.close_overlay(kind, &mut queue);
                }
                Action::Overlay(OverlayAction::CloseTop) => {
                    if let Some(mut overlay) = self.overlays.pop() {
                        let palette = &self.current_palette;
                        let mut ctx = UpdateContext {
                            runtime: &mut self.runtime,
                            palette,
                        };
                        let follow = overlay.update(
                            &Action::Overlay(OverlayAction::Close(OverlayKind::ThemePanel)),
                            &mut ctx,
                        );
                        queue.extend(follow);
                    }
                }
                Action::Overlay(OverlayAction::CloseAll) => {
                    while self.overlays.pop().is_some() {}
                }
                Action::Theme(ThemeAction::Preview(name)) => {
                    self.current_palette = ThemePalette::from_name(name.as_str());
                }
                Action::Theme(ThemeAction::Set(name)) => {
                    self.current_palette = ThemePalette::from_name(name.as_str());
                    self.runtime
                        .update_config(|cfg| cfg.set_theme(name.as_str()));
                    let _ = self.runtime.save_config();
                }
                Action::Theme(ThemeAction::Toggle) => {
                    let current = ThemeName::parse(&self.current_palette.name.as_str())
                        .unwrap_or(ThemeName::Dark);
                    let next = current.toggle();
                    self.process_action(Action::Theme(ThemeAction::Preview(next)));
                    self.process_action(Action::Theme(ThemeAction::Set(next)));
                }
                Action::Search(SearchAction::SwitchProvider(provider)) => {
                    self.runtime
                        .update_config(|cfg| cfg.websearch.default_provider = provider);
                    let _ = self.runtime.save_config();
                }
                Action::Search(SearchAction::SaveApiKey {
                    provider,
                    key,
                    is_cx,
                }) => {
                    self.runtime.update_auth(|auth| {
                        if is_cx {
                            auth.web.google_cx = Some(key);
                        } else {
                            auth.web.search_api_keys.insert(provider, key);
                        }
                    });
                    let _ = self.runtime.save_auth();
                }
                Action::Connect(ConnectAction::SaveApiKey {
                    provider_id,
                    key,
                }) => {
                    if key.trim().is_empty() {
                        self.set_notice("API key was empty");
                        return;
                    }

                    self.runtime
                        .update_auth(|auth| auth.set_api_key(&provider_id, &key));
                    let _ = self.runtime.save_auth();

                    // Resolve the provider's default model and switch to it
                    match self
                        .runtime
                        .config()
                        .resolve_provider_default_model(&self.runtime.auth(), &provider_id)
                    {
                        Ok(model) => {
                            self.runtime.set_active_model(model.clone());

                            // Update composer's image support flag.
                            if let Some(ref mut composer) = self.composer {
                                composer.set_model_supports_images(model.supports_images);
                            }

                            // Persist model to current session if one is active
                            if let Some(session_id) = self.current_session_id {
                                if self
                                    .runtime
                                    .session_manager()
                                    .store()
                                    .load_session_record(session_id)
                                    .ok()
                                    .flatten()
                                    .is_some()
                                {
                                    let _ = self
                                        .runtime
                                        .session_manager()
                                        .update_session_model(
                                            session_id,
                                            &model.provider_id,
                                            &model.provider_display_name,
                                            &model.model_id,
                                            &model.display_name,
                                        );
                                }
                            }

                            self.set_notice(format!(
                                "Connected to {}",
                                model.provider_display_name
                            ));
                        }
                        Err(e) => {
                            self.set_notice(format!(
                                "Connected, but failed to resolve model: {e}"
                            ));
                        }
                    }
                }
                Action::Connect(ConnectAction::PruneOrphans) => {
                    let known_ids = self.runtime.config().provider_ids();
                    let mut pruned = 0usize;
                    self.runtime.update_auth(|auth| {
                        pruned = auth.prune_orphan_providers(&known_ids);
                    });
                    if pruned > 0 {
                        let _ = self.runtime.save_auth();
                        self.set_notice(format!(
                            "Pruned {pruned} orphan provider(s) from auth file"
                        ));
                    } else {
                        self.set_notice("No orphan auth entries to prune");
                    }
                }
                Action::Session(SessionAction::Select(session_id)) => {
                    // Switch to the selected session
                    self.current_session_id = Some(session_id);
                    self.scroll_target = None;

                    // Load session record and messages for chat display
                    let messages = self
                        .runtime
                        .session_manager()
                        .load_messages(session_id)
                        .unwrap_or_default();

                    let chat_context = {
                        let config = self.runtime.config();
                        let active_model = config.resolve_active_model(&self.runtime.auth()).ok();
                        let model_display = active_model
                            .as_ref()
                            .map(|m| m.label())
                            .unwrap_or_default();
                        let provider_display = active_model
                            .as_ref()
                            .map(|m| m.provider_display_name.clone())
                            .unwrap_or_default();
                        let workspace_root = self.runtime.workspace_root().to_string_lossy().to_string();

                        let mut ctx = tidev_tui_old::chat_context::ChatContext::new(
                            session_id,
                            String::new(),
                            workspace_root,
                            messages,
                            None,
                            self.runtime.active_provider_id(),
                            self.runtime.active_model_id(),
                            model_display,
                            provider_display,
                        );

                        if let Ok(Some(record)) = self.runtime.session_manager().load_session(session_id)
                        {
                            ctx.title = record.title;
                        }

                        ctx
                    };

                    let session_title = chat_context.title.clone();

                    // Create or update MessageList
                    self.message_list.get_or_insert_with(MessageList::new)
                        .set_chat_context(chat_context);

                    log::info!("Switching to session: {} ({})", session_title, session_id);

                    // Continue the agent loop if the session has pending work
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rt.continue_session(session_id).await {
                            log::error!("continue_session failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Reload) => {
                    // Broadcast to overlays so SessionPanel reloads its list.
                    let palette = &self.current_palette;
                    let mut ctx = UpdateContext {
                        runtime: &mut self.runtime,
                        palette,
                    };
                    queue.extend(self.overlays.update_all(&action, &mut ctx));
                }
                Action::Session(SessionAction::Fork(message_id)) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };

                    // Load messages from DB
                    let messages = match self.runtime.session_manager().load_messages(session_id) {
                        Ok(msgs) => msgs,
                        Err(e) => {
                            log::error!("Failed to load messages for fork: {e}");
                            return;
                        }
                    };

                    // Find the message index by UUID
                    let message_index = match messages.iter().position(|m| m.id == message_id) {
                        Some(idx) => idx,
                        None => {
                            log::warn!("Fork target message not found: {}", message_id);
                            return;
                        }
                    };

                    // Load session title from DB
                    let session_title = self
                        .runtime
                        .session_manager()
                        .load_session(session_id)
                        .ok()
                        .flatten()
                        .map(|r| r.title)
                        .unwrap_or_default();

                    let workspace_root =
                        self.runtime.workspace_root().to_string_lossy().to_string();
                    let config = self.runtime.config();
                    let auth = self.runtime.auth();
                    let active_model = match config.resolve_active_model(&auth) {
                        Ok(m) => m,
                        Err(e) => {
                            log::error!("Failed to resolve active model for fork: {e}");
                            return;
                        }
                    };

                    // Create new session
                    let new_session_id = uuid::Uuid::new_v4();
                    if let Err(e) = self.runtime.session_manager().create_session(
                        new_session_id,
                        &workspace_root,
                        &active_model.provider_id,
                        &active_model.provider_display_name,
                        &active_model.model_id,
                        &active_model.display_name,
                        &format!("Fork of {}", session_title),
                    ) {
                        log::error!("Failed to create fork session: {e}");
                        return;
                    }

                    // Copy parent's system prompt
                    if !active_model.system_prompt.is_empty() {
                        let _ = self.runtime.session_manager().store().update_session(
                            new_session_id, None, None, None, None,
                            Some(&active_model.system_prompt), None, None, None, None,
                        );
                    }

                    // Copy messages up to the selected message, assigning new IDs
                    let mut id_mapping: std::collections::HashMap<uuid::Uuid, uuid::Uuid> =
                        std::collections::HashMap::new();

                    for original in messages.iter().take(message_index + 1) {
                        let mut new_message = original.clone();
                        let new_id = uuid::Uuid::new_v4();
                        id_mapping.insert(original.id, new_id);
                        new_message.id = new_id;

                        // Update tool_call_id references to new IDs
                        if let Some(ref tool_call_id) = new_message.tool_call_id {
                            if let Ok(old_id) = uuid::Uuid::parse_str(tool_call_id) {
                                if let Some(&new_tool_call_id) = id_mapping.get(&old_id) {
                                    new_message.tool_call_id =
                                        Some(new_tool_call_id.to_string());
                                }
                            }
                        }

                        if let Err(e) = self
                            .runtime
                            .session_manager()
                            .append_message(new_session_id, &new_message)
                        {
                            log::error!("Failed to copy message to fork: {e}");
                            return;
                        }
                    }

                    // Switch to the new session
                    self.current_session_id = Some(new_session_id);
                    self.scroll_target = None;

                    self.set_notice(format!(
                        "Forked session with {} messages",
                        message_index + 1,
                    ));

                    log::info!(
                        "Forked session {} -> {} with {} messages",
                        session_id,
                        new_session_id,
                        message_index + 1,
                    );
                }
                Action::Session(SessionAction::Undo) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };
                    self.set_notice("Undo in progress...");
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rt.undo(session_id).await {
                            log::error!("Undo failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Rename(session_id, title)) => {
                    let final_title = if title.trim().is_empty() {
                        "Untitled session"
                    } else {
                        title.trim()
                    };
                    match self
                        .runtime
                        .session_manager()
                        .update_session(session_id, Some(&final_title), None)
                    {
                        Ok(_) => {
                            self.set_notice("Session title updated");
                            log::info!("Renamed session {} to {}", session_id, final_title);
                        }
                        Err(e) => log::error!("Failed to rename session: {e}"),
                    }
                }
                Action::Session(SessionAction::SetMode(new_mode)) => {
                    self.mode = new_mode;
                    self.pending_mode = None;
                    self.set_notice(format!("Mode switched to {}", self.mode.title()));
                }
                Action::Session(SessionAction::SetPendingMode(mode)) => {
                    self.pending_mode = mode;
                }
                Action::Session(SessionAction::CycleThinkingLevel) => {
                    let next = self.thinking_level.next();
                    self.thinking_level = next.clone();
                    let model = self.runtime.active_model();
                    let _ = self.runtime.set_model_thinking_level(
                        &model.provider_id,
                        &model.model_id,
                        &next.to_string(),
                    );
                    if next.is_supported() {
                        self.set_notice(format!("Thinking: {}", next.display_name()));
                    } else {
                        self.set_notice("Thinking: off");
                    }
                }
                Action::Chat(action) => {
                    match &action {
                        ChatAction::SendMessage { text, attachments } => {
                            let text = text.clone();
                            let attachments = attachments.clone();

                            // Check if this is a /command.
                            if let Some((name, args)) =
                                crate::components::composer::command_palette::CommandRegistry::new()
                                    .parse_invocation(&text)
                            {
                                use crate::components::composer::command_palette::COMMANDS;
                                if let Some(spec) =
                                    crate::components::composer::command_palette::CommandRegistry::new()
                                        .command(&name)
                                {
                                    let actions =
                                        crate::components::composer::command_palette::execute_command(
                                            spec.name,
                                            spec.action,
                                            &args,
                                        );
                                    for action in actions {
                                        self.process_action(action);
                                    }
                                    return;
                                }
                                // Unknown command — fall through to submit as prompt.
                            }

                            // Extract @-reference paths from the text (matching old
                            // `inline_file_references` behaviour).
                            let ref_paths = extract_inline_refs(&text);

                            // Also collect paths from any inline spans (the composer
                            // puts accepted @mention paths into the attachments field as
                            // a placeholder — handled below).
                            let workspace_root = self.runtime.workspace_root().clone();
                            let mut final_attachments =
                                tidev_core::attachment::build_attachments(&workspace_root, &ref_paths);

                            // Append any already-built attachments (images, files from
                            // composer spans).
                            final_attachments.extend(attachments);

                            // If no active session, create one.
                            let session_id = self.current_session_id;
                            let sid = match session_id {
                                Some(id) => id,
                                None => {
                                    match self.runtime.create_default_session("Untitled session") {
                                        Ok(id) => {
                                            self.current_session_id = Some(id);
                                            id
                                        }
                                        Err(e) => {
                                            log::error!("Failed to create session: {e}");
                                            self.set_notice("Failed to create session");
                                            return;
                                        }
                                    }
                                }
                            };

                            // Spawn submission to avoid blocking the UI.
                            let rt = self.runtime.clone();
                            self.set_notice("Sending...");
                            tokio::spawn(async move {
                                if let Err(e) = rt
                                    .submit_prompt_with_attachments(sid, text, final_attachments)
                                    .await
                                {
                                    log::error!("submit_prompt failed: {e}");
                                }
                            });
                        }
                        _ => {
                            // Forward other chat actions (scroll, stream, etc.) to MessageList.
                            if let Some(ref mut chat) = self.message_list {
                                let palette = &self.current_palette;
                                let mut ctx = UpdateContext {
                                    runtime: &mut self.runtime,
                                    palette,
                                };
                                queue.extend(chat.update(&Action::Chat(action), &mut ctx));
                            }
                        }
                    }
                }
                Action::Noop => {}
                Action::Error(msg) => {
                    self.set_notice(msg);
                }
                // ── Tool approval pipeline ──
                Action::WorkspaceBoundaryResponse { path, decision } => {
                    self.record_boundary_decision(&path, &decision);

                    // Resolve the tool's boundary flag based on decision
                    let allowed = matches!(
                        decision,
                        BoundaryDecision::AllowOnce | BoundaryDecision::AllowUntilExit
                    );

                    if self.tool_index < self.pending_tools.len() {
                        // Record the boundary approval in the tool's pending entry
                        // so process_next_tool can skip this check.
                        self.boundary_permissions.insert(
                            path.to_string_lossy().to_string(),
                            allowed,
                        );
                    }

                    self.process_next_tool();
                }
                Action::SensitiveFileResponse { path, decision } => {
                    self.record_sensitive_decision(&path, &decision);

                    if self.tool_index < self.pending_tools.len() {
                        self.sensitive_permissions.insert(
                            path.to_string_lossy().to_string(),
                            matches!(
                                decision,
                                SensitiveFileDecision::AllowOnce
                                    | SensitiveFileDecision::AllowUntilExit
                            ),
                        );
                    }

                    self.process_next_tool();
                }
                Action::PermissionResponse { decision } => {
                    let allow = matches!(
                        decision,
                        PermissionDecision::Allow | PermissionDecision::AllowAndRemember
                    );
                    let remember = matches!(
                        decision,
                        PermissionDecision::AllowAndRemember
                            | PermissionDecision::DenyAndRemember
                    );

                    if self.tool_index < self.pending_tools.len() {
                        let twv = &self.pending_tools[self.tool_index];

                        // Persist to DB if remember
                        if remember {
                            if let Some(session_id) = self.current_session_id {
                                if let Err(e) = self
                                    .runtime
                                    .session_manager()
                                    .store()
                                    .remember_tool_permission(
                                        session_id,
                                        &twv.permission_key,
                                        allow,
                                    )
                                {
                                    log::warn!("Failed to remember tool permission: {e}");
                                }
                            }
                        }

                        // Build the approved tool
                        let (rejection, allow_outside, sensitive_approved) = if allow {
                            let path_str = twv
                                .workspace_boundary_violation
                                .as_ref()
                                .map(|p| p.to_string_lossy().to_string());
                            let sensitive_str = twv
                                .sensitive_file_violation
                                .as_ref()
                                .map(|p| p.to_string_lossy().to_string());
                            (
                                None,
                                path_str
                                    .and_then(|p| Self::is_path_allowed(&self.boundary_permissions, &p))
                                    .unwrap_or(false),
                                sensitive_str
                                    .and_then(|p| Self::is_path_allowed(&self.sensitive_permissions, &p))
                                    .unwrap_or(false),
                            )
                        } else {
                            let msg = if remember {
                                format!("Tool '{}' was denied and remembered", twv.permission_label)
                            } else {
                                format!("Tool '{}' was denied", twv.permission_label)
                            };
                            (
                                Some(ToolExecutionResult::new(msg)),
                                false,
                                false,
                            )
                        };

                        let child_session_id = if twv.tool_call.name == "task" {
                            Some(Uuid::new_v4())
                        } else {
                            None
                        };

                        self.approved_tools.push(ApprovedTool {
                            tool_call: twv.tool_call.clone(),
                            rejection,
                            child_session_id,
                            allow_outside,
                            sensitive_file_approved: sensitive_approved,
                        });
                    }

                    self.tool_index += 1;
                    self.process_next_tool();
                }
                Action::QuestionResponse { output } => {
                    if self.tool_index < self.pending_tools.len() {
                        let twv = &self.pending_tools[self.tool_index];
                        let result = match output {
                            Some(answers) => ToolExecutionResult::new(answers),
                            None => ToolExecutionResult::new(
                                "Tool 'question' was dismissed by user",
                            ),
                        };

                        self.approved_tools.push(ApprovedTool {
                            tool_call: twv.tool_call.clone(),
                            rejection: Some(result),
                            child_session_id: None,
                            allow_outside: false,
                            sensitive_file_approved: false,
                        });
                    }

                    self.tool_index += 1;
                    self.process_next_tool();
                }
                _ => {
                    // Broadcast to all overlays
                    let palette = &self.current_palette;
                    let mut ctx = UpdateContext {
                        runtime: &mut self.runtime,
                        palette,
                    };
                    queue.extend(self.overlays.update_all(&action, &mut ctx));
                }
            }
        }
    }

    fn open_overlay(&mut self, kind: OverlayKind) {
        let component: Option<Box<dyn Component>> = match kind {
            OverlayKind::ThemePanel => {
                let current = ThemeName::parse(&self.current_palette.name.as_str())
                    .unwrap_or(ThemeName::Dark);
                Some(Box::new(ThemePanel::new(current)))
            }
            OverlayKind::AgentsPanel => Some(Box::new(AgentsPanel::new())),
            OverlayKind::SkillsPanel => {
                let catalog = &self.runtime.skills;
                let skills: Vec<SkillItem> = catalog
                    .all()
                    .iter()
                    .map(|s| SkillItem {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        location: s.location.clone(),
                    })
                    .collect();
                Some(Box::new(SkillsPanel::new(skills)))
            }
            OverlayKind::SettingsPanel => {
                let config = self.runtime.config();
                Some(Box::new(SettingsPanel::new(&config)))
            }
            OverlayKind::SearchPanel => {
                let config = self.runtime.config();
                let auth = self.runtime.auth();
                Some(Box::new(SearchPanel::new(
                    &config.websearch.default_provider,
                    &auth,
                )))
            }
            OverlayKind::MessagePanel => {
                let messages = self
                    .message_list
                    .as_ref()
                    .and_then(|ml| ml.chat_context.as_ref())
                    .map(|ctx| {
                        ctx.visible_messages()
                            .iter()
                            .filter(|m| matches!(m.role, tidev_types::message::MessageRole::User))
                            .enumerate()
                            .map(|(i, m)| MessagePanelMessage {
                                message_id: m.id,
                                content: strip_system_reminder_tags(&m.content),
                                created_at: m.created_at,
                                mode: m.mode,
                                original_index: i,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Some(Box::new(MessagePanel::new(messages)))
            }
            OverlayKind::ModelPanel => {
                use crate::components::overlays::model::ModelPanelTab;
                let config = self.runtime.config();
                let auth = self.runtime.auth();
                let active_model = match config.resolve_active_model(&auth) {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to resolve active model: {e}");
                        return;
                    }
                };

                let mut tabs = vec![ModelPanelTab::new(
                    "general",
                    "General",
                    &active_model.label(),
                )];
                for agent_type in AgentType::all() {
                    if *agent_type == AgentType::General {
                        continue;
                    }
                    let ty = agent_type.display_name();
                    let label = config.agent_model_display(ty);
                    tabs.push(ModelPanelTab::new(
                        ty,
                        agent_type.display_name(),
                        &label,
                    ));
                }

                let connected_models = config.connected_models(&auth);
                Some(Box::new(ModelPanel::new(
                    tabs,
                    connected_models,
                    active_model,
                )))
            }
            OverlayKind::SessionPanel => {
                use crate::components::overlays::session::SessionViewMode;
                let store = self.runtime.session_manager().store();
                let sessions = store.list_sessions(1000, 0).unwrap_or_default();
                let current_session_id = self
                    .current_session_id
                    .or_else(|| {
                        self.message_list
                            .as_ref()
                            .and_then(|ml| ml.chat_context.as_ref())
                            .map(|ctx| ctx.session_id)
                    })
                    .unwrap_or(uuid::Uuid::nil());
                Some(Box::new(SessionPanel::new(
                    sessions,
                    SessionViewMode::CurrentWorkspace,
                    current_session_id,
                )))
            }
            OverlayKind::ForkConfirmDialog {
                message_id,
                message_count,
            } => Some(Box::new(ForkConfirmDialog::new(message_id, message_count))),
            OverlayKind::UndoConfirmDialog {
                message_id,
                content,
            } => Some(Box::new(UndoConfirmDialog::new(message_id, content))),
            OverlayKind::RenameDialog => {
                let session_id = self.current_session_id.unwrap_or(uuid::Uuid::nil());
                let title = self
                    .runtime
                    .session_manager()
                    .load_session(session_id)
                    .ok()
                    .flatten()
                    .map(|r| r.title)
                    .unwrap_or_default();
                Some(Box::new(RenameDialog::new(session_id, title)))
            }
            OverlayKind::ImageViewer => {
                // ImageViewer requires data from a chat message (data_url + filename).
                // This is triggered by ChatAction::ToggleImage which will be routed
                // once Chat/MessageList is migrated (Phase 6). For now return None
                // so opening ImageViewer is a no-op until the Chat component provides data.
                None
            }
            OverlayKind::ConnectDialog => {
                Some(Box::new(ConnectDialog::new()))
            }
            OverlayKind::PanelLauncher => {
                Some(Box::new(PanelLauncher::new()))
            }
            // Permission / security dialogs are triggered by handle_tui_request,
            // not by user keystrokes. These branches exist as fallback placeholders.
            OverlayKind::PermissionDialog
            | OverlayKind::QuestionDialog
            | OverlayKind::WorkspaceBoundaryDialog
            | OverlayKind::SensitiveFileDialog
            | OverlayKind::CommandPalette
            | OverlayKind::PanelLauncher => None,
            _ => None,
        };
        if let Some(component) = component {
            self.overlays.push(component);
        }
    }

    fn close_overlay(&mut self, kind: OverlayKind, queue: &mut Vec<Action>) {
        if let Some(mut overlay) = self.overlays.pop() {
            let palette = &self.current_palette;
            let mut ctx = UpdateContext {
                runtime: &mut self.runtime,
                palette,
            };
            queue.extend(
                overlay.update(
                    &Action::Overlay(OverlayAction::Close(kind)),
                    &mut ctx,
                ),
            );
        }
    }

    // ── Drawing ──

    pub fn draw(&mut self, frame: &mut Frame) {
        let palette = self.current_palette;
        let area = frame.area();

        // Background
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        // Determine sidebar visibility and split the layout.
        // Use the same threshold as the old TUI.
        const SIDEBAR_GAP: u16 = 2;
        let sidebar_width = self.runtime.config().ui.sidebar_width;
        let sidebar_visible = area.width
            >= sidebar_width.saturating_add(70).saturating_add(SIDEBAR_GAP);
        let (main_area, sidebar_area) = if sidebar_visible {
            let split = ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Min(20),
                ratatui::layout::Constraint::Length(SIDEBAR_GAP),
                ratatui::layout::Constraint::Length(sidebar_width),
            ])
            .split(area);
            (split[0], Some(split[2]))
        } else {
            self.sidebar_area = None;
            (area, None)
        };

        // Calculate composer height if present.
        let composer_height = self
            .composer
            .as_ref()
            .map(|c| {
                let width = main_area.width.saturating_sub(4);
                c.preferred_height(width, 6).min(main_area.height.saturating_sub(2))
            })
            .unwrap_or(0);

        // Split: message area + composer area (reserve 1 line for notice).
        let notice_height: u16 = 1;
        let (content_area, notice_line) = if composer_height > 0 {
            let split = ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Min(1),
                ratatui::layout::Constraint::Length(composer_height),
                ratatui::layout::Constraint::Length(notice_height),
            ])
            .split(main_area);
            (split[0], split[2])
        } else {
            let split = ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Min(1),
                ratatui::layout::Constraint::Length(notice_height),
            ])
            .split(main_area);
            (split[0], split[1])
        };

        // Chat message area (when session is active)
        if let Some(ref mut chat) = self.message_list {
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                chat_context: None,
                mode: self.mode,
                pending_mode: self.pending_mode,
            };
            chat.draw(frame, content_area, &draw_ctx);
        } else if self.overlays.is_empty() {
            // Welcome / status text when no session or overlay is active
            let welcome = Paragraph::new(Line::from(vec![
                Span::styled(
                    "tidev",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  —  "),
                Span::styled("F1", Style::default().fg(palette.accent)),
                Span::raw(" Theme  ·  "),
                Span::styled("F2", Style::default().fg(palette.accent)),
                Span::raw(" Agents  ·  "),
                Span::styled("F3", Style::default().fg(palette.accent)),
                Span::raw(" Skills  ·  "),
                Span::styled("F4", Style::default().fg(palette.accent)),
                Span::raw(" Settings  ·  "),
                Span::styled("F5", Style::default().fg(palette.accent)),
                Span::raw(" Search  ·  "),
                Span::styled("F6", Style::default().fg(palette.accent)),
                Span::raw(" Messages  ·  "),
                Span::styled("F7", Style::default().fg(palette.accent)),
                Span::raw(" Models  ·  "),
                Span::styled("F8", Style::default().fg(palette.accent)),
                Span::raw(" Sessions  ·  "),
                Span::styled("Ctrl+C", Style::default().fg(palette.accent)),
                Span::raw(" quit"),
            ]))
            .style(Style::default().fg(palette.text).bg(palette.background));
            frame.render_widget(welcome, content_area);
        }

        // ── Composer ─────────────────────────────────────────────────
        // Rendered above the notice line, below the message area.
        if let Some(ref mut composer) = self.composer {
            let composer_area = Rect {
                x: main_area.x,
                y: main_area.bottom().saturating_sub(composer_height + notice_height),
                width: main_area.width,
                height: composer_height,
            };
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                chat_context: None,
                mode: self.mode,
                pending_mode: self.pending_mode,
            };
            composer.draw(frame, composer_area, &draw_ctx);
        }

        // Build DrawContext for overlays
        let draw_ctx = DrawContext {
            palette,
            focused: true,
            chat_context: None,
            mode: self.mode,
            pending_mode: self.pending_mode,
        };

        // Draw overlays
        self.overlays.draw(frame, area, &draw_ctx);

        // ── Sidebar ───────────────────────────────────────────────────
        if let Some(sidebar_area) = sidebar_area {
            self.sidebar_area = Some(sidebar_area);
            let chat_ctx = self
                .message_list
                .as_ref()
                .and_then(|ml| ml.chat_context.as_ref());
            self.sidebar.draw(
                frame,
                sidebar_area,
                palette,
                self.runtime.workspace_root(),
                chat_ctx,
                self.mode,
                self.pending_mode,
                self.context_usage.as_ref(),
                &self.todos,
            );
        }

        // ── Status notice (last_notice) with token usage ──
        let notice_text = if let Some(ref usage) = self.context_usage {
            // Format: "notice · 45.2% (12K/26K)"
            let max_context = self
                .runtime
                .active_model()
                .context_window;
            let total = usage.input_tokens as u64 + usage.output_tokens as u64;
            let pct = if max_context > 0 {
                (total as f64 / max_context as f64 * 100.0).min(100.0)
            } else {
                0.0
            };
            let used_k = usage.input_tokens / 1000;
            let max_k = (max_context as u32) / 1000;
            let token_part = format!("{pct:.1}% ({used_k}K/{max_k}K)");

            match &self.last_notice {
                Some((msg, _)) if !msg.is_empty() => {
                    format!("{msg} · {token_part}")
                }
                _ => token_part,
            }
        } else {
            self.last_notice
                .as_ref()
                .map(|(msg, _)| msg.clone())
                .unwrap_or_default()
        };

        if !notice_text.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    &notice_text,
                    Style::default().fg(palette.muted),
                )))
                .style(Style::default().bg(palette.background)),
                Rect::new(main_area.x + 1, notice_line.y, notice_line.width.saturating_sub(2), 1),
            );
        }

        // ── Toast notification ──
        // Small popup at the top-right, auto-expires.
        if let Some((msg, expires_at)) = &self.toast.clone() {
            if Instant::now() < *expires_at {
                let toast_width = (msg.len() as u16).min(32).saturating_add(2);
                let toast_rect = Rect::new(
                    area.right().saturating_sub(toast_width + 1),
                    area.y + 1,
                    toast_width,
                    3,
                );
                frame.render_widget(Clear, toast_rect);
                let block = Block::default()
                    .style(Style::default().bg(palette.panel).fg(palette.text));
                let centered = format!("\n{}", msg);
                frame.render_widget(
                    Paragraph::new(centered)
                        .style(Style::default().bg(palette.panel).fg(palette.text))
                        .alignment(Alignment::Center)
                        .block(block),
                    toast_rect,
                );
            } else {
                self.toast = None;
            }
        }
    }
}

// ── Inline @-reference extraction ───────────────────────────────────────

/// Extract file/directory paths from `@path` references in the prompt text.
///
/// Mirrors the old `tidev_tui::App::inline_file_references` behaviour:
/// finds `@` that is not preceded by a word character or backtick, and
/// captures the following path.
fn extract_inline_refs(prompt: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    let bytes = prompt.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Find the next '@' byte.
        let at_pos = match bytes[i..].iter().position(|&b| b == b'@') {
            Some(pos) => i + pos,
            None => break,
        };

        // Look-behind: check that '@' is not preceded by a word char or backtick.
        if at_pos > 0 {
            let prev = bytes[at_pos - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'`' {
                i = at_pos + 1;
                continue;
            }
        }

        // Capture the path: starting from at_pos + 1.
        let start = at_pos + 1;
        if start >= len {
            break;
        }

        // Path characters: non-whitespace, not backtick, not comma, not period at end.
        let mut end = start;
        while end < len {
            let c = bytes[end];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
                || c == b'`' || c == b','
            {
                break;
            }
            // Comma is allowed in middle of path (not at end).
            end += 1;
        }

        // Skip empty captures.
        if end == start {
            i = end;
            continue;
        }

        let path = &prompt[start..end];
        // Trim trailing period (allowed at end only for dotted extensions).
        let path = path.trim_end_matches('.');
        if !path.is_empty() && !seen.contains(path) {
            seen.insert(path.to_string());
            paths.push(path.to_string());
        }

        i = end;
    }

    paths
}
