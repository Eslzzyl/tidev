use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    prelude::Frame,
    style::Style,
};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

use super::App;

#[derive(Clone, Debug, Default)]
pub(crate) struct MouseSelectionState {
    anchor: Option<Position>,
    focus: Option<Position>,
    pointer: Option<Position>,
    bounds: Option<Rect>,
    dragging: bool,
    moved: bool,
    pending_copy: bool,
    anchor_scroll_offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SelectionRange {
    start: Position,
    end: Position,
}

pub(crate) struct ClipboardLease {
    #[cfg(target_os = "linux")]
    _clipboard: Option<arboard::Clipboard>,
}

impl ClipboardLease {
    #[cfg(target_os = "linux")]
    fn native_linux(clipboard: arboard::Clipboard) -> Self {
        Self {
            _clipboard: Some(clipboard),
        }
    }
}

impl MouseSelectionState {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn press_with_bounds(
        &mut self,
        position: Position,
        bounds: Option<Rect>,
        scroll_offset: usize,
    ) {
        let bounded = clamp_to_bounds(position, bounds);
        self.anchor = Some(bounded);
        self.focus = Some(bounded);
        self.pointer = Some(position);
        self.bounds = bounds;
        self.dragging = false;
        self.moved = false;
        self.pending_copy = false;
        self.anchor_scroll_offset = scroll_offset;
    }

    pub(crate) fn drag(&mut self, position: Position) {
        self.pointer = Some(position);
        let Some(anchor) = self.anchor else {
            return;
        };

        self.dragging = true;
        let bounded = clamp_to_bounds(position, self.bounds);
        self.focus = Some(bounded);
        self.moved |= anchor != bounded;
    }

    pub(crate) fn release(&mut self, position: Position, current_scroll: usize) {
        self.pointer = Some(position);

        if self.anchor.is_none() {
            self.clear();
            return;
        }

        self.focus = Some(clamp_to_bounds(position, self.bounds));

        if self.has_selection(current_scroll) {
            self.pending_copy = true;
            self.dragging = false;
            return;
        }

        self.clear();
    }

    pub(crate) fn pointer(&self) -> Option<Position> {
        self.pointer
    }

    pub(crate) fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub(crate) fn has_selection(&self, current_scroll: usize) -> bool {
        self.range(current_scroll).is_some()
    }

    pub(crate) fn selection_range(&self, current_scroll: usize) -> Option<(Position, Position)> {
        let range = self.range(current_scroll)?;
        Some((range.start, range.end))
    }

    pub(crate) fn apply_overlay(
        &self,
        buffer: &mut Buffer,
        current_scroll: usize,
        selectable_regions: &[Rect],
        style: Style,
    ) {
        let Some(range) = self.range(current_scroll) else {
            return;
        };

        apply_selection_style(buffer, range, self.bounds, selectable_regions, style);
    }

    pub(crate) fn selected_text(
        &self,
        buffer: &Buffer,
        current_scroll: usize,
        selectable_regions: &[Rect],
    ) -> Option<String> {
        let range = self.range(current_scroll)?;
        let text = extract_selected_text(buffer, range, self.bounds, selectable_regions);
        Some(text)
    }

    pub(crate) fn take_pending_copy(
        &mut self,
        current_scroll: usize,
    ) -> Option<(Position, Position)> {
        if !self.pending_copy {
            return None;
        }

        self.pending_copy = false;
        self.selection_range(current_scroll)
    }

    fn range(&self, current_scroll: usize) -> Option<SelectionRange> {
        let (mut anchor, focus) = match (self.anchor, self.focus) {
            (Some(anchor), Some(focus)) => (anchor, focus),
            _ => return None,
        };

        let dy = current_scroll as i32 - self.anchor_scroll_offset as i32;
        anchor.y = (anchor.y as i32 - dy).clamp(0, u16::MAX as i32) as u16;

        // Allow single character selection if we just clicked without moving,
        // so long as it passes the later is_empty checks inside applicable regions.
        if !self.moved && anchor == focus {
            return Some(SelectionRange::new(anchor, focus));
        }

        if self.moved || anchor != focus {
            Some(SelectionRange::new(anchor, focus))
        } else {
            None
        }
    }
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

impl App {
    pub(crate) fn finish_mouse_selection(&mut self, frame: &mut Frame<'_>) {
        let palette = self.palette();
        let selection_style = Style::default()
            .bg(palette.selection_bg)
            .fg(palette.selection_fg);

        let buffer = frame.buffer_mut();
        self.mouse_selection.apply_overlay(
            buffer,
            self.message_scroll_offset,
            &self.selectable_regions,
            selection_style,
        );

        let Some(_) = self
            .mouse_selection
            .take_pending_copy(self.message_scroll_offset)
        else {
            return;
        };

        let Some(text) = self.mouse_selection.selected_text(
            buffer,
            self.message_scroll_offset,
            &self.selectable_regions,
        ) else {
            return;
        };

        if text.is_empty() {
            return;
        }

        match copy_to_clipboard(&text) {
            Ok(lease) => {
                self.selection_clipboard_lease = lease;
                self.mouse_selection.clear();
                self.toast = Some((
                    "Selection copied to clipboard".to_string(),
                    Instant::now() + Duration::from_secs(3),
                ));
            }
            Err(error) => {
                self.toast = Some((
                    format!("Failed to copy selection: {error}"),
                    Instant::now() + Duration::from_secs(3),
                ));
            }
        }
    }
}

pub(crate) fn copy_to_clipboard(text: &str) -> Result<Option<ClipboardLease>, String> {
    #[cfg(target_os = "linux")]
    {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))?;
        clipboard
            .set_text(text)
            .map_err(|error| format!("failed to set clipboard text: {error}"))?;
        Ok(Some(ClipboardLease::native_linux(clipboard)))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))?;
        clipboard
            .set_text(text)
            .map_err(|error| format!("failed to set clipboard text: {error}"))?;
        Ok(None)
    }
}

