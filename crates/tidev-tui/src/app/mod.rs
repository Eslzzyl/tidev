//! New-architecture App root component.
//!
//! Owns the Runtime, manages the component tree via OverlayStack,
//! routes Actions, and dispatches async commands.

mod actions;
mod backend_events;
mod drawing;
mod events;
mod overlays;
mod tools;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::theme::ThemePalette;
use ratatui::layout::Rect;
use tidev_core::TuiResponse;
use tidev_core::{ApprovedTool, ToolCallWithViolations};
use tidev_types::message::{
    BackendEvent, COMPACTION_MESSAGE_LABEL, Message, MessageAttachment, MessageRole,
};
use tidev_types::prompts::SessionMode;
use tidev_types::reasoning::ThinkingLevelType;
use tidev_types::tools::TodoItem;
use uuid::Uuid;

use crate::component::Component;
use crate::components::overlay_stack::OverlayStack;

use crate::components::chat::MessageList;
use crate::components::composer::Composer;
use crate::components::desktop_notification::NotificationManager;
use crate::components::notification::NotificationState;
use crate::components::selection::MouseSelection;
use crate::components::sidebar::Sidebar;
use ratatui_image::picker::Picker;

/// Token usage statistics for the current/last request.
#[derive(Clone, Debug)]
pub(crate) struct ContextUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub tokens_per_second: Option<f32>,
}

/// Per-session pending tool approval state.
struct PendingApproval {
    response_tx: tokio::sync::mpsc::UnboundedSender<TuiResponse>,
    tools: Vec<ToolCallWithViolations>,
    tool_index: usize,
    approved_tools: Vec<ApprovedTool>,
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
    /// Per-session pending mode switch (applied on next Finished with no tool calls).
    pending_modes: HashMap<Uuid, SessionMode>,
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

    /// Last time an auto-scroll step was performed while dragging a
    /// selection, used to throttle the rate independent of frame rate.
    last_selection_auto_scroll: Option<Instant>,

    /// Cached sidebar area for mouse hit-testing.
    sidebar_area: Option<Rect>,

    /// Cached terminal area for overlay mouse hit-testing.
    terminal_area: Rect,

    /// Current session's todo items (loaded from store).
    todos: Vec<TodoItem>,
    /// Tracks instruction sources already shown as "Loaded instructions from" messages
    /// in the current session.  Pure in-memory dedup — never written to the DB.
    shown_instruction_sources: Vec<String>,
    /// Buffer for instruction sources discovered during tool execution.
    /// Flushed to chat_context (and persisted) on StreamEnd, after all tool
    /// results for this turn have been placed in the message list.
    pending_instruction_sources: HashMap<Uuid, Vec<String>>,

    // ── Tool approval pipeline (per-session) ──
    /// Per-session pending tool approval states.
    pending_approvals: HashMap<Uuid, PendingApproval>,
    /// Which session's approval dialog is currently active.
    active_approval_session: Option<Uuid>,

    // ── In-memory permission caches ──
    /// allowlist for workspace boundary (canonical path → allowed).
    /// Uses prefix matching so allowing a directory allows all files under it.
    boundary_permissions: HashMap<String, bool>,
    /// User-supplied reason for workspace boundary denials (path → reason).
    boundary_reasons: HashMap<String, String>,
    /// allowlist for sensitive file access (canonical path → allowed).
    sensitive_permissions: HashMap<String, bool>,
    /// User-supplied reason for sensitive file denials (path → reason).
    sensitive_reasons: HashMap<String, String>,

    /// Token usage statistics from the last request (for status bar display).
    context_usage: Option<ContextUsage>,
    /// Per-session cache of token usage, preserved across session switches.
    context_usage_cache: HashMap<Uuid, ContextUsage>,

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

    /// Per-session compaction queue flag (compact after current request finishes).
    pending_compacts: HashSet<Uuid>,

    /// Per-session compaction in progress.
    compacting_sessions: HashSet<Uuid>,

    /// Text saved per-session in the composer for restoring on session switch.
    composer_texts: HashMap<Uuid, String>,

