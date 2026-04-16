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

    pub(crate) fn press_with_bounds(&mut self, position: Position, bounds: Option<Rect>) {
        let bounded = clamp_to_bounds(position, bounds);
        self.anchor = Some(bounded);
        self.focus = Some(bounded);
        self.pointer = Some(position);
        self.bounds = bounds;
        self.dragging = false;
        self.moved = false;
        self.pending_copy = false;
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

    pub(crate) fn release(&mut self, position: Position) {
        self.pointer = Some(position);

        if self.anchor.is_none() {
            self.clear();
            return;
        }

        self.focus = Some(clamp_to_bounds(position, self.bounds));

        if self.dragging && self.moved && self.has_selection() {
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

    pub(crate) fn has_selection(&self) -> bool {
        matches!((self.anchor, self.focus), (Some(anchor), Some(focus)) if anchor != focus)
    }

    pub(crate) fn selection_range(&self) -> Option<(Position, Position)> {
        let range = self.range()?;
        Some((range.start, range.end))
    }

    pub(crate) fn apply_overlay(&self, buffer: &mut Buffer, style: Style) {
        let Some(range) = self.range() else {
            return;
        };

        apply_selection_style(buffer, range, self.bounds, style);
    }

    pub(crate) fn selected_text(&self, buffer: &Buffer) -> Option<String> {
        let range = self.range()?;
        let text = extract_selected_text(buffer, range, self.bounds);
        Some(text)
    }

    pub(crate) fn take_pending_copy(&mut self) -> Option<(Position, Position)> {
        if !self.pending_copy {
            return None;
        }

        self.pending_copy = false;
        self.selection_range()
    }

    fn range(&self) -> Option<SelectionRange> {
        let (anchor, focus) = match (self.anchor, self.focus) {
            (Some(anchor), Some(focus)) if self.moved || anchor != focus => (anchor, focus),
            _ => return None,
        };

        Some(SelectionRange::new(anchor, focus))
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
        self.mouse_selection.apply_overlay(buffer, selection_style);

        let Some(_) = self.mouse_selection.take_pending_copy() else {
            return;
        };

        let Some(text) = self.mouse_selection.selected_text(buffer) else {
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
    style: Style,
) {
    let effective_area = effective_area(buffer.area, bounds);
    if effective_area.width == 0 || effective_area.height == 0 {
        return;
    }

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

        buffer.set_style(Rect::new(row_start, y, row_end - row_start + 1, 1), style);
    }
}

fn extract_selected_text(buffer: &Buffer, range: SelectionRange, bounds: Option<Rect>) -> String {
    let effective_area = effective_area(buffer.area, bounds);
    if effective_area.width == 0 || effective_area.height == 0 {
        return String::new();
    }

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

        lines.push(extract_row_text(buffer, y, row_start, row_end));
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    lines
        .into_iter()
        .map(|line| line.trim_end_matches(' ').to_string())
        .collect::<Vec<_>>()
        .join("\n")
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
