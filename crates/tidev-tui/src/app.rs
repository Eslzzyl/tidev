//! New-architecture App root component.
//!
//! Owns the Runtime, manages the component tree via OverlayStack,
//! routes Actions, and dispatches async commands.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::Utc;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use tidev_core::{ApprovedTool, ToolCallWithViolations};
use tidev_core::TuiResponse;
use tidev_types::agent_type::AgentType;
use tidev_types::message::{BackendEvent, Message, MessageAttachment, ToolExecutionResult, COMPACTION_MESSAGE_LABEL};
use tidev_types::tools::QuestionArgs;
use unicode_width::UnicodeWidthStr;
use crate::theme::{ThemeName, ThemePalette};
use tidev_types::prompts::SessionMode;
use tidev_types::reasoning::ThinkingLevelType;
use tidev_types::tools::TodoItem;
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
use crate::components::chat::render::wrap_text_lines;
use crate::components::composer::Composer;
use crate::components::sidebar::Sidebar;
use crate::components::desktop_notification::NotificationManager;
use crate::components::notification::NotificationState;
use crate::components::selection::{MouseSelection, copy_to_clipboard};
use crate::context::{DrawContext, InitContext, UpdateContext};
use ratatui_image::picker::Picker;
use crate::utils::strip_system_reminder_tags;

/// Token usage statistics for the current/last request.
#[derive(Clone, Debug)]
pub(crate) struct ContextUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub tokens_per_second: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppScreen {
    Welcome,
    Chat,
}

/// Max queued prompt cards visible in the frozen area above the composer.
const MAX_VISIBLE_QUEUED_PROMPTS: usize = 4;
/// Max wrapped lines per queued prompt card.
const MAX_QUEUED_PROMPT_LINES: usize = 3;

pub struct App {
    pub(crate) runtime: tidev_core::Runtime,
    overlays: OverlayStack,
    current_palette: ThemePalette,
    should_quit: bool,
    /// Pending scroll target set by ChatAction::ScrollTo (consumed by Chat component).
    scroll_target: Option<uuid::Uuid>,
    /// Text selected in the composer input area, pending clipboard copy in draw().
    pending_input_copy: Option<String>,
    /// Current active session (set by SessionPanel when switching sessions).
    current_session_id: Option<uuid::Uuid>,
    /// Current session mode (Build / Plan).
    mode: SessionMode,
    /// Pending mode switch (applied on next Finished with no tool calls).
    pending_mode: Option<SessionMode>,
    /// Current thinking level for the active model.
    thinking_level: ThinkingLevelType,
    /// Whether the subagent (task tool) is enabled.
    subagent_enabled: bool,
    /// Status notice shown at the bottom of the screen (plain text, no timeout).
    last_notice: Option<(String, Instant)>,
    /// Transient popup notifications (auto-expire).
    notifications: NotificationState,
    /// Desktop/terminal notifications (OSC 9 / BEL).
    desktop_notifications: NotificationManager,
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

    /// Mouse text selection state for the message area.
    mouse_selection: MouseSelection,

    /// Cached sidebar area for mouse hit-testing.
    sidebar_area: Option<Rect>,

    /// Cached terminal area for overlay mouse hit-testing.
    terminal_area: Rect,

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

    /// Transient single-message toast popup (top-right corner, auto-expires).
    toast: Option<(String, Instant)>,

    /// Abort confirmation deadline (set on first Esc press, consumed on second).
    abort_confirmation_deadline: Option<Instant>,

    /// Queue of prompts waiting to be sent (when a request is already in progress).
    pending_prompt_queue: Vec<QueuedPrompt>,

    /// Cached bounds for queued prompt card mouse hit-testing.
    queued_card_bounds: Vec<(usize, Rect)>,
    /// Currently hovered queued prompt index.
    hovered_queued_index: Option<usize>,

    /// Compact queued to run after the current request finishes.
    pending_compact: bool,

    /// Compaction is currently in progress.
    is_compacting: bool,

    /// Current screen state.
    screen: AppScreen,

    /// Spinner animation start time.
    spinner_start: Instant,
    /// Last rendered spinner frame index (0-3 for the 4 ASCII frames).
    /// Used to detect when the spinner has advanced so we can re-dirty
    /// during the pending-request gap and keep the animation alive.
    pub(crate) last_spinner_frame: u64,
    /// Terminal graphics protocol picker (cached to avoid blocking on every
    /// ImageViewer open).
    image_picker: Option<Picker>,
}

/// A prompt queued for submission when the current request finishes.
#[derive(Clone, Debug)]
struct QueuedPrompt {
    prompt: String,
    attachments: Vec<MessageAttachment>,
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
        let notif_config = runtime.config().notifications.clone();
        let subagent_enabled = runtime.config().subagent.enabled;

        Self {
            runtime,
            overlays: OverlayStack::new(),
            current_palette,
            should_quit: false,
            scroll_target: None,
            pending_input_copy: None,
            current_session_id: None,
            mode: SessionMode::Build,
            pending_mode: None,
            thinking_level,
            subagent_enabled,
            last_notice: None,
            notifications: NotificationState::new(),
            desktop_notifications: NotificationManager::new(&notif_config),
            request_rx,
            event_rx,
            pending_response_tx: None,
            pending_tools: Vec::new(),
            tool_index: 0,
            approved_tools: Vec::new(),
            boundary_permissions: HashMap::new(),
            sensitive_permissions: HashMap::new(),
            context_usage: None,
            toast: None,
            abort_confirmation_deadline: None,
            pending_prompt_queue: Vec::new(),
            queued_card_bounds: Vec::new(),
            hovered_queued_index: None,
            pending_compact: false,
            is_compacting: false,
            screen: AppScreen::Welcome,
            spinner_start: Instant::now(),
            last_spinner_frame: 0,
            message_list: None,
            sidebar: Sidebar::new(),
            mouse_selection: MouseSelection::default(),
            sidebar_area: None,
            terminal_area: Rect::new(0, 0, 0, 0),
            todos: Vec::new(),
            image_picker: Picker::from_query_stdio().ok(),
            composer: {
                let mut c = Composer::new("Ask tidev about your code, task, or question...");
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

    /// Whether any component needs re-drawing.
    pub fn is_dirty(&self) -> bool {
        self.message_list.as_ref().is_some_and(|c| c.is_dirty())
            || self.composer.as_ref().is_some_and(|c| c.is_dirty())
    }

    /// Mark all components as clean after rendering.
    pub fn mark_clean(&mut self) {
        if let Some(c) = &mut self.message_list {
            c.mark_clean();
        }
        if let Some(c) = &mut self.composer {
            c.mark_clean();
        }
    }

    // ── Notifications ──

    /// Set a persistent status notice shown at the bottom of the screen.
    pub(crate) fn set_notice(&mut self, msg: impl Into<String>) {
        self.last_notice = Some((msg.into(), Instant::now()));
    }

    /// Set a transient toast notification (auto-expires after `duration`).
    pub(crate) fn set_toast(&mut self, msg: impl Into<String>, duration: std::time::Duration) {
        let msg = msg.into();
        self.notifications.add(msg.clone(), duration);
        self.toast = Some((msg, Instant::now() + duration));
    }

    /// Forward terminal focus change to desktop notification manager.
    pub(crate) fn handle_focus_event(&self, focused: bool) {
        self.desktop_notifications.set_focused(focused);
    }

    /// Accessor for `spinner_start` so tui.rs can compute spinner frame.
    pub(crate) fn spinner_elapsed(&self) -> std::time::Duration {
        self.spinner_start.elapsed()
    }

    /// Whether a compaction is currently in progress.
    pub(crate) fn is_compacting(&self) -> bool {
        self.is_compacting
    }

    /// Whether the app currently has an active request (streaming or pending tool approval).
    pub(crate) fn has_active_request(&self) -> bool {
        // The Runtime is the single source of truth for agent-loop liveness.
        if self.runtime.is_busy() {
            return true;
        }
        // Local UI supplement — pending tool approval dialogs.  The Runtime
        // will also be blocked waiting for approval, but there's a tiny
        // window between receiving the request and blocking the loop.
        if !self.pending_tools.is_empty() {
            return true;
        }
        false
    }

    fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        const FRAME_DURATION_MS: u128 = 100;
        let elapsed = self.spinner_start.elapsed().as_millis();
        let frame_index = (elapsed / FRAME_DURATION_MS) as usize;
        FRAMES[frame_index % FRAMES.len()]
    }

    /// Abort the current request: cancel the agent loop, drop pending approvals,
    /// and clear all pending state.
    fn abort_current_request(&mut self) {
        // Cancel the agent loop.
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            runtime.cancel().await;
        });

