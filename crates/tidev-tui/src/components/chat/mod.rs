//! MessageList component — the virtualised chat message list.
//!
//! Owns the rendering pipeline, layout index, render cache, scroll state,
//! tool call interaction state, subagent tracking, and streaming buffer.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::Frame;
use anyhow::Result;
use lru::LruCache;
use crate::chat_context::ChatContext;
use uuid::Uuid;
use tidev_types::message::{BackendEvent, Message, MessageAttachment};
use tidev_types::prompts::SessionMode;

use tidev_types::tools::canonical_tool_name;
use crate::action::{Action, ChatAction, OverlayAction, OverlayKind, SessionAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::components::chat::layout_index::MessageLayoutIndex;
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, SelectableRegionRange,
};
use crate::components::chat::render as render_mod;
use crate::components::chat::streaming::StreamingBuffer;
use crate::components::chat::tool::tool_call_arguments_are_complete;

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
    /// All open chat contexts, keyed by session_id.
    chat_contexts: HashMap<Uuid, ChatContext>,
    /// Which session is currently displayed.
    active_session_id: Option<Uuid>,

    // ── Rendering infrastructure ──
    layout_index: MessageLayoutIndex,
    render_cache: LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,

    // ── Scroll state ──
    pub scroll_offset: usize,
    pub follow_tail: bool,
    /// Pending scroll target set by ChatAction::ScrollTo.
    pub scroll_target: Option<Uuid>,
    scroll_speed: usize,

    // ── Interaction state ──
    expanded_tool_results: HashSet<Uuid>,

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

    /// Actual on-screen scrollbar rect (for mouse hit-testing).
    scrollbar_rect: Option<Rect>,

    /// Whether the mouse is hovering over the scrollbar area.
    scrollbar_hovered: bool,

    // ── Spinner animation ──
    spinner_start: Instant,

    // ── Streaming state ──
    streaming_buffer: StreamingBuffer,

    // ── Subagent tracking ──
    running_subagents: Vec<render_mod::RunningSubagentInfo>,
    /// Maps tool result message_id → child_session_id for completed subagents.
    completed_subagent_sessions: HashMap<Uuid, Uuid>,
    hovered_inline_subagent: Option<usize>,
    inline_subagent_card_bounds: Vec<(usize, Rect)>,

    // ── Retrying hint (persistent inline display) ──
    retrying_hint: Option<(u32, u32, String, Instant)>,

    // ── Image badge bounds for mouse hit-testing ──
    image_badge_bounds: Vec<(Rect, Uuid, usize)>,

    // ── Dirty tracking ──
    pub(crate) dirty: bool,
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            chat_contexts: HashMap::new(),
            active_session_id: None,
            layout_index: MessageLayoutIndex::new(),
            render_cache: LruCache::new(std::num::NonZeroUsize::new(1200).unwrap()),
            scroll_offset: 0,
            follow_tail: true,
            scroll_target: None,
            scroll_speed: 3,
            expanded_tool_results: HashSet::new(),

            selectable_regions: Vec::new(),
            hovered_card: None,
            card_bounds: Vec::new(),
            content_area: None,
            render_scroll: 0,
            scrollbar_drag: None,
            scrollbar_rect: None,
            scrollbar_hovered: false,
            spinner_start: Instant::now(),
            streaming_buffer: StreamingBuffer::new(),
            running_subagents: Vec::new(),
            completed_subagent_sessions: HashMap::new(),
            hovered_inline_subagent: None,
            inline_subagent_card_bounds: Vec::new(),
            retrying_hint: None,
            image_badge_bounds: Vec::new(),
            dirty: true,
        }
    }

    /// Access the currently active chat context (public for app.rs).
    pub fn active_chat_context(&self) -> Option<&ChatContext> {
        self.active_session_id.and_then(|id| self.chat_contexts.get(&id))
    }

    pub fn active_chat_context_mut(&mut self) -> Option<&mut ChatContext> {
        self.active_session_id.and_then(|id| self.chat_contexts.get_mut(&id))
    }

    /// Switch the displayed session without loading from DB.
    /// Returns true if the session was already cached, false otherwise.
    pub fn switch_to_session(&mut self, session_id: Uuid) -> bool {
        if self.chat_contexts.contains_key(&session_id) {
            self.active_session_id = Some(session_id);
            self.scroll_offset = 0;
            self.follow_tail = true;
            self.layout_index = MessageLayoutIndex::new();
            // Clear render cache on session switch to avoid serving stale
            // entries from a previous session's incomplete rendering pass.
            self.render_cache.clear();
            self.streaming_buffer = StreamingBuffer::new();
            self.selectable_regions.clear();
            self.hovered_inline_subagent = None;
            self.inline_subagent_card_bounds.clear();
            self.retrying_hint = None;
            self.rebuild_subagent_state();

            // Pick up any streaming Assistant message in the target context.
            if let Some(ctx) = self.chat_contexts.get(&session_id) {
                if ctx.messages.iter().any(|m| m.streaming && m.role == tidev_types::message::MessageRole::Assistant) {
                    self.streaming_buffer.recover_or_begin_streaming(
                        &mut self.chat_contexts.get_mut(&session_id).unwrap().messages,
                    );
                }
            }

            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Set the chat context and mark dirty.
    pub fn set_chat_context(&mut self, ctx: ChatContext) {
        let session_id = ctx.session_id;
        self.chat_contexts.insert(session_id, ctx);
        self.active_session_id = Some(session_id);
        self.dirty = true;
        self.scroll_offset = 0;
        self.follow_tail = true;
        self.layout_index = MessageLayoutIndex::new();
        self.render_cache.clear();
        self.streaming_buffer = StreamingBuffer::new();
        self.selectable_regions.clear();
        self.hovered_inline_subagent = None;
        self.inline_subagent_card_bounds.clear();
        self.retrying_hint = None;
        self.rebuild_subagent_state();

        // Pick up any streaming Assistant message in the new context
        // (unlikely for DB-loaded contexts, but harmless).
        if let Some(ctx) = self.chat_contexts.get(&session_id) {
            if ctx.messages.iter().any(|m| m.streaming && m.role == tidev_types::message::MessageRole::Assistant) {
                self.streaming_buffer.recover_or_begin_streaming(
                    &mut self.chat_contexts.get_mut(&session_id).unwrap().messages,
                );
            }
        }
    }

    /// Rebuild subagent state from the current chat_context messages.
    ///
    /// Preserves existing `running_subagents` status_text so ongoing status
    /// updates (via `infer_subagent_status`) survive session switches.
    fn rebuild_subagent_state(&mut self) {
        let session_id = match self.active_session_id {
            Some(id) => id,
            None => return,
        };
        let Some(ctx) = self.chat_contexts.get(&session_id) else { return };
        let messages = ctx.visible_messages();

        // Preserve existing status_text so rebuild doesn't reset it to "Thinking".
        let old_status: std::collections::HashMap<String, String> = self.running_subagents.iter()
            .map(|s| (s.tool_call_id.clone(), s.status_text.clone()))
            .collect();

        self.running_subagents.clear();
        self.completed_subagent_sessions.clear();
        self.hovered_inline_subagent = None;

        let tool_result_ids: std::collections::HashSet<&str> = messages.iter()
            .filter(|m| m.role == tidev_types::message::MessageRole::Tool)
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();

        for msg in messages {
            if msg.role == tidev_types::message::MessageRole::Tool
                && msg.tool_name.as_deref() == Some("task")
                && let Some(csid) = msg.metadata.child_session_id {
                    self.completed_subagent_sessions.insert(msg.id, csid);
                    // Also map by assistant message ID for click hit-testing.
                    let assistant_id = messages.iter()
                        .find(|m| m.role == tidev_types::message::MessageRole::Assistant
                            && m.tool_calls.iter().any(|tc| Some(tc.id.as_str()) == msg.tool_call_id.as_deref()))
                        .map(|m| m.id);
                    if let Some(aid) = assistant_id {
                        self.completed_subagent_sessions.insert(aid, csid);
                    }
                }

            if msg.role == tidev_types::message::MessageRole::Assistant {
                let msg_csid = msg.metadata.child_session_id;
                for tc in &msg.tool_calls {
                    if canonical_tool_name(&tc.name) == Some("task")
                        && !tool_result_ids.contains(tc.id.as_str())
                    {
                        let status = old_status
                            .get(tc.id.as_str())
                            .cloned()
                            .unwrap_or_else(|| "Thinking".to_string());
                        self.running_subagents.push(render_mod::RunningSubagentInfo {
                            tool_call_id: tc.id.clone(),
                            description: extract_task_description(&tc.arguments),
                            subagent_type: extract_subagent_type(&tc.arguments),
                            status_text: status,
                            child_session_id: msg_csid,
                            interrupted: false,
                        });
                    }
                }
            }
        }
    }

    /// Invalidate the layout index (triggers full rebuild on next draw).
    pub fn invalidate_layout(&mut self) {
        self.layout_index.invalidate_all();
        self.dirty = true;
    }

    /// Update token fields on the last streaming assistant message.
    #[allow(clippy::too_many_arguments)]
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
        let session_id = match self.active_session_id {
            Some(id) => id,
            None => return,
        };
        let Some(ref mut chat_context) = self.chat_contexts.get_mut(&session_id) else { return };
        if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| {
            m.role == tidev_types::message::MessageRole::Assistant
        }) {
            let msg_id = msg.id;
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
            self.layout_index.mark_dirty(msg_id);
        }
    }

    /// Mark the last streaming message as an error (on BackendEvent::Failed).
    pub fn mark_streaming_as_error(&mut self, error: &str) {
        let session_id = match self.active_session_id {
            Some(id) => id,
            None => return,
        };
        let Some(ref mut chat_context) = self.chat_contexts.get_mut(&session_id) else { return };
        let msg_id = self.streaming_buffer.current_message_id;
        if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| m.streaming) {
            msg.role = tidev_types::message::MessageRole::Error;
            msg.content = format!("Request failed: {error}");
            msg.completed_at = Some(Utc::now());
            if let Some(mid) = msg_id {
                self.layout_index.mark_dirty(mid);
            }
        }
        self.streaming_buffer.finalise_message(&mut chat_context.messages);
        self.dirty = true;
    }

    /// Finalise the streaming message (if any) and append an error notice.
    /// Called on user abort (double Esc).
    pub fn append_interrupted_message(&mut self) {
        let session_id = match self.active_session_id {
            Some(id) => id,
            None => return,
        };
        let Some(ref mut chat_context) = self.chat_contexts.get_mut(&session_id) else { return };

        // Finalise the streaming message, preserving content and reasoning.
        let finalized_idx = self.streaming_buffer.finalise_message(&mut chat_context.messages);
        if let Some(idx) = finalized_idx {
            if chat_context.messages[idx].content.is_empty()
                && chat_context.messages[idx].reasoning.trim().is_empty()
                && chat_context.messages[idx].tool_calls.is_empty()
            {
                // Interrupted before any content was produced — drop the
                // empty ghost instead of letting it render as "(empty)".
                chat_context.messages.remove(idx);
            } else {
                chat_context.messages[idx].completed_at = Some(Utc::now());
                self.layout_index.mark_dirty(chat_context.messages[idx].id);
            }
        }

        // Clear hover state for the subagent card.
        self.hovered_inline_subagent = None;

        // Mark all running subagents as interrupted instead of clearing them,
        // so their inline cards remain visible and clickable, preserving access
        // to the child subsession conversation.
        for exec in &mut self.running_subagents {
            exec.status_text = "Interrupted".to_string();
            exec.interrupted = true;
        }

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
        // ── 0. Track task subagents only for the currently-active session ─
        // Events for background sessions skip this; rebuild_subagent_state()
        // handles them on session switch.
        let session_id = event.session_id();
        if self.active_session_id == Some(session_id) {
            match event {
                BackendEvent::ToolCallUpdated { tool_call, request_id: _, .. } => {
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
                        } else if tool_call_arguments_are_complete(&tool_call.arguments) {
                            self.running_subagents.push(render_mod::RunningSubagentInfo {
                                tool_call_id: tool_call.id.clone(),
                                description: desc,
                                subagent_type: sub_type,
                                status_text: "Thinking".to_string(),
                                child_session_id: None,
                                interrupted: false,
                            });
                        }
                    }
                }
                BackendEvent::ToolCompleted { tool_call, .. } => {
                    if canonical_tool_name(&tool_call.name) == Some("task") {
                        self.running_subagents.retain(|s| s.tool_call_id != tool_call.id);
                        if self.hovered_inline_subagent.is_some_and(|i| i >= self.running_subagents.len()) {
                            self.hovered_inline_subagent = None;
                        }
                    }
                }
                BackendEvent::SubagentStatus {
                    tool_call_id,
                    status_text,
                    child_session_id,
                    ..
                } => {
                    if let Some(exec) = self.running_subagents.iter_mut()
                        .find(|e| e.tool_call_id == *tool_call_id && !e.interrupted)
                    {
                        exec.status_text = status_text.clone();
                        exec.child_session_id = Some(*child_session_id);
                    }
                }
                _ => {}
            }
        }

        // ── 1. Update running subagent card status ─────────────────────
        // Run this BEFORE the chat_context routing so that events update
        // the inline card regardless of whether the chat_context exists.
        if let Some(text) = infer_subagent_status(event) {
            if let Some(exec) = self.running_subagents.iter_mut()
                .find(|e| e.child_session_id == Some(session_id) && !e.interrupted)
            {
                if exec.status_text != text {
                    exec.status_text = text;
                    self.dirty = true;
                }
            }
        }

        // ── 2. Route to chat_context ───────────────────────────────────
        let chat_context = match self.chat_contexts.get_mut(&session_id) {
            Some(ctx) => ctx,
            None => {
                // Unknown session — nothing more to do.
                return;
            }
        };

        match event {
            BackendEvent::TurnStarting { .. } => {
                if self.active_session_id == Some(session_id) {
                    self.streaming_buffer.begin_streaming(&mut chat_context.messages);
                } else {
                    // Background session: just push a streaming placeholder.
                    // The streaming_buffer is reserved for the active session.
                    let msg = Message::streaming(
                        tidev_types::message::MessageRole::Assistant,
                        String::new(),
                    );
                    chat_context.messages.push(msg);
                }
                self.dirty = true;
            }
            BackendEvent::Delta { content, .. } => {
                let is_active_session = self.active_session_id == Some(session_id);
                if is_active_session && self.streaming_buffer.is_streaming {
                    self.streaming_buffer.push_delta(content, &mut chat_context.messages);
                    if let Some(msg_id) = self.streaming_buffer.current_message_id {
                        self.layout_index.mark_dirty(msg_id);
                    }
                } else if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| {
                    m.streaming
                }) {
                    // Background session or inactive streaming_buffer:
                    // find the streaming message directly in the context.
                    msg.content.push_str(content);
                    self.layout_index.mark_dirty(msg.id);
                } else if is_active_session {
                    // Recovery path for active session: TurnStarting was missed.
                    let mid = self.streaming_buffer.recover_or_begin_streaming(&mut chat_context.messages);
                    self.streaming_buffer.push_delta(content, &mut chat_context.messages);
                    self.layout_index.mark_dirty(mid);
                }
                self.dirty = true;
            }
            BackendEvent::ReasoningDelta { content, .. } => {
                let is_active_session = self.active_session_id == Some(session_id);
                if is_active_session && self.streaming_buffer.is_streaming {
                    self.streaming_buffer.push_reasoning_delta(content, &mut chat_context.messages);
                    if let Some(msg_id) = self.streaming_buffer.current_message_id {
                        self.layout_index.mark_dirty(msg_id);
                    }
                } else if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| {
                    m.streaming
                }) {
                    msg.reasoning.push_str(content);
                    self.layout_index.mark_dirty(msg.id);
                } else if is_active_session {
                    self.streaming_buffer.recover_or_begin_streaming(&mut chat_context.messages);
                    self.streaming_buffer.push_reasoning_delta(content, &mut chat_context.messages);
                    if let Some(msg_id) = self.streaming_buffer.current_message_id {
                        self.layout_index.mark_dirty(msg_id);
                    }
                }
                self.dirty = true;
            }
            BackendEvent::StreamEnd { .. } => {
                let is_active_session = self.active_session_id == Some(session_id);
                let msg_id = if is_active_session {
                    self.streaming_buffer.current_message_id
                } else {
                    None
                };
                if msg_id.is_some() {
                    self.streaming_buffer.finalise_message(&mut chat_context.messages);
                    if let Some(mid) = msg_id {
                        self.layout_index.mark_dirty(mid);
                    }
                } else {
                    // Recovery path: find any streaming Assistant message.
                    if let Some(idx) = chat_context.messages.iter().rposition(|m| {
                        m.streaming && m.role == tidev_types::message::MessageRole::Assistant
                    }) {
                        chat_context.messages[idx].streaming = false;
                        self.layout_index.mark_dirty(chat_context.messages[idx].id);
                    }
                }
                self.dirty = true;
            }
            BackendEvent::ToolCallUpdated { tool_call, request_id: _, .. } => {
                // Recovery path: if TurnStarting was missed but a streaming
                // Assistant message exists (created by Delta recovery), pick
                // it up so we add the tool call to the right message.
                if !self.streaming_buffer.is_streaming
                    && chat_context.messages.iter().any(|m| m.streaming && m.role == tidev_types::message::MessageRole::Assistant)
                {
                    self.streaming_buffer.recover_or_begin_streaming(&mut chat_context.messages);
                }

                // Prefer the currently-streaming message so tool calls from
                // the current turn land on the right assistant message, even
                // if earlier turns have assistant messages in the context.
                let target_id = self.streaming_buffer.current_message_id;
                let target = if let Some(mid) = target_id {
                    chat_context.messages.iter_mut().rev().find(|m| m.id == mid)
                } else {
                    chat_context.messages.iter_mut().rev()
                        .find(|m| m.role == tidev_types::message::MessageRole::Assistant)
                };
                if let Some(msg) = target {
                    msg.upsert_tool_call(tool_call.clone());
                    self.layout_index.mark_dirty(msg.id);
                    self.dirty = true;
                }
                // Note: task tool tracking (RunningSubagentInfo) is handled
                // in Step 0 above, before chat_context routing, so that
                // nested subagents are tracked even when this session's
                // chat_context hasn't been created yet.
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
                } else {
                    // Dedup: if a message for this tool_call_id already exists,
                    // update its content instead of pushing a duplicate.
                    let existing = chat_context.messages.iter_mut().rfind(|m| {
                        m.role == tidev_types::message::MessageRole::Tool
                            && m.tool_call_id.as_deref() == Some(&tool_call.id)
                    });
                    if let Some(existing) = existing {
                        existing.content = result.output.clone();
                    } else {
                        let tool_msg = tidev_types::message::Message::tool_result(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            result.clone(),
                        );
                        chat_context.messages.push(tool_msg);
                    }
                    // Note: running_subagents cleanup for completed task
                    // tools is handled in Step 0 above, before chat_context
                    // routing.

                    // Track child_session_id for subagent task results.
                    if let Some(csid) = result.metadata.child_session_id {
                        // Map by tool message ID.
                        let tool_msg_id = chat_context.messages.iter().rev()
                            .find(|m| m.tool_call_id.as_deref() == Some(&tool_call.id))
                            .map(|m| m.id);
                        if let Some(msg_id) = tool_msg_id {
                            self.completed_subagent_sessions.insert(msg_id, csid);
                        }
                        // Also map by assistant message ID for click hit-testing.
                        let assistant_msg_id = chat_context.messages.iter().rev()
                            .find(|m| m.role == tidev_types::message::MessageRole::Assistant
                                && m.tool_calls.iter().any(|tc| tc.id == tool_call.id))
                            .map(|m| m.id);
                        if let Some(msg_id) = assistant_msg_id {
                            self.completed_subagent_sessions.insert(msg_id, csid);
                        }
                    }
                    self.dirty = true;
                }
            }
            BackendEvent::ShellOutput { session_id: _, tool_call_id, content, finished, .. } => {
                let existing = chat_context.messages.iter_mut().rev().find(|m| {
                    m.role == tidev_types::message::MessageRole::Tool
                        && m.tool_call_id.as_deref() == Some(tool_call_id)
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
                    msg.tool_call_id = Some(tool_call_id.clone());
                    msg.tool_name = Some("bash".to_string());
                    msg.streaming = !*finished;
                    chat_context.messages.push(msg);
                }
                self.dirty = true;
            }
            BackendEvent::Retrying {
                attempt,
                max_attempts,
                reason,
                retry_after_secs,
                ..
            } => {
                let deadline =
                    Instant::now() + Duration::from_secs(retry_after_secs.unwrap_or(0) as u64);
                self.retrying_hint = Some((*attempt, *max_attempts, reason.clone(), deadline));
                self.dirty = true;
            }
            BackendEvent::Finished { .. } => {
                self.retrying_hint = None;
                self.dirty = true;
            }
            BackendEvent::SubagentStatus {
                tool_call_id,
                assistant_message,
                child_session_id,
                ..
            } => {
                // running_subagents update (child_session_id + status_text) is
                // handled in stage 0, before chat_context routing, so that
                // nested subagents are linked even when this chat_context
                // hasn't been visited yet.  Here we only sync the metadata
                // and assistant message which require the chat_context.
                // Sync child_session_id into the assistant message's metadata
                // so rebuild_subagent_state() can recover it from messages.
                if let Some(msg) = chat_context.messages.iter_mut().rev().find(|m| {
                    m.role == tidev_types::message::MessageRole::Assistant
                        && m.tool_calls.iter().any(|tc| tc.id == *tool_call_id)
                }) {
                    msg.metadata.child_session_id = Some(*child_session_id);
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
            BackendEvent::SidebarSnapshotReady { tool_call_id, file_diffs_json, .. } => {
                if let Some(msg) = chat_context.messages.iter_mut().find(|m| m.tool_call_id.as_deref() == Some(tool_call_id)) {
                    msg.file_diffs = Some(file_diffs_json.clone());
                    self.layout_index.mark_dirty(msg.id);
                    self.dirty = true;
                }
            }
            BackendEvent::ContextCompacted { compacted, summary, model_id, completed_at, .. } => {
                if *compacted {
                    self.follow_tail = true;
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
            BackendEvent::Failed { .. } => {
                self.retrying_hint = None;
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Handle mouse click: find which selectable region was clicked
    /// and toggle tool result expansion for the associated block.
    /// Returns an Action if a subsession switch is requested.
    pub fn handle_mouse_click(&mut self, x: u16, y: u16) -> Option<Action> {
        // Check image badge bounds first — click on an image badge opens ImageViewer.
        if let Some((_, msg_id, att_idx)) = self
            .image_badge_bounds
            .iter()
            .find(|(rect, _, _)| rect.contains((x, y).into()))
        {
            let session_id = self.active_session_id?;
            let ctx = self.chat_contexts.get(&session_id)?;
            let msg = ctx.visible_messages().iter().find(|m| m.id == *msg_id)?;
            if let Some(MessageAttachment::Image { data, filename, .. }) = msg.attachments.get(*att_idx)
            {
                return Some(Action::Overlay(OverlayAction::Open(
                    OverlayKind::ImageViewer {
                        data: data.clone(),
                        filename: filename.clone(),
                    },
                )));
            }
        }

        let scroll = self.scroll_offset;
        let y_u = y as usize;
        let absolute_line = scroll + y_u;

        // Check inline subagent card bounds next (Rect-based hit detection).
        if let Some(hit) = self
            .inline_subagent_card_bounds
            .iter()
            .find(|(_, rect)| rect.contains((x, y).into()))
        {
            let exec_idx = hit.0;
            if let Some(sa) = self.running_subagents.get(exec_idx)
                && let Some(csid) = sa.child_session_id {
                    return Some(Action::Session(SessionAction::Select(csid)));
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
                // Completed subagent result: navigate to subsession.
                if let Some(&child_sid) = self.completed_subagent_sessions.get(&block.message_id) {
                    return Some(Action::Session(SessionAction::Select(child_sid)));
                }
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
    ///
    /// Uses the layout index (which has accurate line counts from the actual
    /// rendering pipeline) instead of computing a simplified line-count formula
    /// that would be wrong for word-wrapped / tool-call-heavy content.
    fn resolve_scroll_to_message(&self, messages: &[tidev_types::message::Message], target_id: uuid::Uuid) -> Option<usize> {
        self.layout_index.find_scroll_offset(messages, target_id)
    }
}

impl Component for MessageList {
    fn init(&mut self, ctx: &InitContext) -> Result<()> {
        self.scroll_speed = ctx.config.ui.scroll_speed as usize;
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
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
                if self.active_session_id.is_some() {
                    let total = self.layout_index.total_lines;
                    let viewport = self.content_area.map(|r| r.height as usize).unwrap_or(20).max(1);
                    let max_scroll = total.saturating_sub(viewport);
                    let current = if self.follow_tail { max_scroll } else { self.scroll_offset.min(max_scroll) };
                    let new_scroll = (current as isize + delta).max(0) as usize;
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

            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let session_id = match self.active_session_id {
            Some(id) => id,
            None => return,
        };
        let Some(chat_context) = self.chat_contexts.get(&session_id) else {
            return;
        };

        self.selectable_regions.clear();

        // Resolve scroll target if set
        if let Some(target_id) = self.scroll_target.take()
            && let Some(scroll) = self.resolve_scroll_to_message(&chat_context.messages, target_id) {
                self.scroll_offset = scroll;
                self.follow_tail = false;
            }

        self.card_bounds.clear();
        self.content_area = None;
        self.render_scroll = 0;
        self.inline_subagent_card_bounds.clear();
        self.image_badge_bounds.clear();
        let mut render_content_area = Rect::default();
        let mut render_scroll = 0;
        let mut inline_running_card_ranges = Vec::new();
        let mut image_badge_infos = Vec::new();
        render_mod::render_messages(
            frame,
            rect,
            ctx.workspace_root,
            &mut self.layout_index,
            &mut self.render_cache,
            chat_context,
            ctx.palette,
            &mut self.scroll_offset,
            &mut self.follow_tail,
            &mut self.expanded_tool_results,
            &self.running_subagents,
            self.spinner_start,
            self.hovered_card,
            self.hovered_inline_subagent,
            &self.retrying_hint,
            &mut self.card_bounds,
            &mut self.selectable_regions,
            &mut inline_running_card_ranges,
            &mut image_badge_infos,
            &mut render_content_area,
            &mut render_scroll,
            self.scrollbar_hovered,
        );
        self.content_area = Some(render_content_area);
        self.render_scroll = render_scroll;
        // Store the actual scrollbar rect for mouse hit-testing.
        self.scrollbar_rect = compute_scrollbar_rect(rect);

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

        // Convert image badge infos to screen-space Rects for mouse hit-testing.
        // Card lines have 2-space indent + "┃ " prefix before content.
        let indent_col: u16 = 4;
        for info in &image_badge_infos {
            let abs_line = info.card_start_line + info.badge_line_offset;
            let screen_line = abs_line.saturating_sub(render_scroll);
            if screen_line >= viewport {
                continue;
            }
            let screen_x = render_content_area.x + indent_col + info.badge_col as u16;
            let screen_y = render_content_area.y + screen_line as u16;
            let rect = Rect::new(screen_x, screen_y, info.badge_width as u16, 1);
            self.image_badge_bounds
                .push((rect, info.message_id, info.attachment_index));
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

    /// Return the scrollbar area, if visible.
    pub fn scrollbar_area(&self) -> Option<Rect> {
        self.scrollbar_rect
    }

    /// Set whether the mouse is hovering over the scrollbar.
    pub fn set_scrollbar_hovered(&mut self, hovered: bool) {
        if self.scrollbar_hovered != hovered {
            self.scrollbar_hovered = hovered;
            self.dirty = true;
        }
    }

    /// Maximum scroll offset.
    pub fn max_scroll(&self) -> usize {
        self.layout_index.total_lines.saturating_sub(self.content_area.map_or(0, |a| a.height as usize))
    }

    /// Start a scrollbar drag: call on mouse down on scrollbar.
    /// Jumps to the clicked position first (click-to-jump), then starts drag tracking.
    pub fn start_scrollbar_drag(&mut self, mouse_y: u16) {
        let Some(sb_area) = self.scrollbar_area() else { return };
        let max_scroll = self.max_scroll();
        if max_scroll == 0 {
            return;
        }
        // Click-to-jump: map click position to scroll offset
        let track_height = sb_area.height as usize;
        let click_y = mouse_y.saturating_sub(sb_area.y) as f32;
        let target_scroll = ((click_y / track_height.max(1) as f32) * max_scroll as f32).round() as usize;
        self.scroll_offset = target_scroll.min(max_scroll);
        self.follow_tail = self.scroll_offset >= max_scroll;
        self.dirty = true;
        // Start drag tracking for subsequent drag events
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

    /// Number of running subagents (excludes interrupted/cancelled ones).
    pub fn running_subagents_count(&self) -> usize {
        self.running_subagents.iter().filter(|s| !s.interrupted).count()
    }

    /// Whether a given session_id is currently being run as a subagent.
    pub fn is_subagent_running(&self, session_id: Uuid) -> bool {
        self.running_subagents
            .iter()
            .any(|s| s.child_session_id == Some(session_id))
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

/// Compute the scrollbar rect from the full chat area.
/// Mirrors the scrollbar positioning logic in `render::compute_content_layout`.
fn compute_scrollbar_rect(rect: Rect) -> Option<Rect> {
    if rect.width <= render_mod::LEFT_MARGIN + render_mod::SCROLLBAR_WIDTH {
        return None;
    }
    Some(Rect {
        x: rect.x + rect.width - render_mod::SCROLLBAR_WIDTH,
        y: rect.y,
        width: render_mod::SCROLLBAR_WIDTH,
        height: rect.height,
    })
}
