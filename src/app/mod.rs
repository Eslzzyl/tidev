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
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use uuid::Uuid;

mod commands;
mod input;
mod render;
mod runtime;
mod ui;

pub use commands::{CommandAction, CommandPaletteState, CommandRegistry};
pub use input::Composer;
pub use input::at_mention;
pub use input::event;
pub use input::mouse_selection;
pub use render::diff_render;
pub use render::render_chat;
pub use render::render_dialog;
pub use runtime::run;
pub use runtime::state;
pub use runtime::subagent;
pub use runtime::undo;
pub use ui::balance_panel;
pub use ui::connect;
pub use ui::mcp_panel;
pub use ui::message_panel;
pub use ui::model_panel;
pub use ui::permission;
use ui::permission::SubagentStatus;
pub use ui::question;
pub use ui::session_panel;
pub use ui::settings_panel;
pub use ui::stats_panel;
pub use ui::theme_panel;

use runtime::state::*;

use crate::{
    app::at_mention::{AtMentionKind, AtMentionState, current_at_fragment},
    app::input::SnippetState,
    app::mcp_panel::McpPanelState,
    app::message_panel::MessagePanelState,
    app::model_panel::ModelPanelState,
    app::mouse_selection::{ClipboardLease, MouseSelectionState},
    app::permission::{
        PendingToolExecution, PermissionDialogState, RunningSubagentExecution, RunningToolExecution,
    },
    app::question::QuestionDialogState,
    app::session_panel::SessionPanelState,
    app::settings_panel::SettingsPanelState,
    app::theme_panel::ThemePanelState,
    app::ui::rename::RenameSessionDialogState,
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    instructions,
    llm::LlmClient,
    mcp::McpManager,
    notifications,
    prompts::{SessionMode, init_command},
    provider_setup::ConnectDialog,
    session::{AssistantTurn, BackendEvent, Conversation, Message, MessageAttachment, MessageRole},
    snapshot::SnapshotService,
    storage::SessionStore,
    theme::{ThemeManager, ThemeName},
    tooling::{FileReadTracker, TodoItem, ToolRegistry},
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
    file_read_tracker: Arc<FileReadTracker>,
    commands: CommandRegistry,
    command_palette: CommandPaletteState,
    connect_dialog: Option<ConnectDialog>,
    theme_panel: Option<ThemePanelState>,
    model_panel: Option<ModelPanelState>,
    message_panel: Option<MessagePanelState>,
    session_panel: Option<SessionPanelState>,
    settings_panel: Option<SettingsPanelState>,
    rename_dialog: Option<RenameSessionDialogState>,
    mcp_panel: Option<McpPanelState>,
    agents_panel: Option<ui::agents_panel::AgentsPanelState>,
    at_mention: AtMentionState,
    snippet_state: SnippetState,
    pending_tool_execution: Option<PendingToolExecution>,
    permission_dialog: Option<PermissionDialogState>,
    question_dialog: Option<QuestionDialogState>,
    running_tool_executions: Vec<RunningToolExecution>,
    running_subagent_executions: Vec<RunningSubagentExecution>,
    pending_assistant_turns: std::collections::HashSet<Uuid>,
    cached_sessions: std::collections::HashMap<Uuid, CachedSessionRuntime>,
    compacting_sessions: std::collections::HashSet<Uuid>,
    leader_key_pending: bool,
    composer: Composer,
    draft_attachments: Vec<MessageAttachment>,
    pending_prompt_queue: std::collections::VecDeque<QueuedPrompt>,
    pending_request: bool,
    active_request_id: u64,
    abort_confirmation_deadline: Option<Instant>,
    last_notice: Option<String>,
    toast: Option<(String, Instant)>,
    mouse_selection: MouseSelectionState,
    retrying_hint: Option<(u32, u32, String, Option<u32>)>,
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
    input_area: Cell<Option<Rect>>,
    /// Scroll offset for the input box when content exceeds visible area.
    input_scroll_offset: usize,
    /// Whether we're currently dragging in the input area (for text selection).
    input_dragging: bool,
    selection_clipboard_lease: Option<ClipboardLease>,
    last_render_time: Instant,
    render_throttled: bool,
    backend_tx: UnboundedSender<BackendEvent>,
    backend_rx: UnboundedReceiver<BackendEvent>,
    spinner_start: Instant,
    context_usage: Option<state::ContextUsage>,
    snapshot: SnapshotService,
    cleanup_cancel: Arc<std::sync::atomic::AtomicBool>,
    loaded_instruction_sources: Vec<String>,
    /// Cached instruction file contents to avoid redundant I/O
    instruction_content_cache: std::collections::HashMap<String, String>,
    expanded_tool_results: std::collections::HashSet<Uuid>,
    tool_result_card_bounds: Vec<(Uuid, Rect)>,
    pub(crate) selectable_regions: Vec<Rect>,
    message_scroll_target: Option<Uuid>,
    todos: Vec<TodoItem>,
    stats_panel: Option<ui::stats_panel::StatsPanelState>,
    balance_panel: Arc<Mutex<Option<ui::balance_panel::BalancePanelState>>>,
    notifications: notifications::NotificationManager,
    /// DeepSeek thinking level for the current model
    thinking_level: crate::config::reasoning::ThinkingLevelType,
}

pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    let mut app = App::new()?;
    app.run(&runtime)
}

impl App {
    /// Build prompt attachments for @ references with truncation like opencode.
    /// Uses build_at_reference_attachment for @ references to apply read tool truncation.
    fn build_prompt_attachments(&self, prompt: &str) -> Result<Vec<MessageAttachment>> {
        let mut attachments = Vec::new();
        let mut seen_paths = std::collections::BTreeSet::new();

        for path in self.inline_file_references(prompt) {
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            // Use build_at_reference_attachment for @ references with truncation
            match self.build_at_reference_attachment(&path)? {
                Some(attachment) => attachments.push(attachment),
                None => continue,
            }
        }

        // Add draft attachments (pasted files) without truncation
        attachments.extend(self.draft_attachments.iter().cloned());
        Ok(attachments)
    }

    /// Build attachment for @ reference with truncation like opencode's read tool.
    fn build_at_reference_attachment(&self, path: &str) -> Result<Option<MessageAttachment>> {
        use crate::tooling::builtin::file::read_file_for_at_reference;

        let absolute = self.resolve_workspace_path(path);
        let metadata = match std::fs::metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(_error) => return Ok(None),
        };

        if metadata.is_dir() {
            let tree = build_directory_tree(&absolute, 2, 80)?;
            return Ok(Some(MessageAttachment::DirectoryReference {
                path: path.trim_end_matches(['/', '\\']).to_string(),
                tree: Arc::new(tree),
            }));
        }

