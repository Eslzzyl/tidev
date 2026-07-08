//! Mouse text selection for the rendered message area.
//!
//! Mirrors the old `tidev_tui::input::mouse_selection` behaviour.
//! The selection works by:
//! 1. Capturing mouse press/drag/release events (handled by App)
//! 2. Computing the selected screen-cell range
//! 3. Applying a highlight style to the rendered frame buffer
//! 4. Extracting the selected text on release and copying to clipboard

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::prelude::Style;
use unicode_width::UnicodeWidthStr;

/// State tracking an active (or recently-finished) mouse selection.
#[derive(Clone, Debug, Default)]
pub(crate) struct MouseSelection {
    /// Where the drag started (screen coordinates, clamped to bounds).
    anchor: Option<Position>,
    /// Current drag end (screen coordinates, clamped to bounds).
    focus: Option<Position>,
    /// Pointer position (unclamped, for visual tracking).
    pointer: Option<Position>,
    /// Bounds rect that selection is clamped to (the message content area).
    bounds: Option<Rect>,
    /// Whether a drag is in progress (mouse button held).
    dragging: bool,
    /// Whether the pointer has moved at all since press.
    moved: bool,
    /// Set on release when a valid selection was made → triggers clipboard copy.
    pending_copy: bool,
    /// Scroll offset at the time anchor was established.
    anchor_scroll_offset: usize,
}

impl MouseSelection {
    /// Start a selection: call on mouse button down.
    pub fn press(&mut self, position: Position, bounds: Option<Rect>, scroll_offset: usize) {
        let clamped = clamp_to_bounds(position, bounds);
        self.anchor = Some(clamped);
        self.focus = Some(clamped);
        self.pointer = Some(position);
        self.bounds = bounds;
        self.dragging = false;
        self.moved = false;
        self.pending_copy = false;
        self.anchor_scroll_offset = scroll_offset;
    }

    /// Update selection during drag: call on mouse move while button held.
    pub fn drag(&mut self, position: Position) {
        self.pointer = Some(position);
        let Some(anchor) = self.anchor else { return };
        self.dragging = true;
        let clamped = clamp_to_bounds(position, self.bounds);
        self.focus = Some(clamped);
        self.moved |= anchor != clamped;
    }

    /// End selection: call on mouse button up.
    pub fn release(&mut self, position: Position, _current_scroll: usize) {
        self.pointer = Some(position);
        if self.anchor.is_none() {
            self.clear();
            return;
        }
        self.focus = Some(clamp_to_bounds(position, self.bounds));

        if self.has_selection(_current_scroll) {
            let effective = self.anchor != self.focus;
            if effective {
                self.pending_copy = true;
            }
        }
        self.dragging = false;
    }

    /// Whether there is an active non-zero selection.
    pub fn has_selection(&self, current_scroll: usize) -> bool {
        self.compute_range(current_scroll).is_some()
    }

    /// Apply a highlight style to the frame buffer for the selected region.
    pub fn apply_overlay(
        &self,
        buffer: &mut Buffer,
        current_scroll: usize,
        selectable_regions: &[Rect],
        style: Style,
    ) {
        let Some(range) = self.compute_range(current_scroll) else { return };
        apply_selection_style(buffer, range, self.bounds, selectable_regions, style);
    }

    /// Extract the currently selected text from the rendered buffer.
    pub fn selected_text(
        &self,
        buffer: &Buffer,
        current_scroll: usize,
        selectable_regions: &[Rect],
    ) -> Option<String> {
        let range = self.compute_range(current_scroll)?;
        Some(extract_selected_text(buffer, range, self.bounds, selectable_regions))
    }

    /// Check if a pending copy is waiting and return the selection range
    /// (consuming the flag).
    pub fn take_pending_copy(&mut self, current_scroll: usize) -> Option<(Position, Position)> {
        if !self.pending_copy {
            return None;
        }
        self.pending_copy = false;
        self.compute_range(current_scroll).map(|r| (r.start, r.end))
    }

    /// Clear the entire selection state.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn pointer(&self) -> Option<Position> {
        self.pointer
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Compute the effective selection range, adjusting anchor for scroll offset.
    fn compute_range(&self, current_scroll: usize) -> Option<SelectionRange> {
        let (mut anchor, focus) = match (self.anchor, self.focus) {
            (Some(a), Some(f)) => (a, f),
            _ => return None,
        };

        // Adjust anchor position by the scroll delta since press.
        let dy = current_scroll as i32 - self.anchor_scroll_offset as i32;
        anchor.y = (anchor.y as i32 - dy).clamp(0, u16::MAX as i32) as u16;

        if !self.moved && anchor == focus {
            return None;
        }

        if self.moved || anchor != focus {
            Some(SelectionRange::new(anchor, focus))
        } else {
            None
        }
    }
}

/// A normalised (start ≤ end) selection range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SelectionRange {
    start: Position,
    end: Position,
}

impl SelectionRange {
    fn new(a: Position, b: Position) -> Self {
        let (start, end) = if (a.y, a.x) <= (b.y, b.x) {
            (a, b)
        } else {
            (b, a)
        };
        Self { start, end }
    }
}

// ---------------------------------------------------------------------------
// apply_selection_style
// ---------------------------------------------------------------------------

