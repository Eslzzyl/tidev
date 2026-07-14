//! MessageList component — the virtualised chat message list.
//!
//! Owns the rendering pipeline, layout index, render cache, scroll state,
//! tool call interaction state, subagent tracking, and streaming buffer.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::Frame;
use anyhow::Result;
use lru::LruCache;
use crate::chat_context::ChatContext;
use uuid::Uuid;
use tidev_types::message::BackendEvent;
use tidev_types::prompts::SessionMode;

use tidev_types::tools::canonical_tool_name;
use crate::action::{Action, ChatAction, SessionAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::components::chat::layout_index::MessageLayoutIndex;
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, SelectableRegionRange,
};
use crate::components::chat::render as render_mod;
use crate::components::chat::streaming::StreamingBuffer;

// ---------------------------------------------------------------------------
// ScrollbarDrag
// ---------------------------------------------------------------------------

/// State for an in-progress scrollbar drag interaction.
#[derive(Clone, Debug)]
struct ScrollbarDrag {
    /// Scroll offset when the drag started.
    start_scroll: usize,
    /// Mouse Y position (screen coordinate) when the drag started.
    start_mouse_y: u16,
    /// Maximum scroll value at drag start.
    max_scroll: usize,
}

pub(crate) mod layout_index;
pub(crate) mod render_cache;
pub(crate) mod render;
pub(crate) mod streaming;
pub(crate) mod tool;

// ---------------------------------------------------------------------------
// MessageList
// ---------------------------------------------------------------------------

pub(crate) struct MessageList {
    /// The current chat context (messages and session info).
    pub chat_context: Option<ChatContext>,

    // ── Rendering infrastructure ──
    layout_index: MessageLayoutIndex,
    render_cache: LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    render_tick: u64,

    // ── Scroll state ──
    pub scroll_offset: usize,
    pub follow_tail: bool,
    /// Pending scroll target set by ChatAction::ScrollTo.
    pub scroll_target: Option<Uuid>,

    // ── Interaction state ──
    expanded_tool_results: HashSet<Uuid>,
    expanded_tool_outputs: HashMap<Uuid, String>,
    selectable_regions: Vec<SelectableRegionRange>,
    hovered_card: Option<Uuid>,
    card_bounds: Vec<(Uuid, usize, usize)>,

    /// The area into which messages were rendered (for mouse selection clamping).
    pub content_area: Option<Rect>,
    /// The scroll offset applied to the Paragraph widget (may differ from
    /// scroll_offset when headers or padding shift visible content).
    render_scroll: usize,

    /// Scrollbar drag state (None = not dragging).
    scrollbar_drag: Option<ScrollbarDrag>,

    // ── Spinner animation ──
    spinner_start: Instant,

    // ── Streaming state ──
    streaming_buffer: StreamingBuffer,
    current_streaming_message_id: Option<Uuid>,

    // ── Subagent tracking ──
    running_subagents: Vec<render_mod::RunningSubagentInfo>,
    hovered_inline_subagent: Option<usize>,
    inline_subagent_card_bounds: Vec<(usize, Rect)>,

    // ── Bash tool tracking (for ShellOutput streaming) ──
    bash_tool_call_id: Option<String>,