        if let Some(mime) = image_mime_from_path(&absolute) {
            let bytes = std::fs::read(&absolute)?;
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

        // For text files, read with truncation like opencode's read tool
        match read_file_for_at_reference(&self.workspace_root, path) {
            Ok((tool_output, truncated)) => {
                // Also read full content for display purposes
                let content = std::fs::read_to_string(&absolute).unwrap_or_else(|_| String::new());
                Ok(Some(MessageAttachment::FileReference {
                    path: path.to_string(),
                    content: Arc::new(content),
                    tool_output: Some(Arc::new(tool_output)),
                    truncated,
                }))
            }
            Err(_error) => {
                // Fall back to full content if read fails
                let content = std::fs::read_to_string(&absolute).unwrap_or_else(|_| String::new());
                Ok(Some(MessageAttachment::FileReference {
                    path: path.to_string(),
                    content: Arc::new(content),
                    tool_output: None,
                    truncated: false,
                }))
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

    fn start_assistant_turn(&mut self, runtime: &Runtime) -> Result<()> {
        crate::log_info!(
            "start_assistant_turn: session_id={}, message_count={}",
            self.conversation.session_id,
            self.conversation.messages.len()
        );

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

        self.update_loaded_instruction_sources(&instruction_sources)?;

        let mut assistant_message = Message::streaming(MessageRole::Assistant, "");
        assistant_message.mode = Some(self.mode);
        self.conversation.push(assistant_message);

        let messages = self
            .context_manager
            .build_request_messages(&self.conversation, self.mode);
        let tools = self.tools.all_definitions();
        let tx = self.backend_tx.clone();
        let session_id = self.conversation.session_id;

        // 获取 thinking level：从最后一条用户消息获取，如果没有则 fallback 到模型默认
        let thinking_level = self
            .conversation
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.thinking_level.clone())
            .unwrap_or_else(|| self.thinking_level.clone());

        runtime.spawn(async move {
            llm.stream_chat(
                session_id,
                request_id,
                model,
                messages,
                tools,
                tx,
                thinking_level,
            )
            .await;
        });

        Ok(())
    }

    fn refresh_tools(&mut self) {
        let mcp = self.tools.mcp_manager();
        let file_read_tracker = self.tools.file_read_tracker();
        self.tools = ToolRegistry::new(
            self.workspace_root.clone(),
            self.paths.config_dir.clone(),
            self.config.skills.clone(),
            mcp,
            self.config.permissions.clone(),
            file_read_tracker,
            self.config.rtk.enabled,
        );
    }

    fn compose_system_prompt(&mut self) -> (String, Vec<String>) {
        let base_prompt = self.active_model.system_prompt.trim();
        let mode_reminder = self.mode.reminder();
        let (instruction_prompt, sources, new_cache) = instructions::system_prompt_and_sources_with_cache(
            &self.workspace_root,
            &self.paths.config_dir,
            &self.config.instructions,
            &self.instruction_content_cache,
        )
        .unwrap_or_default();

        // Update the cache with newly loaded contents
        self.instruction_content_cache = new_cache;

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

        // Add system environment information
        let system_info = crate::system_info::SystemInfo::detect();
        let working_dir = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let is_git = crate::system_info::is_git_repo(&self.workspace_root);
        prompt.push_str("\n\nHere is some useful information about the environment:\n<env>\n  ");
        prompt.push_str(&format!("Working directory: {}\n  ", working_dir));
        prompt.push_str(&format!(
            "Workspace root folder: {}\n  ",
            self.workspace_root.display()
        ));
        prompt.push_str(&format!(
            "Is directory a git repo: {}\n  ",
            if is_git { "yes" } else { "no" }
        ));
        prompt.push_str(&system_info.format_env());
        prompt.push_str("\n</env>");

        (prompt, sources)
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
        };
        if event_type != "Delta"
            && event_type != "ReasoningDelta"
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

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.content.push_str(&content);
                    let message_id = message.id;
                    self.message_layout_index.borrow_mut().valid = false;
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

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.reasoning.push_str(&content);
                    let message_id = message.id;
                    self.message_layout_index.borrow_mut().valid = false;
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
                self.running_tool_executions.clear();
                self.cancel_running_subagents();
                self.abort_confirmation_deadline = None;
                self.retrying_hint = None;

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
                    self.drain_queued_prompts(runtime);
                    return Ok(());
                }

                let message = Message::new(MessageRole::Error, format!("Request failed: {error}"));
                self.conversation.push(message.clone());
                self.store
                    .append_message(self.conversation.session_id, &message)?;
                self.last_notice = Some(error);
                self.drain_queued_prompts(runtime);
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
                    self.record_tool_result(running.tool_call, result)?;
                    self.try_start_parallel_execution(runtime)?;
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

                if let Some(execution) = self
                    .running_subagent_executions
                    .iter_mut()
                    .find(|execution| execution.child_session_id == child_session_id)
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
                    let display_result = result.preview_for_storage(Some(tool_call.name.as_str()));
                    let message = Message::tool_result(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        display_result,
                    );
                    self.store.append_tool_event(
                        parent_session_id,
                        message.id,
                        &tool_call.name,
                        &tool_call.arguments,
                        &result.output,
                    )?;
                    self.store.append_message(parent_session_id, &message)?;
                    self.pending_assistant_turns.insert(parent_session_id);
                    crate::log_info!(
                        "SubagentCompleted: marked parent_session_id={} as pending assistant turn",
                        parent_session_id
                    );
                }