fn apply_selection_style(
    buffer: &mut Buffer,
    range: SelectionRange,
    bounds: Option<Rect>,
    selectable_regions: &[Rect],
    style: Style,
) {
    let effective_area = effective_area(buffer.area, bounds);
    if effective_area.width == 0 || effective_area.height == 0 {
        return;
    }

    let relevant_regions: Vec<Rect> = selectable_regions
        .iter()
        .filter(|&r| r.intersects(effective_area))
        .copied()
        .collect();

    let area_left = effective_area.x;
    let area_top = effective_area.y;
    let area_right = effective_area.x.saturating_add(effective_area.width);
    let area_bottom = effective_area.y.saturating_add(effective_area.height);

    let start_x = range.start.x.max(area_left);
    let start_y = range.start.y.max(area_top);
    let end_x = range.end.x.min(area_right.saturating_sub(1));
    let end_y = range.end.y.min(area_bottom.saturating_sub(1));

    if start_x > end_x || start_y > end_y {
        return;
    }

    for y in start_y..=end_y {
        let row_start = if y == start_y { start_x } else { area_left };
        let row_end = if y == end_y {
            end_x
        } else {
            area_right.saturating_sub(1)
        };

        if row_start > row_end {
            continue;
        }

        if !relevant_regions.is_empty() {
            for region in &relevant_regions {
                if y >= region.y && y < region.y + region.height {
                    let rect_start = row_start.max(region.x);
                    let rect_end = row_end.min(region.x + region.width - 1);
                    if rect_start <= rect_end {
                        let mut actual_end = None;
                        for x in (rect_start..=rect_end).rev() {
                            if let Some(cell) = buffer.cell((x, y)) {
                                let sym = cell.symbol();
                                if sym != " " && !sym.is_empty() {
                                    actual_end = Some(x);
                                    break;
                                }
                            }
                        }
                        if let Some(e) = actual_end {
                            buffer
                                .set_style(Rect::new(rect_start, y, e - rect_start + 1, 1), style);
                        } else if rect_start == row_start {
                            buffer.set_style(Rect::new(rect_start, y, 1, 1), style);
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
                        actual_end = Some(x);
                        break;
                    }
                }
            }
            if let Some(e) = actual_end {
                buffer.set_style(Rect::new(row_start, y, e - row_start + 1, 1), style);
            } else {
                buffer.set_style(Rect::new(row_start, y, 1, 1), style);
            }
        }
    }
}

fn extract_selected_text(
    buffer: &Buffer,
    range: SelectionRange,
    bounds: Option<Rect>,
    selectable_regions: &[Rect],
) -> String {
    let effective_area = effective_area(buffer.area, bounds);
    if effective_area.width == 0 || effective_area.height == 0 {
        return String::new();
    }

    let relevant_regions: Vec<Rect> = selectable_regions
        .iter()
        .filter(|&r| r.intersects(effective_area))
        .copied()
        .collect();

    let area_left = effective_area.x;
    let area_top = effective_area.y;
    let area_right = effective_area.x.saturating_add(effective_area.width);
    let area_bottom = effective_area.y.saturating_add(effective_area.height);

    let start_x = range.start.x.max(area_left);
    let start_y = range.start.y.max(area_top);
    let end_x = range.end.x.min(area_right.saturating_sub(1));
    let end_y = range.end.y.min(area_bottom.saturating_sub(1));

    if start_x > end_x || start_y > end_y {
        return String::new();
    }

    let mut lines = Vec::new();
    for y in start_y..=end_y {
        let row_start = if y == start_y { start_x } else { area_left };
        let row_end = if y == end_y {
            end_x
        } else {
            area_right.saturating_sub(1)
        };

        if row_start > row_end {
            lines.push(String::new());
            continue;
        }

        if !relevant_regions.is_empty() {
            let mut row_segments = Vec::new();
            for region in &relevant_regions {
                if y >= region.y && y < region.y + region.height {
                    let rect_start = row_start.max(region.x);
                    let rect_end = row_end.min(region.x + region.width - 1);
                    if rect_start <= rect_end {
                        row_segments.push((rect_start, rect_end));
                    }
                }
            }
            if row_segments.is_empty() {
                lines.push(String::new());
            } else {
                row_segments.sort_by_key(|r| r.0);
                let mut row_text = String::new();
                for (rect_start, rect_end) in row_segments {
                    row_text.push_str(&extract_row_text(buffer, y, rect_start, rect_end));
                }
                lines.push(row_text);
            }
        } else {
            lines.push(extract_row_text(buffer, y, row_start, row_end));
        }
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    let mut joined_text = String::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_end_matches(' ');
        let trailing_spaces = line.chars().count() - trimmed.chars().count();
        joined_text.push_str(trimmed);

        if i + 1 < lines.len() {
            let next_line = &lines[i + 1];
            let next_line_trimmed = next_line.trim_start_matches(' ');
            if next_line_trimmed.is_empty() {
                joined_text.push('\n');
            } else {
                let first_word_width = next_line_trimmed
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .count();
                if trailing_spaces < first_word_width || trailing_spaces == 0 {
                    let last_char = trimmed.chars().last().unwrap_or(' ');
                    let first_next_char = next_line_trimmed.chars().next().unwrap_or(' ');
                    if trailing_spaces > 0 || (last_char.is_ascii() && first_next_char.is_ascii()) {
                        joined_text.push(' ');
                    }
                } else {
                    joined_text.push('\n');
                }
            }
        }
    }

    joined_text
}

fn effective_area(frame_area: Rect, bounds: Option<Rect>) -> Rect {
    bounds
        .map(|bounds| frame_area.intersection(bounds))
        .unwrap_or(frame_area)
}

fn clamp_to_bounds(position: Position, bounds: Option<Rect>) -> Position {
    let Some(bounds) = bounds else {
        return position;
    };

    if bounds.width == 0 || bounds.height == 0 {
        return position;
    }

    let left = bounds.x;
    let right = bounds.x.saturating_add(bounds.width).saturating_sub(1);
    let top = bounds.y;
    let bottom = bounds.y.saturating_add(bounds.height).saturating_sub(1);

    Position::new(position.x.clamp(left, right), position.y.clamp(top, bottom))
}

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

#[cfg(test)]
mod tests {
    use super::{MouseSelectionState, apply_selection_style, extract_selected_text};
    use ratatui::{
        buffer::Buffer,
        layout::{Position, Rect},
        style::{Color, Style},
    };

    #[test]
    fn selected_text_respects_row_ranges() {
        let buffer = Buffer::with_lines(["abcd", "efgh"]);
        let mut selection = MouseSelectionState::default();
        selection.press_with_bounds(Position::new(1, 0), None);
        selection.drag(Position::new(1, 1));
        selection.release(Position::new(1, 1));

        let text = selection.selected_text(&buffer).unwrap();
        assert_eq!(text, "bcd\nef");
    }

    #[test]
    fn apply_selection_style_overlays_cells() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        buffer.set_string(0, 0, "abcd", Style::default());
        buffer.set_string(0, 1, "efgh", Style::default());

        apply_selection_style(
            &mut buffer,
            super::SelectionRange::new(Position::new(1, 0), Position::new(2, 1)),
            None,
            Style::default().bg(Color::Blue).fg(Color::White),
        );

        assert_eq!(buffer.cell((1, 0)).unwrap().bg, Color::Blue);
        assert_eq!(buffer.cell((2, 1)).unwrap().fg, Color::White);
    }

    #[test]
    fn selected_text_trims_trailing_padding() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        buffer.set_string(0, 0, "hi", Style::default());

        let text = extract_selected_text(
            &buffer,
            super::SelectionRange::new(Position::new(0, 0), Position::new(5, 0)),
            None,
        );

        assert_eq!(text, "hi");
    }

    #[test]
    fn selection_bounds_isolate_middle_rows() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 3));
        buffer.set_string(0, 0, "aaaabbbb", Style::default());
        buffer.set_string(0, 1, "ccccdddd", Style::default());
        buffer.set_string(0, 2, "eeeeffff", Style::default());

        apply_selection_style(
            &mut buffer,
            super::SelectionRange::new(Position::new(1, 0), Position::new(1, 2)),
            Some(Rect::new(0, 0, 4, 3)),
            Style::default().bg(Color::Blue),
        );

        assert_eq!(buffer.cell((3, 1)).unwrap().bg, Color::Blue);
        assert_eq!(buffer.cell((6, 1)).unwrap().bg, Color::Reset);
    }
}