    // ── Dirty tracking ──
    pub(crate) dirty: bool,
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            chat_context: None,
            layout_index: MessageLayoutIndex::new(),
            render_cache: LruCache::new(std::num::NonZeroUsize::new(1200).unwrap()),
            render_tick: 0,
            scroll_offset: 0,
            follow_tail: true,
            scroll_target: None,
            expanded_tool_results: HashSet::new(),
            expanded_tool_outputs: HashMap::new(),
            selectable_regions: Vec::new(),
            hovered_card: None,
            card_bounds: Vec::new(),
            content_area: None,
            render_scroll: 0,
            scrollbar_drag: None,
            spinner_start: Instant::now(),
            streaming_buffer: StreamingBuffer::new(),
            current_streaming_message_id: None,
            running_subagents: Vec::new(),
            hovered_inline_subagent: None,
            inline_subagent_card_bounds: Vec::new(),
            bash_tool_call_id: None,
            dirty: true,
        }
    }

    /// Set the chat context and mark dirty.
    pub fn set_chat_context(&mut self, ctx: ChatContext) {
        self.chat_context = Some(ctx);
        self.dirty = true;
        self.scroll_offset = 0;
        self.follow_tail = true;
        self.render_cache.clear();
        self.layout_index = MessageLayoutIndex::new();
        self.streaming_buffer = StreamingBuffer::new();
        self.current_streaming_message_id = None;
        self.selectable_regions.clear();
        self.running_subagents.clear();
        self.hovered_inline_subagent = None;
        self.inline_subagent_card_bounds.clear();
        self.bash_tool_call_id = None;
    }

    /// Invalidate the layout index (triggers full rebuild on next draw).
    pub fn invalidate_layout(&mut self) {
        self.layout_index.invalidate_all();
        self.dirty = true;
    }

    /// Update token fields on the last streaming assistant message.
    pub fn set_last_message_tokens(
        &mut self,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        total_tokens: Option<u32>,
        cache_read_tokens: Option<u32>,
        cache_write_tokens: Option<u32>,
        tokens_per_second: Option<f32>,
        model_id: Option<String>,
        completed_at: Option<DateTime<Utc>>,
        mode: Option<SessionMode>,
    ) {
        let Some(ref mut chat_context) = self.chat_context else { return };
        if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| {
            m.role == tidev_types::message::MessageRole::Assistant
        }) {
            msg.input_tokens = input_tokens;
            msg.output_tokens = output_tokens;
            msg.total_tokens = total_tokens;
            msg.cache_read_tokens = cache_read_tokens;
            msg.cache_write_tokens = cache_write_tokens;
            msg.tokens_per_second = tokens_per_second;
            msg.model_id = model_id;
            if let Some(completed) = completed_at {
                msg.completed_at = Some(completed);
            }
            if let Some(mode) = mode {
                msg.mode = Some(mode);
            }
        }
    }

    /// Mark the last streaming message as an error (on BackendEvent::Failed).
    pub fn mark_streaming_as_error(&mut self, error: &str) {
        let Some(ref mut chat_context) = self.chat_context else { return };
        if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| m.streaming) {
            msg.role = tidev_types::message::MessageRole::Error;
            msg.streaming = false;
            msg.content = format!("Request failed: {error}");
        }
        self.streaming_buffer.is_streaming = false;
        self.current_streaming_message_id = None;
        self.dirty = true;
    }

    /// Finalise the streaming message (if any) and append an error notice.
    /// Called on user abort (double Esc).
    pub fn append_interrupted_message(&mut self) {
        let Some(ref mut chat_context) = self.chat_context else { return };

        // Finalise the streaming message, preserving content and reasoning.
        if let Some(idx) = self.streaming_buffer.finalise_message(&mut chat_context.messages) {
            chat_context.messages[idx].completed_at = Some(Utc::now());
            self.layout_index.mark_dirty(chat_context.messages[idx].id);
        }

        // Ensure streaming state is fully reset.
        self.current_streaming_message_id = None;

        // Append an error message to indicate the interruption.
        let mut err_msg = tidev_types::message::Message::new(
            tidev_types::message::MessageRole::Error,
            "Request interrupted by user",
        );
        err_msg.completed_at = Some(Utc::now());
        chat_context.messages.push(err_msg);

        self.render_cache.clear();
        self.dirty = true;
    }

    /// Handle a backend event for streaming or tool results.
    pub fn handle_backend_event(&mut self, event: &BackendEvent) {
        let Some(ref mut chat_context) = self.chat_context else { return };

        // Ignore events from other sessions (e.g. after switching sessions
        // while the old session's agent loop is still running).
        if event.session_id() != chat_context.session_id {
            // Not our session — check if it belongs to a running subagent.
            let child_sid = event.session_id();
            if let Some(exec) = self.running_subagents.iter_mut().find(|e| e.child_session_id == Some(child_sid)) {
                if let Some(text) = infer_subagent_status(event) {
                    if exec.status_text != text {
                        exec.status_text = text;
                        self.dirty = true;
                    }
                }
            }
            return;
        }

        match event {
            BackendEvent::TurnStarting { .. } => {
                let message_id = self.streaming_buffer.begin_streaming(&mut chat_context.messages);
                self.current_streaming_message_id = Some(message_id);
                self.follow_tail = true;
                self.dirty = true;
            }
            BackendEvent::Delta { content, .. } => {
                if self.streaming_buffer.is_streaming {
                    self.streaming_buffer.push_delta(content);
                    self.streaming_buffer.sync_pending(&mut chat_context.messages);
                    if let Some(msg_id) = self.streaming_buffer.current_message_id {
                        self.layout_index.mark_dirty(msg_id);
                    }
                } else if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| {
                    m.streaming && m.role == tidev_types::message::MessageRole::System
                }) {
                    msg.content.push_str(content);
                    self.layout_index.mark_dirty(msg.id);
                }
                self.dirty = true;
            }
            BackendEvent::ReasoningDelta { content, .. } => {
                self.streaming_buffer.push_reasoning_delta(content);
                self.streaming_buffer.sync_pending(&mut chat_context.messages);
                if let Some(msg_id) = self.streaming_buffer.current_message_id {
                    self.layout_index.mark_dirty(msg_id);
                }
                self.dirty = true;
            }
            BackendEvent::StreamEnd { .. } => {
                self.streaming_buffer.finalise_message(&mut chat_context.messages);
                self.current_streaming_message_id = None;
                self.dirty = true;
            }
            BackendEvent::ToolCallUpdated { tool_call, request_id, .. } => {
                if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| m.role == tidev_types::message::MessageRole::Assistant) {
                    msg.upsert_tool_call(tool_call.clone());
                    self.layout_index.mark_dirty(msg.id);
                    self.dirty = true;
                }
                if tool_call.name == "bash" {
                    self.bash_tool_call_id = Some(tool_call.id.clone());
                }
                if tool_call.name == "task" {
                    let desc = extract_task_description(&tool_call.arguments);
                    let sub_type = extract_subagent_type(&tool_call.arguments);
                    let already_tracking = self.running_subagents.iter().any(|s| s.tool_call_id == tool_call.id);
                    if already_tracking {
                        if let Some(exec) = self.running_subagents.iter_mut().find(|s| s.tool_call_id == tool_call.id) {
                            if exec.subagent_type.is_empty() && !sub_type.is_empty() {
                                exec.subagent_type = sub_type;
                            }
                            if exec.description.is_empty() && !desc.is_empty() {
                                exec.description = desc;
                                self.dirty = true;
                            }
                        }
                    } else {
                        self.running_subagents.push(render_mod::RunningSubagentInfo {
                            request_id: *request_id,
                            tool_call_id: tool_call.id.clone(),
                            description: desc,
                            subagent_type: sub_type,
                            status_text: "Thinking".to_string(),
                            child_session_id: None,
                        });
                    }
                }
            }
            BackendEvent::ToolCompleted { tool_call, result, .. } => {
                if tool_call.name == "bash" {
                    // Bash output was streamed via ShellOutput — find and finalize
                    // the existing streaming Tool message instead of creating a new one.
                    if let Some(idx) = chat_context.messages.iter().rposition(|m| {
                        m.role == tidev_types::message::MessageRole::Tool
                            && m.tool_call_id.as_deref() == Some(&tool_call.id)
                            && m.streaming
                    }) {
                        chat_context.messages[idx].content = result.output.clone();
                        chat_context.messages[idx].streaming = false;
                        self.dirty = true;
                    }
                    self.bash_tool_call_id = None;
                } else {
                    let tool_msg = tidev_types::message::Message::tool_result(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        result.clone(),
                    );
                    chat_context.messages.push(tool_msg);
                    self.dirty = true;
                }
            }
            BackendEvent::ShellOutput { content, finished, .. } => {
                let bash_id = match &self.bash_tool_call_id {
                    Some(id) => id.clone(),
                    None => return,
                };
                let existing = chat_context.messages.iter_mut().rev().find(|m| {
                    m.role == tidev_types::message::MessageRole::Tool
                        && m.tool_call_id.as_deref() == Some(&bash_id)
                });
                if let Some(msg) = existing {
                    msg.content = content.clone();
                    if *finished {
                        msg.streaming = false;
                    }
                } else {
                    let mut msg = tidev_types::message::Message::streaming(
                        tidev_types::message::MessageRole::Tool,
                        content.clone(),
                    );
                    msg.tool_call_id = Some(bash_id);
                    msg.tool_name = Some("bash".to_string());
                    msg.streaming = !*finished;
                    chat_context.messages.push(msg);
                }
                self.dirty = true;
            }
            BackendEvent::Finished { .. } => {
                self.dirty = true;
            }
            BackendEvent::SubagentStatus {
                request_id,
                status_text,
                assistant_message,
                child_session_id,
                ..
            } => {
                // Match by request_id (backend now sends parent_request_id).
                let idx = self.running_subagents.iter().position(|e| e.request_id == *request_id);
                if let Some(idx) = idx {
                    let exec = &mut self.running_subagents[idx];
                    exec.status_text = status_text.clone();
                    exec.child_session_id = Some(*child_session_id);
                } else if let Some(exec) = self.running_subagents.last_mut() {
                    exec.status_text = status_text.clone();
                    exec.child_session_id = Some(*child_session_id);
                }
                if let Some(msg) = assistant_message {
                    let existing = chat_context.messages.iter_mut().find(|m| m.id == msg.id);
                    if let Some(existing) = existing {
                        if !msg.content.is_empty() {
                            existing.content = msg.content.clone();
                        }
                        if !msg.reasoning.is_empty() {
                            existing.reasoning = msg.reasoning.clone();
                        }
                        if !msg.tool_calls.is_empty() {
                            existing.tool_calls.clone_from(&msg.tool_calls);
                        }
                        self.layout_index.mark_dirty(msg.id);
                    } else {
                        chat_context.messages.push(msg.clone());
                    }
                }
                self.dirty = true;
            }
            BackendEvent::SubagentCompleted { tool_call, result, .. } => {
                self.running_subagents.retain(|s| s.tool_call_id != tool_call.id);
                let tool_msg = tidev_types::message::Message::tool_result(
                    tool_call.id.clone(),
                    tool_call.name.clone(),
                    result.clone(),
                );
                chat_context.messages.push(tool_msg);
                self.dirty = true;
            }
            BackendEvent::SidebarSnapshotReady { message_id, file_diffs_json, .. } => {
                if let Some(msg) = chat_context.messages.iter_mut().find(|m| m.id == *message_id) {
                    msg.file_diffs = Some(file_diffs_json.clone());
                    self.layout_index.mark_dirty(*message_id);
                    self.dirty = true;
                }
            }
            BackendEvent::ContextCompacted { compacted, summary, model_id, completed_at, .. } => {
                if *compacted {
                    if let Some(summary) = summary {
                        // The summary was already streamed via Delta events into
                        // the last streaming message.  If manual compaction found
                        // a streaming System message, finalize it.  Otherwise
                        // create a compaction message.
                        let found = chat_context.messages.iter_mut().rev().find(|m| {
                            m.streaming && m.role == tidev_types::message::MessageRole::System
                        });
                        if let Some(msg) = found {
                            msg.streaming = false;
                            msg.model_id = model_id.clone();
                            msg.completed_at = *completed_at;
                        } else {
                            let mut compaction_msg = tidev_types::message::Message::new(
                                tidev_types::message::MessageRole::System,
                                format!("Compaction\n\n{}", summary),
                            );
                            compaction_msg.model_id = model_id.clone();
                            compaction_msg.completed_at = *completed_at;
                            chat_context.messages.push(compaction_msg);
                        }
                    }
                    self.layout_index.invalidate_all();
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    /// Handle mouse click: find which selectable region was clicked
    /// and toggle tool result expansion for the associated block.
    /// Returns an Action if a subsession switch is requested.
    pub fn handle_mouse_click(&mut self, x: u16, y: u16) -> Option<Action> {
        let scroll = self.scroll_offset;
        let y_u = y as usize;
        let absolute_line = scroll + y_u;

        // Check inline subagent card bounds first (Rect-based hit detection).
        if let Some(hit) = self
            .inline_subagent_card_bounds
            .iter()
            .find(|(_, rect)| rect.contains((x, y).into()))
        {
            let exec_idx = hit.0;
            if let Some(sa) = self.running_subagents.get(exec_idx) {
                if let Some(csid) = sa.child_session_id {
                    return Some(Action::Session(SessionAction::Select(csid)));
                }
            }
        }

        // Use the layout index to find which block was clicked.
        let block_idx = self
            .layout_index
            .blocks
            .partition_point(|b| b.start_line + b.line_count <= absolute_line);
        if block_idx < self.layout_index.blocks.len() {
            let block = &self.layout_index.blocks[block_idx];
            if block.message_count > 0 {
                if self.expanded_tool_results.contains(&block.message_id) {
                    self.expanded_tool_results.remove(&block.message_id);
                } else {
                    self.expanded_tool_results.insert(block.message_id);
                }
                self.layout_index.mark_dirty(block.message_id);
                self.dirty = true;
            }
        }

        None
    }

    /// Calculate the scroll offset that brings a specific message into view.
    fn resolve_scroll_to_message(&self, messages: &[tidev_types::message::Message], target_id: uuid::Uuid) -> Option<usize> {
        let mut offset = 0usize;
        let mut i = 0;
        while i < messages.len() {
            if messages[i].id == target_id {
                return Some(offset);
            }
            let count = if messages[i].role == tidev_types::message::MessageRole::Assistant {
                let mut c = 1;
                while i + c < messages.len()
                    && messages[i + c].role == tidev_types::message::MessageRole::Tool
                {
                    c += 1;
                }
                c
            } else {
                1
            };
            let line_count = match messages[i].role {
                tidev_types::message::MessageRole::Tool => 0,
                _ => {
                    let content_lines = messages[i].content.lines().count().max(1);
                    let tool_lines = messages[i].tool_calls.len().saturating_mul(3);
                    2 + content_lines + tool_lines
                }
            };
            offset += line_count;
            i += count;
        }
        None
    }
}

impl Component for MessageList {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                Some(Action::Chat(ChatAction::ScrollDelta(-3)))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Some(Action::Chat(ChatAction::ScrollDelta(3)))
            }
            KeyCode::PageUp => {
                let page = self.content_area.map(|r| (r.height as isize).max(1)).unwrap_or(10);
                Some(Action::Chat(ChatAction::ScrollDelta(-page)))
            }
            KeyCode::PageDown => {
                let page = self.content_area.map(|r| (r.height as isize).max(1)).unwrap_or(10);
                Some(Action::Chat(ChatAction::ScrollDelta(page)))
            }
            KeyCode::Home => {
                self.scroll_offset = 0;
                self.follow_tail = false;
                self.dirty = true;
                None
            }
            KeyCode::End => {
                let total = self.layout_index.total_lines;
                let max_scroll = total.saturating_sub(20);
                self.scroll_offset = max_scroll;
                self.follow_tail = true;
                self.dirty = true;
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Chat(ChatAction::ScrollDelta(delta)) => {
                if self.chat_context.is_some() {
                    let total = self.layout_index.total_lines;
                    let viewport = self.content_area.map(|r| r.height as usize).unwrap_or(20).max(1);
                    let max_scroll = total.saturating_sub(viewport);
                    let new_scroll = (self.scroll_offset as isize + delta).max(0) as usize;
                    self.scroll_offset = new_scroll.min(max_scroll);
                    self.follow_tail = self.scroll_offset >= max_scroll;
                    self.dirty = true;
                }
                vec![]
            }
            Action::Chat(ChatAction::ScrollTo(message_id)) => {
                self.scroll_target = Some(*message_id);
                self.follow_tail = false;
                self.dirty = true;
                vec![]
            }
            Action::Chat(ChatAction::ToggleToolResult(message_id)) => {
                if self.expanded_tool_results.contains(message_id) {
                    self.expanded_tool_results.remove(message_id);
                } else {
                    self.expanded_tool_results.insert(*message_id);
                }
                self.layout_index.mark_dirty(*message_id);
                self.dirty = true;
                vec![]
            }
            Action::Chat(ChatAction::StreamDelta { message_id, delta: _ }) => {
                self.streaming_buffer.is_streaming = true;
                self.current_streaming_message_id = Some(*message_id);
                self.dirty = true;
                vec![]
            }
            Action::Chat(ChatAction::StreamEnd(message_id)) => {
                self.streaming_buffer.is_streaming = false;
                self.current_streaming_message_id = None;
                self.dirty = true;
                vec![]
            }
            Action::Chat(ChatAction::CancelGeneration) => {
                self.streaming_buffer.is_streaming = false;
                self.current_streaming_message_id = None;
                self.dirty = true;
                vec![]
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let Some(ref chat_context) = self.chat_context else {
            return;
        };

        self.render_tick += 1;
        self.selectable_regions.clear();

        // Resolve scroll target if set
        if let Some(target_id) = self.scroll_target.take() {
            if let Some(scroll) = self.resolve_scroll_to_message(&chat_context.messages, target_id) {
                self.scroll_offset = scroll;
                self.follow_tail = false;
            }
        }

        self.card_bounds.clear();
        self.content_area = None;
        self.render_scroll = 0;
        self.inline_subagent_card_bounds.clear();
        let mut render_content_area = Rect::default();
        let mut render_scroll = 0;
        let mut inline_running_card_ranges = Vec::new();
        render_mod::render_messages(
            frame,
            rect,
            &mut self.layout_index,
            &mut self.render_cache,
            chat_context,
            ctx.palette,
            &mut self.scroll_offset,
            &mut self.follow_tail,
            &mut self.expanded_tool_results,
            &mut self.expanded_tool_outputs,
            self.streaming_buffer.is_streaming,
            self.current_streaming_message_id,
            &mut self.render_tick,
            &self.running_subagents,
            self.spinner_start,
            self.hovered_card,
            self.hovered_inline_subagent,
            &mut self.card_bounds,
            &mut self.selectable_regions,
            &mut inline_running_card_ranges,
            &mut render_content_area,
            &mut render_scroll,
        );
        self.content_area = Some(render_content_area);
        self.render_scroll = render_scroll;

        // Convert inline running card ranges to screen rects for mouse interaction
        let viewport = render_content_area.height as usize;
        for card_range in &inline_running_card_ranges {
            let abs_start = card_range.start_line;
            let abs_end = card_range.end_line;

            let screen_start = abs_start.saturating_sub(render_scroll);
            let screen_end = abs_end.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= viewport {
                continue;
            }

            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(viewport)) as u16;

            if visible_start < visible_end {
                let card_rect = Rect {
                    x: render_content_area.x,
                    y: render_content_area.y.saturating_add(visible_start),
                    width: render_content_area.width,
                    height: visible_end.saturating_sub(visible_start),
                };
                self.inline_subagent_card_bounds.push((card_range.execution_index, card_rect));
            }
        }

        self.dirty = false;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn mark_clean(&mut self) {
        self.dirty = false;
    }
}

impl MessageList {
    /// Return the selectable regions as screen-space Rects for mouse selection
    /// clamping.  Converts from absolute line numbers to screen coordinates
    /// (mirrors old TUI's chat_render conversion at lines 620-636).
    pub fn selectable_region_rects(&self) -> Vec<ratatui::layout::Rect> {
        use crate::components::chat::render_cache::SelectableRegionRange;
        let render_scroll = self.render_scroll;
        let Some(area) = self.content_area else {
            return Vec::new();
        };
        self.selectable_regions
            .iter()
            .filter_map(|r: &SelectableRegionRange| {
                let visible_start = r.start_line.saturating_sub(render_scroll);
                let visible_end = r.end_line.saturating_sub(render_scroll);
                if visible_start >= visible_end {
                    return None;
                }
                let y = area.y.saturating_add(visible_start as u16);
                let height = (visible_end - visible_start) as u16;
                let min_x = area.x.saturating_add(r.min_x);
                let max_x = r
                    .max_x
                    .map(|mx| area.x.saturating_add(mx))
                    .unwrap_or(area.x.saturating_add(area.width));
                let width = max_x.saturating_sub(min_x);
                if width == 0 {
                    return None;
                }
                Some(ratatui::layout::Rect::new(min_x, y, width, height))
            })
            .collect()
    }

    /// Return the scrollbar area (rightmost column of content_area), if visible.
    pub fn scrollbar_area(&self) -> Option<Rect> {
        let area = self.content_area?;
        if area.width < 3 {
            return None;
        }
        Some(Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y,
            width: 1,
            height: area.height,
        })
    }

    /// Maximum scroll offset.
    pub fn max_scroll(&self) -> usize {
        self.layout_index.total_lines.saturating_sub(self.content_area.map_or(0, |a| a.height as usize))
    }

    /// Start a scrollbar drag: call on mouse down on scrollbar.
    pub fn start_scrollbar_drag(&mut self, mouse_y: u16) {
        let max_scroll = self.max_scroll();
        if max_scroll == 0 {
            return;
        }
        self.scrollbar_drag = Some(ScrollbarDrag {
            start_scroll: self.scroll_offset,
            start_mouse_y: mouse_y,
            max_scroll,
        });
    }

    /// Continue a scrollbar drag: call on mouse drag.
    pub fn continue_scrollbar_drag(&mut self, mouse_y: u16) {
        let Some(ref drag) = self.scrollbar_drag else { return };
        let track_height = self.content_area.map_or(1, |a| a.height as usize);
        if track_height == 0 {
            return;
        }
        let delta_y = mouse_y as isize - drag.start_mouse_y as isize;
        let scroll_delta = (delta_y as f32 / track_height as f32) * drag.max_scroll as f32;
        let new_scroll =
            (drag.start_scroll as isize + scroll_delta.round() as isize)
                .max(0)
                .min(drag.max_scroll as isize) as usize;
        self.scroll_offset = new_scroll;
        self.follow_tail = self.scroll_offset >= drag.max_scroll;
    }

    /// End a scrollbar drag.
    pub fn end_scrollbar_drag(&mut self) {
        self.scrollbar_drag = None;
    }

    /// Whether a scrollbar drag is in progress.
    pub fn is_scrollbar_dragging(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    /// Whether the message list is currently receiving streaming content.
    pub fn is_streaming(&self) -> bool {
        self.streaming_buffer.is_streaming
    }

    /// Number of running subagents.
    pub fn running_subagents_count(&self) -> usize {
        self.running_subagents.len()
    }

    /// Description of the first running subagent, if any.
    pub fn first_subagent_description(&self) -> Option<&str> {
        self.running_subagents.first().map(|s| s.description.as_str())
    }

    /// Subagent type of the first running subagent, if any.
    pub fn first_subagent_type(&self) -> Option<&str> {
        self.running_subagents.first().map(|s| s.subagent_type.as_str())
    }

    /// Update hovered card based on mouse position.
    /// `x`, `y` are screen coordinates. Call on every mouse move event.
    pub fn set_hovered_card(&mut self, x: u16, y: u16) {
        let scroll = self.scroll_offset;
        let absolute_line = scroll + y as usize;

        let prev = self.hovered_card;
        self.hovered_card = self
            .card_bounds
            .iter()
            .find(|&&(_, start, end)| absolute_line >= start && absolute_line < end)
            .map(|&(id, _, _)| id);
        if self.hovered_card != prev {
            self.dirty = true;
        }

        // Update inline subagent card hover
        let prev_inline = self.hovered_inline_subagent;
        self.hovered_inline_subagent = self
            .inline_subagent_card_bounds
            .iter()
            .find(|(_, rect)| rect.contains((x, y).into()))
            .map(|(idx, _)| *idx);
        if self.hovered_inline_subagent != prev_inline {
            self.dirty = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the task description from a task tool call's JSON arguments.
fn extract_task_description(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v.get("description").and_then(|d| d.as_str().map(|s| s.to_string())))
        .unwrap_or_default()
}

/// Extract the subagent type from a task tool call's JSON arguments.
fn extract_subagent_type(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v.get("subagent_type").and_then(|t| t.as_str().map(|s| s.to_string())))
        .unwrap_or_default()
}

/// Infer a human-readable subagent status from a child-session [BackendEvent].
///
/// This replaces the old backend-driven SubagentStatus relay.  Since the
/// subagent's streaming events arrive on the shared event channel with
/// the child session_id, the TUI can infer the equivalent status without
/// any backend changes.
fn infer_subagent_status(event: &BackendEvent) -> Option<String> {
    match event {
        BackendEvent::Delta { .. } => Some("Writing output".to_string()),
        BackendEvent::ReasoningDelta { .. } => Some("Thinking".to_string()),
        BackendEvent::ToolCallUpdated { tool_call, .. } => {
            let name = canonical_tool_name(&tool_call.name)
                .unwrap_or(&tool_call.name);
            Some(format!("Tool: {name}"))
        }
        BackendEvent::ToolCompleted { tool_call, .. } => {
            let name = canonical_tool_name(&tool_call.name)
                .unwrap_or(&tool_call.name);
            Some(format!("Completed: {name}"))
        }
        BackendEvent::Finished { .. }
        | BackendEvent::StreamEnd { .. }
        | BackendEvent::TurnStarting { .. } => Some("Thinking".to_string()),
        _ => None,
    }
}
