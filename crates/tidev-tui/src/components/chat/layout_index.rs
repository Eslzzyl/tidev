//! Message block layout index for virtualised message rendering.
//!
//! The layout index maintains a mapping from messages to their positions in the
//! rendered output, enabling:
//! 1. Binary search to find visible messages without rendering everything
//! 2. Incremental updates when only the last few messages change
//! 3. Accurate scroll position calculations

use uuid::Uuid;

// ---------------------------------------------------------------------------
// MessageBlock
// ---------------------------------------------------------------------------

/// A block in the message layout index representing a renderable unit.
///
/// Each block contains either:
/// - A single User/System/Error/Shell message
/// - An Assistant message with its associated Tool results
#[derive(Clone, Debug)]
pub(crate) struct MessageBlock {
    /// ID of the primary message in this block.
    #[allow(dead_code)]
    pub message_id: Uuid,
    /// Starting index in the messages array.
    pub message_start_idx: usize,
    /// Number of messages in this block (1 for User/System/Error, 1+ for Assistant+Tool).
    pub message_count: usize,
    /// Starting line number in the rendered output.
    pub start_line: usize,
    /// Total lines consumed by this block.
    pub line_count: usize,
}

// ---------------------------------------------------------------------------
// MessageLayoutIndex
// ---------------------------------------------------------------------------

/// Layout index for efficient viewport virtualisation.
///
/// The index is invalidated (needs a full rebuild) when:
/// - Window width changes (line counts become invalid)
/// - Messages are added or removed
/// - Cache is cleared
#[derive(Clone, Debug)]
pub(crate) struct MessageLayoutIndex {
    /// Sorted list of message blocks in render order.
    pub blocks: Vec<MessageBlock>,
    /// Total lines across all blocks.
    pub total_lines: usize,
    /// Width used for calculating line counts.
    pub width: usize,
    /// Whether the index is valid and up-to-date.
    pub valid: bool,
    /// Whether the index was last built while assistant streaming was active.
    pub contains_streaming_messages: bool,
    /// Message IDs whose blocks need recomputation (set by content-only
    /// invalidations, cleared after incremental update is applied).
    pub dirty_messages: Vec<Uuid>,
}

impl MessageLayoutIndex {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            total_lines: 0,
            width: 0,
            valid: false,
            contains_streaming_messages: false,
            dirty_messages: Vec::new(),
        }
    }

    /// Reset the index for a full rebuild.
    pub fn reset(&mut self, width: usize, force_rebuild: bool) {
        self.blocks.clear();
        self.total_lines = 0;
        self.width = width;
        self.valid = true;
        self.contains_streaming_messages = force_rebuild;
        self.dirty_messages.clear();
    }

    /// Whether a full rebuild is needed given the current state.
    pub fn needs_full_rebuild(
        &self,
        message_count: usize,
        width: usize,
        force_rebuild: bool,
    ) -> bool {
        if force_rebuild {
            return true;
        }
        if self.contains_streaming_messages != force_rebuild {
            return true;
        }
        if !self.valid {
            return true;
        }
        if self.width != width {
            return true;
        }
        let indexed_count = self
            .blocks
            .last()
            .map(|b| b.message_start_idx + b.message_count)
            .unwrap_or(0);
        if indexed_count != message_count {
            return true;
        }
        if self.blocks.is_empty() && message_count > 0 {
            return true;
        }
        false
    }

    /// Mark a message's block as needing recomputation (content-only change).
    pub fn mark_dirty(&mut self, message_id: Uuid) {
        if !self.dirty_messages.contains(&message_id) {
            self.dirty_messages.push(message_id);
        }
    }

    /// Find message blocks that intersect with the visible viewport.
    ///
    /// Uses binary search (`partition_point`) for O(log n) complexity.
    /// Returns blocks with a small buffer zone (5 lines above and below)
    /// to ensure smooth scrolling.
    pub fn find_visible_blocks(&self, scroll: usize, viewport_height: usize) -> Vec<MessageBlock> {
        if self.blocks.is_empty() {
            return Vec::new();
        }

        let viewport_height = viewport_height.max(1);
        let max_scroll = self.total_lines.saturating_sub(viewport_height);
        let clamped_scroll = scroll.min(max_scroll);

        const BUFFER: usize = 5;
        let visible_start = clamped_scroll.saturating_sub(BUFFER);
        let visible_end = clamped_scroll.saturating_add(viewport_height).saturating_add(BUFFER);

        // Binary search for the first block that could be visible.
        let first_visible = self
            .blocks
            .partition_point(|block| block.start_line + block.line_count <= visible_start);

        // Collect all visible blocks.
        let mut visible = Vec::new();
        for block in self.blocks.iter().skip(first_visible) {
            if block.start_line >= visible_end {
                break;
            }
            visible.push(block.clone());
        }

        visible
    }
}