                self.notifications.notify("Subagent finished");
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
        }

        Ok(())
    }

    fn finish_assistant_turn(&mut self, turn: AssistantTurn, runtime: &Runtime) -> Result<()> {
        crate::log_info!(
            "finish_assistant_turn: tool_calls_count={}, finish_reason={:?}",
            turn.tool_calls.len(),
            turn.finish_reason
        );
        let turn_mode = self
            .conversation
            .messages
            .iter()
            .rev()
            .find_map(|message| {
                if matches!(message.role, MessageRole::Assistant) && message.streaming {
                    message.mode
                } else {
                    None
                }
            })
            .or_else(|| {
                self.conversation
                    .messages
                    .iter()
                    .rev()
                    .find(|message| matches!(message.role, MessageRole::User))
                    .and_then(|message| message.mode)
            })
            .unwrap_or(self.mode);
        let mut persisted_message = None;

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.content = turn.content.clone();
            message.reasoning = turn.reasoning.clone();
            message.tool_calls = turn.tool_calls.clone();
            message.streaming = false;
            if message.mode.is_none() {
                message.mode = Some(turn_mode);
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

            self.begin_tool_execution(turn.tool_calls, turn_mode, runtime)?;
            return Ok(());
        }

        // End of round: apply pending mode switch if any
        if let Some(new_mode) = self.pending_mode.take() {
            self.mode = new_mode;
            self.refresh_tools();
            self.last_notice = Some(format!("Mode switched to {}", new_mode.as_str()));
        }

        self.pending_request = false;
        self.abort_confirmation_deadline = None;

        if let Err(error) = self.finalize_snapshot_for_last_user_message_sync(runtime) {
            crate::log_warn!("failed to finalize snapshot: {}", error);
        }

        self.last_notice = Some(match turn.finish_reason.as_deref() {
            Some(reason) if reason != "stop" => format!("Response finished ({reason})"),
            _ => "Response complete".to_string(),
        });
        self.schedule_context_compaction_for_session(self.conversation.session_id, runtime, None);
        self.drain_queued_prompts(runtime);

        self.notifications.notify("Response complete");

        Ok(())
    }

    fn queue_prompt(&mut self, prompt: String, attachments: Vec<MessageAttachment>) {
        self.pending_prompt_queue
            .push_back(QueuedPrompt::new(prompt, attachments));
    }

    fn drain_queued_prompts(&mut self, runtime: &Runtime) {
        while !self.pending_request {
            let Some(queued_prompt) = self.pending_prompt_queue.pop_front() else {
                break;
            };

            if let Err(error) =
                self.submit_prompt_now(queued_prompt.prompt, queued_prompt.attachments, runtime)
            {
                self.last_notice = Some(error.to_string());
                break;
            }

            if self.pending_request {
                break;
            }
        }
    }

    fn submit_prompt_now(
        &mut self,
        prompt: String,
        attachments: Vec<MessageAttachment>,
        runtime: &Runtime,
    ) -> Result<()> {
        let prompt = prompt.trim().to_string();

        if prompt.is_empty() && attachments.is_empty() {
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
                self.conversation.clear_context_state();
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
            self.running_tool_executions.clear();
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

        self.start_assistant_turn(runtime)
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
