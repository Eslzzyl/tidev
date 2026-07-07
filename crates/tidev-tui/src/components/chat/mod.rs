//! MessageList component — the virtualised chat message list.
//!
//! Owns the rendering pipeline, layout index, render cache, scroll state,
//! tool call interaction state, subagent tracking, and streaming buffer.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::Frame;
use anyhow::Result;
use lru::LruCache;
use tidev_tui_old::chat_context::ChatContext;
use uuid::Uuid;
use tidev_types::message::BackendEvent;

use crate::action::{Action, ChatAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::components::chat::layout_index::MessageLayoutIndex;
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, SelectableRegionRange,
};
use crate::components::chat::render as render_mod;
use crate::components::chat::streaming::StreamingBuffer;

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

    // ── Streaming state ──
    streaming_buffer: StreamingBuffer,
    current_streaming_message_id: Option<Uuid>,

    // ── Subagent tracking ──
    running_subagents: Vec<render_mod::RunningSubagentInfo>,

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
            streaming_buffer: StreamingBuffer::new(),
            current_streaming_message_id: None,
            running_subagents: Vec::new(),
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
                        });
                    }
                }
            }
            BackendEvent::ToolCompleted { tool_call, result, .. } => {
                let tool_msg = tidev_types::message::Message::tool_result(
                    tool_call.id.clone(),
                    tool_call.name.clone(),
                    result.clone(),
                );
                chat_context.messages.push(tool_msg);
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
                ..
            } => {
                if let Some(exec) = self.running_subagents.last_mut() {
                    exec.status_text = status_text.clone();
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
            _ => {}
        }
    }

    /// Handle mouse click: find which selectable region was clicked,
    /// and toggle tool result expansion if applicable.
    pub fn handle_mouse_click(&mut self, x: u16, y: u16) {
        let scroll = self.scroll_offset;
        let absolute_line = scroll + y as usize;
        for region in &self.selectable_regions {
            if absolute_line >= region.start_line && absolute_line < region.end_line {
                if x >= region.min_x && region.max_x.map_or(true, |max| x <= max) {
                    if let Some(ref ctx) = self.chat_context {
                        let mut line = 0usize;
                        for msg in &ctx.messages {
                            if msg.role != tidev_types::message::MessageRole::Tool {
                                let msg_lines = 1 + msg.content.lines().count()
                                    + msg.tool_calls.len().saturating_mul(3);
                                if absolute_line >= line && absolute_line < line + msg_lines {
                                    if self.expanded_tool_results.contains(&msg.id) {
                                        self.expanded_tool_results.remove(&msg.id);
                                    } else {
                                        self.expanded_tool_results.insert(msg.id);
                                    }
                                    self.dirty = true;
                                    return;
                                }
                                line += msg_lines;
                            }
                        }
                    }
                }
            }
        }
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

        // Resolve scroll target if set
        if let Some(target_id) = self.scroll_target.take() {
            if let Some(scroll) = self.resolve_scroll_to_message(&chat_context.messages, target_id) {
                self.scroll_offset = scroll;
                self.follow_tail = false;
            }
        }

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
