use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use fancy_regex::Regex;
use std::sync::LazyLock;
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

/// Regex for detecting @ file/directory references in composer text.
/// Look-behind ensures @ is not preceded by word chars or backticks.
static AT_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?<![\w`])@(\.?[^\s`.,]*(?:\.[^\s`.,]+)*)").unwrap()
});

/// Kind of an inline span in the composer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InlineSpanKind {
    /// @file/@dir reference accepted via autocomplete.
    AtReference,
    /// Image attachment placeholder (pasted from clipboard).
    Image,
}

/// A byte range in the composer text that should be rendered as an atomic
/// styled badge. The cursor skips over the entire span; backspace/delete
/// removes it as a unit.
#[derive(Clone, Debug)]
pub(crate) struct InlineSpan {
    /// Byte start index in `text` (inclusive).
    pub start: usize,
    /// Byte end index in `text` (exclusive).
    pub end: usize,
    /// Display text (identical to text content for @ refs, badge text for images).
    #[allow(dead_code)]
    pub display: String,
    /// Kind of span.
    pub kind: InlineSpanKind,
    /// Associated data URL for image spans (base64-encoded data URL).
    pub data_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Composer {
    text: String,
    cursor: usize,
    preferred_column: Option<usize>,
    visual_line_hint: Option<usize>,
    history: Vec<String>,
    history_cursor: Option<usize>,
    draft: String,
    placeholder: String,
    /// Anchor position for text selection. When Some, there's an active selection
    /// from min(anchor, cursor) to max(anchor, cursor).
    selection_anchor: Option<usize>,
    /// Atomic inline spans rendered as styled badges.
    /// Kept sorted by `start`, non-overlapping.
    pub(crate) spans: Vec<InlineSpan>,
}

impl Composer {
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            preferred_column: None,
            visual_line_hint: None,
            history: Vec::new(),
            history_cursor: None,
            draft: String::new(),
            placeholder: placeholder.into(),
            selection_anchor: None,
            spans: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<String>) {
        self.placeholder = placeholder.into();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
        self.draft.clear();
        self.selection_anchor = None;
        self.spans.clear();
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
        self.selection_anchor = None;
        self.spans.clear();
        self.detect_at_spans();
    }

    pub fn remember_submission(&mut self, submission: &str) {
        if submission.trim().is_empty() {
            self.history_cursor = None;
            self.draft.clear();
            return;
        }

        if self
            .history
            .last()
            .is_none_or(|previous| previous != submission)
        {
            self.history.push(submission.to_string());
        }

        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
        self.draft.clear();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        self.handle_key_with_history(key, true)
    }

    pub fn handle_key_with_history(
        &mut self,
        key: KeyEvent,
        record_history: bool,
    ) -> Option<String> {
        let allow_history_navigation = record_history;

        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => match c {
                'a' => {
                    // Ctrl+A: select all
                    self.select_all();
                }
                'e' => {
                    self.cursor = self.text.len();
                    self.preferred_column = None;
                    self.selection_anchor = None;
                }
                'j' => {
                    self.insert_char('\n');
                }
                'u' => {
                    self.text.clear();
                    self.cursor = 0;
                    self.preferred_column = None;
                    self.selection_anchor = None;
                    self.spans.clear();
                }
                'k' => {
                    let old_len = self.text.len();
                    self.text.truncate(self.cursor);
                    self.remove_spans_in_range(self.cursor, old_len);
                    self.preferred_column = None;
                    self.selection_anchor = None;
                }
                'n' if allow_history_navigation => {
                    self.select_next_history();
                }
                _ => {}
            },
            // Command+A on macOS (SUPER modifier)
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::SUPER) => {
                self.select_all();
            }
            // Command+E on macOS: move to end of line
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::SUPER) => {
                self.cursor = self.text.len();
                self.preferred_column = None;
                self.selection_anchor = None;
            }
            KeyCode::Char(c) => {
                self.insert_char(c);
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.insert_char('\n');
                } else {
                    let submission = self.text.trim_end().to_string();
                    if submission.is_empty() {
                        return None;
                    }

                    if record_history {
                        self.remember_submission(&submission);
                    }
                    self.clear();
                    return Some(submission);
                }
            }
            KeyCode::Backspace => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.delete_to_line_start();
                } else if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.delete_previous_word();
                } else {
                    self.delete_previous_char();
                }
            }
            KeyCode::Delete => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.delete_to_line_start();
                } else if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.delete_previous_word();
                } else {
                    self.delete_next_char();
                }
            }
            KeyCode::Left => {
                self.move_left();
            }
            KeyCode::Right => {
                self.move_right();
            }
            KeyCode::Home => {
                self.cursor = self.line_start(self.cursor);
                self.cursor = self.snap_to_span_edge(self.cursor);
                self.preferred_column = None;
                self.selection_anchor = None;
            }
            KeyCode::End => {
                self.cursor = self.line_end(self.cursor);
                self.cursor = self.snap_to_span_edge(self.cursor);
                self.preferred_column = None;
                self.selection_anchor = None;
            }
            KeyCode::Tab => {
                self.insert_str("    ");
            }
            _ => {}
        }

        None
    }

    pub fn preferred_height(&self, width: u16, max_lines: u16) -> u16 {
        let mut visible_lines = display_line_count(&self.text, width as usize) as u16;
        if self.cursor_wraps_to_next_row(width as usize) {
            visible_lines = visible_lines.saturating_add(1);
        }

        visible_lines.min(max_lines).saturating_add(2)
    }

    pub fn cursor_position(&self, width: u16) -> (u16, u16) {
        let width = width as usize;
        if width == 0 {
            return (0, 0);
        }

        let lines = visual_lines(&self.text, width);
        let cursor = self.cursor.min(self.text.len());
        let hinted_line = self.visual_line_hint.and_then(|line_index| {
            lines.get(line_index).and_then(|line| {
                if cursor >= line.start && cursor <= line.end {
                    Some(line_index)
                } else {
                    None
                }
            })
        });
        let line_index = hinted_line
            .or_else(|| {
                lines
                    .iter()
                    .enumerate()
                    .rposition(|(_, line)| line.start <= cursor)
            })
            .unwrap_or(0);
        let line = lines[line_index];
        let column = display_width(&self.text[line.start..cursor]);

        (line_index as u16, column as u16)
    }

    pub fn move_up(&mut self, width: u16) {
        self.move_vertical(width, -1);
    }

    pub fn move_down(&mut self, width: u16) {
        self.move_vertical(width, 1);
    }

    /// Compute the text position from a visual position WITHOUT snapping to
    /// span edges. Used for click detection to determine if the raw click
    /// landed inside a span.
    pub(crate) fn raw_text_position_at_visual(&self, width: u16, line: u16, column: u16) -> usize {
        let width = width as usize;
        if width == 0 {
            return 0;
        }
        let lines = visual_lines(&self.text, width);
        if lines.is_empty() {
            return 0;
        }
        let line_index = line.min(lines.len().saturating_sub(1) as u16) as usize;
        cursor_from_visual_position(&self.text, lines[line_index], column as usize)
    }

    pub fn set_cursor_at_visual_position(&mut self, width: u16, line: u16, column: u16) {
        let width = width as usize;
        if width == 0 {
            return;
        }

        let lines = visual_lines(&self.text, width);
        if lines.is_empty() {
            self.cursor = 0;
            self.preferred_column = Some(column as usize);
            return;
        }

        let line_index = line.min(lines.len().saturating_sub(1) as u16) as usize;
        let line = lines[line_index];
        let column = column as usize;
        self.cursor = snap_to_span_edge_static(&self.spans, cursor_from_visual_position(&self.text, line, column));
        self.preferred_column = Some(column);
        self.visual_line_hint = Some(line_index);
    }

    pub fn display_line_count(&self, width: usize) -> usize {
        display_line_count(&self.text, width)
    }

    pub fn cursor_wraps_to_next_row(&self, width: usize) -> bool {
        if width == 0 || self.cursor != self.text.len() {
            return false;
        }

        visual_lines(&self.text, width)
            .last()
            .is_some_and(|line| line.width == width)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    // ========== Inline Span API ==========

    /// Public accessor for spans (used by rendering code).
    pub(crate) fn spans(&self) -> &[InlineSpan] {
        &self.spans
    }

    /// Register a new inline span. Maintains sorted, non-overlapping invariant.
    pub(crate) fn register_span(
        &mut self,
        start: usize,
        end: usize,
        display: String,
        kind: InlineSpanKind,
        data_url: Option<String>,
    ) {
        let start = start.min(self.text.len());
        let end = end.min(self.text.len()).max(start);
        if start >= end {
            return;
        }
        let span = InlineSpan {
            start,
            end,
            display,
            kind,
            data_url,
        };
        // Insert sorted by start
        let pos = self.spans.partition_point(|s| s.start < span.start);
        self.spans.insert(pos, span);
    }

    /// Find the span that strictly contains `pos` (start < pos < end), if any.
    /// Edges are excluded so clicking at the boundary doesn't trigger.
    pub(crate) fn span_at(&self, pos: usize) -> Option<&InlineSpan> {
        self.spans
            .iter()
            .find(|s| pos > s.start && pos < s.end)
    }

    /// Find the span that ends exactly at `pos` (cursor is right after it).
    pub(crate) fn span_before(&self, pos: usize) -> Option<&InlineSpan> {
        self.spans.iter().find(|s| s.end == pos)
    }

    /// Find the span that starts exactly at `pos` (cursor is right before it).
    pub(crate) fn span_after(&self, pos: usize) -> Option<&InlineSpan> {
        self.spans.iter().find(|s| s.start == pos)
    }

    /// If `pos` is strictly inside a span (start < pos < end), return span.start;
    /// otherwise return `pos` unchanged.
    pub(crate) fn snap_to_span_edge(&self, pos: usize) -> usize {
        if let Some(span) = self.spans.iter().find(|s| pos > s.start && pos < s.end) {
            span.start
        } else {
            pos
        }
    }

    /// Remove any spans that overlap with the byte range `[start, end)`.
    fn remove_spans_in_range(&mut self, start: usize, end: usize) {
        self.spans.retain(|s| s.end <= start || s.start >= end);
    }

    /// Adjust all spans after a text edit at `edit_start` that replaced
    /// `old_len` bytes with `new_len` bytes.
    fn adjust_after_edit(&mut self, edit_start: usize, old_len: usize, new_len: usize) {
        let edit_end = edit_start + old_len;
        let delta = new_len as isize - old_len as isize;

        // Remove spans that overlap with the edited region
        self.spans
            .retain(|s| s.end <= edit_start || s.start >= edit_end);

        // Shift spans that start at or after the edit end
        for span in &mut self.spans {
            if span.start >= edit_end {
                span.start = (span.start as isize + delta) as usize;
                span.end = (span.end as isize + delta) as usize;
            }
        }
    }

    /// Scan the composer text with the @ reference regex and register spans
    /// for every match. Existing AtReference spans are cleared first.
    pub(crate) fn detect_at_spans(&mut self) {
        // Remove existing AtReference spans
        self.spans.retain(|s| s.kind != InlineSpanKind::AtReference);

        let text = self.text.clone();
        let mut start = 0;
        while let Some(caps) = AT_REF_RE.captures(&text[start..]).unwrap() {
            if let Some(path_match) = caps.get(1) {
                if path_match.as_str().is_empty() {
                    break;
                }
                let abs_start = start + path_match.start() - 1; // include the '@'
                let abs_end = start + path_match.end();
                let display = text[abs_start..abs_end].to_string();
                self.register_span(abs_start, abs_end, display, InlineSpanKind::AtReference, None);
                start += path_match.end();
            } else {
                break;
            }
        }
    }

    // ========== Selection API ==========

    /// Returns true if there's an active text selection.
    pub fn has_selection(&self) -> bool {
        self.selection_anchor
            .is_some_and(|anchor| anchor != self.cursor)
    }

    /// Returns the selection range as (start, end) byte indices, or None if no selection.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.and_then(|anchor| {
            if anchor == self.cursor {
                None
            } else {
                let start = anchor.min(self.cursor);
                let end = anchor.max(self.cursor);
                Some((start, end))
            }
        })
    }

    /// Returns the selected text, or None if no selection.
    pub fn selected_text(&self) -> Option<&str> {
        self.selection_range()
            .map(|(start, end)| &self.text[start..end])
    }

    /// Selects all text in the composer.
    pub fn select_all(&mut self) {
        if !self.text.is_empty() {
            self.selection_anchor = Some(0);
            self.cursor = self.text.len();
            self.preferred_column = None;
            self.visual_line_hint = None;
        }
    }

    /// Clears the current selection without changing cursor position.
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// Sets the selection anchor at the current cursor position.
    /// Used when starting a mouse drag selection.
    pub fn start_selection(&mut self) {
        self.selection_anchor = Some(self.cursor);
    }

    /// Sets the selection to a specific range.
    pub fn set_selection(&mut self, start: usize, end: usize) {
        let start = start.min(self.text.len());
        let end = end.min(self.text.len()).max(start);
        self.selection_anchor = Some(start);
        self.cursor = end;
        self.preferred_column = None;
        self.visual_line_hint = None;
    }

    /// Deletes the current selection if any, returns true if deletion occurred.
    fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection_range() {
            self.remove_spans_in_range(start, end);
            self.adjust_after_edit(start, end - start, 0);
            self.text.drain(start..end);
            self.cursor = start;
            self.selection_anchor = None;
            self.preferred_column = None;
            self.visual_line_hint = None;
            self.history_cursor = None;
            true
        } else {
            // Clear any stale zero-width selection anchor (e.g. from a mouse
            // click without drag).  If we don't clear it here, the next
            // `insert_char` will move the cursor past the anchor and make it
            // look like a real selection — causing the *next* character to
            // silently delete the one we just inserted.
            self.selection_anchor = None;
            false
        }
    }

    pub fn visual_lines(&self, width: usize) -> Vec<Range<usize>> {
        visual_lines(&self.text, width)
            .into_iter()
            .map(|l| l.start..l.end)
            .collect()
    }

    fn insert_char(&mut self, ch: char) {
        // If there's a selection, replace it with the new character
        self.delete_selection();
        // Snap cursor out of any span before inserting
        self.cursor = self.snap_to_span_edge(self.cursor);
        let insert_pos = self.cursor;
        self.text.insert(insert_pos, ch);
        self.adjust_after_edit(insert_pos, 0, ch.len_utf8());
        self.cursor += ch.len_utf8();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    pub fn insert_str(&mut self, value: &str) {
        // If there's a selection, replace it with the new text
        self.delete_selection();
        // Snap cursor out of any span before inserting
        self.cursor = self.snap_to_span_edge(self.cursor);
        let insert_pos = self.cursor;
        self.text.insert_str(insert_pos, value);
        self.adjust_after_edit(insert_pos, 0, value.len());
        self.cursor += value.len();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        let start = start.min(self.text.len());
        let end = end.min(self.text.len()).max(start);
        let old_len = end - start;
        self.remove_spans_in_range(start, end);
        self.adjust_after_edit(start, old_len, replacement.len());
        self.text.replace_range(start..end, replacement);
        self.cursor = start + replacement.len();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn delete_previous_char(&mut self) {
        // If there's a selection, delete it instead
        if self.delete_selection() {
            return;
        }

        if self.cursor == 0 {
            return;
        }

        // If cursor is strictly inside a span, snap to its start (don't delete)
        if let Some(span) = self.spans.iter().find(|s| self.cursor > s.start && self.cursor < s.end)
        {
            self.cursor = span.start;
            self.preferred_column = None;
            self.visual_line_hint = None;
            return;
        }

        // If cursor is right after a span, delete the entire span
        if let Some(span) = self.span_before(self.cursor) {
            let span_start = span.start;
            let span_end = span.end;
            self.text.drain(span_start..span_end);
            self.cursor = span_start;
            self.spans.retain(|s| s.start != span_start || s.end != span_end);
            self.adjust_after_edit(span_start, span_end - span_start, 0);
            self.preferred_column = None;
            self.visual_line_hint = None;
            self.history_cursor = None;
            return;
        }

        let previous = self.previous_char_boundary(self.cursor);
        self.text.drain(previous..self.cursor);
        self.adjust_after_edit(previous, self.cursor - previous, 0);
        self.cursor = previous;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn delete_previous_word(&mut self) {
        // If there's a selection, delete it instead
        if self.delete_selection() {
            return;
        }

        let cursor = self.cursor.min(self.text.len());
        if cursor == 0 {
            return;
        }

        // If cursor is inside or right after a span, delete the entire span
        if let Some(span) = self.spans.iter().find(|s| cursor >= s.start && cursor <= s.end) {
            let span_start = span.start;
            let span_end = span.end;
            self.text.drain(span_start..span_end);
            self.cursor = span_start;
            self.spans.retain(|s| s.start != span_start || s.end != span_end);
            self.adjust_after_edit(span_start, span_end - span_start, 0);
            self.preferred_column = None;
            self.visual_line_hint = None;
            self.history_cursor = None;
            return;
        }

        let mut boundary = cursor;
        while boundary > 0 {
            let previous = self.previous_char_boundary(boundary);
            // `previous` is always <= boundary; when text is non-empty and
            // boundary > 0 there is guaranteed to be at least one character
            // in `previous..boundary`, but we guard defensively anyway.
            if previous >= boundary {
                boundary = previous;
                break;
            }
            let ch = match self.text[previous..boundary].chars().next() {
                Some(c) => c,
                None => break,
            };
            if !ch.is_whitespace() {
                break;
            }
            boundary = previous;
        }

        while boundary > 0 {
            let previous = self.previous_char_boundary(boundary);
            if previous >= boundary {
                boundary = previous;
                break;
            }
            let ch = match self.text[previous..boundary].chars().next() {
                Some(c) => c,
                None => break,
            };
            if ch.is_whitespace() {
                break;
            }
            boundary = previous;
        }

        // Guard against boundary exceeding text length (defensive)
        let boundary = boundary.min(self.text.len());
        let old_len = cursor - boundary;
        self.text.drain(boundary..cursor);
        self.adjust_after_edit(boundary, old_len, 0);
        self.cursor = boundary;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn delete_to_line_start(&mut self) {
        // If there's a selection, delete it instead
        if self.delete_selection() {
            return;
        }

        if self.cursor == 0 {
            return;
        }

        let start = self.line_start(self.cursor);
        let old_len = self.cursor - start;
        self.remove_spans_in_range(start, self.cursor);
        self.adjust_after_edit(start, old_len, 0);
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn delete_next_char(&mut self) {
        // If there's a selection, delete it instead
        if self.delete_selection() {
            return;
        }

        if self.cursor >= self.text.len() {
            return;
        }

        // If cursor is strictly inside a span, snap to its end (don't delete)
        if let Some(span) = self.spans.iter().find(|s| self.cursor >= s.start && self.cursor < s.end)
        {
            self.cursor = span.end;
            self.preferred_column = None;
            self.visual_line_hint = None;
            return;
        }

        // If cursor is right before a span, delete the entire span
        if let Some(span) = self.span_after(self.cursor) {
            let span_start = span.start;
            let span_end = span.end;
            self.text.drain(span_start..span_end);
            self.cursor = span_start;
            self.spans.retain(|s| s.start != span_start || s.end != span_end);
            self.adjust_after_edit(span_start, span_end - span_start, 0);
            self.preferred_column = None;
            self.visual_line_hint = None;
            self.history_cursor = None;
            return;
        }

        let next = self.next_char_boundary(self.cursor);
        self.text.drain(self.cursor..next);
        self.adjust_after_edit(self.cursor, next - self.cursor, 0);
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn move_left(&mut self) {
        let new_pos = self.previous_char_boundary(self.cursor);
        // If we landed inside a span, jump to its start
        self.cursor = if new_pos < self.cursor {
            self.snap_to_span_edge(new_pos)
        } else {
            new_pos
        };
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.selection_anchor = None; // Clear selection on cursor movement
    }

    fn move_right(&mut self) {
        let new_pos = self.next_char_boundary(self.cursor);
        // If we landed inside a span, jump to its end
        self.cursor = if let Some(span) = self.spans.iter().find(|s| new_pos > s.start && new_pos < s.end) {
            span.end
        } else {
            new_pos
        };
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.selection_anchor = None; // Clear selection on cursor movement
    }

    fn select_next_history(&mut self) {
        let Some(index) = self.history_cursor else {
            return;
        };

        if index + 1 < self.history.len() {
            self.history_cursor = Some(index + 1);
            self.text = self.history[index + 1].clone();
        } else {
            self.history_cursor = None;
            self.text = self.draft.clone();
        }

        self.cursor = self.text.len();
        self.preferred_column = None;
        self.visual_line_hint = None;
    }

    fn previous_char_boundary(&self, index: usize) -> usize {
        if index == 0 {
            return 0;
        }

        self.text
            .char_indices()
            .take_while(|(byte_index, _)| *byte_index < index)
            .map(|(byte_index, _)| byte_index)
            .last()
            .unwrap_or(0)
    }

    fn next_char_boundary(&self, index: usize) -> usize {
        if index >= self.text.len() {
            return self.text.len();
        }

        self.text[index..]
            .char_indices()
            .nth(1)
            .map(|(relative_index, _)| index + relative_index)
            .unwrap_or(self.text.len())
    }

    fn line_start(&self, index: usize) -> usize {
        self.text[..index]
            .rfind('\n')
            .map(|position| position + 1)
            .unwrap_or(0)
    }

    fn line_end(&self, index: usize) -> usize {
        self.text[index..]
            .find('\n')
            .map(|position| index + position)
            .unwrap_or(self.text.len())
    }

    fn move_vertical(&mut self, width: u16, delta: isize) {
        let width = width as usize;
        if width == 0 || delta == 0 {
            return;
        }

        let lines = visual_lines(&self.text, width);
        let (current_line, current_column) = self.cursor_position(width as u16);
        let desired_column = self.preferred_column.unwrap_or(current_column as usize);
        let last_line = lines.len().saturating_sub(1) as isize;
        let target_line = (current_line as isize + delta).clamp(0, last_line) as usize;

        if target_line == current_line as usize {
            self.preferred_column = Some(desired_column);
            self.visual_line_hint = Some(target_line);
            return;
        }

        self.cursor = snap_to_span_edge_static(
            &self.spans,
            cursor_from_visual_position(&self.text, lines[target_line], desired_column),
        );
        self.preferred_column = Some(desired_column);
        self.visual_line_hint = Some(target_line);
        self.selection_anchor = None; // Clear selection on vertical movement
    }
}

fn display_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.split('\n').count().max(1);
    }

    visual_lines(text, width).len().max(1)
}

#[derive(Clone, Copy, Debug)]
struct VisualLine {
    start: usize,
    end: usize,
    width: usize,
}

fn visual_lines(text: &str, width: usize) -> Vec<VisualLine> {
    if width == 0 {
        return vec![VisualLine {
            start: 0,
            end: text.len(),
            width: 0,
        }];
    }

    let mut lines = Vec::new();
    let mut line_start = 0usize;
    let mut current_width = 0usize;

    for (byte_index, ch) in text.char_indices() {
        if ch == '\n' {
            lines.push(VisualLine {
                start: line_start,
                end: byte_index,
                width: current_width,
            });
            line_start = byte_index + ch.len_utf8();
            current_width = 0;
            continue;
        }

        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width > 0 && current_width + char_width > width {
            lines.push(VisualLine {
                start: line_start,
                end: byte_index,
                width: current_width,
            });
            line_start = byte_index;
            current_width = 0;
        }

        current_width += char_width;
    }

    lines.push(VisualLine {
        start: line_start,
        end: text.len(),
        width: current_width,
    });

    lines
}

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// Standalone version of snap_to_span_edge that takes a span slice.
/// If `pos` is strictly inside a span, returns `span.start`; otherwise `pos`.
fn snap_to_span_edge_static(spans: &[InlineSpan], pos: usize) -> usize {
    if let Some(span) = spans.iter().find(|s| pos > s.start && pos < s.end) {
        span.start
    } else {
        pos
    }
}

fn cursor_from_visual_position(text: &str, line: VisualLine, target_column: usize) -> usize {
    let mut current_column = 0usize;
    let mut cursor = line.start;

    for (relative_index, ch) in text[line.start..line.end].char_indices() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        let next_cursor = line.start + relative_index + ch.len_utf8();

        if char_width == 0 {
            cursor = next_cursor;
            continue;
        }

        if target_column <= current_column {
            return cursor;
        }

        if target_column < current_column + char_width {
            if (target_column - current_column) * 2 < char_width {
                return cursor;
            }

            return next_cursor;
        }

        current_column += char_width;
        cursor = next_cursor;
    }

    let _ = line.width;
    line.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_j_inserts_newline() {
        let mut composer = Composer::new("placeholder");

        let result = composer.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));

        assert!(result.is_none());
        assert_eq!(composer.text(), "\n");
    }

    #[test]
    fn preferred_height_counts_trailing_newline() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello\n".to_string());

        assert_eq!(composer.preferred_height(10, 10), 4);
    }

    #[test]
    fn preferred_height_wraps_long_lines() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("abcdefghij".to_string());

        assert_eq!(composer.preferred_height(4, 10), 5);
    }

    #[test]
    fn preferred_height_adds_cursor_row_for_full_width_trailing_line() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("abcd".to_string());

        assert_eq!(composer.preferred_height(4, 10), 4);
        assert!(composer.cursor_wraps_to_next_row(4));
    }

    #[test]
    fn cursor_position_wraps_long_lines() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("abcdefg".to_string());
        composer.replace_range(0, 7, "abcdefg");
        composer.cursor = 7;

        assert_eq!(composer.cursor_position(4), (1, 3));
    }

    #[test]
    fn cursor_position_tracks_mixed_width_wraps() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("ab中文cd".to_string());
        composer.cursor = composer.text().len();

        assert_eq!(composer.cursor_position(4), (1, 4));
    }

    #[test]
    fn vertical_movement_preserves_visual_column() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("ab中文cd".to_string());
        composer.cursor = composer.text().len();

        composer.move_up(4);
        assert_eq!(composer.cursor_position(4), (0, 4));

        composer.move_down(4);
        assert_eq!(composer.cursor_position(4), (1, 4));
    }

    #[test]
    fn visual_position_mapping_follows_wrapped_lines() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("ab中文cd".to_string());

        composer.set_cursor_at_visual_position(4, 0, 2);
        assert_eq!(composer.cursor_position(4), (0, 2));

        composer.set_cursor_at_visual_position(4, 1, 4);
        assert_eq!(composer.cursor_position(4), (1, 4));
    }

    #[test]
    fn ctrl_backspace_deletes_previous_word() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));

        assert!(result.is_none());
        assert_eq!(composer.text(), "hello ");
        assert_eq!(composer.cursor(), 6);
    }

    #[test]
    fn alt_backspace_deletes_previous_word() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));

        assert!(result.is_none());
        assert_eq!(composer.text(), "hello ");
        assert_eq!(composer.cursor(), 6);
    }

    #[test]
    fn super_backspace_deletes_to_line_start() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER));

        assert!(result.is_none());
        assert_eq!(composer.text(), "");
        assert_eq!(composer.cursor(), 0);
    }

    #[test]
    fn ctrl_backspace_skips_trailing_whitespace() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world   ".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));

        assert!(result.is_none());
        assert_eq!(composer.text(), "hello ");
        assert_eq!(composer.cursor(), 6);
    }
}
