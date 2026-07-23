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

use base64::Engine as _;


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
    pub fn release(&mut self, position: Position, current_scroll: usize) {
        self.pointer = Some(position);
        if self.anchor.is_none() {
            self.clear();
            return;
        }
        self.focus = Some(clamp_to_bounds(position, self.bounds));

        if self.has_selection(current_scroll) {
            let effective = self.anchor != self.focus;
            self.pending_copy = effective;
            self.dragging = false;
            if !self.pending_copy {
                self.clear();
            }
            return;
        }

        self.clear();
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

    /// Shift the selection by a scroll delta preserving content-relative
    /// positions.  Called when the user manually scrolls (scroll wheel)
    /// during an active selection so the selected content stays the same.
    ///
    /// When actively dragging (`self.dragging`) the focus is left at the
    /// current mouse screen position so the selection naturally extends as
    /// new content scrolls into view — matching auto-scroll behaviour.
    /// When not dragging the focus is shifted together with the anchor so
    /// that a simple press (without drag) does not accidentally create a
    /// spurious non-zero selection range.
    pub fn shift_for_scroll(&mut self, delta: isize) {
        let dy = delta as i32;
        if let Some(ref mut a) = self.anchor {
            a.y = (a.y as i32 - dy).clamp(0, u16::MAX as i32) as u16;
        }
        if !self.dragging {
            // When not dragging, shift focus together with anchor so a
            // plain press + scroll doesn't accidentally create a selection.
            if let Some(ref mut f) = self.focus {
                f.y = (f.y as i32 - dy).clamp(0, u16::MAX as i32) as u16;
            }
        }
        // pointer is the raw mouse screen position – do NOT adjust it
        // (auto-scroll reads pointer against the screen-area bounds).

        if delta >= 0 {
            self.anchor_scroll_offset =
                self.anchor_scroll_offset.saturating_add(delta as usize);
        } else {
            self.anchor_scroll_offset =
                self.anchor_scroll_offset.saturating_sub((-delta) as usize);
        }
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

        if !relevant.is_empty() {
            for region in &relevant {
                if y >= region.y && y < region.y + region.height {
                    let rstart = row_start.max(region.x);
                    let rend = row_end.min(region.x + region.width - 1);
                    if rstart <= rend {
                        let mut actual_end = None;
                        for x in (rstart..=rend).rev() {
                            if let Some(cell) = buffer.cell((x, y)) {
                                let sym = cell.symbol();
                                if sym != " " && !sym.is_empty() {
                                    // Cover all cells occupied by this wide character.
                                    let w = UnicodeWidthStr::width(sym).max(1) as u16;
                                    actual_end = Some((x + w - 1).min(rend));
                                    break;
                                }
                            }
                        }
                        if let Some(e) = actual_end {
                            for x in rstart..=e {
                                if let Some(cell) = buffer.cell_mut((x, y)) {
                                    cell.set_style(style);
                                }
                            }
                        } else if rstart == row_start
                            && let Some(cell) = buffer.cell_mut((rstart, y)) {
                                cell.set_style(style);
                            }
                    }
                }
            }
        } else {
            let mut actual_end = None;
            for x in (row_start..=row_end).rev() {
                if let Some(cell) = buffer.cell((x, y)) {
                    let sym = cell.symbol();
                    if sym != " " && !sym.is_empty() {
                        // Cover all cells occupied by this wide character.
                        let w = UnicodeWidthStr::width(sym).max(1) as u16;
                        actual_end = Some((x + w - 1).min(row_end));
                        break;
                    }
                }
            }
            if let Some(e) = actual_end {
                for x in row_start..=e {
                    if let Some(cell) = buffer.cell_mut((x, y)) {
                        cell.set_style(style);
                    }
                }
            } else {
                if let Some(cell) = buffer.cell_mut((row_start, y)) {
                    cell.set_style(style);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// extract_selected_text
// ---------------------------------------------------------------------------

/// Helper: extract text from a horizontal range of cells, correctly skipping
/// trailing cells of wide characters (CJK, emoji, etc.).
///
/// ratatui stores wide characters across 2+ cells. The first cell holds the
/// symbol, while trailing cells have `symbol = None` — but `Cell::symbol()`
/// returns `" "` for `None`, which would insert spurious spaces.  We use
/// `UnicodeWidthStr::width()` to skip trailing cells.
fn extract_row_text(buffer: &Buffer, y: u16, start_x: u16, end_x: u16) -> String {
    let mut text = String::new();
    let mut x = start_x;
    while x <= end_x {
        let Some(cell) = buffer.cell((x, y)) else {
            break;
        };
        let symbol = cell.symbol();
        text.push_str(symbol);
        let width = UnicodeWidthStr::width(symbol).max(1) as u16;
        x = x.saturating_add(width);
        if x == 0 {
            break;
        }
    }
    text
}

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
                if let Some(last) = merged.last_mut()
                    && s <= last.1 + 1 {
                        last.1 = last.1.max(e);
                        continue;
                    }
                merged.push((s, e));
            }

            let mut line_text = String::new();
            for (seg_start, seg_end) in &merged {
                line_text.push_str(&extract_row_text(buffer, y, *seg_start, *seg_end));
            }
            lines.push(line_text);
        } else {
            let line_text = extract_row_text(buffer, y, row_start, row_end);
            lines.push(line_text);
        }
    }

    // Remove trailing empty lines, but keep internal empty lines.
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    if lines.is_empty() {
        return String::new();
    }

    // Join lines with smart wrapping logic (mirrors old TUI behaviour):
    // - preserve indentation (newline when trailing spaces < next indent)
    // - join soft-wrapped lines with space
    let mut result = String::new();
    for i in 0..lines.len() {
        let line = &lines[i];
        let trimmed = line.trim_end_matches(' ');
        let trailing_spaces = line.len() - trimmed.len();
        result.push_str(trimmed);

        if i + 1 < lines.len() {
            let next_line = &lines[i + 1];
            let next_trimmed = next_line.trim_start_matches(' ');
            if next_trimmed.is_empty() {
                result.push('\n');
            } else {
                let first_word_width = next_trimmed
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .len();
                if trailing_spaces < first_word_width || trailing_spaces == 0 {
                    let last_char = trimmed.chars().last().unwrap_or(' ');
                    let first_next_char = next_trimmed.chars().next().unwrap_or(' ');
                    if trailing_spaces > 0 || (last_char.is_ascii() && first_next_char.is_ascii()) {
                        result.push(' ');
                    }
                } else {
                    result.push('\n');
                }
            }
        }
    }

    result
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
///
/// Uses `arboard` for native clipboard access (local desktop).  Falls back
/// to the **OSC 52** terminal escape sequence when the native clipboard is
/// unavailable (e.g. over SSH, inside tmux, or in a container without a
/// display server).  OSC 52 writes the selected text to the **client-side**
/// clipboard via the terminal emulator.
pub(crate) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    // 1. Try native clipboard first.
    if let Err(err) = copy_via_arboard(text) {
        log::debug!("arboard clipboard failed (SSH?): {err}; falling back to OSC 52");
        copy_via_osc52(text)?;
    }
    Ok(())
}

/// Try native `arboard` clipboard.
fn copy_via_arboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("failed to set clipboard text: {e}"))
}

/// Copy text via OSC 52 escape sequence to the terminal emulator's clipboard.
///
/// Writes the sequence immediately to stdout (with flush), which is safe
/// inside `ratatui::Terminal::draw()` — the escape sequence is sent to the
/// terminal alongside the frame buffer update.
///
/// When running inside tmux (detected via `$TMUX`), the sequence is wrapped
/// with tmux's DCS passthrough so tmux forwards it to the client terminal.
fn copy_via_osc52(text: &str) -> Result<(), String> {
    use std::io::Write;

    let base64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());

    let sequence = if std::env::var("TMUX").is_ok() {
        // Inside tmux: wrap with tmux's DCS passthrough so it reaches the
        // client terminal rather than being consumed by tmux's clipboard.
        format!("\x1bPtmux;\x1b]52;c;{base64}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{base64}\x07")
    };

    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(sequence.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|e| format!("failed to write OSC 52 sequence: {e}"))
}
