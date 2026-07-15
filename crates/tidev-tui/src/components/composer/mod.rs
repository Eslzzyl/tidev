//! Composer — the text input component for composing messages.
//!
//! Owns the text buffer, cursor, selection, history, inline spans, and
//! all associated autocomplete states (command palette, @-mention, snippet).
//!
//! ## Architecture
//!
//! The Computer itself (this module) handles text-editing logic.  It delegates
//! rendering to [`render`] and embeds three inline-autocomplete sub-systems
//! that are drawn as popups above the input area:
//!
//! * [`CommandPalette`] — /command suggestions
//! * [`at_mention`]     — @file path autocomplete  
//! * [`snippet`]        — Text snippet insertion

pub(crate) mod at_mention;
pub(crate) mod command_palette;
pub(crate) mod render;
pub(crate) mod snippet;



use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::Frame;
use tidev_search::current_at_fragment;
use unicode_width::UnicodeWidthChar;



use tidev_types::message::MessageAttachment;
use crate::action::{Action, ChatAction};
use crate::component::Component;
use crate::context::DrawContext;

pub(crate) use at_mention::AtMentionState;
pub(crate) use command_palette::{CommandPaletteState, CommandRegistry};
pub(crate) use snippet::SnippetState;

// ---------------------------------------------------------------------------
// InlineSpan
// ---------------------------------------------------------------------------

/// A byte-range in the composer text that renders as an atomic styled badge.
///
/// The cursor skips over the entire span; backspace/delete removes it as a
/// unit.  Spans are kept sorted by `start` and non-overlapping.
#[derive(Clone, Debug)]
pub(crate) struct InlineSpan {
    /// Byte start index in `text` (inclusive).
    pub start: usize,
    /// Byte end index in `text` (exclusive).
    pub end: usize,
    /// Raw image bytes (None for non-image spans).
    pub image_data: Option<Vec<u8>>,
    /// Display filename for the image (None for non-image spans).
    pub image_filename: Option<String>,
}

