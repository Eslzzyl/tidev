//! Types for the message render cache (LRU) and selectable region tracking.
//!
//! The cache stores pre-rendered output for each message block so that
//! re-rendering only happens when content changes.

use ratatui::style::Color;
use ratatui::text::Line;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// MessageRenderCacheKey / Kind / Value / Entry
// ---------------------------------------------------------------------------

/// What kind of render cache entry this is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MessageRenderCacheKind {
    /// Rendered assistant message cards (role label + markdown body).
    Cards,
    /// Rendered tool call output for a specific tool_call_id.
    ToolCall(String),
}

/// Key for the message render cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MessageRenderCacheKey {
    pub session_id: Uuid,
    pub message_id: Uuid,
    pub width: usize,
    #[allow(dead_code)]
    pub is_round_end: bool,
    pub kind: MessageRenderCacheKind,
}

/// Value stored in the render cache.
#[derive(Clone, Debug)]
pub(crate) enum MessageRenderCacheValue {
    /// Rendered card lines (each card has a background colour + lines).
    Cards(Vec<(Color, Vec<Line<'static>>)>),
    /// Rendered tool result lines + associated selectable regions.
    ToolResult(Vec<Line<'static>>, Vec<SelectableRegionRange>),
}

/// A single entry in the render cache.
#[derive(Clone, Debug)]
pub(crate) struct MessageRenderCacheEntry {
    pub value: MessageRenderCacheValue,
}

// ---------------------------------------------------------------------------
// SelectableRegionRange
// ---------------------------------------------------------------------------

/// A range of lines that can be clicked or selected with the mouse.
#[derive(Clone, Debug)]
pub(crate) struct SelectableRegionRange {
    pub start_line: usize,
    pub end_line: usize,
    pub min_x: u16,
    pub max_x: Option<u16>,
}
