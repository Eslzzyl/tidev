//! MessageList component — the virtualised chat message list.
//!
//! Owns the rendering pipeline, layout index, render cache, scroll state,
//! tool call interaction state, subagent tracking, and streaming buffer.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::Frame;
use anyhow::Result;
use lru::LruCache;
use crate::chat_context::ChatContext;
use uuid::Uuid;
use tidev_types::message::BackendEvent;

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

    /// Scrollbar drag state (None = not dragging).
    scrollbar_drag: Option<ScrollbarDrag>,

    // ── Spinner animation ──
    spinner_start: Instant,

    // ── Streaming state ──
    streaming_buffer: StreamingBuffer,
    current_streaming_message_id: Option<Uuid>,

    // ── Subagent tracking ──
    running_subagents: Vec<render_mod::RunningSubagentInfo>,

    // ── Bash tool tracking (for ShellOutput streaming) ──
    bash_tool_call_id: Option<String>,

    // ── Dirty tracking ──
    dirty: bool,
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
            scrollbar_drag: None,
            spinner_start: Instant::now(),
            streaming_buffer: StreamingBuffer::new(),
            current_streaming_message_id: None,
            running_subagents: Vec::new(),
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

    /// Handle a backend event for streaming or tool results.
    pub fn handle_backend_event(&mut self, event: &BackendEvent) {
        let Some(ref mut chat_context) = self.chat_context else { return };

        // Ignore events from other sessions (e.g. after switching sessions
        // while the old session's agent loop is still running).
        if event.session_id() != chat_context.session_id {
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
                self.streaming_buffer.push_delta(content);
                self.streaming_buffer.sync_pending(&mut chat_context.messages);
                if let Some(msg_id) = self.streaming_buffer.current_message_id {
                    self.layout_index.mark_dirty(msg_id);
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
            BackendEvent::ToolCallUpdated { tool_call, .. } => {
                if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| m.role == tidev_types::message::MessageRole::Assistant) {
                    msg.upsert_tool_call(tool_call.clone());
                    self.layout_index.mark_dirty(msg.id);
                    self.dirty = true;
                }
                if tool_call.name == "bash" {
                    self.bash_tool_call_id = Some(tool_call.id.clone());
                }
                if tool_call.name == "task" {
                    let already_tracking = self.running_subagents.iter().any(|s| s.tool_call_id == tool_call.id);
                    if !already_tracking {
                        let desc = extract_task_description(&tool_call.arguments);
                        let sub_type = extract_subagent_type(&tool_call.arguments);
                        self.running_subagents.push(render_mod::RunningSubagentInfo {
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
                status_text,
                current_tool_call: _,
                assistant_message,
                content_delta: _,
                reasoning_delta: _,
                child_session_id,
                ..
            } => {
                if let Some(exec) = self.running_subagents.last_mut() {
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
            BackendEvent::ContextCompacted { compacted, summary, .. } => {
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
                        } else {
                            let compaction_msg = tidev_types::message::Message::new(
                                tidev_types::message::MessageRole::System,
                                format!("Compaction\n\n{}", summary),
                            );
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

        // Check subagent card bounds first.
        if !self.running_subagents.is_empty() {
            let msg_end_line = self.layout_index.total_lines;
            let card_start = msg_end_line.saturating_sub(scroll);
            for (i, sa) in self.running_subagents.iter().enumerate() {
                if let Some(csid) = sa.child_session_id {
                    let card_y = card_start + (i * 2);
                    if y_u >= card_y && y_u < (card_y + 2) {
                        return Some(Action::Session(SessionAction::Select(csid)));
                    }
                }
            }
        }

        // Use the layout index to find which block was clicked.
        // The index has accurate line counts from the rendering pipeline,
        // so this is far more reliable than the old heuristic.
        let block_idx = self
            .layout_index
            .blocks
            .partition_point(|b| b.start_line + b.line_count <= absolute_line);
        if block_idx < self.layout_index.blocks.len() {
            let block = &self.layout_index.blocks[block_idx];
            // Only toggle for assistant and user blocks (tool blocks are
            // absorbed into their parent assistant block).
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
                Some(Action::Chat(ChatAction::ScrollDelta(-10)))
            }
            KeyCode::PageDown => {
                Some(Action::Chat(ChatAction::ScrollDelta(10)))
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
                    let viewport = 20;
                    let max_scroll = total.saturating_sub(viewport).max(0);
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
        self.content_area = Some(rect);

        // Resolve scroll target if set
        if let Some(target_id) = self.scroll_target.take() {
            if let Some(scroll) = self.resolve_scroll_to_message(&chat_context.messages, target_id) {
                self.scroll_offset = scroll;
                self.follow_tail = false;
            }
        }

        self.card_bounds.clear();
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
            &mut self.card_bounds,
        );

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
    /// Return the selectable regions as Rects for mouse selection clamping.
    pub fn selectable_region_rects(&self) -> Vec<ratatui::layout::Rect> {
        use crate::components::chat::render_cache::SelectableRegionRange;
        self.selectable_regions.iter().map(|r: &SelectableRegionRange| {
            ratatui::layout::Rect::new(r.min_x, r.start_line as u16, r.max_x.unwrap_or(u16::MAX), (r.end_line - r.start_line) as u16)
        }).collect()
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