/// Paint a highlight style over the selected cell range in the buffer.
/// Respects `selectable_regions` — cells outside those rects are not styled.
fn apply_selection_style(
    buffer: &mut Buffer,
    range: SelectionRange,
    bounds: Option<Rect>,
    selectable_regions: &[Rect],
    style: Style,
) {
    let effective = effective_area(buffer.area, bounds);
    if effective.width == 0 || effective.height == 0 {
        return;
    }

    let relevant: Vec<Rect> = selectable_regions
        .iter()
        .filter(|r| r.intersects(effective))
        .copied()
        .collect();

    let left = effective.x;
    let top = effective.y;
    let right = effective.x + effective.width;

    let start_y = range.start.y.max(top);
    let end_y = range.end.y.min(top + effective.height - 1);

    for y in start_y..=end_y {
        let row_start = if y == range.start.y {
            range.start.x
        } else {
            left
        };
        let row_end = if y == range.end.y {
            range.end.x
        } else {
            right.saturating_sub(1)
        };

        if row_start > row_end {
            continue;
        }

        if relevant.is_empty() {
            for x in row_start..=row_end {
                if let Some(cell) = buffer.cell_mut((x, y)) {
                    cell.set_style(style);
                }
            }
        } else {
            for x in row_start..=row_end {
                let in_any_region = relevant.iter().any(|r| r.contains(Position::new(x, y)));
                if in_any_region {
                    if let Some(cell) = buffer.cell_mut((x, y)) {
                        cell.set_style(style);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// extract_selected_text
// ---------------------------------------------------------------------------

/// Extract the text content of the selected cell range from the buffer.
fn extract_selected_text(
    buffer: &Buffer,
    range: SelectionRange,
    bounds: Option<Rect>,
    selectable_regions: &[Rect],
) -> String {
    let effective = effective_area(buffer.area, bounds);
    if effective.width == 0 || effective.height == 0 {
        return String::new();
    }

    let relevant: Vec<Rect> = selectable_regions
        .iter()
        .filter(|r| r.intersects(effective))
        .copied()
        .collect();

    let left = effective.x;
    let top = effective.y;
    let right = effective.x + effective.width;

    let start_y = range.start.y.max(top);
    let end_y = range.end.y.min(top + effective.height - 1);

    let mut lines: Vec<String> = Vec::new();

    for y in start_y..=end_y {
        let row_start = if y == range.start.y {
            range.start.x
        } else {
            left
        };
        let row_end = if y == range.end.y {
            range.end.x
        } else {
            right.saturating_sub(1)
        };

        if row_start > row_end {
            lines.push(String::new());
            continue;
        }

        if !relevant.is_empty() {
            let mut segments = Vec::new();
            for region in &relevant {
                if y >= region.y && y < region.y + region.height {
                    let rect_start = row_start.max(region.x);
                    let rect_end = row_end.min(region.x + region.width - 1);
                    if rect_start <= rect_end {
                        segments.push((rect_start, rect_end));
                    }
                }
            }
            if segments.is_empty() {
                lines.push(String::new());
                continue;
            }

            // Merge overlapping segments.
            segments.sort();
            let mut merged: Vec<(u16, u16)> = Vec::new();
            for (s, e) in segments {
                if let Some(last) = merged.last_mut() {
                    if s <= last.1 + 1 {
                        last.1 = last.1.max(e);
                        continue;
                    }
                }
                merged.push((s, e));
            }

            let mut line_text = String::new();
            for (seg_start, seg_end) in &merged {
                for x in *seg_start..=*seg_end {
                    if let Some(cell) = buffer.cell((x, y)) {
                        line_text.push_str(cell.symbol());
                    }
                }
            }
            // Trim trailing whitespace from merged segments.
            let trimmed = line_text.trim_end().to_string();
            lines.push(trimmed);
        } else {
            let mut line_text = String::new();
            for x in row_start..=row_end {
                if let Some(cell) = buffer.cell((x, y)) {
                    line_text.push_str(cell.symbol());
                }
            }
            let trimmed = line_text.trim_end().to_string();
            lines.push(trimmed);
        }
    }

    // Remove trailing empty lines, but keep internal empty lines.
    while lines.last().map_or(false, |l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Clamp a position to the given bounds (if any).
fn clamp_to_bounds(pos: Position, bounds: Option<Rect>) -> Position {
    match bounds {
        Some(r) => Position::new(
            pos.x.clamp(r.x, r.x + r.width.saturating_sub(1)),
            pos.y.clamp(r.y, r.y + r.height.saturating_sub(1)),
        ),
        None => pos,
    }
}

/// Compute the effective area: intersection of buffer area and bounds.
fn effective_area(buffer_area: Rect, bounds: Option<Rect>) -> Rect {
    match bounds {
        Some(b) => {
            let x = buffer_area.x.max(b.x);
            let y = buffer_area.y.max(b.y);
            let right = (buffer_area.x + buffer_area.width).min(b.x + b.width);
            let bottom = (buffer_area.y + buffer_area.height).min(b.y + b.height);
            Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
        }
        None => buffer_area,
    }
}

// ---------------------------------------------------------------------------
// Clipboard helper
// ---------------------------------------------------------------------------

/// Copy text to the system clipboard.
pub(crate) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("failed to set clipboard text: {e}"))
}