    /// Session IDs with active agent loops (from Runtime, refreshed each frame).
    active_sessions: HashSet<Uuid>,

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
    session_id: Uuid,
    mode: SessionMode,
    thinking_level: ThinkingLevelType,
}

impl App {
    pub fn new(
        runtime: tidev_core::Runtime,
        request_rx: tokio::sync::mpsc::UnboundedReceiver<tidev_core::TuiRequest>,
        event_rx: tokio::sync::mpsc::UnboundedReceiver<BackendEvent>,
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
            pending_modes: HashMap::new(),
            thinking_level,
            subagent_enabled,
            last_notice: None,
            notifications: NotificationState::new(),
            desktop_notifications: NotificationManager::new(&notif_config),
            request_rx: Some(request_rx),
            event_rx: Some(event_rx),
            pending_approvals: HashMap::new(),
            active_approval_session: None,
            boundary_permissions: HashMap::new(),
            boundary_reasons: HashMap::new(),
            sensitive_permissions: HashMap::new(),
            sensitive_reasons: HashMap::new(),
            context_usage: None,
            context_usage_cache: HashMap::new(),
            toast: None,
            abort_confirmation_deadline: None,
            pending_prompt_queue: Vec::new(),
            queued_card_bounds: Vec::new(),
            hovered_queued_index: None,
            pending_compacts: HashSet::new(),
            compacting_sessions: HashSet::new(),
            composer_texts: HashMap::new(),
            active_sessions: HashSet::new(),
            screen: AppScreen::Welcome,
            spinner_start: Instant::now(),
            last_spinner_frame: 0,
            message_list: None,
            sidebar: Sidebar::new(),
            mouse_selection: MouseSelection::default(),
            last_selection_auto_scroll: None,
            sidebar_area: None,
            terminal_area: Rect::new(0, 0, 0, 0),
            todos: Vec::new(),
            shown_instruction_sources: Vec::new(),
            pending_instruction_sources: HashMap::new(),
            image_picker: {
                log::info!("[img] from_query_stdio START");
                let r = Picker::from_query_stdio();
                log::info!(
                    "[img] from_query_stdio END: {:?}",
                    r.as_ref()
                        .map(|p| (p.protocol_type(), p.font_size(), p.capabilities()))
                );
                r.ok()
            },
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

    /// Show "Loaded instructions from ..." messages for newly discovered
    /// instruction sources.  Deduplicates against sources already shown in
    /// this session (tracked by `shown_instruction_sources`).
    ///
    /// **Does not persist.** The caller is responsible for persisting via
    /// `Runtime::append_message` when cross-session replay is needed.
    fn show_instruction_sources(&mut self, sources: &[String]) {
        let new_sources: Vec<&String> = sources
            .iter()
            .filter(|s| !self.shown_instruction_sources.contains(s))
            .collect();
        if new_sources.is_empty() {
            return;
        }

        // Shorten canonical paths to workspace-relative for display.
        let ws_root = self.runtime.workspace_root();
        let to_rel = |s: &str| -> String {
            let path = std::path::Path::new(s);
            path.strip_prefix(ws_root)
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let display_paths: Vec<String> = new_sources.iter().map(|s| to_rel(s)).collect();

        // Build the display text matching the old v0.6.x format.
        let content = if display_paths.len() == 1 {
            format!("Loaded instructions from {}", display_paths[0])
        } else {
            format!(
                "Loaded {} instruction files: {}",
                display_paths.len(),
                display_paths.join(", "),
            )
        };

        // Push as a System message into the current session's chat context
        // for immediate display.
        if let Some(sid) = self.current_session_id
            && let Some(ref mut chat) = self.message_list
        {
            if let Some(ref mut ctx) = chat.active_chat_context_mut()
                && ctx.session_id == sid
            {
                let system_msg = Message::new(MessageRole::System, &content);
                ctx.push(system_msg);
            }
            // Force a layout rebuild so the new message is rendered on
            // the next frame, even if no other dirty-triggering event
            // follows.
            chat.invalidate_layout();
        }

        // Mark as shown in memory only — the backend owns `session_instruction_sources`.
        let owned: Vec<String> = new_sources.into_iter().cloned().collect();
        self.shown_instruction_sources.extend(owned);
    }

    /// Accessor for `spinner_start` so tui.rs can compute spinner frame.
    pub(crate) fn spinner_elapsed(&self) -> std::time::Duration {
        self.spinner_start.elapsed()
    }

    /// Whether a compaction is currently in progress for the current session.
    pub(crate) fn is_compacting(&self) -> bool {
        self.current_session_id
            .is_some_and(|sid| self.compacting_sessions.contains(&sid))
    }

    /// Update the set of sessions with active agent loops.
    pub(crate) fn set_active_sessions(&mut self, sessions: Vec<Uuid>) {
        self.active_sessions = sessions.into_iter().collect();
    }

    /// Whether the app currently has an active request (streaming or pending tool approval)
    /// for the current session.
    pub(crate) fn has_active_request(&self) -> bool {
        // Check if the *current session* has an active agent loop.
        if let Some(sid) = self.current_session_id {
            if self.runtime.is_session_busy(sid) {
                return true;
            }
            // Local UI supplement — pending tool approval dialogs for this session.
            if self.pending_approvals.contains_key(&sid) {
                return true;
            }
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

    /// Abort the current request: cancel the current session's agent loop,
    /// drop pending approvals, and clear all pending state.
    fn abort_current_request(&mut self) {
        let session_id = self.current_session_id;

        // Cancel only the current session's agent loop.
        if let Some(sid) = session_id {
            let rt = self.runtime.clone();
            tokio::spawn(async move {
                rt.cancel_session(sid).await;
            });
        }

        // Finalise streaming message and append an error notice.
        if let Some(ref mut chat) = self.message_list {
            chat.append_interrupted_message();
        }

        // Drop pending approvals for the current session.
        if let Some(sid) = session_id {
            self.pending_approvals.remove(&sid);
            if self.active_approval_session == Some(sid) {
                self.active_approval_session = None;
            }
            self.pending_compacts.remove(&sid);
            self.compacting_sessions.remove(&sid);
            self.pending_modes.remove(&sid);
        }

        // Clear queued prompts (all — they were for the user's current intent).
        self.pending_prompt_queue.clear();

        // Reset abort state.
        self.abort_confirmation_deadline = None;

        self.set_notice("Request cancelled");
    }

    pub(crate) fn has_pending_prompts(&self) -> bool {
        !self.pending_prompt_queue.is_empty()
    }

    /// Submit queued prompts now that their session's request has finished.
    ///
    /// Only submits prompts for sessions that are not currently busy.
    /// Other sessions' prompts remain queued.
    pub(crate) fn flush_pending_prompt_queue(&mut self) {
        let mut i = 0;
        while i < self.pending_prompt_queue.len() {
            let session_id = self.pending_prompt_queue[i].session_id;
            if self.runtime.is_session_busy(session_id) {
                i += 1;
                continue;
            }
            let queued = self.pending_prompt_queue.remove(i);
            let text = queued.prompt;
            let attachments = queued.attachments;
            let mode = queued.mode;
            let thinking_level = queued.thinking_level;
            let rt = self.runtime.clone();
            tokio::spawn(async move {
                if let Err(e) = rt
                    .submit_prompt_with_attachments(
                        session_id,
                        mode,
                        text,
                        attachments,
                        Some(thinking_level),
                    )
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
        self.compacting_sessions.insert(session_id);
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
}

// ── Inline @-reference extraction ───────────────────────────────────────

/// Extract file/directory paths from `@path` references in the prompt text.
///
/// Mirrors the old `tidev_tui::App::inline_file_references` behaviour:
/// finds `@` that is not preceded by a word character or backtick, and
/// captures the following path.
pub(super) fn extract_inline_refs(prompt: &str) -> Vec<String> {
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
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == b'`' || c == b',' {
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
