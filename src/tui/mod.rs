use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossterm::{
    cursor::Show,
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use image::ImageEncoder;
use ratatui::layout::{Position, Rect};
use std::{
    cell::{Cell, RefCell},
    env, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod commands;
mod core;
mod input;
mod panel_launcher;
mod render;
mod ui;

pub use commands::{CommandAction, CommandPaletteState, CommandRegistry};
pub use core::run;
pub use core::state;
pub use core::undo;
pub use input::Composer;
pub use input::at_mention;
pub use input::event;
pub use input::mouse_selection;
pub(crate) use panel_launcher::PanelAction;
pub use render::chat_dialog;
pub use render::chat_render;
pub use render::diff_render;
pub use ui::balance_panel;
pub use ui::connect;
pub use ui::mcp_panel;
pub use ui::memory_panel;
pub use ui::message_panel;
pub use ui::model_panel;
pub use ui::permission;
use ui::permission::SubagentStatus;
pub use ui::question;
pub use ui::session_panel;
pub use ui::settings_panel;
pub use ui::stats_panel;
pub use ui::theme_panel;

use core::state::*;

use crate::{
    agent::runtime::{AgentRuntime, PendingToolApproval},
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    instructions,
    llm::LlmClient,
    mcp::McpManager,
    memory::MemoryStore,
    notifications,
    prompts::{SessionMode, init_command},
    provider_setup::ConnectDialog,
    session::{
        AssistantTurn, BackendEvent, COMPACTION_MESSAGE_LABEL, Conversation, Message,
        MessageAttachment, MessageRole, ToolCall, ToolExecutionResult,
    },
    shared::file_search::current_at_fragment,
    snapshot::{FileDiff, SnapshotService},
    storage::SessionStore,
    theme::{ThemeManager, ThemeName},
    tooling::{FileReadTracker, TodoItem, ToolRegistry},
    tui::at_mention::{AtMentionKind, AtMentionState},
    tui::input::SnippetState,
    tui::input::shell_completion::ShellCompletionState,
    tui::mcp_panel::McpPanelState,
    tui::memory_panel::MemoryPanelState,
    tui::message_panel::MessagePanelState,
    tui::model_panel::ModelPanelState,
    tui::mouse_selection::{ClipboardLease, MouseSelectionState},
    tui::permission::{
        PendingToolExecution, PermissionDialogState, RunningSubagentExecution,
        RunningToolExecution, SandboxElevationDialog,
    },
    tui::question::QuestionDialogState,
    tui::session_panel::SessionPanelState,
    tui::settings_panel::SettingsPanelState,
    tui::theme_panel::ThemePanelState,
    tui::ui::rename::RenameSessionDialogState,
    tui::ui::workspace_boundary::WorkspaceBoundaryDialogState,
    utils::TokenUsage,
};

struct App {
    should_quit: bool,
    screen: Screen,
    workspace_root: PathBuf,
    paths: ConfigPaths,
    config: AppConfig,
    auth: AuthStore,
    store: SessionStore,
    llm: LlmClient,
    http_client: Arc<reqwest::Client>,
    theme: ThemeManager,
    mode: SessionMode,
    /// Pending mode switch that will take effect on the next user message.
    pending_mode: Option<SessionMode>,
    active_model: ActiveModel,
    conversation: Conversation,
    context_manager: ContextManager,
    tools: ToolRegistry,
    /// Shared AgentRuntime for compose_static_system_prompt / build_request_messages.
    agent: AgentRuntime,
    file_read_tracker: Arc<FileReadTracker>,
    commands: CommandRegistry,
    command_palette: CommandPaletteState,
    panel_launcher: panel_launcher::PanelLauncherState,
    connect_dialog: Option<ConnectDialog>,
    theme_panel: Option<ThemePanelState>,
    model_panel: Option<ModelPanelState>,
    message_panel: Option<MessagePanelState>,
    session_panel: Option<SessionPanelState>,
    settings_panel: Option<SettingsPanelState>,
    rename_dialog: Option<RenameSessionDialogState>,
    mcp_panel: Option<McpPanelState>,
    agents_panel: Option<ui::agents_panel::AgentsPanelState>,
    skills_panel: Option<ui::skills_panel::SkillsPanelState>,
    sandbox_panel: Option<ui::sandbox_panel::SandboxPanelState>,
    search_panel: Option<ui::search_panel::SearchPanelState>,
    at_mention: AtMentionState,
    snippet_state: SnippetState,
    shell_completion: ShellCompletionState,
    pending_tool_execution: Option<PendingToolExecution>,
    permission_dialog: Option<PermissionDialogState>,
    sandbox_elevation: Option<SandboxElevationDialog>,
    workspace_boundary_dialog: Option<WorkspaceBoundaryDialogState>,
    sensitive_file_dialog: Option<ui::sensitive::SensitiveFileDialogState>,
    /// In-memory sensitive file permissions (path -> allowed).
    /// Cleared when tidev exits or session switches.
    sensitive_file_permissions: std::collections::HashMap<String, bool>,
    /// In-memory workspace boundary permissions (path -> allowed).
    /// Cleared when tidev exits or session switches.
    workspace_boundary_permissions: std::collections::HashMap<String, bool>,
    /// Per-batch: tool calls approved for outside-workspace access (tool_call.id -> allow_outside).
    /// Populated during permission checking, consumed during parallel execution dispatch.
    workspace_boundary_approved: std::collections::HashMap<String, bool>,
    /// Per-batch: tool calls approved for sensitive file reads (tool_call.id -> bool).
    /// Populated during permission checking, consumed during parallel execution dispatch.
    sensitive_file_approved: std::collections::HashMap<String, bool>,
    question_dialog: Option<QuestionDialogState>,
    fork_confirm_dialog: Option<ui::fork_confirm::ForkConfirmDialogState>,
    undo_confirm_dialog: Option<ui::undo_confirm::UndoConfirmDialogState>,
    running_tool_executions: Vec<RunningToolExecution>,
    running_subagent_executions: Vec<RunningSubagentExecution>,
    pending_assistant_turns: std::collections::HashSet<Uuid>,
    cached_sessions: std::collections::HashMap<Uuid, CachedSessionRuntime>,
    compacting_sessions: std::collections::HashSet<Uuid>,
    leader_key_pending: bool,
    composer: Composer,
    draft_attachments: Vec<MessageAttachment>,
    pending_request: bool,
    /// Display-only queue of messages waiting to be processed by the agent loop.
    /// Kept for UI rendering — actual queueing goes through AgentRuntime.
    pending_prompt_queue: std::collections::VecDeque<crate::tui::core::state::QueuedPrompt>,
    active_request_id: u64,
    /// Cancel token for the current agent loop. Cancelled when the user
    /// double-presses Esc, causing the agent loop to stop at its next
    /// cancellation check point.
    request_cancel_token: Option<CancellationToken>,
    /// Current foreground session ID, shared with the background inactivity
    /// check task so it does not summarise the active session.
    current_session_id: Arc<RwLock<Uuid>>,
    /// Cancel token for the background inactivity check loop.
    inactivity_check_cancel: CancellationToken,
    abort_confirmation_deadline: Option<Instant>,
    last_notice: Option<String>,
    toast: Option<(String, Instant)>,
    mouse_selection: MouseSelectionState,
    retrying_hint: Option<(u32, u32, String, Instant)>,
    message_scroll_offset: usize,
    message_follow_tail: bool,
    message_viewport_lines: usize,
    message_total_lines: usize,
    message_render_cache:
        RefCell<std::collections::HashMap<MessageRenderCacheKey, MessageRenderCacheEntry>>,
    message_render_cache_tick: Cell<u64>,
    message_render_cache_hits: Cell<u64>,
    message_render_cache_misses: Cell<u64>,
    /// Layout index for viewport virtualization.
    /// Enables O(log n) lookup of visible messages via binary search,
    /// avoiding full traversal on every frame.
    message_layout_index: RefCell<MessageLayoutIndex>,
    message_content_area: Option<Rect>,
    message_scrollbar_area: Option<Rect>,
    scrollbar_drag_state: Option<state::ScrollbarDragState>,
    sidebar_area: Option<Rect>,
    sidebar_scroll_offset: usize,
    sidebar_total_lines: usize,
    input_area: Cell<Option<Rect>>,
    /// Overlay rect of the memory panel (for mouse hit-testing).
    memory_panel_overlay: Cell<Option<Rect>>,
    /// Overlay rects for various panels (for mouse hit-testing).
    theme_panel_overlay: Cell<Option<Rect>>,
    model_panel_overlay: Cell<Option<Rect>>,
    session_panel_overlay: Cell<Option<Rect>>,
    message_panel_overlay: Cell<Option<Rect>>,
    skills_panel_overlay: Cell<Option<Rect>>,
    settings_panel_overlay: Cell<Option<Rect>>,
    agents_panel_overlay: Cell<Option<Rect>>,
    mcp_panel_overlay: Cell<Option<Rect>>,
    balance_panel_overlay: Cell<Option<Rect>>,
    stats_panel_overlay: Cell<Option<Rect>>,
    /// Scroll offset for the input box when content exceeds visible area.
    input_scroll_offset: usize,
    /// Whether we're currently dragging in the input area (for text selection).
    input_dragging: bool,
    selection_clipboard_lease: Option<ClipboardLease>,
    last_render_time: Instant,
    render_throttled: bool,
    /// Dirty flag: set to true whenever UI state changes.
    /// The main loop skips terminal.draw() when this is false,
    /// eliminating wasted CPU during idle periods.
    dirty: bool,
    backend_tx: UnboundedSender<BackendEvent>,
    backend_rx: UnboundedReceiver<BackendEvent>,
    spinner_start: Instant,
    /// Last rendered spinner frame index (increments every 100ms).
    /// Used for lazy rendering: only redraw when the spinner visually changes.
    last_spinner_frame: u64,
    context_usage: Option<state::ContextUsage>,
    snapshot: SnapshotService,
    cleanup_cancel: Arc<std::sync::atomic::AtomicBool>,
    loaded_instruction_sources: Vec<String>,
    /// Cached instruction file contents to avoid redundant I/O
    instruction_content_cache: std::collections::HashMap<String, String>,
    expanded_tool_results: std::collections::HashSet<Uuid>,
    tool_result_card_bounds: Vec<(Uuid, Rect)>,
    /// Maps tool_call_id → child_session_id for subagent task navigation
    subagent_task_map: std::collections::HashMap<String, Uuid>,
    /// Running subagent card screen bounds: (execution_index, screen_rect)
    /// Recalculated every frame in render_messages()
    running_subagent_card_bounds: Vec<(usize, Rect)>,
    /// Permission channel receiver — receives [`PendingToolApproval`] from
    /// the spawned `run_agent_loop` task when tool calls need approval.
    pending_permission_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::agent::runtime::PendingToolApproval>>,
    /// The oneshot sender for the current pending permission approval.
    /// Set when we receive a `PendingToolApproval` and consumed when we
    /// send the response back to `run_agent_loop`.
    pending_permission_response:
        Option<tokio::sync::oneshot::Sender<Vec<crate::agent::runtime::ApprovedTool>>>,
    /// Buffer for rejected tool results during permission channel processing.
    /// Cleared when the approval response is sent.
    pending_rejected_tools: Vec<(ToolCall, ToolExecutionResult)>,
    pub(crate) selectable_regions: Vec<Rect>,
    message_scroll_target: Option<Uuid>,
    todos: Vec<TodoItem>,
    /// Intermediate snapshot hashes captured after each tool execution step within a round.
    /// Used at round end to compute per-step patches.
    step_snapshot_hashes: Vec<String>,
    /// Per-step file lists cached during capture_step_snapshot to avoid
    /// re-computing expensive patch() calls during finalization.
    step_cached_file_lists: Vec<Vec<String>>,
    /// Cumulative lightweight FileDiff entries accumulated across steps in the current round.
    /// Updated per-step for sidebar display; replaced by full diff at round end.
    step_cached_file_diffs: Option<Vec<FileDiff>>,
    /// The previous snapshot hash, used for computing per-step lightweight diffs.
    step_prev_hash: Option<String>,
    stats_panel: Option<ui::stats_panel::StatsPanelState>,
    balance_panel: Arc<Mutex<Option<ui::balance_panel::BalancePanelState>>>,
    notifications: notifications::NotificationManager,
    /// Whether the input is in shell command mode (triggered by `!` prefix).
    shell_mode: bool,
    /// DeepSeek thinking level for the current model
    thinking_level: crate::config::reasoning::ThinkingLevelType,
    /// Cross-session memory store
    memory_store: Arc<MemoryStore>,
    /// Memory management panel
    memory_panel: Option<MemoryPanelState>,
    /// TUI terminal session for raw mode / alternate screen management.
    /// Used to suspend/resume the TUI when launching external editors.
    terminal_session: Option<TerminalSession>,
    /// Flag set after TUI suspend/resume cycle to force a full terminal redraw
    /// on the next event loop iteration (ratatui's frame buffer is stale after
    /// leaving and re-entering the alternate screen).
    force_full_redraw: bool,
}
pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    let mut app = App::new()?;
    app.run(&runtime)?;
    // Don't wait for blocking tasks (e.g. tool executions that are
    // still running) — the program is exiting and the OS will clean
    // up any orphaned child processes.
    runtime.shutdown_background();
    Ok(())
}

impl App {
    /// Build prompt attachments for @ references with truncation like opencode.
    /// Uses build_at_reference_attachment for @ references to apply read tool truncation.
    /// Returns (attachments, instruction_sources) where instruction_sources contains
    /// the deduplicated paths of nearby instruction files that were loaded.
    fn build_prompt_attachments(&self, prompt: &str) -> Result<(Vec<MessageAttachment>, Vec<String>)> {
        let mut attachments = Vec::new();
        let mut all_instruction_sources = Vec::new();
        let mut seen_paths = std::collections::BTreeSet::new();

        for path in self.inline_file_references(prompt) {
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            // Use build_at_reference_attachment for @ references with truncation
            match self.build_at_reference_attachment(&path)? {
                Some((attachment, sources)) => {
                    attachments.push(attachment);
                    for source in sources {
                        if !all_instruction_sources.contains(&source) {
                            all_instruction_sources.push(source);
                        }
                    }
                }
                None => continue,
            }
        }

        // Add draft attachments (pasted files) without truncation
        attachments.extend(self.draft_attachments.iter().cloned());
        Ok((attachments, all_instruction_sources))
    }

    /// Build attachment for @ reference with truncation like opencode's read tool.
    /// Returns (MessageAttachment, instruction_sources) where instruction_sources
    /// contains the paths of nearby instruction files that were loaded.
    fn build_at_reference_attachment(&self, path: &str) -> Result<Option<(MessageAttachment, Vec<String>)>> {
        use crate::tooling::builtin::file::read_file_for_at_reference;

        let absolute = self.resolve_workspace_path(path);
        let metadata = match std::fs::metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(_error) => return Ok(None),
        };

        if metadata.is_dir() {
            let tree = build_directory_tree(&absolute, 2, 80)?;
            return Ok(Some((MessageAttachment::DirectoryReference {
                path: path.trim_end_matches(['/', '\\']).to_string(),
                tree: Arc::new(tree),
            }, Vec::new())));
        }

        if let Some(mime) = image_mime_from_path(&absolute) {
            let bytes = std::fs::read(&absolute)?;
            let filename = absolute
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_string();
            let data_url = format!("data:{mime};base64,{}", BASE64_STANDARD.encode(bytes));
            return Ok(Some((MessageAttachment::Image {
                filename,
                mime: mime.to_string(),
                data_url,
            }, Vec::new())));
        }

        // For text files, read with truncation like opencode's read tool
        match read_file_for_at_reference(&self.workspace_root, path, false) {
            Ok((mut tool_output, truncated)) => {
                // Load nearby instruction files (like the read tool does)
                let mut instruction_sources = Vec::new();
                if let Ok(nearby) = instructions::resolve_nearby_instructions(
                    &self.workspace_root,
                    &self.paths.config_dir,
                    &absolute,
                )
                && !nearby.is_empty()
                {
                    let mut reminders = Vec::new();
                    for (ipath, content) in nearby {
                        instruction_sources.push(ipath.to_string_lossy().to_string());
                        reminders.push(content);
                    }
                    tool_output.push_str(&format!(
                        "\n\n<system-reminder>\n{}\n</system-reminder>",
                        reminders.join("\n\n")
                    ));
                }

                // Also read full content for display purposes
                let content = std::fs::read_to_string(&absolute).unwrap_or_else(|_| String::new());
                Ok(Some((MessageAttachment::FileReference {
                    path: path.to_string(),
                    content: Arc::new(content),
                    tool_output: Some(Arc::new(tool_output)),
                    truncated,
                }, instruction_sources)))
            }
            Err(_error) => {
                // Fall back to full content if read fails
                let content = std::fs::read_to_string(&absolute).unwrap_or_else(|_| String::new());
                Ok(Some((MessageAttachment::FileReference {
                    path: path.to_string(),
                    content: Arc::new(content),
                    tool_output: None,
                    truncated: false,
                }, Vec::new())))
            }
        }
    }

    #[allow(dead_code)]
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
                tree: Arc::new(tree),
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
            content: Arc::new(content),
            tool_output: None,
            truncated: false,
        }))
    }

    /// Parse @ references using fancy-regex like opencode.
    /// Regex: `(?<![\w\`])@(\.?[^\s\`.,]*(?:\.[^\s\`.,]+)*)`
    /// Uses look-behind to ensure @ is not preceded by word characters or backticks.
    fn inline_file_references(&self, prompt: &str) -> Vec<String> {
        use fancy_regex::Regex;
        // Look-behind: (?<![\w`]) ensures @ is not preceded by word chars or backticks
        let re = Regex::new(r"(?<![\w`])@(\.?[^\s`.,]*(?:\.[^\s`.,]+)*)").unwrap();
        let mut paths = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        let mut start = 0;
        while let Some(caps) = re.captures(&prompt[start..]).unwrap() {
            if let Some(path_match) = caps.get(1) {
                let path = path_match.as_str();
                if path.is_empty() {
                    break;
                }
                if !seen.insert(path.to_string()) {
                    // Move to next position
                    start += path_match.start() + 1;
                    continue;
                }
                paths.push(path.to_string());
                // Move past this match
                start += path_match.start() + 1;
            } else {
                break;
            }
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

    fn refresh_tools(&mut self) {
        let mcp = self.tools.mcp_manager();
        let file_read_tracker = self.tools.file_read_tracker();
        let worktree = Self::find_git_worktree(&self.workspace_root);
        self.tools = ToolRegistry::new(
            self.workspace_root.clone(),
            self.paths.config_dir.clone(),
            self.config.skills.clone(),
            mcp,
            self.config.permissions.clone(),
            file_read_tracker,
            self.memory_store.clone(),
            self.config.rtk.enabled,
            worktree,
            self.config.websearch.clone(),
            Arc::new(self.auth.clone()),
        );
        // Set sandbox policy based on current mode
        let sandbox_policy = self.mode.sandbox_policy(&self.config.sandbox);
        self.tools.set_sandbox_policy(Some(sandbox_policy));
        // Also sync to the agent's ToolRegistry (separate copy at init)
        let agent_policy = self.mode.sandbox_policy(&self.config.sandbox);
        self.agent.tools.set_sandbox_policy(Some(agent_policy));
    }

    /// Find the git worktree root by looking for a .git directory,
    /// starting from the given path and walking up to the ancestors.
    fn find_git_worktree(start: &Path) -> Option<PathBuf> {
        for ancestor in start.ancestors() {
            if ancestor.join(".git").is_dir() {
                return Some(ancestor.to_path_buf());
            }
        }
        None
    }

    fn update_loaded_instruction_sources(&mut self, sources: &[String]) -> Result<()> {
        let display_sources: Vec<String> = sources
            .iter()
            .map(|source| self.display_instruction_source(source))
            .collect();

        // Find newly loaded sources
        let mut newly_loaded = Vec::new();
        for source in &display_sources {
            if !self.loaded_instruction_sources.contains(source) {
                newly_loaded.push(source.clone());
            }
        }

        if !newly_loaded.is_empty() {
            let content = if newly_loaded.len() == 1 {
                format!("Loaded instructions from {}", newly_loaded[0])
            } else {
                format!(
                    "Loaded {} instruction files: {}",
                    newly_loaded.len(),
                    newly_loaded.join(", ")
                )
            };

            self.push_message(MessageRole::System, content)?;

            // Merge newly loaded sources instead of overwriting the entire list.
            // This prevents previously loaded sources (like root AGENTS.md) from being
            // "re-discovered" as new when deep directory instructions are added.
            for source in newly_loaded {
                if !self.loaded_instruction_sources.contains(&source) {
                    if let Err(e) = self
                        .store
                        .append_instruction_source(self.conversation.session_id, &source)
                    {
                        crate::log_warn!("Failed to save instruction source to database: {}", e);
                    }
                    self.loaded_instruction_sources.push(source);
                }
            }
        }
        Ok(())
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

    #[allow(dead_code)]
    fn push_system_message(&mut self, content: impl Into<String>) -> Result<()> {
        self.push_message(MessageRole::System, content)
    }

    #[allow(dead_code)]
    fn push_message(&mut self, role: MessageRole, content: impl Into<String>) -> Result<()> {
        let message = Message::new(role, content);
        self.conversation.push(message.clone());
        self.store
            .append_message(self.conversation.session_id, &message)?;
        self.screen = Screen::Chat;
        Ok(())
    }

    fn process_backend_events(&mut self, runtime: &Runtime) -> Result<()> {
        // Coalesce consecutive Delta and ReasoningDelta events to reduce
        // per-frame cache invalidation overhead during LLM streaming.
        let mut coalesced_delta: Option<(Uuid, u64, String)> = None;
        let mut coalesced_reasoning: Option<(Uuid, u64, String)> = None;
        let mut event_count = 0;
        const MAX_EVENTS_PER_BATCH: usize = 200;

        while let Ok(event) = self.backend_rx.try_recv() {
            event_count += 1;
            if event_count > MAX_EVENTS_PER_BATCH {
                // Put the event back? No, we can't with try_recv.
                // Instead, we just stop processing and leave remaining
                // events in the channel for the next frame.
                // To avoid losing the event, we need to re-insert it
                // into the channel. Since we can't, we'll process it
                // but stop after this one.
                // Actually, once we've exceeded MAX_EVENTS_PER_BATCH,
                // the remaining events will be picked up next frame.
                // But we've already consumed this event from the channel.
                // The best we can do is: don't break; instead, just
                // skip the coalescing optimization for the overflow.
                // For fairness, process the event directly.
                self.flush_coalesced_events(
                    &mut coalesced_delta,
                    &mut coalesced_reasoning,
                    runtime,
                )?;
                self.handle_backend_event(event, runtime)?;
                // Don't continue draining; leave rest for next frame
                break;
            }

            match event {
                BackendEvent::Delta {
                    session_id,
                    request_id,
                    content,
                } => {
                    // Coalesce consecutive Delta events for the same request
                    if let Some((_, _, ref mut acc)) = coalesced_delta {
                        acc.push_str(&content);
                    } else {
                        coalesced_delta = Some((session_id, request_id, content));
                    }
                }
                BackendEvent::ReasoningDelta {
                    session_id,
                    request_id,
                    content,
                } => {
                    // Coalesce consecutive ReasoningDelta events
                    if let Some((_, _, ref mut acc)) = coalesced_reasoning {
                        acc.push_str(&content);
                    } else {
                        coalesced_reasoning = Some((session_id, request_id, content));
                    }
                }
                _other => {
                    // Flush coalesced events before processing a non-delta event
                    // to preserve ordering (deltas must arrive before Finished).
                    self.flush_coalesced_events(
                        &mut coalesced_delta,
                        &mut coalesced_reasoning,
                        runtime,
                    )?;
                    self.handle_backend_event(_other, runtime)?;
                }
            }
        }

        // Flush any remaining coalesced events
        self.flush_coalesced_events(&mut coalesced_delta, &mut coalesced_reasoning, runtime)?;

        // Check for pending permission approvals from the agent runtime.
        // Take ownership of the receiver to avoid borrow conflicts.
        if let Some(mut rx) = self.pending_permission_rx.take() {
            while let Ok(approval) = rx.try_recv() {
                crate::log_info!(
                    "process_backend_events: received PendingToolApproval with {} tool call(s)",
                    approval.tool_calls.len()
                );
                // Store the response channel
                self.pending_permission_response = Some(approval.response_tx);
                self.pending_rejected_tools.clear();

                // Create PendingToolExecution and start permission processing
                self.begin_tool_execution(approval.tool_calls, approval.mode, runtime)?;
            }
            // Put the receiver back
            self.pending_permission_rx = Some(rx);
        }

        Ok(())
    }

    /// Flush coalesced Delta and ReasoningDelta events by sending them
    /// as single merged events through `handle_backend_event`.
    fn flush_coalesced_events(
        &mut self,
        delta: &mut Option<(Uuid, u64, String)>,
        reasoning: &mut Option<(Uuid, u64, String)>,
        runtime: &Runtime,
    ) -> Result<()> {
        if let Some((sid, rid, content)) = delta.take() {
            self.handle_backend_event(
                BackendEvent::Delta {
                    session_id: sid,
                    request_id: rid,
                    content,
                },
                runtime,
            )?;
        }
        if let Some((sid, rid, content)) = reasoning.take() {
            self.handle_backend_event(
                BackendEvent::ReasoningDelta {
                    session_id: sid,
                    request_id: rid,
                    content,
                },
                runtime,
            )?;
        }
        Ok(())
    }

    fn handle_backend_event(&mut self, event: BackendEvent, runtime: &Runtime) -> Result<()> {
        self.dirty = true;
        // Sandbox elevation requests are handled here, outside the per-session
        // dispatch, because they carry a oneshot sender that must not be moved
        // into the event handler's match.
        if let BackendEvent::SandboxElevationRequest { response_tx, .. } = event {
            // Extract the sender from the Arc wrapper
            let sender = response_tx.lock().unwrap().take();
            self.sandbox_elevation = Some(SandboxElevationDialog::new(sender));
            return Ok(());
        }

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
            BackendEvent::ToolCallUpdated { .. } => "ToolCallUpdated",
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
            BackendEvent::InstructionsLoaded { .. } => "InstructionsLoaded",
            BackendEvent::ContextCompacted { .. } => "ContextCompacted",
            BackendEvent::SidebarSnapshotReady { .. } => "SidebarSnapshotReady",
            BackendEvent::ShellOutput { .. } => "ShellOutput",
            BackendEvent::TurnStarting { .. } => "TurnStarting",
            BackendEvent::SandboxElevationRequest { .. } => "SandboxElevationRequest",
        };
        if event_type != "Delta"
            && event_type != "ReasoningDelta"
            && event_type != "ToolCallUpdated"
            && event_type != "UsageStats"
            && event_type != "SubagentStatus"
            && event_type != "InstructionsLoaded"
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

                // Clear retry hint — a successful retry resumes streaming
                self.retrying_hint = None;

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && (matches!(message.role, MessageRole::Assistant)
                        || (matches!(message.role, MessageRole::System)
                            && message.content.starts_with(COMPACTION_MESSAGE_LABEL)))
                {
                    message.content.push_str(&content);
                    let message_id = message.id;
                    // Incremental update via dirty_messages (set by invalidate_*) is
                    // sufficient — content-only changes do not need a full layout rebuild.
                    self.invalidate_active_message_render_cache_for(message_id);
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

                // Clear retry hint — a successful retry resumes reasoning
                self.retrying_hint = None;

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.reasoning.push_str(&content);
                    let message_id = message.id;
                    // Incremental update via dirty_messages is sufficient.
                    self.invalidate_active_message_render_cache_for(message_id);
                }
            }
            BackendEvent::ToolCallUpdated {
                session_id: _,
                request_id,
                tool_call,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                // Clear retry hint — a successful retry resumes tool calls
                self.retrying_hint = None;

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.upsert_tool_call(tool_call);
                    let message_id = message.id;
                    self.invalidate_active_message_render_cache_for(message_id);
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

                self.retrying_hint = None;
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

                let deadline =
                    Instant::now() + Duration::from_secs(retry_after_secs.unwrap_or(0) as u64);
                self.retrying_hint = Some((attempt, max_attempts, reason, deadline));
            }
            BackendEvent::Failed {
                session_id: _,
                request_id,
                error,
            } => {
                if !self.is_active_request(request_id) {
                    // This may be a Failed event from a subagent child session
                    // (the child session has its own request_id).  Try to match
                    // it against running_subagent_executions and clean up.
                    if self
                        .running_subagent_executions
                        .iter()
                        .any(|e| e.request_id == request_id)
                    {
                        crate::log_info!(
                            "Failed event for subagent child session request_id={}, cleaning up",
                            request_id
                        );
                        self.running_subagent_executions
                            .retain(|e| e.request_id != request_id);
                    }
                    return Ok(());
                }

                self.pending_request = false;
                self.pending_tool_execution = None;
                self.permission_dialog = None;
                self.question_dialog = None;
                self.fork_confirm_dialog = None;
                self.running_tool_executions.clear();
                self.workspace_boundary_approved.clear();
                self.cancel_running_subagents();
                self.abort_confirmation_deadline = None;
                self.retrying_hint = None;
                // Clean up cancel token and permission channel so the agent
                // loop can exit promptly.
                self.request_cancel_token.take();
                self.pending_permission_response = None;
                self.pending_permission_rx = None;

                self.notifications
                    .notify(&format!("Request failed: {}", error));

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.role = MessageRole::Error;
                    message.streaming = false;
                    message.content = format!("Request failed: {error}");
                    let persisted = message.clone();
                    let message_id = message.id;
                    self.invalidate_active_message_render_cache_for(message_id);
                    self.store
                        .append_message(self.conversation.session_id, &persisted)?;
                    self.last_notice = Some(error.clone());
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

                let running_idx = self
                    .running_tool_executions
                    .iter()
                    .position(|r| r.request_id == request_id && r.tool_call.id == tool_call.id);

                if let Some(idx) = running_idx {
                    let running = self.running_tool_executions.remove(idx);

                    // For bash tool calls streamed via ShellOutput, find and
                    // finalize the existing Tool message instead of creating
                    // a duplicate via record_tool_result.
                    if tool_call.name == "bash" {
                        let tool_idx = self.conversation.messages.iter().rposition(|m| {
                            m.role == MessageRole::Tool
                                && m.tool_call_id.as_deref() == Some(&tool_call.id)
                        });
                        if let Some(tool_idx) = tool_idx {
                            let display_result = result.preview_for_storage(Some("bash"));
                            let message_id = self.conversation.messages[tool_idx].id;
                            self.conversation.messages[tool_idx].content = display_result.output;
                            self.conversation.messages[tool_idx].streaming = false;
                            self.conversation.messages[tool_idx].attachments =
                                display_result.attachments;
                            // Incremental update via dirty_messages is sufficient;
                            // the parent Assistant message is added below.
                            if let Some(assistant_id) = self
                                .conversation
                                .messages
                                .iter()
                                .rev()
                                .skip_while(|m| m.id != message_id)
                                .find(|m| m.role == MessageRole::Assistant)
                                .map(|m| m.id)
                            {
                                self.invalidate_active_message_render_cache_for(assistant_id);
                            }
                            // Persist the final message
                            let persisted = self.conversation.messages[tool_idx].clone();
                            if let Err(e) = self
                                .store
                                .append_message(self.conversation.session_id, &persisted)
                            {
                                crate::log_warn!("ToolCompleted/bash: failed to persist: {}", e);
                            }
                        } else {
                            // Fallback: no streaming message existed
                            self.record_tool_result(tool_call.clone(), result)?;
                        }
                    } else {
                        self.record_tool_result(running.tool_call, result)?;
                    }

                    // Capture step snapshot for per-step undo tracking and sidebar updates
                    self.capture_step_snapshot(runtime);

                    // If all tools in this round have completed, finalize the
                    // snapshot so patch_files are attributed to the correct user
                    // message rather than left dangling for the next Finished event.
                    if self.running_tool_executions.is_empty()
                        && self.running_subagent_executions.is_empty()
                        && let Err(error) =
                            self.finalize_snapshot_for_last_user_message_sync(runtime)
                    {
                        crate::log_warn!("ToolCompleted: failed to finalize snapshot: {}", error);
                    }

                    // Also clean up running_subagent_executions for task tools.
                    // Match by tool_call.id instead of request_id so that
                    // parallel subagents (which share the same request_id) are
                    // each removed individually rather than all at once.
                    if tool_call.name == "task"
                        && let Some(pos) = self.running_subagent_executions.iter().position(|e| {
                            e.request_id == request_id && e.tool_call.id == tool_call.id
                        })
                    {
                        self.running_subagent_executions.remove(pos);
                    }
                }
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

                if let Some(execution) =
                    self.running_subagent_executions
                        .iter_mut()
                        .find(|execution| {
                            // Must match BOTH request_id and child_session_id so that
                            // parallel subagents (which share the same parent request_id)
                            // each get their own status updates instead of all going
                            // to the first matching execution.
                            execution.request_id == request_id
                                && execution.child_session_id == child_session_id
                        })
                {
                    execution.status = SubagentStatus::from_status_text(&status_text);
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
                            let message_id = existing.id;
                            existing.content = message.content.clone();
                            existing.reasoning = message.reasoning.clone();
                            existing.tool_calls = message.tool_calls.clone();
                            existing.streaming = message.streaming;
                            self.invalidate_active_message_render_cache_for(message_id);
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
                        // Invalidate layout index and render cache since we added a new message
                        self.message_layout_index.borrow_mut().valid = false;
                        self.clear_message_render_cache();
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

                // Try to find and remove from running_subagent_executions.
                // May already be gone if ToolCompleted cleaned it up first.
                let parent_session_id = {
                    let idx = self
                        .running_subagent_executions
                        .iter()
                        .position(|execution| {
                            execution.request_id == request_id
                                && execution.child_session_id == child_session_id
                                && execution.tool_call.id == tool_call.id
                        });
                    idx.map(|i| self.running_subagent_executions.remove(i).parent_session_id)
                };

                if let Some(parent_session_id) = parent_session_id {
                    crate::log_info!(
                        "Removed running_subagent_execution, remaining count={}, parent_session_id={}",
                        self.running_subagent_executions.len(),
                        parent_session_id
                    );

                    if self.conversation.session_id == parent_session_id {
                        // We're on the parent session.  ToolCompleted already
                        // called record_tool_result, so we only update the
                        // UI notice here.
                    } else {
                        // User switched to the child session view.
                        // ToolCompleted may not have processed the result
                        // for the parent session, so write it to DB directly.
                        crate::log_info!(
                            "SubagentCompleted: user switched away from parent session, writing to database directly"
                        );
                        let display_result = if tool_call.name == "task" {
                            result.clone()
                        } else {
                            result.preview_for_storage(Some(tool_call.name.as_str()))
                        };
                        let message = Message::tool_result(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            display_result,
                        );
                        self.store.append_message(parent_session_id, &message)?;
                        self.pending_assistant_turns.insert(parent_session_id);
                        crate::log_info!(
                            "SubagentCompleted: marked parent_session_id={} as pending assistant turn",
                            parent_session_id
                        );
                    }
                }

                // Update UI notice based on remaining subagents
                if self.running_subagent_executions.is_empty() {
                    self.last_notice = None;
                } else {
                    let count = self.running_subagent_executions.len();
                    let label = if count == 1 { "subagent" } else { "subagents" };
                    self.last_notice = Some(format!("Waiting for {} {}...", count, label));
                }
            }
            BackendEvent::UsageStats {
                session_id: _,
                request_id,
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                duration_ms,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                let token_usage = TokenUsage::new(
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                );
                let tokens_per_second = token_usage.tokens_per_second(duration_ms);

                self.context_usage = Some(state::ContextUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    model_id: model_id.clone(),
                    tokens_per_second,
                });

                if let Some(message) = self.conversation.messages.last_mut()
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.input_tokens = Some(input_tokens);
                    message.output_tokens = Some(output_tokens);
                    message.total_tokens = Some(total_tokens);
                    message.cache_read_tokens = Some(cache_read_tokens);
                    message.cache_write_tokens = Some(cache_write_tokens);
                    message.tokens_per_second = tokens_per_second;
                }

                let _ = self.store.record_usage(
                    &self.active_model.provider_id,
                    &model_id,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                );
            }
            BackendEvent::InstructionsLoaded {
                session_id: _,
                sources,
            } => {
                self.update_loaded_instruction_sources(&sources)?;
            }
            BackendEvent::ContextCompacted {
                session_id,
                compacted,
                manual,
                summary,
                retained_from,
                error,
            } => {
                self.apply_context_compaction(
                    session_id,
                    compacted,
                    manual,
                    summary,
                    retained_from,
                    error,
                );
            }
            BackendEvent::SidebarSnapshotReady {
                session_id: _,
                request_id: _,
                message_id,
                file_diffs_json,
            } => {
                crate::log_info!(
                    "handle_backend_event: SidebarSnapshotReady message_id={}",
                    message_id
                );
                // Update the message with full file diffs (including patches)
                if let Some(msg) = self
                    .conversation
                    .messages
                    .iter_mut()
                    .find(|m| m.id == message_id)
                {
                    msg.file_diffs = Some(file_diffs_json.clone());
                    // Also persist to database
                    if let Err(e) = self.store.update_message_file_diffs(
                        self.conversation.session_id,
                        message_id,
                        &file_diffs_json,
                    ) {
                        crate::log_warn!(
                            "SidebarSnapshotReady: failed to persist file_diffs: {}",
                            e
                        );
                    }
                    // Invalidate render cache so sidebar re-renders
                    self.invalidate_active_message_render_cache_for(message_id);
                }
            }
            BackendEvent::ShellOutput {
                session_id: _,
                content,
                finished,
                exit_code: _,
            } => {
                // Stream bash output into a ToolResult message in real-time,
                // preserving the original tool card style.
                // Match by the running bash execution's tool_call_id to
                // correctly handle multiple tool calls in the same turn.
                let running_bash = self
                    .running_tool_executions
                    .iter()
                    .find(|r| r.tool_call.name == "bash");
                let bash_tool_call_id = running_bash.map(|r| r.tool_call.id.as_str());

                if let Some(tool_call_id) = bash_tool_call_id {
                    let existing = self.conversation.messages.iter().rposition(|m| {
                        m.role == MessageRole::Tool
                            && m.streaming
                            && m.tool_call_id.as_deref() == Some(tool_call_id)
                    });

                    if let Some(idx) = existing {
                        // Update streaming tool message content.
                        // The render cache is keyed by the parent Assistant
                        // message's ID, so we must invalidate that one.
                        let message_id = self.conversation.messages[idx].id;
                        self.conversation.messages[idx].content = content.clone();
                        if finished {
                            self.conversation.messages[idx].streaming = false;
                        }
                        // Incremental update via dirty_messages is sufficient;
                        // the parent Assistant message is added below.
                        if let Some(assistant_id) = self
                            .conversation
                            .messages
                            .iter()
                            .rev()
                            .skip_while(|m| m.id != message_id)
                            .find(|m| m.role == MessageRole::Assistant)
                            .map(|m| m.id)
                        {
                            self.invalidate_active_message_render_cache_for(assistant_id);
                        }
                        if finished {
                            let persisted = self.conversation.messages[idx].clone();
                            if let Err(e) = self
                                .store
                                .append_message(self.conversation.session_id, &persisted)
                            {
                                crate::log_warn!("ShellOutput: failed to persist message: {}", e);
                            }
                        }
                    } else {
                        // Create a new streaming ToolResult message.
                        let running = running_bash.unwrap();
                        let mut msg = Message::new(MessageRole::Tool, &content);
                        msg.tool_call_id = Some(running.tool_call.id.clone());
                        msg.tool_name = Some(running.tool_call.name.clone());
                        msg.streaming = !finished;
                        self.conversation.push(msg);
                        self.message_layout_index.borrow_mut().valid = false;
                        self.clear_message_render_cache();
                    }
                }
            }
            BackendEvent::TurnStarting {
                session_id,
                request_id,
            } => {
                crate::log_info!(
                    "TurnStarting: new request_id={}, previous active_request_id={}",
                    request_id,
                    self.active_request_id
                );

                // Only handle TurnStarting for the current conversation
                // session.  Child sessions (from parallel subagents) send
                // their own TurnStarting, which must not overwrite the
                // parent's active_request_id — otherwise subsequent parent
                // events (ToolCompleted, SubagentCompleted, etc.) would
                // fail the is_active_request check and be silently ignored.
                if session_id != self.conversation.session_id {
                    return Ok(());
                }

                // Ignore stale TurnStarting from a cancelled/aborted agent
                // loop.  If no cancel token exists, no agent loop is
                // running to serve this turn.
                if self.request_cancel_token.is_none() {
                    crate::log_info!(
                        "TurnStarting ignored: no active cancel token (request was aborted)"
                    );
                    return Ok(());
                }

                self.active_request_id = request_id;

                // If the previous turn finished without tool calls, this
                // TurnStarting is from a queued message being dequeued by
                // the agent loop.  Pop from the display queue, push a user
                // message card into the conversation, and reset
                // pending_request so the UI shows the spinner again.
                if !self.pending_request {
                    if let Some(queued) = self.pending_prompt_queue.pop_front() {
                        let mut user_message = Message::new(MessageRole::User, &queued.prompt);
                        user_message.attachments = queued.attachments;
                        user_message.mode = queued.mode;
                        user_message.thinking_level = queued.thinking_level;
                        self.conversation.push(user_message);

                        // Show "Loaded instructions" notification below the user message
                        if let Err(e) = self.update_loaded_instruction_sources(&queued.instruction_sources) {
                            crate::log_warn!("Failed to update instruction sources for queued prompt: {}", e);
                        }
                    }
                    self.pending_request = true;
                    self.last_notice = Some(match self.mode {
                        SessionMode::Plan => "Planning...".to_string(),
                        SessionMode::Build => "Thinking...".to_string(),
                    });
                }

                // Create a new streaming assistant message for the next turn
                let mut assistant_message = Message::streaming(MessageRole::Assistant, "");
                assistant_message.mode = Some(self.mode);
                self.conversation.push(assistant_message);
            }
            BackendEvent::SandboxElevationRequest { .. } => {
                // Handled in handle_backend_event before dispatch
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

        // Update the in-memory streaming message with the final turn data.
        // Persistence is handled by `run_agent_loop` internally.
        let mut finished_message_id = None;
        let has_tool_calls = !turn.tool_calls.is_empty();
        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.content = turn.content.clone();
            message.reasoning = turn.reasoning.clone();
            message.tool_calls = turn.tool_calls.clone();
            message.streaming = false;
            finished_message_id = Some(message.id);
            if message.mode.is_none() {
                message.mode = Some(self.mode);
            }

            if let Some(ref usage) = self.context_usage {
                message.input_tokens = Some(usage.input_tokens);
                message.output_tokens = Some(usage.output_tokens);
                message.total_tokens = Some(usage.total_tokens);
                message.cache_read_tokens = Some(usage.cache_read_tokens);
                message.cache_write_tokens = Some(usage.cache_write_tokens);
                message.model_id = Some(usage.model_id.clone());
                message.completed_at = Some(chrono::Utc::now());
            }
        }

        if let Some(message_id) = finished_message_id {
            self.invalidate_active_message_render_cache_for(message_id);
        }

        // If no tool calls, this is the final turn — clean up.
        // If tool calls exist, the permission channel will handle approval
        // and `run_agent_loop` will continue the loop.
        if !has_tool_calls {
            self.last_notice = Some(match turn.finish_reason.as_deref() {
                Some(reason) if reason != "stop" => format!("Response finished ({reason})"),
                _ => "Response complete".to_string(),
            });
            self.pending_request = false;
            self.abort_confirmation_deadline = None;

            // Apply pending mode switch if any
            if let Some(new_mode) = self.pending_mode.take() {
                self.mode = new_mode;
                self.refresh_tools();
                self.last_notice = Some(format!("Mode switched to {}", new_mode.as_str()));
            }

            if let Err(error) = self.finalize_snapshot_for_last_user_message_sync(runtime) {
                crate::log_warn!("failed to finalize snapshot: {}", error);
            }

            self.notifications.notify("Response complete");
        } else {
            // Tool calls are present — keep `pending_request` true.
            // Permission approval will happen via the channel, and
            // `run_agent_loop` will execute approved tools automatically.
            self.last_notice = Some(format!(
                "Processing {} tool call(s)...",
                turn.tool_calls.len()
            ));
        }

        Ok(())
    }

    fn queue_prompt(&mut self, prompt: String, attachments: Vec<MessageAttachment>, instruction_sources: Vec<String>) {
        // If there's a pending mode switch, use that mode for the queued message
        // so the user's intent to switch modes takes effect on the next message.
        let mode = self.pending_mode.unwrap_or(self.mode);
        let thinking_level = self.thinking_level.clone();

        // Queue via runtime for processing
        let msg = crate::agent::runtime::QueuedUserMessage {
            content: prompt.clone(),
            attachments: attachments.clone(),
            mode: Some(mode),
            thinking_level: Some(thinking_level.clone()),
        };
        self.agent.queue_user_message(msg);

        // Add to display queue for UI rendering
        // instruction_sources will be shown when the queued message is processed
        // (in TurnStarting handler) so the notification appears below the user message.
        self.pending_prompt_queue
            .push_back(crate::tui::core::state::QueuedPrompt::new(
                prompt,
                attachments,
                Some(mode),
                Some(thinking_level),
                instruction_sources,
            ));
    }

    fn submit_prompt_now(
        &mut self,
        prompt: String,
        attachments: Vec<MessageAttachment>,
        instruction_sources: Vec<String>,
        runtime: &Runtime,
    ) -> Result<()> {
        let _t_submit = std::time::Instant::now();
        let prompt = prompt.trim().to_string();

        if prompt.is_empty() && attachments.is_empty() {
            return Ok(());
        }

        if self.screen == Screen::Welcome {
            let _t_session = std::time::Instant::now();
            let session_exists = self
                .store
                .load_session_record(self.conversation.session_id)?
                .is_some();

            if !session_exists {
                let session_id = Uuid::new_v4();
                self.conversation.session_id = session_id;
                self.conversation.clear_context_state();
                let _t_create = std::time::Instant::now();
                self.store.create_session(
                    session_id,
                    self.workspace_root.as_path(),
                    &self.active_model.provider_id,
                    &self.active_model.provider_display_name,
                    &self.active_model.model_id,
                    &self.active_model.display_name,
                    "Untitled session",
                )?;
                crate::log_info!("agent: create_session took {:?}", _t_create.elapsed());

                // Compose the immutable static system prompt and persist it.
                let _t_prompt = std::time::Instant::now();
                let static_prompt = self
                    .agent
                    .compose_static_system_prompt(&self.active_model.system_prompt);
                crate::log_info!(
                    "agent: compose_static_system_prompt took {:?}",
                    _t_prompt.elapsed()
                );
                self.active_model.system_prompt = static_prompt.clone();
                if let Err(e) = self
                    .store
                    .update_session_system_prompt(session_id, &static_prompt)
                {
                    crate::log_warn!("failed to persist static system prompt: {}", e);
                }
            }
            crate::log_info!("agent: session init took {:?}", _t_session.elapsed());
            self.context_manager = ContextManager::new();
            self.pending_tool_execution = None;
            self.permission_dialog = None;
            self.question_dialog = None;
            self.fork_confirm_dialog = None;
            self.running_tool_executions.clear();
            self.workspace_boundary_approved.clear();
            self.abort_confirmation_deadline = None;
            self.active_request_id = self.active_request_id.wrapping_add(1);
        }

        self.screen = Screen::Chat;
        self.command_palette.clear();
        self.connect_dialog = None;

        if self.conversation.is_reverted() {
            self.discard_reverted_branch()?;
            self.context_manager = ContextManager::new();
            self.conversation.clear_context_state();
        }

        if attachments.iter().any(MessageAttachment::is_image) && !self.active_model.supports_images
        {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let mut user_message = Message::new(MessageRole::User, prompt.clone());
        user_message.attachments = attachments;
        user_message.mode = Some(self.mode);
        user_message.thinking_level = Some(self.thinking_level.clone());
        self.conversation.push(user_message.clone());
        self.store
            .append_message(self.conversation.session_id, &user_message)?;

        // Persist and display nearby instruction sources from @ references
        // below the user message, so the notification appears after the user's
        // message card rather than above it.
        if let Err(e) = self.update_loaded_instruction_sources(&instruction_sources) {
            crate::log_warn!("Failed to update instruction sources for prompt: {}", e);
        }

        self.draft_attachments.clear();

        if let Err(error) = self.capture_prompt_snapshot(user_message.id, runtime) {
            self.last_notice = Some(format!("Workspace snapshot unavailable: {error}"));
        }

        if self.conversation.messages.len() == 1 || self.conversation.title == "Untitled session" {
            self.conversation.update_title_from_prompt(&prompt);
            self.store
                .update_session_title(self.conversation.session_id, &self.conversation.title)?;
        }

        self.scroll_messages_to_bottom();

        self.schedule_context_compaction_for_session(self.conversation.session_id, runtime, None);

        // Load instruction files so "Loaded instructions from ..." appears before
        // the streaming assistant message is created by spawn_agent_loop.
        // This is done here (not in the agent loop) to avoid corrupting the
        // conversation message order (the streaming message must remain last).
        let (_, sources, new_cache) = instructions::system_prompt_and_sources_with_cache(
            &self.workspace_root,
            &self.paths.config_dir,
            &self.agent.instructions,
            &self.instruction_content_cache,
        )
        .unwrap_or_default();
        self.update_loaded_instruction_sources(&sources)?;
        self.instruction_content_cache = new_cache.clone();
        self.agent.instruction_content_cache = new_cache;

        crate::log_info!("agent: submit_prompt_now took {:?}", _t_submit.elapsed());
        self.spawn_agent_loop(runtime)
    }

    /// Spawn the agent loop in a background task.
    ///
    /// Creates a permission channel for tool call approval and spawns
    /// `run_agent_loop_with_permission_channel` to handle the full
    /// LLM + tool execution loop.
    fn spawn_agent_loop(&mut self, runtime: &Runtime) -> Result<()> {
        crate::log_info!(
            "spawn_agent_loop: session_id={}, message_count={}",
            self.conversation.session_id,
            self.conversation.messages.len()
        );

        self.pending_request = true;
        self.abort_confirmation_deadline = None;
        self.active_request_id = self.active_request_id.wrapping_add(1);
        // Cancel any existing agent loop before starting a new one
        if let Some(token) = self.request_cancel_token.take() {
            token.cancel();
        }
        let cancel_token = CancellationToken::new();
        self.request_cancel_token = Some(cancel_token.clone());
        // Clear display queue — runtime will pick up queued messages
        self.pending_prompt_queue.clear();
        let request_id = self.active_request_id;
        crate::log_info!("spawn_agent_loop: new request_id={}", request_id);

        self.last_notice = Some(match self.mode {
            SessionMode::Plan => "Planning...".to_string(),
            SessionMode::Build => "Thinking...".to_string(),
        });

        // Create a streaming message in the in-memory conversation for UI display
        let mut assistant_message = Message::streaming(MessageRole::Assistant, "");
        assistant_message.mode = Some(self.mode);
        self.conversation.push(assistant_message);

        // Create permission channel for tool call approval
        let (permission_tx, permission_rx) =
            tokio::sync::mpsc::unbounded_channel::<PendingToolApproval>();
        self.pending_permission_rx = Some(permission_rx);
        self.pending_permission_response = None;

        // Clone resources for the spawned task
        let mut agent = self.agent.clone();
        let tx = self.backend_tx.clone();
        let session_id = self.conversation.session_id;
        let model = self.active_model.clone();
        let mode = self.mode;
        let context_summary = self.conversation.context_summary.clone();
        let context_retained_from = self.conversation.context_retained_from;
        let thinking_level = self
            .conversation
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.thinking_level.clone())
            .unwrap_or_else(|| self.thinking_level.clone());

        runtime.spawn(async move {
            let mut context_manager =
                ContextManager::from_state(context_summary, context_retained_from);

            if let Err(e) = agent
                .run_agent_loop_with_permission_channel(
                    crate::agent::runtime::AgentLoopConfig {
                        session_id,
                        model,
                        context_manager: &mut context_manager,
                        mode,
                        thinking_level,
                        event_tx: tx,
                        cancel_token: Some(cancel_token),
                    },
                    request_id,
                    permission_tx,
                )
                .await
            {
                crate::log_error!("spawn_agent_loop: agent loop failed: {}", e);
            }

            crate::log_info!(
                "spawn_agent_loop: agent loop completed for session {}",
                session_id
            );
        });

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
            EnableFocusChange,
            crossterm::cursor::Hide,
        )
        .context("failed to enter alternate screen")?;

        Ok(Self)
    }

    /// Suspend the TUI: leave alternate screen, disable raw mode, show cursor.
    /// Call before spawning an external editor (GUI or terminal).
    fn suspend(&self) -> Result<()> {
        disable_raw_mode().context("failed to disable raw mode")?;
        crossterm::execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
            DisableFocusChange,
            Show,
        )
        .context("failed to leave alternate screen")?;
        Ok(())
    }

    /// Resume the TUI: re-enable raw mode, re-enter alternate screen, hide cursor.
    /// Call after the external editor exits.
    fn resume(&self) -> Result<()> {
        enable_raw_mode().context("failed to enable raw mode")?;
        crossterm::execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            EnableFocusChange,
            crossterm::cursor::Hide,
        )
        .context("failed to re-enter alternate screen")?;
        Ok(())
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
            DisableFocusChange,
            Show,
        );
    }
}
