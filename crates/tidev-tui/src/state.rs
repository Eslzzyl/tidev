//! Core TUI state types — rendering cache, layout index, scroll state.
//!
//! These types are shared between the input event handlers, the renderer,
//! and the dialog/panel overlay rendering code.

use ratatui::style::Color;
use ratatui::text::Line;
use std::time::Instant;
use uuid::Uuid;

use tidev_types::message::{Message, MessageAttachment};
use tidev_types::prompts::SessionMode;

pub(crate) const MESSAGE_RENDER_CACHE_MAX_ENTRIES: usize = 1200;

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Welcome,
    Chat,
}

// ---------------------------------------------------------------------------
// ContextUsage
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub(crate) struct ContextUsage {
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
    pub(crate) total_tokens: u32,
    pub(crate) cache_read_tokens: u32,
    pub(crate) cache_write_tokens: u32,
    pub(crate) model_id: String,
    pub(crate) tokens_per_second: Option<f32>,
}

// ---------------------------------------------------------------------------
// QueuedPrompt
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) struct QueuedPrompt {
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<MessageAttachment>,
    pub(crate) mode: Option<SessionMode>,
    pub(crate) instruction_sources: Vec<String>,
}

impl QueuedPrompt {
    pub(crate) fn new(
        prompt: impl Into<String>,
        attachments: Vec<MessageAttachment>,
        mode: Option<SessionMode>,
        instruction_sources: Vec<String>,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            attachments,
            mode,
            instruction_sources,
        }
    }
}

// ---------------------------------------------------------------------------
// ScrollbarDragState
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub(crate) struct ScrollbarDragState {
    pub(crate) start_scroll: usize,
    pub(crate) start_mouse_y: u16,
    pub(crate) max_scroll: usize,
}

// ---------------------------------------------------------------------------
// MessageLayoutIndex
// ---------------------------------------------------------------------------

/// A block in the message layout index representing a renderable unit.
#[derive(Clone, Debug)]
pub(crate) struct MessageBlock {
    pub(crate) message_id: Uuid,
    pub(crate) message_start_idx: usize,
    pub(crate) message_count: usize,
    pub(crate) start_line: usize,
    pub(crate) line_count: usize,
}

/// Layout index for efficient viewport virtualization.
#[derive(Clone, Debug, Default)]
pub(crate) struct MessageLayoutIndex {
    pub(crate) blocks: Vec<MessageBlock>,
    pub(crate) total_lines: usize,
    pub(crate) width: usize,
    pub(crate) valid: bool,
    pub(crate) contains_streaming_messages: bool,
    pub(crate) dirty_messages: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Render cache
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) enum MessageRenderCacheKind {
    Cards,
    ToolCall(String),
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct MessageRenderCacheKey {
    pub(crate) session_id: Uuid,
    pub(crate) message_id: Uuid,
    pub(crate) width: usize,
    pub(crate) is_round_end: bool,
    pub(crate) kind: MessageRenderCacheKind,
}

#[derive(Clone, Debug)]
pub(crate) enum MessageRenderCacheValue {
    Cards(Vec<(Color, Vec<Line<'static>>)>),
    ToolResult(Vec<Line<'static>>, Vec<SelectableRegionRange>),
}

#[derive(Clone, Debug)]
pub(crate) struct MessageRenderCacheEntry {
    pub(crate) value: MessageRenderCacheValue,
    pub(crate) last_used_tick: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct SelectableRegionRange {
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) min_x: u16,
    pub(crate) max_x: Option<u16>,
}

// ---------------------------------------------------------------------------
// CachedSessionRuntime
// ---------------------------------------------------------------------------

/// A lightened in-memory cache of a non-current session's runtime state,
/// needed for subagent operations (cancellation, orphan tool result recording).
#[derive(Clone, Debug)]
pub(crate) struct CachedSessionRuntime {
    pub(crate) messages: Vec<Message>,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) mode: SessionMode,
}

// ---------------------------------------------------------------------------
// NotificationState
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub(crate) struct NotificationState {
    notifications: Vec<(String, Instant)>,
    focused: bool,
}

impl NotificationState {
    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.focused
    }

    pub(crate) fn add(&mut self, message: String) {
        self.notifications.push((message, Instant::now()));
    }

    pub(crate) fn messages(&self) -> &[(String, Instant)] {
        &self.notifications
    }
}