        // Finalise streaming message and append an error notice.
        if let Some(ref mut chat) = self.message_list {
            chat.append_interrupted_message();
        }

        // Drop the pending response channel so the agent loop unblocks.
        self.pending_response_tx = None;

        // Clear pending tools, queued prompts, and queued compact.
        self.pending_tools.clear();
        self.tool_index = 0;
        self.approved_tools.clear();
        self.pending_prompt_queue.clear();
        self.pending_compact = false;
        self.is_compacting = false;

        // Reset abort state.
        self.abort_confirmation_deadline = None;

        self.set_notice("Request cancelled");
    }

    /// Submit queued prompts now that the current request has finished.
    fn flush_pending_prompt_queue(&mut self) {
        while let Some(queued) = self.pending_prompt_queue.first() {
            let text = queued.prompt.clone();
            let attachments = queued.attachments.clone();
            self.pending_prompt_queue.remove(0);

            let Some(session_id) = self.current_session_id else { continue };

            let mode = self.mode;
            let thinking_level = self.thinking_level.clone();
            let rt = self.runtime.clone();
            tokio::spawn(async move {
                if let Err(e) = rt
                    .submit_prompt_with_attachments(session_id, mode, text, attachments, Some(thinking_level))
                    .await
                {
                    log::error!("flush queued prompt failed: {e}");
                }
            });
        }
    }

    /// Start compaction immediately (push streaming message, spawn task).
    fn execute_compact(&mut self) {
        let session_id = match self.current_session_id {
            Some(id) => id,
            None => return,
        };
        // Push streaming compaction message immediately so the
        // divider line and initial state are visible.
        if let Some(ref mut chat) = self.message_list {
            if let Some(ref mut ctx) = chat.active_chat_context_mut() {
                let msg = Message::streaming(
                    tidev_types::message::MessageRole::System,
                    format!("{}\n\n", COMPACTION_MESSAGE_LABEL),
                );
                ctx.push(msg);
            }
            chat.invalidate_layout();
        }
        self.set_notice("Compacting session context...");
        self.is_compacting = true;
        let rt = self.runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = rt.compact_session(session_id, None).await {
                log::error!("Compact failed: {e}");
            }
        });
    }

    /// Open the composer content in an external editor. The TUI is suspended
    /// while the editor runs, then the edited text replaces the composer.
    fn open_external_editor(&mut self) {
        let Some(ref mut composer) = self.composer else {
            self.set_notice("No composer available");
            return;
        };

        let text = composer.text().to_string();
        if text.is_empty() {
            self.set_notice("No text to edit");
            return;
        }

        let ui_config = self.runtime.config().ui;
        match crate::editor::open_external_editor(&text, &ui_config) {
            Ok(edited) => {
                composer.set_text(edited);
                self.set_notice("Editor closed — text updated");
            }
            Err(e) => {
                self.set_notice(format!("Editor: {e}"));
            }
        }

        // Force full redraw after suspend/resume.
        if let Some(ref mut chat) = self.message_list {
            chat.invalidate_layout();
        }
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
                session_id,
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                duration_ms,
                ..
            } if Some(session_id) == self.current_session_id => {
                // Store context usage for display in status bar.
                self.context_usage = Some(ContextUsage {
                    input_tokens,
                    output_tokens,
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
                        Some(model_id.clone()),
                        Some(Utc::now()),
                        Some(self.mode),
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
                session_id,
                attempt,
                max_attempts,
                reason,
                ..
            } if Some(session_id) == self.current_session_id => {
                log::info!("Retrying (attempt {attempt}/{max_attempts}): {reason}");
                self.set_toast(
                    format!("Retry {attempt}/{max_attempts}: {reason}"),
                    std::time::Duration::from_secs(5),
                );
            }
            BackendEvent::Failed {
                session_id,
                error,
                ..
            } if Some(session_id) == self.current_session_id => {
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
                self.desktop_notifications.notify(&format!("Request failed: {error}"));
            }
            BackendEvent::Finished {
                session_id,
                turn,
                ..
            } if Some(session_id) == self.current_session_id => {
                // Apply pending mode switch on final turn (no tool calls).
                if turn.tool_calls.is_empty() {
                    if let Some(new_mode) = self.pending_mode.take() {
                        self.mode = new_mode;
                        self.set_notice(format!("Mode switched to {}", self.mode.title()));
                    }
                    self.desktop_notifications.notify("Response complete");
                }

                // Process any queued prompts now that the request finished.
                self.flush_pending_prompt_queue();

                // If a compact was queued and no request is active, run it now.
                if self.pending_compact && !self.has_active_request() {
                    self.pending_compact = false;
                    self.execute_compact();
                }
            }
            BackendEvent::ContextCompacted { error: Some(ref msg), .. } => {
                self.is_compacting = false;
                self.set_notice(format!("Compaction failed: {msg}"));
            }
            BackendEvent::ContextCompacted { error: None, .. } => {
                self.is_compacting = false;
                self.set_notice("Context compacted");
            }
            BackendEvent::UserMessageCreated { session_id, message } => {
                if let Some(ref mut chat) = self.message_list {
                    if let Some(ref mut ctx) = chat.active_chat_context_mut()
                        && ctx.session_id == session_id {
                            ctx.push(message);
                        }
                    chat.invalidate_layout();
                }
            }
            BackendEvent::MessagesTruncated { session_id, kept_count } => {
                if let Some(ref mut chat) = self.message_list {
                    if let Some(ref mut ctx) = chat.active_chat_context_mut()
                        && ctx.session_id == session_id {
                            ctx.messages.truncate(kept_count);
                            ctx.revert_message_id = None;
                        }
                    chat.invalidate_layout();
                }
            }
            BackendEvent::UndoCompleted {
                target_id,
                message_content,
                ..
            } => {
                if let Some(ref mut chat) = self.message_list {
                    if let Some(ref mut ctx) = chat.active_chat_context_mut() {
                        if target_id == Uuid::nil() {
                            ctx.revert_message_id = None;
                        } else {
                            ctx.revert_message_id = Some(target_id);
                        }
                    }
                    chat.follow_tail = true;
                    chat.invalidate_layout();
                }
                if let Some(ref mut composer) = self.composer {
                    if !message_content.is_empty() {
                        composer.set_text(strip_system_reminder_tags(&message_content));
                    } else {
                        composer.clear();
                    }
                }
                self.set_notice("Undo complete");
            }
            _ => {
                // Events already forwarded to MessageList above:
                //   Delta, ReasoningDelta, ToolCallUpdated, Finished, ToolCompleted,
                //   SubagentStatus, TurnStarting, StreamEnd,
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
            let (boundary_path, sensitive_path, is_question, args, perm_key, perm_label,
                  needs_confirmation, tc)
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
                    twv.needs_confirmation,
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

            // Step 4: PermissionDialog — only for tools that need confirmation
            // (Write/Edit/Execute). Read-only tools (Read/Search/Session) that
            // pass the boundary & sensitive file checks are auto-approved here.
            if needs_confirmation {
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
            } else {
                log::info!(
                    "Auto-approving tool {} (no confirmation needed) ({}/{})",
                    perm_label,
                    current_index,
                    total
                );
                self.approved_tools.push(ApprovedTool {
                    tool_call: tc,
                    rejection: None,
                    child_session_id: None,
                    allow_outside: false,
                    sensitive_file_approved: false,
                });
                self.tool_index += 1;
                continue;
            }
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
    fn record_boundary_decision(&mut self, path: &Path, decision: &BoundaryDecision) {
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
    fn record_sensitive_decision(&mut self, path: &Path, decision: &SensitiveFileDecision) {
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
        // 0. Esc: close any composer popup first (overrides abort confirmation).
        if key.code == KeyCode::Esc
            && self.composer.as_ref().is_some_and(|c| c.has_popup())
            && self.overlays.is_empty()
        {
            if let Some(ref mut composer) = self.composer {
                composer.handle_key_event(key);
            }
            return;
        }

        // 0. Abort confirmation: double-Esc to cancel current request.
        if key.code == KeyCode::Esc
            && self.overlays.is_empty()
            && (self.has_active_request() || !self.pending_prompt_queue.is_empty())
        {
            if self.abort_confirmation_deadline
                .is_some_and(|deadline| deadline > Instant::now())
            {
                self.abort_current_request();
                return;
            }
            self.abort_confirmation_deadline = Some(Instant::now() + Duration::from_secs(3));
            self.set_notice("Press Esc again within 3 seconds to stop the current request");
            return;
        }
        self.abort_confirmation_deadline = None;

        // 0. Ctrl+C: clear input (overrides quit — Ctrl+D is the quit shortcut).
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            if let Some(ref mut composer) = self.composer
                && !composer.is_empty() {
                    composer.clear();
                    self.set_notice("Input cleared");
                }
            return;
        }

        // 0a. Alt+E: open external editor with current composer text.
        if key.code == KeyCode::Char('e') && key.modifiers == KeyModifiers::ALT {
            self.open_external_editor();
            return;
        }

        // 1. Global shortcuts (unaffected by overlays)
        if let Some(action) = self.handle_global_key(key) {
            self.process_action(action);
            return;
        }

        // 1a. Message scrolling keys work even when overlays are open.
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
            && let Some(ref mut chat) = self.message_list
                && let Some(action) = chat.handle_key_event(key) {
                    self.process_action(action);
                    return;
                }

        // 2. OverlayStack top-first
        if let Some(action) = self.overlays.handle_key_event(key) {
            self.process_action(action);
            return;
        }

        // 2a. Subsession navigation (when parent_session_id is set).
        if let Some(ref chat) = self.message_list
            && let Some(ctx) = chat.active_chat_context()
                && ctx.parent_session_id.is_some() {
                    match key.code {
                        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                            self.handle_subsession_navigation(key);
                            return;
                        }
                        _ => {}
                    }
                }

        // 2b. Tab: session mode switch (only when no composer popup is active).
        if key.code == KeyCode::Tab && key.modifiers.is_empty()
            && !self.composer.as_ref().is_some_and(|c| c.has_popup()) {
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
        if let Some(ref mut composer) = self.composer
            && let Some(action) = composer.handle_key_event(key) {
                self.process_action(action);
                return;
            }

        // 4. MessageList (only when no overlay/composer consumed the event)
        if let Some(ref mut chat) = self.message_list
            && let Some(action) = chat.handle_key_event(key) {
                self.process_action(action);
            }
    }

    /// Handle bracketed paste text from the terminal (⌘V / Shift+Insert).
    ///
    /// Routes the pasted content to the composer when no overlay is active.
    /// When the pasted text is empty (clipboard contains only image data),
    /// falls back to direct clipboard reading for image paste.
    pub(crate) fn handle_paste(&mut self, text: String) {
        // If an overlay is open, defer paste — the overlay will handle
        // paste via its own Ctrl+V + arboard logic for now.
        if !self.overlays.is_empty() {
            return;
        }
        if let Some(ref mut composer) = self.composer {
            if let Some(action) = composer.handle_paste(&text) {
                self.process_action(action);
            }
        }
    }

    /// Single dispatch point for all crossterm events.
    ///
    /// Both the batch drain (Phase 1a) and the idle wait (Phase 3) in the
    /// event loop call this method so that every event variant is handled
    /// in exactly one place.
    pub(crate) fn handle_crossterm_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key_event(key),
            Event::Mouse(mouse) => self.handle_mouse_event(mouse),
            Event::Paste(text) => self.handle_paste(text),
            Event::Resize(w, h) => self.handle_resize(w, h),
            Event::FocusGained => self.handle_focus_event(true),
            Event::FocusLost => self.handle_focus_event(false),
        }
    }

    /// Handle Tab key for session mode switching.
    fn handle_tab_mode_switch(&mut self) {
        if self.pending_mode.is_some() {
            // Cancel pending mode switch.
            self.pending_mode = None;
            self.set_notice("Mode switch cancelled");
        } else if self.has_active_request() || !self.pending_prompt_queue.is_empty() {
            // Request in progress: defer mode switch until request completes.
            let new_mode = self.mode.toggle();
            self.pending_mode = Some(new_mode);
            self.set_notice(format!(
                "Mode will switch to {} on next message",
                new_mode.title()
            ));
        } else {
            // Idle: switch mode immediately.
            self.mode = self.mode.toggle();
            self.set_notice(format!("Mode switched to {}", self.mode.title()));
        }
    }

    /// Navigate between subsessions.
    fn handle_subsession_navigation(&mut self, key: KeyEvent) {
        let Some(ref chat) = self.message_list else { return };
        let Some(ctx) = chat.active_chat_context() else { return };
        let Some(parent_id) = ctx.parent_session_id else { return };
        let current_id = ctx.session_id;

        match key.code {
            KeyCode::Up => {
                // Switch to parent session in-memory (no DB load).
                if let Some(chat) = self.message_list.as_mut() {
                    chat.switch_to_session(parent_id);
                    self.current_session_id = Some(parent_id);
                }
            }
            KeyCode::Down => {
                // Switch to the last (most recently delegated) child.
                let all = self.runtime.session_manager().store()
                    .list_sessions_unfiltered(1000, 0).unwrap_or_default();
                let children: Vec<_> = all.into_iter()
                    .filter(|s| s.parent_session_id == Some(parent_id))
                    .collect();
                if let Some(target) = children.last()
                    && let Some(chat) = self.message_list.as_mut() {
                        if chat.switch_to_session(target.session_id) {
                            self.current_session_id = Some(target.session_id);
                        } else {
                            self.switch_to_session(target.session_id);
                        }
                    }
            }
            KeyCode::Left | KeyCode::Right => {
                let step = if key.code == KeyCode::Left { -1isize } else { 1 };
                let all = self.runtime.session_manager().store()
                    .list_sessions_unfiltered(1000, 0).unwrap_or_default();
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
                if let Some(target) = children.get(next_index)
                    && let Some(chat) = self.message_list.as_mut() {
                        if chat.switch_to_session(target.session_id) {
                            self.current_session_id = Some(target.session_id);
                        } else {
                            self.switch_to_session(target.session_id);
                        }
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

        let position = ratatui::layout::Position::new(mouse.column, mouse.row);

        // Route mouse events to overlays first (top overlay has priority)
        if let Some(action) = self.overlays.handle_mouse_event(mouse, self.terminal_area) {
            self.process_action(action);
            return;
        }

        // Sidebar scroll (scroll events in the sidebar area)
        if let Some(sidebar_area) = self.sidebar_area
            && sidebar_area.contains(position) {
                let speed = self.runtime.config().ui.scroll_speed as usize;
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        self.sidebar.scroll_down(speed);
                    }
                    MouseEventKind::ScrollUp => {
                        self.sidebar.scroll_up(speed);
                    }
                    _ => {}
                }
                return;
            }

        // Determine the message content area bounds for selection clamping.
        let msg_bounds = self
            .message_list
            .as_ref()
            .and_then(|ml| ml.content_area);

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Check scrollbar click first.
                if let Some(ref mut chat) = self.message_list {
                    let sb_area = chat.scrollbar_area();
                    if sb_area.is_some_and(|a| a.contains(position)) {
                        chat.start_scrollbar_drag(position.y);
                        return;
                    }
                }

                // MessageList click-to-expand or subsession navigation (non-drag).
                // Run BEFORE mouse selection so interactive elements get priority.
                if let Some(ref mut chat) = self.message_list
                    && let Some(action) = chat.handle_mouse_click(mouse.column, mouse.row) {
                        self.process_action(action);
                        return;
                    }

                // Composer input area: set cursor and start selection.
                if let Some(ref mut composer) = self.composer {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        composer.handle_mouse_down(position, text_area);
                        self.mouse_selection.clear();
                        return;
                    }
                }

                // Start mouse selection if within message area (no interactive hit).
                if msg_bounds.is_some_and(|b| b.contains(position)) {
                    let scroll_offset = self
                        .message_list
                        .as_ref()
                        .map(|ml| ml.scroll_offset)
                        .unwrap_or(0);

                    // Refine bounds to the specific selectable region under the cursor
                    // (mirrors old TUI's selection_bounds_for_position).
                    let area = msg_bounds.unwrap();
                    let refined = self
                        .message_list
                        .as_ref()
                        .and_then(|ml| {
                            let hit = ml.selectable_region_rects().iter().find(|r| r.contains(position)).copied();
                            hit.map(|r| Rect { x: r.x, y: area.y, width: r.width, height: area.height })
                                .or(Some(area))
                        });

                    self.mouse_selection.press(position, refined, scroll_offset);
                }
            }
            MouseEventKind::Moved => {
                if let Some(ref mut chat) = self.message_list {
                    chat.set_hovered_card(mouse.column, mouse.row);
                }
                // Check queued prompt card hover.
                let pos = ratatui::layout::Position::new(mouse.column, mouse.row);
                self.hovered_queued_index = self.queued_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(pos))
                    .map(|(i, _)| *i);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Check scrollbar drag first.
                if let Some(ref mut chat) = self.message_list
                    && (chat.scrollbar_area().is_some_and(|a| a.contains(position))
                        || chat.is_scrollbar_dragging())
                    {
                        chat.continue_scrollbar_drag(position.y);
                        return;
                    }
                // Composer input area drag (extends selection).
                if let Some(ref mut composer) = self.composer
                    && composer.is_input_dragging()
                {
                    composer.handle_mouse_drag(position, composer.last_text_area);
                    return;
                }
                // Always update drag position (old TUI unconditional behaviour).
                self.mouse_selection.drag(position);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(ref mut chat) = self.message_list {
                    chat.end_scrollbar_drag();
                }

                // Composer input area: finalize selection and queue clipboard copy.
                if let Some(ref mut composer) = self.composer
                    && let Some(selected) = composer.handle_mouse_up(position) {
                        self.pending_input_copy = Some(selected);
                    }

                // Composer image badge click: open ImageViewer.
                if !self.mouse_selection.is_dragging()
                    && let Some(ref mut composer) = self.composer {
                        let text_area = composer.last_text_area;
                        if text_area.contains(position) {
                            let scroll = composer.input_scroll_offset as u16;
                            let local_y = position.y.saturating_sub(text_area.y);
                            let local_x = position.x.saturating_sub(text_area.x);
                            let target_line = scroll.saturating_add(local_y);
                            let raw_pos = composer.raw_text_position_at_visual(
                                text_area.width,
                                target_line,
                                local_x,
                            );
                            if let Some(span) = composer.span_at(raw_pos)
                                && let Some(data) = &span.image_data
                            {
                                let action = Action::Overlay(OverlayAction::Open(
                                    OverlayKind::ImageViewer {
                                        data: data.clone(),
                                        filename: span
                                            .image_filename
                                            .clone()
                                            .unwrap_or_default(),
                                    },
                                ));
                                self.mouse_selection
                                    .release(position, 0);
                                self.process_action(action);
                                return;
                            }
                        }
                    }

                let scroll_offset = self
                    .message_list
                    .as_ref()
                    .map(|ml| ml.scroll_offset)
                    .unwrap_or(0);
                self.mouse_selection.release(position, scroll_offset);
                // Clipboard copy is handled in draw() where we have access to the frame buffer.
            }
            MouseEventKind::ScrollDown => {
                // Check composer input area first (mirrors old TUI behaviour).
                if let Some(ref mut composer) = self.composer {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        composer.handle_mouse_scroll_down(text_area.width, text_area.height);
                        return;
                    }
                }
                self.mouse_selection.clear();
                let speed = self.runtime.config().ui.scroll_speed as isize;
                if self.message_list.is_some() {
                    self.process_action(Action::Chat(ChatAction::ScrollDelta(speed)));
                }
            }
            MouseEventKind::ScrollUp => {
                // Check composer input area first (mirrors old TUI behaviour).
                if let Some(ref mut composer) = self.composer {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        composer.handle_mouse_scroll_up();
                        return;
                    }
                }
                self.mouse_selection.clear();
                let speed = self.runtime.config().ui.scroll_speed as isize;
                if self.message_list.is_some() {
                    self.process_action(Action::Chat(ChatAction::ScrollDelta(-speed)));
                }
            }
            _ => {}
        }
    }

    /// Per-frame auto-scroll while dragging a mouse selection near the
    /// top/bottom edge of the message content area.
    pub fn update_mouse_selection_auto_scroll(&mut self) {
        if !self.mouse_selection.is_dragging() {
            return;
        }
        let Some(area) = self
            .message_list
            .as_ref()
            .and_then(|ml| ml.content_area)
        else {
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

        let speed = self.runtime.config().ui.scroll_speed as usize;
        if pointer.y <= top_threshold {
            let chat = self.message_list.as_mut().unwrap();
            let new_scroll = chat.scroll_offset.saturating_sub(speed);
            chat.scroll_offset = new_scroll.min(chat.max_scroll());
            chat.follow_tail = false;
            chat.dirty = true;
        } else if pointer.y >= bottom_threshold {
            let chat = self.message_list.as_mut().unwrap();
            let new_scroll = chat.scroll_offset.saturating_add(speed);
            chat.scroll_offset = new_scroll.min(chat.max_scroll());
            chat.follow_tail = chat.scroll_offset >= chat.max_scroll();
            chat.dirty = true;
        }
    }

    /// Per-frame auto-scroll while dragging a mouse selection in the
    /// composer input area near the top/bottom edge.
    pub fn update_input_area_auto_scroll(&mut self) {
        let Some(ref mut composer) = self.composer else { return };
        let text_area = composer.last_text_area;
        if text_area.width == 0 || text_area.height == 0 { return; }
        let Some(pointer) = self.mouse_selection.pointer() else { return; };
        composer.update_drag_auto_scroll(pointer, text_area);
    }

    pub fn handle_resize(&mut self, _w: u16, _h: u16) {
        // Full layout rebuild on resize (width change invalidates all line counts).
        if let Some(ref mut chat) = self.message_list {
            chat.invalidate_layout();
        }
        self.sidebar_area = None;
        self.mouse_selection.clear();
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
                    let is_model_panel = kind == OverlayKind::ModelPanel;
                    let is_settings_panel = kind == OverlayKind::SettingsPanel;
                    self.close_overlay(kind, &mut queue);
                    if is_model_panel
                        && let Some(ref mut composer) = self.composer {
                            let model = self.runtime.active_model();
                            composer.set_model_supports_images(model.supports_images);
                        }
                    if is_settings_panel {
                        self.subagent_enabled = self.runtime.config().subagent.enabled;
                    }
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
                            if let Some(session_id) = self.current_session_id
                                && self
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
                    // Ignore if already on this session
                    if self.current_session_id == Some(session_id) {
                        return;
                    }

                    // Switch to the selected session
                    self.current_session_id = Some(session_id);
                    self.scroll_target = None;
                    self.screen = AppScreen::Chat;

                    // Load session record and messages for chat display
                    let messages = self
                        .runtime
                        .session_manager()
                        .load_messages(session_id)
                        .unwrap_or_default();

                    // Refresh the Runtime's in-memory message buffer so the
                    // next submit_prompt picks up the latest data from the store.
                    let rt = self.runtime.clone();
                    let sid = session_id;
                    tokio::spawn(async move {
                        rt.reload_message_buffer(sid).await;
                    });

                    let chat_context = {
                        let config = self.runtime.config();
                        let active_model = config.resolve_active_model(&self.runtime.auth()).ok();
                        let model_display = active_model
                            .as_ref()
                            .map(|m| m.display_name.clone())
                            .unwrap_or_default();
                        let provider_display = active_model
                            .as_ref()
                            .map(|m| m.provider_display_name.clone())
                            .unwrap_or_default();

                        let mut ctx = crate::chat_context::ChatContext::new(
                            session_id,
                            String::new(),
                            messages,
                            None,
                            model_display,
                            provider_display,
                        );
                        if let Ok(Some(record)) = self.runtime.session_manager().load_session(session_id)
                        {
                            ctx.title = record.title;
                            ctx.parent_session_id = record.parent_session_id;
                        }

                        ctx
                    };

                    let session_title = chat_context.title.clone();

                    // Create or update MessageList
                    self.message_list.get_or_insert_with(MessageList::new)
                        .set_chat_context(chat_context);

                    log::info!("Switching to session: {} ({})", session_title, session_id);

                    // Close the session panel overlay (mirrors old Enter → select + close).
                    queue.push(Action::Overlay(OverlayAction::Close(
                        OverlayKind::SessionPanel,
                    )));
                }
                Action::Session(SessionAction::Reload) => {
                    // Broadcast to overlays so SessionPanel reloads its list.
                    let ctx = UpdateContext {
                        runtime: &mut self.runtime,
                    };
                    queue.extend(self.overlays.update_all(&action, &ctx));
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
                        None,
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
                        if let Some(ref tool_call_id) = new_message.tool_call_id
                            && let Ok(old_id) = uuid::Uuid::parse_str(tool_call_id)
                                && let Some(&new_tool_call_id) = id_mapping.get(&old_id) {
                                    new_message.tool_call_id =
                                        Some(new_tool_call_id.to_string());
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
                Action::Session(SessionAction::Redo) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };
                    self.set_notice("Redo in progress...");
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rt.redo(session_id).await {
                            log::error!("Redo failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Compact) => {
                    // Guard: no session.
                    if self.current_session_id.is_none() {
                        return;
                    }
                    // If a request is in progress, queue the compact.
                    if self.has_active_request() {
                        self.pending_compact = true;
                        self.set_notice("Compaction queued");
                        return;
                    }
                    self.execute_compact();
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
                        .update_session(session_id, Some(final_title), None)
                    {
                        Ok(_) => {
                            self.set_notice("Session title updated");
                            log::info!("Renamed session {} to {}", session_id, final_title);
                        }
                        Err(e) => log::error!("Failed to rename session: {e}"),
                    }
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
                Action::Session(SessionAction::Create) => {
                    self.current_session_id = None;

                    let config = self.runtime.config();
                    let auth = self.runtime.auth();
                    let active_model = config.resolve_active_model(&auth).ok();
                    let chat_context = crate::chat_context::ChatContext::new(
                        uuid::Uuid::nil(),
                        String::new(),
                        Vec::new(),
                        None,
                        active_model.as_ref().map(|m| m.display_name.clone()).unwrap_or_default(),
                        active_model.as_ref().map(|m| m.provider_display_name.clone()).unwrap_or_default(),
                    );
                    self.message_list
                        .get_or_insert_with(MessageList::new)
                        .set_chat_context(chat_context);

                    self.screen = AppScreen::Welcome;

                    if let Some(ref mut composer) = self.composer {
                        composer.clear();
                    }

                    self.pending_tools.clear();
                    self.tool_index = 0;
                    self.approved_tools.clear();
                    self.pending_response_tx = None;
                    self.abort_confirmation_deadline = None;
                    self.context_usage = None;
                    self.pending_prompt_queue.clear();
                    self.pending_compact = false;
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
                                && let Some(spec) =
                                    crate::components::composer::command_palette::CommandRegistry::new()
                                        .command(&name)
                                {
                                    let actions =
                                        crate::components::composer::command_palette::execute_command(
                                            spec.action,
                                            &args,
                                        );
                                    for action in actions {
                                        self.process_action(action);
                                    }
                                    return;
                                }
                                // Unknown command — fall through to submit as prompt.

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

                            // If there's already an active request, queue the prompt.
                            if self.has_active_request() {
                                self.pending_prompt_queue.push(QueuedPrompt {
                                    prompt: text.clone(),
                                    attachments: final_attachments.clone(),
                                });
                                let queued_count = self.pending_prompt_queue.len();
                                self.set_notice(format!(
                                    "Prompt queued ({} pending)",
                                    queued_count
                                ));
                                return;
                            }

                            // If no active session, create one and enter Chat mode.
                            let session_id = self.current_session_id;
                            let sid = match session_id {
                                Some(id) => id,
                                None => {
                                    match self.runtime.create_default_session("Untitled session") {
                                        Ok(id) => {
                                            self.current_session_id = Some(id);

                                            // Initialize MessageList for the new session.
                                            let config = self.runtime.config();
                                            let auth = self.runtime.auth();
                                            let active_model = config.resolve_active_model(&auth)
                                                .ok();
                                            let model_display = active_model.as_ref()
                                                .map(|m| m.display_name.clone()).unwrap_or_default();
                                            let provider_display = active_model.as_ref()
                                                .map(|m| m.provider_display_name.clone())
                                                .unwrap_or_default();
                                            let chat_context = crate::chat_context::ChatContext::new(
                                                id,
                                                String::new(),
                                                Vec::new(),
                                                None,
                                                model_display,
                                                provider_display,
                                            );
                                            self.message_list
                                                .get_or_insert_with(MessageList::new)
                                                .set_chat_context(chat_context);
                                            self.screen = AppScreen::Chat;

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
                            let mode = self.mode;
                            let thinking_level = self.thinking_level.clone();
                            let rt = self.runtime.clone();
                            let text_for_title = text.clone();
                            self.set_notice("Sending...");
                            if let Some(ref mut chat) = self.message_list {
                                chat.follow_tail = true;
                            }
                            tokio::spawn(async move {
                                if let Err(e) = rt
                                    .submit_prompt_with_attachments(sid, mode, text, final_attachments, Some(thinking_level))
                                    .await
                                {
                                    log::error!("submit_prompt failed: {e}");
                                }
                            });

                            // Update session title from prompt (matching old behaviour).
                            if let Some(ref mut chat) = self.message_list
                                && let Some(ref mut ctx) = chat.active_chat_context_mut()
                                    && (ctx.title.is_empty() || ctx.title == "Untitled session") {
                                        let title = title_from_prompt(&text_for_title);
                                        ctx.title = title.clone();
                                        if let Err(e) = self.runtime
                                            .session_manager()
                                            .update_session(sid, Some(&title), None)
                                        {
                                            log::error!("Failed to update session title: {e}");
                                        }
                                    }
                        }
                        ChatAction::SetInput(text) => {
                            if let Some(ref mut composer) = self.composer {
                                composer.set_text(text.clone());
                            }
                        }
                        _ => {
                            // Forward other chat actions (scroll, stream, etc.) to MessageList.
                            if let Some(ref mut chat) = self.message_list {
                                let ctx = UpdateContext {
                                    runtime: &mut self.runtime,
                                };
                                queue.extend(chat.update(&Action::Chat(action), &ctx));
                            }
                        }
                    }
                }
                Action::Notice(msg) => {
                    self.set_notice(msg);
                }
                Action::Noop => {}
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
                        if remember
                            && let Some(session_id) = self.current_session_id
                                && let Err(e) = self
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
            }
        }
    }

    fn open_overlay(&mut self, kind: OverlayKind) {
        let kind_for_update = kind.clone();
        let component: Option<Box<dyn Component>> = match kind {
            OverlayKind::ThemePanel => {
                let current = ThemeName::parse(self.current_palette.name.as_str())
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
                        content: catalog.render_skill(&s.name).unwrap_or_default(),
                        is_bundled: s.directory.starts_with("__builtin__"),
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
                    .and_then(|ml| ml.active_chat_context())
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
                let workspace_root = self
                    .runtime
                    .workspace_root()
                    .display()
                    .to_string();
                let sessions = store
                    .list_sessions_for_workspace(&workspace_root, 1000, 0)
                    .unwrap_or_default();
                let current_session_id = self
                    .current_session_id
                    .or_else(|| {
                        self.message_list
                            .as_ref()
                            .and_then(|ml| ml.active_chat_context())
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
            OverlayKind::ImageViewer { data, filename } => {
                ImageViewer::from_raw(data, filename, self.image_picker.clone())
                    .map(|v| Box::new(v) as Box<dyn Component>)
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
            | OverlayKind::SensitiveFileDialog => None,
        };
        if let Some(mut component) = component {
            let config = self.runtime.config();
            let auth = self.runtime.auth();
            let init_ctx = InitContext {
                config: &config,
                auth: &auth,
            };
            let _ = component.init(&init_ctx);
            self.overlays.push(component);

            // Trigger initial lazy-load for the new overlay (e.g. populate preview cache)
            if let Some(top) = self.overlays.last_mut() {
                let ctx = UpdateContext {
                    runtime: &mut self.runtime,
                };
                let _ = top.update(&Action::Overlay(OverlayAction::Open(kind_for_update)), &ctx);
            }
        }
    }

    fn close_overlay(&mut self, kind: OverlayKind, queue: &mut Vec<Action>) {
        if let Some(mut overlay) = self.overlays.pop() {
            let ctx = UpdateContext {
                runtime: &mut self.runtime,
            };
            queue.extend(
                overlay.update(
                    &Action::Overlay(OverlayAction::Close(kind)),
                    &ctx,
                ),
            );
        }
    }

    // ── Drawing ──

    fn footer_status_text(&self) -> String {
        let queued_count = self.pending_prompt_queue.len();

        // 1. Esc again to stop (abort confirmation)
        if self.has_active_request()
            && self.abort_confirmation_deadline
                .is_some_and(|deadline| deadline > Instant::now())
        {
            return "Esc again to stop".to_string();
        }

        // 2. Token usage helper
        let token_status = self.context_usage.as_ref().map(|usage| {
            let max_context = self.runtime.active_model().context_window;
            let total = usage.input_tokens as u64 + usage.output_tokens as u64;
            let pct = if max_context > 0 {
                (total as f64 / max_context as f64 * 100.0).min(100.0)
            } else {
                0.0
            };
            let used_k = usage.input_tokens / 1000;
            let max_k = (max_context as u32) / 1000;
            format!("{pct:.1}% ({used_k}K/{max_k}K)")
        });

        // 3. Active request — show spinner + status
        if self.has_active_request() {
            let spinner = self.loading_spinner();

            // Check for subsession
            let parent_session_id = self.message_list.as_ref()
                .and_then(|ml| ml.active_chat_context())
                .and_then(|ctx| ctx.parent_session_id);

            let status = if parent_session_id.is_some() {
                // Check if this subsession's own subagent is still running.
                let session_id = self.message_list.as_ref()
                    .and_then(|ml| ml.active_chat_context())
                    .map(|ctx| ctx.session_id);
                let subagent_running = session_id.is_some_and(|sid| {
                    self.message_list.as_ref()
                        .is_some_and(|ml| ml.is_subagent_running(sid))
                });
                if subagent_running {
                    format!("{spinner} Thinking...")
                } else {
                    "Subsession active · Up: parent  Left/Right: switch subagent".to_string()
                }
            } else if let Some(ref ml) = self.message_list {
                let sub_count = ml.running_subagents_count();
                if sub_count > 0 {
                    let label = if sub_count == 1 { "subagent" } else { "subagents" };
                    format!("{spinner} Waiting for {sub_count} {label}")
                } else if ml.is_streaming() {
                    match self.pending_mode.as_ref() {
                        Some(pending) => {
                            format!("{spinner} {} → {} (on completion)", self.mode.title(), pending.title())
                        }
                        None => format!("{spinner} {}", self.mode.title()),
                    }
                } else if !self.pending_tools.is_empty() {
                    format!("{spinner} Running tools")
                } else {
                    match self.pending_mode.as_ref() {
                        Some(pending) => {
                            format!("{spinner} {} → {} (on completion)", self.mode.title(), pending.title())
                        }
                        None => format!("{spinner} {}", self.mode.title()),
                    }
                }
            } else {
                match self.pending_mode.as_ref() {
                    Some(pending) => {
                        format!("{spinner} {} → {} (on completion)", self.mode.title(), pending.title())
                    }
                    None => format!("{spinner} {}", self.mode.title()),
                }
            };

            let extra = match (queued_count, self.pending_compact) {
                (0, false) => String::new(),
                (1, false) => " · queued 1".to_string(),
                (q, false) => format!(" · queued {q}"),
                (0, true) => " · compact pending".to_string(),
                (q, true) => format!(" · queued {q} · compact pending"),
            };
            let status = format!("{status}{extra}");

            if let Some(ref t) = token_status {
                return format!("{status} · {t}");
            }
            return status;
        }

        // 3b. Compacting in progress — show spinner + status
        if self.is_compacting {
            let spinner = self.loading_spinner();
            let status = format!("{spinner} Compacting...");
            if let Some(ref t) = token_status {
                return format!("{status} · {t}");
            }
            return status;
        }

        // 4. Queued messages or compact pending (not streaming)
        let has_pending = queued_count > 0 || self.pending_compact;
        if has_pending {
            let compact_part = if self.pending_compact { " · compact pending" } else { "" };
            let status = if queued_count == 1 {
                format!("1 queued message{compact_part}")
            } else if queued_count > 1 {
                format!("{queued_count} queued messages{compact_part}")
            } else {
                "compact pending".to_string()
            };
            if let Some(ref t) = token_status {
                return format!("{status} · {t}");
            }
            return status;
        }

        // 5. Token usage only
        if let Some(t) = token_status {
            return t;
        }

        // 6. Last notice
        if let Some((msg, _)) = &self.last_notice
            && !msg.is_empty() {
                return msg.clone();
            }

        // 7. Subsession navigation hint
        let is_subsession = self.message_list.as_ref()
            .and_then(|ml| ml.active_chat_context())
            .and_then(|ctx| ctx.parent_session_id)
            .is_some();
        if is_subsession {
            return "Subsession active · Up: parent  Left/Right: switch subagent".to_string();
        }

        // 8. Ready
        "Ready".to_string()
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let palette = self.current_palette;
        let area = frame.area();
        self.terminal_area = area;

        // Background
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        if self.screen == AppScreen::Welcome {
            self.draw_welcome(frame);
            // Draw overlays on top of welcome content.
            let draw_ctx = DrawContext {
                palette,
                focused: true,
                mode: self.mode,
                pending_mode: self.pending_mode,
                model_display: None,
                provider_display: None,
                thinking_level: None,
                subagent_disabled: !self.subagent_enabled,
                workspace_root: self.runtime.workspace_root(),
            };
            self.overlays.draw(frame, area, &draw_ctx);
            return;
        }

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

        // Determine if in a subsession.
        let is_subsession = self.message_list.as_ref()
            .and_then(|ml| ml.active_chat_context())
            .and_then(|ctx| ctx.parent_session_id)
            .is_some();
        const SUBSESSION_NAV_HEIGHT: u16 = 3;

        // Calculate bottom-bar height: subsession nav or composer.
        let bottom_height = if is_subsession {
            SUBSESSION_NAV_HEIGHT
        } else {
            self.composer.as_ref().map(|c| {
                let width = main_area.width.saturating_sub(4);
                c.preferred_height(width, 6).saturating_add(2).min(main_area.height.saturating_sub(2))
            }).unwrap_or(0)
        };

        // Calculate queued prompts area height (frozen area above input box).
        let queued_height = if !is_subsession {
            let count = self.pending_prompt_queue.len();
            if count > 0 {
                let visible = count.min(MAX_VISIBLE_QUEUED_PROMPTS);
                let text_width = main_area.width.saturating_sub(5).max(1) as usize;
                let mut inner: usize = 0;
                for (i, q) in self.pending_prompt_queue.iter().take(visible).enumerate() {
                    let wrapped = wrap_text_lines(&q.prompt, text_width, MAX_QUEUED_PROMPT_LINES);
                    inner += wrapped.len();
                    // Separator between items (not after last)
                    if i + 1 < visible {
                        inner += 1;
                    }
                }
                // +1 for "+N more" overflow, +2 for block top/bottom borders
                let overflow = if count > MAX_VISIBLE_QUEUED_PROMPTS { 1 } else { 0 };
                (inner + overflow + 2)
                    .min(main_area.height.saturating_sub(6) as usize / 2)
                    .min(15)
            } else {
                0
            }
        } else {
            0
        };

        // Split: message area + queued area + bottom bar + notice line.
        let notice_height: u16 = 1;
        let (content_area, queued_area, bottom_area, notice_line) = if bottom_height > 0 {
            let split = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(queued_height as u16),
                Constraint::Length(bottom_height),
                Constraint::Length(notice_height),
            ])
            .split(main_area);
            (split[0], split[1], split[2], split[3])
        } else {
            let split = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(notice_height),
            ])
            .split(main_area);
            (split[0], Rect::default(), Rect::default(), split[1])
        };

        // Chat message area (when session is active)
        if let Some(ref mut chat) = self.message_list {
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                mode: self.mode,
                pending_mode: self.pending_mode,
                model_display: None,
                provider_display: None,
                thinking_level: None,
                subagent_disabled: !self.subagent_enabled,
                workspace_root: self.runtime.workspace_root(),
            };
            chat.draw(frame, content_area, &draw_ctx);
        }

        // Render queued prompts above the composer
        self.queued_card_bounds.clear();
        if queued_height > 0 {
            self.render_queued_prompts(frame, queued_area);
        }

        // ── Bottom bar ───────────────────────────────────────────────
        // Subsession: navigation hints.  Normal session: composer.
        if is_subsession {
            // Match the background with the message panel area.
            let bg_rect = Rect {
                x: main_area.x + 2,
                y: bottom_area.y,
                width: bottom_area.width.saturating_sub(2),
                height: bottom_area.height,
            };
            frame.render_widget(
                Block::default().style(Style::default().bg(palette.panel)),
                bg_rect,
            );
            let hint = Line::from(vec![
                Span::styled("Up", Style::default().fg(palette.accent_soft)),
                Span::styled(": return to parent  ", Style::default().fg(palette.muted)),
                Span::styled("Left", Style::default().fg(palette.accent_soft)),
                Span::styled("/", Style::default().fg(palette.muted)),
                Span::styled("Right", Style::default().fg(palette.accent_soft)),
                Span::styled(": switch subagent", Style::default().fg(palette.muted)),
            ]);
            let y_offset = bg_rect.height.saturating_sub(1) / 2;
            let content_rect = Rect {
                x: bg_rect.x,
                y: bg_rect.y + y_offset,
                width: bg_rect.width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(hint).alignment(Alignment::Center).style(Style::default().fg(palette.text)),
                content_rect,
            );
        } else if let Some(ref mut composer) = self.composer {
            if composer.has_popup() {
                composer.sync_autocomplete();
            }
            let active_model = self.runtime.active_model();
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                mode: self.mode,
                pending_mode: self.pending_mode,
                model_display: Some(&active_model.display_name),
                provider_display: Some(&active_model.provider_display_name),
                thinking_level: Some(&active_model.thinking_level),
                subagent_disabled: !self.subagent_enabled,
                workspace_root: self.runtime.workspace_root(),
            };
            composer.draw(frame, bottom_area, &draw_ctx);
        }

        // Build DrawContext for overlays
        let draw_ctx = DrawContext {
            palette,
            focused: true,
            mode: self.mode,
            pending_mode: self.pending_mode,
            model_display: None,
            provider_display: None,
            thinking_level: None,
            subagent_disabled: !self.subagent_enabled,
            workspace_root: self.runtime.workspace_root(),
        };

        // ── Sidebar ───────────────────────────────────────────────────
        if let Some(sidebar_area) = sidebar_area {
            self.sidebar_area = Some(sidebar_area);
            let chat_ctx = self
                .message_list
                .as_ref()
                .and_then(|ml| ml.active_chat_context());
            self.sidebar.draw(
                frame,
                sidebar_area,
                palette,
                self.runtime.workspace_root(),
                chat_ctx,
                self.context_usage.as_ref(),
                &self.todos,
            );
        }

        // Draw overlays (on top of everything, including sidebar)
        self.overlays.draw(frame, area, &draw_ctx);

        // ── Footer status line (right-aligned, matching v0.6.x) ──
        let status_text = self.footer_status_text();
        let status_width = status_text.width().min(notice_line.width.saturating_sub(2) as usize) as u16;
        let status_x = notice_line.x + notice_line.width.saturating_sub(2).saturating_sub(status_width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &status_text,
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.background)),
            Rect::new(status_x, notice_line.y, status_width, 1),
        );

        // ── Toast notification ──
        // Small popup at the top-right of the message area, auto-expires.
        // Mirrors old TUI's render_toast: positioned relative to message content area.
        if let Some((msg, expires_at)) = &self.toast.clone() {
            if Instant::now() < *expires_at {
                let chat_area = self
                    .message_list
                    .as_ref()
                    .and_then(|ml| ml.content_area);
                if let Some(chat_area) = chat_area {
                    let toast_width = (msg.len() as u16).min(32).saturating_add(2);
                    let toast_rect = Rect::new(
                        chat_area.right().saturating_sub(toast_width + 1),
                        chat_area.y + 1,
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
                }
            } else {
                self.toast = None;
            }
        }

        // ── Mouse selection overlay ──
        // Apply after all widgets have been drawn, so the selection style
        // paints on top of the rendered content.
        let scroll_offset = self
            .message_list
            .as_ref()
            .map(|ml| ml.scroll_offset)
            .unwrap_or(0);
        let selectable_rects = self
            .message_list
            .as_ref()
            .map(|ml| ml.selectable_region_rects())
            .unwrap_or_default();
        let sel_style = Style::default()
            .bg(palette.selection_bg)
            .fg(palette.selection_fg);

        if self.mouse_selection.has_selection(scroll_offset) {
            self.mouse_selection.apply_overlay(
                frame.buffer_mut(),
                scroll_offset,
                &selectable_rects,
                sel_style,
            );
        }

        // Handle pending clipboard copy (set by mouse up in handle_mouse_event).
        if self.mouse_selection.take_pending_copy(scroll_offset).is_some()
            && let Some(text) = self.mouse_selection.selected_text(
                frame.buffer_mut(),
                scroll_offset,
                &selectable_rects,
            )
                && !text.is_empty() {
                    match copy_to_clipboard(&text) {
                        Ok(()) => {
                            self.mouse_selection.clear();
                            self.set_toast("Selection copied to clipboard", std::time::Duration::from_secs(3));
                        }
                        Err(e) => {
                            self.mouse_selection.clear();
                            self.set_toast(format!("Copy failed: {e}"), std::time::Duration::from_secs(5));
                        }
                    }
                }

        // Handle pending clipboard copy from composer input area.
        if let Some(text) = self.pending_input_copy.take()
            && !text.is_empty() {
                match copy_to_clipboard(&text) {
                    Ok(()) => {
                        self.set_toast("Selection copied to clipboard", std::time::Duration::from_secs(3));
                    }
                    Err(e) => {
                        self.set_toast(format!("Copy failed: {e}"), std::time::Duration::from_secs(5));
                    }
                }
            }
    }

    /// Render the welcome screen with logo, subtitle, and composer.
    fn draw_welcome(&mut self, frame: &mut Frame) {
        let palette = self.current_palette;
        let area = frame.area();

        // Centered card — exact match to old TUI's render_welcome
        let card_width = self
            .runtime
            .config()
            .ui
            .welcome_width
            .min(area.width.saturating_sub(4).max(32));
        let card_height = 20u16.min(area.height.saturating_sub(2).max(10));
        let card = Rect::new(
            (area.width - card_width) / 2,
            (area.height - card_height) / 2,
            card_width,
            card_height,
        );

        let card_inner_width = card.width.saturating_sub(7);

        let inner = card.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let composer_height = self
            .composer
            .as_ref()
            .map(|c| {
                c.preferred_height(
                    card_inner_width,
                    self.runtime.config().ui.max_input_lines,
                )
                .saturating_add(2)
            })
            .unwrap_or(5);

        let sections = Layout::vertical([
            Constraint::Length(8),
            Constraint::Length(1),
            Constraint::Length(composer_height),
        ])
        .split(inner);

        // ASCII art logo
        let ascii_art = Paragraph::new(
            r#"░▒▓████████▓▒░▒▓█▓▒░▒▓███████▓▒░░▒▓████████▓▒░▒▓█▓▒░░▒▓█▓▒░ 
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░      ░▒▓█▓▒░░▒▓█▓▒░ 
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░       ░▒▓█▓▒▒▓█▓▒░  
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓██████▓▒░  ░▒▓█▓▒▒▓█▓▒░  
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░        ░▒▓█▓▓█▓▒░   
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░        ░▒▓█▓▓█▓▒░   
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓███████▓▒░░▒▓████████▓▒░  ░▒▓██▓▒░    "#,
        )
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(ascii_art, sections[0]);

        // Subtitle
        let subtitle = Paragraph::new("Terminal AI assistant for focused coding work")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.muted));
        frame.render_widget(subtitle, sections[1]);

        // Composer input block — pass section area directly, exactly as old TUI
        if let Some(ref mut composer) = self.composer {
            if composer.has_popup() {
                composer.sync_autocomplete();
            }
            let active_model = self.runtime.active_model();
            let draw_ctx = DrawContext {
                palette,
                focused: true,
                mode: self.mode,
                pending_mode: self.pending_mode,
                model_display: Some(&active_model.display_name),
                provider_display: Some(&active_model.provider_display_name),
                thinking_level: Some(&active_model.thinking_level),
                subagent_disabled: !self.subagent_enabled,
                workspace_root: self.runtime.workspace_root(),
            };
            composer.draw(frame, sections[2], &draw_ctx);
        }

        // Workspace path on the very last row
        let workspace_path = self.runtime.workspace_root().display().to_string();
        let display_path = workspace_path.replace(
            &dirs::home_dir().unwrap_or_default().display().to_string(),
            "~",
        );
        let workspace_area = Rect::new(
            area.x + 1,
            area.bottom() - 1,
            area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &display_path,
                Style::default().fg(palette.muted),
            ))),
            workspace_area,
        );

        // Notice, if any, on the row directly above workspace path
        if let Some((message, _)) = &self.last_notice
            && !message.is_empty() {
                let notice_y = area.bottom().saturating_sub(2);
                if notice_y < workspace_area.y {
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            message,
                            Style::default().fg(palette.muted),
                        ))),
                        Rect::new(area.x + 1, notice_y, area.width.saturating_sub(2), 1),
                    );
                }
            }
    }

    /// Render a frozen area above the composer showing queued (pending) prompts.
    /// Each queued message is word-wrapped into up to [`MAX_QUEUED_PROMPT_LINES`] lines.
    /// Cards are separated by a thin rule. Each card is independently hover-highlighted.
    fn render_queued_prompts(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = &self.current_palette;
        let count = self.pending_prompt_queue.len();
        let visible = count.min(MAX_VISIBLE_QUEUED_PROMPTS);

        // Build title: " QUEUE " badge with background color + count
        let title = Line::from(vec![
            Span::styled(
                " QUEUE ",
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {} ", count), Style::default().fg(palette.muted)),
        ]);

        // Align with composer: left_inset=2 (bg) + inner_margin=2 (text)
        let left_inset: u16 = 2;
        let block_area = Rect {
            x: area.x + left_inset,
            y: area.y,
            width: area.width.saturating_sub(left_inset),
            height: area.height,
        };

        let block = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(title)
            .title_alignment(Alignment::Left);

        // Inner content matches composer's text area (x+4, width-5).
        // Offset y by 1 to leave room for the block's title on the first row.
        let inner = Rect {
            x: block_area.x + left_inset,
            y: block_area.y + 1,
            width: block_area.width.saturating_sub(left_inset + 1),
            height: block_area.height.saturating_sub(1),
        };
        let inner_height = inner.height as usize;
        let width = inner.width.max(1) as usize;

        let mut y_offset: u16 = 0;

        for (i, queued) in self.pending_prompt_queue.iter().take(visible).enumerate() {
            if y_offset as usize >= inner_height {
                break;
            }

            // Word-wrap the prompt into up to MAX_QUEUED_PROMPT_LINES lines
            let wrapped_lines = wrap_text_lines(&queued.prompt, width, MAX_QUEUED_PROMPT_LINES);
            let row_text_height = wrapped_lines.len();
            let has_separator = i + 1 < visible;
            let row_height = row_text_height + if has_separator { 1 } else { 0 };

            // Clamp to available space
            let available = inner_height.saturating_sub(y_offset as usize);
            if available == 0 {
                break;
            }
            let render_height = row_height.min(available);

            // Record bounds for hover hit-testing
            let row_rect = Rect::new(
                inner.x,
                inner.y + y_offset,
                inner.width,
                render_height as u16,
            );
            self.queued_card_bounds.push((i, row_rect));

            // Apply hover highlight
            let is_hovered = self.hovered_queued_index == Some(i);
            if is_hovered {
                let hover_bg = palette.hover_bg(palette.panel);
                frame.render_widget(
                    Block::default().style(Style::default().bg(hover_bg)),
                    row_rect,
                );
            }

            // Render each wrapped line of the prompt
            let text_style = if is_hovered {
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
                    .fg(palette.muted)
                    .add_modifier(Modifier::ITALIC)
            };

            for line_text in wrapped_lines.iter() {
                if y_offset as usize >= inner_height {
                    break;
                }
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(line_text.clone(), text_style)))
                        .wrap(ratatui::widgets::Wrap { trim: false }),
                    Rect::new(inner.x, inner.y + y_offset, inner.width, 1),
                );
                y_offset += 1;
            }

            // Separator line (not after last visible item)
            if has_separator && (y_offset as usize) < inner_height {
                let sep_width = width.saturating_sub(2);
                let sep = "─".repeat(sep_width);
                let sep_style = if is_hovered {
                    Style::default().fg(palette.text)
                } else {
                    Style::default().fg(palette.border)
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(sep, sep_style))),
                    Rect::new(
                        inner.x + 1,
                        inner.y + y_offset,
                        inner.width.saturating_sub(2),
                        1,
                    ),
                );
                y_offset += 1;
            }
        }

        // Overflow indicator
        if count > MAX_VISIBLE_QUEUED_PROMPTS && (y_offset as usize) < inner_height {
            let more_text = format!("+{} more...", count - MAX_VISIBLE_QUEUED_PROMPTS);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    more_text,
                    Style::default().fg(palette.muted),
                ))),
                Rect::new(inner.x, inner.y + y_offset, inner.width, 1),
            );
        }

        // Render block last so it draws borders on top
        frame.render_widget(block, block_area);
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

/// Extract a session title from a user prompt (first line, trimmed, max 48 chars).
fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or("Untitled session").trim();
    if first_line.is_empty() {
        return "Untitled session".to_string();
    }
    let mut title: String = first_line.chars().take(48).collect();
    if first_line.chars().count() > 48 {
        title.push_str("...");
    }
    title
}