// ---------------------------------------------------------------------------
// VisualLine (cached per (text, width))
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct VisualLine {
    start: usize,
    end: usize,
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

pub(crate) struct Composer {
    /// The raw text buffer.
    text: String,
    /// Byte-offset cursor position.
    cursor: usize,
    /// Preferred visual column for vertical navigation (up/down).
    preferred_column: Option<usize>,
    /// Cached visual-line hint (used during vertical movement).
    visual_line_hint: Option<usize>,

    /// Sent-message history (oldest first, newest last).
    history: Vec<String>,
    /// Index into `history` for reverse-search (`None` = at the bottom / draft).
    history_cursor: Option<usize>,
    /// The draft text being composed when history was first entered.
    draft: String,

    /// Placeholder text shown when the buffer is empty.
    placeholder: String,

    /// Selection anchor.  When `Some`, there is an active selection from
    /// `min(anchor, cursor)` to `max(anchor, cursor)`.
    selection_anchor: Option<usize>,

    /// Atomic inline spans (@-refs, images).  Kept sorted by `start`.
    spans: Vec<InlineSpan>,

    /// Dirty flag.
    dirty: bool,

    // ── Inline autocomplete subsystems ──────────────────────────────

    /// /command suggestion popup.
    command_palette: CommandPaletteState,
    /// Command registry for fuzzy matching.
    commands: CommandRegistry,
    /// @file path autocomplete.
    at_mention: AtMentionState,
    /// Text snippet insertion.
    snippet_state: SnippetState,
    /// Workspace root (set via init or setter).
    workspace_root: PathBuf,
    /// Config directory (for snippet loading).
    config_dir: PathBuf,
    /// Whether the current model supports image attachments.
    model_supports_images: bool,
    /// Scroll offset for multi-line input area.
    pub(crate) input_scroll_offset: usize,
    /// Last input area width (set during draw, used by keyboard handler).
    last_input_width: u16,
    /// Last visible line count in the input area (set during draw).
    last_visible_lines: usize,
    /// Screen rect of the text input area (set during draw, used for mouse
    /// hit-testing on image badges and mouse interaction).
    pub(crate) last_text_area: Rect,
    /// Whether a mouse drag is in progress in the input area.
    input_dragging: bool,
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
            dirty: true,
            command_palette: CommandPaletteState::new(),
            commands: CommandRegistry::new(),
            at_mention: AtMentionState::new(),
            snippet_state: SnippetState::new(),
            workspace_root: PathBuf::new(),
            config_dir: PathBuf::new(),
            model_supports_images: false,
            input_scroll_offset: 0,
            last_input_width: 0,
            last_visible_lines: 0,
            last_text_area: Rect::default(),
            input_dragging: false,
        }
    }

    // ── Accessors ───────────────────────────────────────────────────────

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Whether any autocomplete popup (command palette, @-mention, or snippet) is visible.
    pub fn has_popup(&self) -> bool {
        self.command_palette.visible || self.at_mention.visible || self.snippet_state.visible
    }

    pub fn spans(&self) -> &[InlineSpan] {
        &self.spans
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.and_then(|anchor| {
            if anchor == self.cursor {
                None
            } else {
                Some((anchor.min(self.cursor), anchor.max(self.cursor)))
            }
        })
    }

    /// Replace the entire content (used by SetInput).
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.selection_anchor = None;
        self.spans.clear();
        self.command_palette.sync(&self.text, &self.commands);
        self.at_mention
            .sync(&self.workspace_root, &self.text, self.cursor);
        self.snippet_state.sync(
            &self.workspace_root,
            &self.config_dir,
            &self.text,
            self.cursor,
        );
        self.dirty = true;
    }

    /// Empty the buffer.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
        self.draft.clear();
        self.selection_anchor = None;
        self.spans.clear();
        self.command_palette.clear();
        self.at_mention.clear();
        self.snippet_state.clear();
        self.dirty = true;
    }

    /// Set the file search index for @mention autocomplete.
    pub fn set_file_search_index(&mut self, index: Arc<tidev_search::FileSearchIndex>) {
        self.at_mention.set_index(index);
    }

    /// Set the workspace root (for @mention path resolution).
    pub fn set_workspace_root(&mut self, root: PathBuf) {
        self.workspace_root = root;
    }

    /// Set the config directory (for snippet loading).
    pub fn set_config_dir(&mut self, dir: PathBuf) {
        self.config_dir = dir;
    }

    /// Update whether the current model supports image attachments.
    pub fn set_model_supports_images(&mut self, supports: bool) {
        self.model_supports_images = supports;
    }

    /// Record a submission in history.
    pub fn remember_submission(&mut self, submission: &str) {
        if submission.trim().is_empty() {
            self.history_cursor = None;
            self.draft.clear();
            return;
        }
        if self.history.last().is_none_or(|last| last != submission) {
            self.history.push(submission.to_string());
        }
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
        self.draft.clear();
        self.command_palette.clear();
    }

    /// Register an inline span.  Spans must not overlap and are kept sorted.
    /// `image_data` and `image_filename` are `None` for non-image spans.
    pub fn register_span(
        &mut self,
        start: usize,
        end: usize,
        image_data: Option<Vec<u8>>,
        image_filename: Option<String>,
    ) {
        // Remove any existing span that overlaps the new range.
        self.spans.retain(|s| s.end <= start || s.start >= end);
        self.spans.push(InlineSpan { start, end, image_data, image_filename });
        self.spans.sort_by_key(|s| s.start);
    }

    // ── Key handling ────────────────────────────────────────────────────

    /// Handle a keyboard event.  Returns `Some(text)` when the user submits.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            // ── Ctrl-combos ─────────────────────────────────────────────
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => match c {
                'a' => {
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
                'n' => {
                    self.select_next_history();
                }
                'p' => {
                    self.select_prev_history();
                }
                _ => {}
            },

            // ── macOS Cmd-combos ────────────────────────────────────────
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::SUPER) => {
                self.select_all();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::SUPER) => {
                self.cursor = self.text.len();
                self.preferred_column = None;
                self.selection_anchor = None;
            }

            // ── Regular character ───────────────────────────────────────
            KeyCode::Char(c) => {
                self.insert_char(c);
            }

            // ── Enter / Submit ──────────────────────────────────────────
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
                    self.remember_submission(&submission);
                    self.clear();
                    return Some(submission);
                }
            }

            // ── Backspace ───────────────────────────────────────────────
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

            // ── Delete ──────────────────────────────────────────────────
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

            // ── Navigation ──────────────────────────────────────────────
            KeyCode::Left => {
                self.move_left();
            }
            KeyCode::Right => {
                self.move_right();
            }
            KeyCode::Up => {
                // Handled by Component::handle_key_event for history vs vertical.
            }
            KeyCode::Down => {
                // Handled by Component::handle_key_event for history vs vertical.
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

            // ── Tab ─────────────────────────────────────────────────────
            KeyCode::Tab => {
                self.insert_str("    ");
            }

            _ => {}
        }

        self.dirty = true;
        None
    }

    // ── History ─────────────────────────────────────────────────────────

    pub fn select_prev_history(&mut self) {
        if self.history.is_empty() {
            return;
        }

        if self.history_cursor.is_none() {
            // Entering history: save draft
            self.draft = self.text.clone();
            self.history_cursor = self.history.len().checked_sub(1);
        } else if let Some(index) = self.history_cursor
            && index > 0 {
                self.history_cursor = Some(index - 1);
            }

        if let Some(index) = self.history_cursor
            && index < self.history.len() {
                self.text = self.history[index].clone();
            }
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.dirty = true;
    }

    pub fn select_next_history(&mut self) {
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
        self.dirty = true;
    }

    // ── Selection ───────────────────────────────────────────────────────

    pub fn select_all(&mut self) {
        if !self.text.is_empty() {
            self.selection_anchor = Some(0);
            self.cursor = self.text.len();
            self.preferred_column = None;
            self.visual_line_hint = None;
        }
    }

    // ── Visual line helpers ─────────────────────────────────────────────

    /// Return the number of visual lines the text occupies at a given width.
    pub fn display_line_count(&self, width: usize) -> usize {
        if width == 0 {
            return self.text.split('\n').count().max(1);
        }
        let lines = compute_visual_lines(&self.text, width);
        lines.len().max(1)
    }

    /// Compute visual lines without caching (read-only path).
    fn compute_visual_lines(&self, width: usize) -> Vec<VisualLine> {
        compute_visual_lines(&self.text, width)
    }

    /// Public accessor for render.rs — returns `Range<usize>` slices.
    /// Return the (line_index, column) of the cursor at a given width.
    pub fn cursor_position(&self, width: u16) -> (u16, u16) {
        let width = width as usize;
        if width == 0 {
            return (0, 0);
        }

        let lines = self.compute_visual_lines(width);
        let cursor = self.cursor.min(self.text.len());

        // Check visual_line_hint first (used during vertical navigation).
        let line_index = self
            .visual_line_hint
            .and_then(|hinted| {
                lines.get(hinted).and_then(|line| {
                    if cursor >= line.start && cursor <= line.end {
                        Some(hinted)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                lines.iter().enumerate().rposition(|(_, line)| line.start <= cursor)
            })
            .unwrap_or(0);

        let line = &lines[line_index];
        let column = display_width(&self.text[line.start..cursor]);

        (line_index as u16, column as u16)
    }

    /// Return the span that contains the given byte position, if any.
    pub fn span_at(&self, pos: usize) -> Option<&InlineSpan> {
        self.spans.iter().find(|s| s.start <= pos && pos < s.end)
    }

    /// Convert a visual (line, column) pair to a raw byte position in the text
    /// buffer.  The inverse of `cursor_position`.
    pub fn raw_text_position_at_visual(&self, width: u16, line: u16, column: u16) -> usize {
        let width = width as usize;
        if width == 0 {
            return 0;
        }
        let lines = self.compute_visual_lines(width);
        if lines.is_empty() {
            return 0;
        }
        let line_index = (line as usize).min(lines.len().saturating_sub(1));
        let vl = lines[line_index];
        let mut col: usize = 0;
        for (i, c) in self.text[vl.start..vl.end].char_indices() {
            if col >= column as usize {
                return vl.start + i;
            }
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        }
        vl.end
    }

    /// Preferred height in terminal rows for a given input width.
    pub fn preferred_height(&self, width: u16, max_lines: u16) -> u16 {
        let mut visible_lines = self.display_line_count(width as usize) as u16;
        if self.cursor_wraps_to_next_row(width as usize) {
            visible_lines = visible_lines.saturating_add(1);
        }
        // Add 2 rows for vertical padding (top/bottom margins).
        visible_lines.min(max_lines).saturating_add(2)
    }

    /// Whether the cursor visually wraps to a new empty row (blinking on a
    /// blank line below all content).
    pub fn cursor_wraps_to_next_row(&self, width: usize) -> bool {
        if self.text.is_empty() || width == 0 {
            return false;
        }
        let lines = self.compute_visual_lines(width);
        if lines.is_empty() {
            return false;
        }
        let last = &lines[lines.len() - 1];
        let end_col = display_width(&self.text[last.start..last.end]);
        self.cursor >= last.end && end_col == width
    }

    /// Move the cursor up one visual line.
    pub fn move_up(&mut self, width: u16) {
        self.move_vertical(width as usize, -1);
    }

    /// Move the cursor down one visual line.
    pub fn move_down(&mut self, width: u16) {
        self.move_vertical(width as usize, 1);
    }

    fn move_vertical(&mut self, width: usize, delta: isize) {
        if width == 0 || delta == 0 {
            return;
        }
        let lines = self.compute_visual_lines(width);
        let (current_line, current_column) = self.cursor_position(width as u16);
        let desired = self.preferred_column.unwrap_or(current_column as usize);
        let last_line = lines.len().saturating_sub(1) as isize;
        let target = (current_line as isize + delta).clamp(0, last_line) as usize;

        if target == current_line as usize {
            self.preferred_column = Some(desired);
            self.visual_line_hint = Some(target);
            return;
        }

        self.cursor = snap_to_span_edge_static(
            &self.spans,
            cursor_from_visual_position(&self.text, lines[target], desired),
        );
        self.preferred_column = Some(desired);
        self.visual_line_hint = Some(target);
        self.selection_anchor = None;
        self.dirty = true;
    }

    // ── Editing primitives ──────────────────────────────────────────────

    fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        self.cursor = self.snap_to_span_edge(self.cursor);
        let pos = self.cursor;
        self.text.insert(pos, ch);
        self.adjust_after_edit(pos, 0, ch.len_utf8());
        self.cursor = pos + ch.len_utf8();
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    pub fn insert_str(&mut self, value: &str) {
        self.delete_selection();
        self.cursor = self.snap_to_span_edge(self.cursor);
        let pos = self.cursor;
        self.text.insert_str(pos, value);
        self.adjust_after_edit(pos, 0, value.len());
        self.cursor = pos + value.len();
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
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        // If cursor is strictly inside a span, snap to its start.
        let inside_span = self
            .spans
            .iter()
            .find(|s| self.cursor > s.start && self.cursor < s.end)
            .map(|s| s.start);
        if let Some(start) = inside_span {
            self.cursor = start;
            self.preferred_column = None;
            self.visual_line_hint = None;
            self.history_cursor = None;
            return;
        }
        // If cursor is right after a span, delete the entire span.
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
        let prev = self.previous_char_boundary(self.cursor);
        let deleted = self.cursor - prev;
        self.text.drain(prev..self.cursor);
        self.adjust_after_edit(prev, deleted, 0);
        self.cursor = prev;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn delete_next_char(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        // If cursor is strictly inside a span, snap to its end.
        let inside_span = self
            .spans
            .iter()
            .find(|s| self.cursor >= s.start && self.cursor < s.end)
            .map(|s| s.end);
        if let Some(end) = inside_span {
            self.cursor = end;
            self.preferred_column = None;
            self.visual_line_hint = None;
            self.history_cursor = None;
            return;
        }
        // If cursor is right before a span, delete the entire span.
        let after = self.span_after(self.cursor).map(|s| (s.start, s.end));
        if let Some((start, end)) = after {
            self.text.drain(start..end);
            self.cursor = start;
            self.spans.retain(|s| s.start != start);
            self.adjust_after_edit(start, end - start, 0);
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

    fn delete_previous_word(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = find_word_boundary(&self.text, self.cursor, -1);
        let old_cursor = self.cursor;
        self.remove_spans_in_range(start, self.cursor);
        self.adjust_after_edit(start, old_cursor - start, 0);
        self.text.drain(start..old_cursor);
        self.cursor = start;
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.history_cursor = None;
    }

    fn delete_to_line_start(&mut self) {
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

    /// Delete the current selection, returning true if anything was deleted.
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

    // ── Navigation ──────────────────────────────────────────────────────

    fn move_left(&mut self) {
        let new_pos = self.previous_char_boundary(self.cursor);
        self.cursor = if new_pos < self.cursor {
            self.snap_to_span_edge(new_pos)
        } else {
            new_pos
        };
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.selection_anchor = None;
    }

    fn move_right(&mut self) {
        let new_pos = self.next_char_boundary(self.cursor);
        self.cursor = if let Some(span) = self
            .spans
            .iter()
            .find(|s| new_pos > s.start && new_pos < s.end)
        {
            span.end
        } else {
            new_pos
        };
        self.preferred_column = None;
        self.visual_line_hint = None;
        self.selection_anchor = None;
        self.history_cursor = None;
    }

    // ── Span management ─────────────────────────────────────────────────

    /// Snap the cursor out of a span boundary: if `pos` is strictly inside a
    /// span, return the span's start; otherwise return `pos`.
    fn snap_to_span_edge(&self, pos: usize) -> usize {
        if let Some(span) = self
            .spans
            .iter()
            .find(|s| pos > s.start && pos < s.end)
        {
            span.start
        } else {
            pos
        }
    }

    /// Return the span that contains the given byte position, if any.
    /// Find the span that ends exactly at `pos` (cursor is right after it).
    fn span_before(&self, pos: usize) -> Option<&InlineSpan> {
        self.spans.iter().find(|s| s.end == pos)
    }

    /// Find the span that starts exactly at `pos` (cursor is right before it).
    fn span_after(&self, pos: usize) -> Option<&InlineSpan> {
        self.spans.iter().find(|s| s.start == pos)
    }

    /// Remove all spans that overlap the byte range `[start, end)`.
    fn remove_spans_in_range(&mut self, start: usize, end: usize) {
        self.spans.retain(|s| s.end <= start || s.start >= end);
    }

    /// Adjust span positions after an edit at `offset` where `old_len`
    /// bytes were replaced with `new_len` bytes.
    fn adjust_after_edit(&mut self, offset: usize, old_len: usize, new_len: usize) {
        let delta = new_len as isize - old_len as isize;
        if delta == 0 {
            return;
        }
        for span in self.spans.iter_mut() {
            if span.start >= offset {
                span.start = span.start.wrapping_add_signed(delta);
                span.end = span.end.wrapping_add_signed(delta);
            } else if span.end > offset && old_len > 0 {
                // Span overlaps the edited region — trim from the end.
                span.end = offset.min(span.end);
            }
        }
    }

    // ── Char boundary helpers ───────────────────────────────────────────

    fn previous_char_boundary(&self, index: usize) -> usize {
        if index == 0 {
            return 0;
        }
        self.text
            .char_indices()
            .take_while(|(i, _)| *i < index)
            .map(|(i, _)| i)
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
            .map(|(rel, _)| index + rel)
            .unwrap_or(self.text.len())
    }

    fn line_start(&self, index: usize) -> usize {
        self.text[..index]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    }

    fn line_end(&self, index: usize) -> usize {
        self.text[index..]
            .find('\n')
            .map(|pos| index + pos)
            .unwrap_or(self.text.len())
    }

    /// Refresh command palette, @mention, and snippet states after input change.
    pub(crate) fn sync_autocomplete(&mut self) {
        self.command_palette.sync(&self.text, &self.commands);
        self.at_mention
            .sync(&self.workspace_root, &self.text, self.cursor);

        // Snippets are lower priority: suppress when command palette or @-mention is active.
        if self.command_palette.visible || self.at_mention.visible {
            self.snippet_state.clear();
        } else {
            self.snippet_state.sync(
                &self.workspace_root,
                &self.config_dir,
                &self.text,
                self.cursor,
            );
        }
    }

    /// Accept the currently selected @mention suggestion and insert it as an
    /// atomic inline span.
    fn accept_at_mention(&mut self) {
        let text = self.text.clone();
        let cursor = self.cursor;
        let Some((start, _query)) = current_at_fragment(&text, cursor) else {
            self.at_mention.clear();
            return;
        };
        let Some(selection) = self.at_mention.selected().cloned() else {
            self.at_mention.clear();
            return;
        };

        let replacement = match selection.kind {
            at_mention::AtMentionKind::Directory => {
                format!("@{}/", selection.path.trim_end_matches('/'))
            }
            _ => format!("@{}", selection.path),
        };

        self.replace_range(start, cursor, &replacement);
        let span_end = self.cursor;
        self.register_span(
            start,
            span_end,
            None,
            None,
        );
        self.at_mention.clear();
        self.sync_autocomplete();
    }

    /// Accept the currently selected snippet and insert it.
    fn accept_snippet(&mut self) {
        let Some(completion) = self.snippet_state.apply_completion() else {
            self.snippet_state.clear();
            return;
        };

        let cursor = self.cursor;
        let query = self.snippet_state.query.clone();

        // The query length in bytes corresponds to the word start offset.
        let actual_start = cursor.saturating_sub(query.len());

        self.replace_range(actual_start, cursor, &completion);
        self.snippet_state.clear();
        self.sync_autocomplete();
    }
    /// Ensure the cursor is visible by adjusting the scroll offset.
    fn ensure_input_cursor_visible(&mut self) {
        let width = self.last_input_width as usize;
        if width == 0 {
            return;
        }
        let visible_lines = self.last_visible_lines;
        let (cursor_line, _) = self.cursor_position(self.last_input_width);
        let cursor_line = cursor_line as usize;
        let total_lines = self.display_line_count(width);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        if cursor_line < self.input_scroll_offset {
            self.input_scroll_offset = cursor_line;
        } else if cursor_line >= self.input_scroll_offset + visible_lines {
            self.input_scroll_offset = (cursor_line + 1).saturating_sub(visible_lines);
        }

        self.input_scroll_offset = self.input_scroll_offset.min(max_scroll);
    }

    // ── Mouse interaction (mirrors old TUI behaviour) ─────────────────

    /// Handle mouse down in the input text area.
    pub(crate) fn handle_mouse_down(&mut self, position: Position, text_area: Rect) {
        let scroll = self.input_scroll_offset as u16;
        let local_y = position.y.saturating_sub(text_area.y);
        let local_x = position.x.saturating_sub(text_area.x);
        let target_line = scroll.saturating_add(local_y);
        let raw_pos = self.raw_text_position_at_visual(text_area.width, target_line, local_x);
        self.cursor = raw_pos;
        self.selection_anchor = Some(raw_pos);
        self.input_dragging = true;
        self.dirty = true;
    }

    /// Handle mouse drag in the input text area (extends selection).
    pub(crate) fn handle_mouse_drag(&mut self, position: Position, text_area: Rect) {
        if !self.input_dragging {
            return;
        }
        let scroll = self.input_scroll_offset as u16;
        let local_y = position
            .y
            .clamp(text_area.y, text_area.y + text_area.height.saturating_sub(1))
            .saturating_sub(text_area.y);
        let local_x = position.x.saturating_sub(text_area.x);
        let target_line = scroll.saturating_add(local_y);
        let raw_pos = self.raw_text_position_at_visual(text_area.width, target_line, local_x);
        self.cursor = raw_pos;
        self.dirty = true;
    }

    /// Handle mouse up in the input text area.
    /// Returns the selected text (for auto-copy), or None.
    pub(crate) fn handle_mouse_up(&mut self, _position: Position) -> Option<String> {
        if !self.input_dragging {
            return None;
        }
        self.input_dragging = false;
        let selected = self
            .selection_range()
            .map(|(start, end)| self.text[start..end].to_string())
            .filter(|s| !s.is_empty());
        self.dirty = true;
        selected
    }

    /// Whether a mouse drag is active in the input area.
    pub(crate) fn is_input_dragging(&self) -> bool {
        self.input_dragging
    }

    /// Scroll input area up by one visual line (mouse wheel).
    pub(crate) fn handle_mouse_scroll_up(&mut self) {
        if self.input_scroll_offset > 0 {
            self.input_scroll_offset -= 1;
            self.dirty = true;
        }
    }

    /// Scroll input area down by one visual line (mouse wheel).
    pub(crate) fn handle_mouse_scroll_down(&mut self, width: u16, visible_lines: u16) {
        let total_lines = self.display_line_count(width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines as usize);
        if self.input_scroll_offset < max_scroll {
            self.input_scroll_offset += 1;
            self.dirty = true;
        }
    }

    /// Per-frame auto-scroll during mouse drag near top/bottom edges.
    /// Returns true if auto-scroll was performed.
    pub(crate) fn update_drag_auto_scroll(&mut self, pointer: Position, text_area: Rect) -> bool {
        if !self.input_dragging {
            return false;
        }
        let visible_lines = text_area.height as usize;
        let total_lines = self.display_line_count(text_area.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        if pointer.y < text_area.y && self.input_scroll_offset > 0 {
            self.input_scroll_offset -= 1;
            self.dirty = true;
            return true;
        }
        if pointer.y >= text_area.y.saturating_add(text_area.height.saturating_sub(1))
            && self.input_scroll_offset < max_scroll
        {
            self.input_scroll_offset += 1;
            self.dirty = true;
            return true;
        }
        false
    }
}

// =====================================================================
// Component trait implementation
// =====================================================================

impl Component for Composer {
    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn mark_clean(&mut self) {
        self.dirty = false;
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        // ── Command palette visible — hijack navigation keys ────────
        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc => {
                    self.command_palette.clear();
                    self.dirty = true;
                    return None;
                }
                KeyCode::Up => {
                    self.command_palette.move_selection(-1);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Down => {
                    self.command_palette.move_selection(1);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Tab => {
                    if let Some(completion) = self.command_palette.completion() {
                        self.set_text(completion);
                    }
                    self.command_palette.sync(&self.text, &self.commands);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(selected) = self.command_palette.selected() {
                        let command_line = format!("/{}", selected.spec.name);
                        self.remember_submission(&command_line);
                        self.clear();
                        self.command_palette.clear();
                        return Some(Action::Chat(ChatAction::SendMessage {
                            text: command_line,
                            attachments: Vec::new(),
                        }));
                    }
                }
                _ => {}
            }
        }

        // ── AtMention visible — hijack navigation keys ──────────────
        if self.at_mention.visible && !self.at_mention.suggestions.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    self.at_mention.clear();
                    self.dirty = true;
                    return None;
                }
                KeyCode::Up => {
                    self.at_mention.move_selection(-1);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Down => {
                    self.at_mention.move_selection(1);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_at_mention();
                    self.dirty = true;
                    return None;
                }
                _ => {}
            }
        }

        // ── Snippet visible — hijack navigation keys ────────────────
        if self.snippet_state.visible && !self.snippet_state.snippets.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    self.snippet_state.clear();
                    self.dirty = true;
                    return None;
                }
                KeyCode::Up => {
                    self.snippet_state.move_selection(-1);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Down => {
                    self.snippet_state.move_selection(1);
                    self.dirty = true;
                    return None;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_snippet();
                    self.dirty = true;
                    return None;
                }
                _ => {}
            }
        }

        // ── Up/Down: vertical scroll OR history navigation ─────────────
        if matches!(key.code, KeyCode::Up | KeyCode::Down)
            && key.modifiers.is_empty()
            && !self.command_palette.visible
            && !self.at_mention.visible
            && !self.snippet_state.visible
        {
            let width = self.last_input_width;
            let (cursor_line, _) = self.cursor_position(width);
            let cursor_line = cursor_line as usize;
            let total_lines = self.display_line_count(width as usize);
            let visible_lines = self.last_visible_lines;
            let max_scroll = total_lines.saturating_sub(visible_lines);

            match key.code {
                KeyCode::Up => {
                    // History navigation when cursor at first visible line
                    // and scroll is already at top.
                    if cursor_line == self.input_scroll_offset
                        && self.input_scroll_offset == 0
                    {
                        self.select_prev_history();
                    } else if cursor_line == self.input_scroll_offset {
                        // Scroll up one line.
                        self.input_scroll_offset =
                            self.input_scroll_offset.saturating_sub(1);
                    } else {
                        // Move cursor up one visual line.
                        self.move_up(width);
                    }
                }
                KeyCode::Down => {
                    if cursor_line + 1 >= (self.input_scroll_offset + visible_lines).min(total_lines)
                        && self.input_scroll_offset >= max_scroll
                    {
                        // At bottom — history next.
                        self.select_next_history();
                    } else if cursor_line + 1 >= (self.input_scroll_offset + visible_lines).min(total_lines) {
                        // Scroll down one line.
                        self.input_scroll_offset =
                            (self.input_scroll_offset + 1).min(max_scroll);
                    } else {
                        // Move cursor down one visual line.
                        self.move_down(width);
                    }
                }
                _ => {}
            }
            self.ensure_input_cursor_visible();
            self.sync_autocomplete();
            self.dirty = true;
            return None;
        }

        // Ctrl+N / Ctrl+P: also history.
        if matches!(key.code, KeyCode::Char('n') | KeyCode::Char('p'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Char('n') => self.select_next_history(),
                KeyCode::Char('p') => self.select_prev_history(),
                _ => {}
            }
            self.sync_autocomplete();
            self.dirty = true;
            return None;
        }

        // ── Ctrl+V: clipboard paste ───────────────────────────────────
        if matches!(key.code, KeyCode::Char('v'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::SUPER)
        {
            // 1. Try text paste.
            if let Some(text) = crate::utils::paste_from_clipboard() {
                self.insert_str(&text);
                self.sync_autocomplete();
                self.dirty = true;
                return None;
            }
            // 2. Try image paste (if model supports it).
            if self.model_supports_images
                && let Some((filename, _mime, data, file_size)) =
                    crate::utils::paste_image_from_clipboard()
                {
                    let placeholder = format!("[Image: {}]", filename);
                    let insert_pos = self.cursor;
                    self.insert_str(&placeholder);
                    let end_pos = self.cursor;
                    self.register_span(
                        insert_pos,
                        end_pos,
                        Some(data),
                        Some(filename),
                    );
                    self.dirty = true;
                    log::info!("Pasted image: {} bytes", file_size);
                    return None;
                }
            return None;
        }

        // ── Delegate to internal key handler ──────────────────────────
        let submitted = self.handle_key(key);

        // After any key, refresh autocomplete states.
        self.sync_autocomplete();

        if let Some(text) = submitted {
            let attachments: Vec<MessageAttachment> = self.spans.iter()
                .filter_map(|s| {
                    s.image_data.as_ref().map(|data| MessageAttachment::Image {
                        data: data.clone(),
                        filename: s.image_filename.clone().unwrap_or_default(),
                        mime: "image/png".to_string(),
                        file_size: data.len() as u64,
                    })
                })
                .collect();
            return Some(Action::Chat(ChatAction::SendMessage {
                text,
                attachments,
            }));
        }

        None
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        render::draw_composer(self, frame, rect, ctx);
        self.dirty = false;
    }
}

// =====================================================================
// Free functions (shared with render.rs and other modules)
// =====================================================================

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// Compute visual lines for a text at a given width.
pub(crate) fn compute_visual_lines(text: &str, width: usize) -> Vec<VisualLine> {
    visual_lines_inner(text, width)
}

/// Compute visual lines for a text at a given width.
fn visual_lines_inner(text: &str, width: usize) -> Vec<VisualLine> {
    if width == 0 {
        return vec![VisualLine {
            start: 0,
            end: text.len(),
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
            });
            line_start = byte_index;
            current_width = 0;
        }

        current_width += char_width;
    }

    lines.push(VisualLine {
        start: line_start,
        end: text.len(),
    });

    lines
}

/// Compute the text position from a visual position within a specific line.
fn cursor_from_visual_position(text: &str, line: VisualLine, column: usize) -> usize {
    let mut visual_col = 0usize;
    for (byte_offset, ch) in text[line.start..line.end].char_indices() {
        if visual_col >= column {
            return line.start + byte_offset;
        }
        visual_col += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    line.end
}

/// Standalone snap_to_span_edge that takes a span slice directly.
fn snap_to_span_edge_static(spans: &[InlineSpan], pos: usize) -> usize {
    if let Some(span) = spans.iter().find(|s| pos > s.start && pos < s.end) {
        span.start
    } else {
        pos
    }
}

/// Find the word boundary (start/end) in the given direction.
fn find_word_boundary(text: &str, cursor: usize, direction: isize) -> usize {
    let bytes = text.as_bytes();
    let len = bytes.len();

    if cursor >= len && direction > 0 {
        return len;
    }
    if cursor == 0 && direction < 0 {
        return 0;
    }

    if direction < 0 {
        // Search backwards for a word boundary.
        let mut pos = cursor.saturating_sub(1);
        // Skip trailing whitespace.
        while pos > 0 && bytes[pos].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip the word itself.
        while pos > 0 && !bytes[pos].is_ascii_whitespace() {
            pos -= 1;
        }
        if pos == 0 && !bytes[0].is_ascii_whitespace() {
            0
        } else {
            pos + 1
        }
    } else {
        // Search forwards for a word boundary.
        let mut pos = cursor;
        // Skip leading whitespace.
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip the word itself.
        while pos < len && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        pos
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_insert_and_submit() {
        let mut c = Composer::new(">");
        assert_eq!(c.handle_key(key(KeyCode::Char('h'))), None);
        assert_eq!(c.handle_key(key(KeyCode::Char('i'))), None);
        assert_eq!(c.text(), "hi");

        let submitted = c.handle_key(key(KeyCode::Enter));
        assert_eq!(submitted, Some("hi".to_string()));
        assert!(c.is_empty());
    }

    #[test]
    fn test_empty_enter_no_submit() {
        let mut c = Composer::new(">");
        assert!(c.handle_key(key(KeyCode::Enter)).is_none());
    }

    #[test]
    fn test_shift_enter_newline() {
        let mut c = Composer::new(">");
        c.handle_key(key(KeyCode::Char('a')));
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(c.handle_key(shift_enter), None);
        assert_eq!(c.text(), "a\n");
    }

    #[test]
    fn test_backspace() {
        let mut c = Composer::new(">");
        c.set_text("hello".to_string());
        c.cursor = c.text.len();
        c.handle_key(key(KeyCode::Backspace));
        assert_eq!(c.text(), "hell");
    }

    #[test]
    fn test_ctrl_backspace_word_delete() {
        let mut c = Composer::new(">");
        c.set_text("hello world".to_string());
        c.cursor = c.text.len();
        c.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));
        assert_eq!(c.text(), "hello ");
    }

    #[test]
    fn test_history() {
        let mut c = Composer::new(">");
        c.handle_key(key(KeyCode::Char('a')));
        assert_eq!(c.handle_key(key(KeyCode::Enter)), Some("a".to_string()));

        c.handle_key(key(KeyCode::Char('b')));
        assert_eq!(c.handle_key(key(KeyCode::Enter)), Some("b".to_string()));

        // Up once → "b" (most recent)
        c.select_prev_history();
        assert_eq!(c.text(), "b");

        // Up again → "a"
        c.select_prev_history();
        assert_eq!(c.text(), "a");

        // Down → back to "b"
        c.select_next_history();
        assert_eq!(c.text(), "b");

        // Down → back to draft (empty)
        c.select_next_history();
        assert_eq!(c.text(), "");
    }

    #[test]
    fn test_inline_span_snap() {
        let mut c = Composer::new(">");
        c.set_text("hello @file.txt world".to_string());
        c.register_span(6, 15, None, None);

        // Cursor at 10 (inside span) should snap to 6.
        let snapped = c.snap_to_span_edge(10);
        assert_eq!(snapped, 6);

        // Right arrow from 6 should jump to 15.
        c.cursor = 6;
        c.move_right();
        assert_eq!(c.cursor, 15);
    }

    #[test]
    fn test_visual_lines_simple() {
        let lines = compute_visual_lines("hello", 10);
        assert_eq!(lines.len(), 1);
        assert_eq!(&lines[0].start, &0);
    }

    #[test]
    fn test_visual_lines_wrap() {
        let lines = compute_visual_lines("hello world", 5);
        assert!(lines.len() > 1);
    }

    #[test]
    fn test_display_width() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("中文"), 4);
    }

    #[test]
    fn test_delete_previous_char_at_span_edge() {
        let mut c = Composer::new(">");
        c.set_text("hi there".to_string());
        c.cursor = c.text.len();
        c.handle_key(key(KeyCode::Backspace));
        assert_eq!(c.text(), "hi ther");
    }

    #[test]
    fn test_submit_trims_trailing_whitespace() {
        let mut c = Composer::new(">");
        c.handle_key(key(KeyCode::Char('h')));
        c.handle_key(key(KeyCode::Char('i')));
        c.text.push_str("   ");
        c.cursor = c.text.len();

        let submitted = c.handle_key(key(KeyCode::Enter));
        assert_eq!(submitted, Some("hi".to_string()));
    }
}
