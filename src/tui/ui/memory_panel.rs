use anyhow::Result;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use std::cell::Cell;
use uuid::Uuid;

use crate::memory::{MemoryEntry, MemoryStore, MemoryType};
use crate::tui::input::Composer;

use super::App;

#[derive(Clone, Debug, PartialEq)]
pub enum MemoryPanelMode {
    /// Browse and select memories
    Browse,
    /// Adding a new memory
    Add,
    /// Editing an existing memory
    Edit,
    /// Confirm delete
    DeleteConfirm,
}

/// Which part of the Browse two-pane layout has keyboard focus.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelFocus {
    /// Left list is active. ↑/↓ navigates items. Right shows markdown preview.
    List,
    /// Right pane shows raw content text for editing. Arrow keys move cursor.
    ContentEdit,
}

#[derive(Clone, Debug)]
pub struct MemoryPanelState {
    pub mode: MemoryPanelMode,
    pub selected_index: usize,
    pub memories: Vec<MemoryEntry>,
    pub filter_type: Option<MemoryType>,
    /// Scroll offset for the right-side content preview (browse mode)
    pub preview_scroll: usize,
    /// Which pane is focused
    pub focus: PanelFocus,
    /// Text editor for inline content editing (right pane in ContentEdit mode)
    pub content_editor: Composer,
    /// Snapshot of content before editing began (for Esc to cancel)
    pub content_edit_snapshot: String,
    /// Width of the right pane editor (set during render, used for cursor movement)
    pub editor_width: Cell<u16>,
    /// For mouse hit-testing (set during render)
    pub panel_rect: Option<Rect>,
    pub left_rect: Option<Rect>,
    pub right_rect: Option<Rect>,
    /// For Add/Edit mode
    pub edit_title: String,
    pub edit_content: String,
    pub edit_type: MemoryType,
    pub edit_tags: String,
    pub edit_id: Option<Uuid>,
    /// Search query text (empty = no search filter). Typed when search is active.
    pub query: String,
    /// Whether search input mode is active. When true, printable chars go to query.
    pub search_active: bool,
}

impl Default for MemoryPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPanelState {
    pub fn new() -> Self {
        Self {
            mode: MemoryPanelMode::Browse,
            selected_index: 0,
            memories: Vec::new(),
            filter_type: None,
            preview_scroll: 0,
            focus: PanelFocus::List,
            content_editor: Composer::new(""),
            content_edit_snapshot: String::new(),
            editor_width: Cell::new(40),
            panel_rect: None,
            left_rect: None,
            right_rect: None,
            edit_title: String::new(),
            edit_content: String::new(),
            edit_type: MemoryType::Project,
            edit_tags: String::new(),
            edit_id: None,
            query: String::new(),
            search_active: false,
        }
    }

    pub fn load(&mut self, store: &MemoryStore, workspace_root: &str) -> Result<()> {
        self.memories = store.get_or_load(workspace_root)?;
        self.selected_index = self
            .selected_index
            .min(self.memories.len().saturating_sub(1));
        Ok(())
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let q = self.query.trim().to_lowercase();
        self.memories
            .iter()
            .enumerate()
            .filter(|(_, m)| self.filter_type.is_none_or(|t| m.memory_type == t))
            .filter(|(_, m)| {
                if q.is_empty() {
                    return true;
                }
                m.title.to_lowercase().contains(&q)
                    || m.content.to_lowercase().contains(&q)
                    || m.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&MemoryEntry> {
        let filtered = self.filtered_indices();
        let idx = filtered.get(self.selected_index)?;
        self.memories.get(*idx)
    }

    pub fn move_selection(&mut self, delta: isize) {
        let filtered = self.filtered_indices();
        if filtered.is_empty() {
            return;
        }
        let len = filtered.len() as isize;
        let current = self.selected_index.min(filtered.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len);
        self.selected_index = filtered.get(next as usize).copied().unwrap_or(0);
        self.preview_scroll = 0;
    }

    pub fn start_add(&mut self) {
        self.mode = MemoryPanelMode::Add;
        self.edit_title.clear();
        self.edit_content.clear();
        self.edit_type = MemoryType::Project;
        self.edit_tags.clear();
        self.edit_id = None;
    }

    pub fn confirm_save(&mut self, store: &MemoryStore, workspace_root: &str) -> Result<()> {
        let now = Utc::now();
        let tags: Vec<String> = self
            .edit_tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if let Some(id) = self.edit_id {
            let mut entry = MemoryEntry {
                id,
                workspace_root: workspace_root.to_string(),
                memory_type: self.edit_type,
                title: self.edit_title.clone(),
                content: self.edit_content.clone(),
                tags,
                source_session_id: None,
                created_at: now,
                updated_at: now,
                usage_count: 0,
                active: true,
                concepts: vec![],
                files: vec![],
                strength: 0.0,
                importance: 5,
                version: 1,
                parent_id: None,
                supersedes: vec![],
                related_ids: vec![],
                is_latest: true,
            };
            if let Some(existing) = self.memories.iter().find(|e| e.id == id) {
                entry.created_at = existing.created_at;
                entry.usage_count = existing.usage_count;
            }
            store.update(&entry)?;
        } else {
            let entry = MemoryEntry {
                id: Uuid::new_v4(),
                workspace_root: workspace_root.to_string(),
                memory_type: self.edit_type,
                title: self.edit_title.clone(),
                content: self.edit_content.clone(),
                tags,
                source_session_id: None,
                created_at: now,
                updated_at: now,
                usage_count: 0,
                active: true,
                concepts: vec![],
                files: vec![],
                strength: 0.0,
                importance: 5,
                version: 1,
                parent_id: None,
                supersedes: vec![],
                related_ids: vec![],
                is_latest: true,
            };
            store.add(&entry)?;
        }
        self.mode = MemoryPanelMode::Browse;
        self.load(store, workspace_root)
    }

    pub fn confirm_delete(&mut self, store: &MemoryStore, workspace_root: &str) -> Result<()> {
        if self.mode == MemoryPanelMode::DeleteConfirm {
            if let Some(entry) = self.selected_entry().cloned() {
                store.delete(workspace_root, entry.id)?;
            }
            self.mode = MemoryPanelMode::Browse;
            self.load(store, workspace_root)?;
        }
        Ok(())
    }

    pub fn cycle_filter_type(&mut self) {
        self.filter_type = match self.filter_type {
            None => Some(MemoryType::User),
            Some(MemoryType::User) => Some(MemoryType::Project),
            Some(MemoryType::Project) => Some(MemoryType::Feedback),
            Some(MemoryType::Feedback) => Some(MemoryType::Reference),
            Some(MemoryType::Reference) => Some(MemoryType::Pattern),
            Some(MemoryType::Pattern) => Some(MemoryType::Preference),
            Some(MemoryType::Preference) => Some(MemoryType::Architecture),
            Some(MemoryType::Architecture) => Some(MemoryType::Bug),
            Some(MemoryType::Bug) => Some(MemoryType::Workflow),
            Some(MemoryType::Workflow) => Some(MemoryType::Fact),
            Some(MemoryType::Fact) => Some(MemoryType::Lesson),
            Some(MemoryType::Lesson) => Some(MemoryType::Insight),
            Some(MemoryType::Insight) => None,
        };
        self.selected_index = 0;
        self.preview_scroll = 0;
    }

    /// Enter inline content edit mode for the selected memory entry.
    pub fn enter_content_edit(&mut self) {
        if let Some(entry) = self.selected_entry().cloned() {
            self.content_edit_snapshot = entry.content.clone();
            self.content_editor.set_text(entry.content);
            self.focus = PanelFocus::ContentEdit;
        }
    }

    /// Save the edited content and return to list-focus browse mode.
    pub fn save_content_edit(&mut self, store: &MemoryStore, workspace_root: &str) -> Result<()> {
        if let Some(entry) = self.selected_entry().cloned() {
            let mut updated = entry;
            updated.content = self.content_editor.text().to_string();
            store.update(&updated)?;
            self.load(store, workspace_root)?;
        }
        self.focus = PanelFocus::List;
        self.preview_scroll = 0;
        Ok(())
    }

    /// Cancel editing and restore the original content.
    pub fn cancel_content_edit(&mut self) {
        self.content_editor.set_text(self.content_edit_snapshot.clone());
        self.focus = PanelFocus::List;
    }
}

impl App {
    pub(crate) fn handle_memory_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        let Some(panel) = self.memory_panel.clone() else {
            return Ok(());
        };

        match panel.mode {
            MemoryPanelMode::Browse => {
                self.handle_memory_panel_browse_key(panel, key, runtime)?;
            }
            MemoryPanelMode::Add | MemoryPanelMode::Edit => {
                self.handle_memory_panel_edit_key(panel, key)?;
            }
            MemoryPanelMode::DeleteConfirm => {
                self.handle_memory_panel_delete_key(panel, key, runtime)?;
            }
        }
        Ok(())
    }
    fn handle_memory_panel_browse_key(
        &mut self,
        panel: MemoryPanelState,
        key: KeyEvent,
        _runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        match panel.focus {
            PanelFocus::List => self.handle_browse_list_key(panel, key),
            PanelFocus::ContentEdit => self.handle_browse_edit_key(panel, key),
        }
    }

    /// Keys active when the left list is focused.
    fn handle_browse_list_key(
        &mut self,
        panel: MemoryPanelState,
        key: KeyEvent,
    ) -> Result<()> {
        // If search is active, intercept printable keys and special keys
        if panel.search_active {
            match key.code {
                KeyCode::Esc => {
                    let mut next = panel;
                    if !next.query.is_empty() {
                        // Clear query and stay in search mode
                        next.query.clear();
                    } else {
                        // Exit search mode
                        next.search_active = false;
                    }
                    next.selected_index = 0;
                    next.preview_scroll = 0;
                    self.memory_panel = Some(next);
                }
                KeyCode::Backspace => {
                    let mut next = panel;
                    next.query.pop();
                    next.selected_index = 0;
                    next.preview_scroll = 0;
                    self.memory_panel = Some(next);
                }
                KeyCode::Enter => {
                    // Exit search mode, keep query as filter
                    let mut next = panel;
                    next.search_active = false;
                    self.memory_panel = Some(next);
                }
                KeyCode::Up | KeyCode::Down => {
                    // Still allow navigation while searching
                    let mut next = panel;
                    let delta = if key.code == KeyCode::Up { -1 } else { 1 };
                    next.move_selection(delta);
                    self.memory_panel = Some(next);
                }
                KeyCode::Char(ch) => {
                    let mut next = panel;
                    next.query.push(ch);
                    next.selected_index = 0;
                    next.preview_scroll = 0;
                    self.memory_panel = Some(next);
                }
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Up => {
                let mut next = panel;
                next.move_selection(-1);
                self.memory_panel = Some(next);
            }
            KeyCode::Down => {
                let mut next = panel;
                next.move_selection(1);
                self.memory_panel = Some(next);
            }
            KeyCode::Enter => {
                let mut next = panel;
                next.enter_content_edit();
                self.memory_panel = Some(next);
            }
            KeyCode::Esc => {
                self.close_memory_panel();
            }
            KeyCode::Char('/') => {
                // Enter search mode
                let mut next = panel;
                next.search_active = true;
                next.query.clear();
                self.memory_panel = Some(next);
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let mut next = panel;
                next.start_add();
                self.memory_panel = Some(next);
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                let mut next = panel;
                next.enter_content_edit();
                self.memory_panel = Some(next);
            }
            KeyCode::Char('d') | KeyCode::Char('D') if panel.selected_entry().is_some() => {
                let mut next = panel;
                next.mode = MemoryPanelMode::DeleteConfirm;
                self.memory_panel = Some(next);
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                let mut next = panel;
                next.cycle_filter_type();
                self.memory_panel = Some(next);
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                let mut next = panel;
                next.mode = MemoryPanelMode::Add;
                next.edit_type = MemoryType::Project;
                next.edit_title.clear();
                next.edit_content.clear();
                next.edit_tags.clear();
                next.edit_id = None;
                // Pre-fill with selected memory's type
                if let Some(entry) = next.selected_entry() {
                    next.edit_type = entry.memory_type;
                }
                self.memory_panel = Some(next);
            }
            KeyCode::Left => {
                let mut next = panel;
                next.preview_scroll = next.preview_scroll.saturating_sub(3);
                self.memory_panel = Some(next);
            }
            KeyCode::Right => {
                let mut next = panel;
                next.preview_scroll = next.preview_scroll.saturating_add(3);
                self.memory_panel = Some(next);
            }
            _ => {}
        }
        Ok(())
    }

    /// Keys active when the right content pane is in edit mode.
    fn handle_browse_edit_key(
        &mut self,
        panel: MemoryPanelState,
        key: KeyEvent,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                let mut next = panel;
                next.cancel_content_edit();
                self.memory_panel = Some(next);
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT)
                {
                    // Shift+Enter / Alt+Enter → insert newline
                    let mut next = panel;
                    next.content_editor.handle_key(key);
                    self.memory_panel = Some(next);
                } else {
                    // Plain Enter → save and return to list focus
                    let ws = self.workspace_root.display().to_string();
                    let mut next = panel;
                    next.save_content_edit(&self.memory_store, &ws)?;
                    self.memory_panel = Some(next);
                }
            }
            KeyCode::Up => {
                // Move cursor up one visual line
                let mut next = panel;
                let width = next.editor_width.get().max(1);
                let (current_line, current_col) =
                    next.content_editor.cursor_position(width);
                if current_line > 0 {
                    next.content_editor.set_cursor_at_visual_position(
                        width,
                        current_line - 1,
                        current_col,
                    );
                }
                self.memory_panel = Some(next);
            }
            KeyCode::Down => {
                // Move cursor down one visual line
                let mut next = panel;
                let width = next.editor_width.get().max(1);
                let lines = next.content_editor.visual_lines(width as usize);
                let (current_line, current_col) =
                    next.content_editor.cursor_position(width);
                if (current_line as usize) + 1 < lines.len() {
                    next.content_editor.set_cursor_at_visual_position(
                        width,
                        current_line + 1,
                        current_col,
                    );
                }
                self.memory_panel = Some(next);
            }
            _ => {
                // Delegate all other keys to the Composer
                let mut next = panel;
                next.content_editor.handle_key(key);
                self.memory_panel = Some(next);
            }
        }
        Ok(())
    }
    fn handle_memory_panel_edit_key(
        &mut self,
        panel: MemoryPanelState,
        key: KeyEvent,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                let mut next = panel;
                next.mode = MemoryPanelMode::Browse;
                self.memory_panel = Some(next);
            }
            KeyCode::Enter => {
                let ws = self.workspace_root.display().to_string();
                let mut next = panel;
                next.confirm_save(&self.memory_store, &ws)?;
                self.memory_panel = Some(next);
            }
            KeyCode::Tab => {
                // Cycle memory type
                let mut next = panel;
                next.edit_type = match next.edit_type {
                    MemoryType::User => MemoryType::Project,
                    MemoryType::Project => MemoryType::Feedback,
                    MemoryType::Feedback => MemoryType::Reference,
                    MemoryType::Reference => MemoryType::Pattern,
                    MemoryType::Pattern => MemoryType::Preference,
                    MemoryType::Preference => MemoryType::Architecture,
                    MemoryType::Architecture => MemoryType::Bug,
                    MemoryType::Bug => MemoryType::Workflow,
                    MemoryType::Workflow => MemoryType::Fact,
                    MemoryType::Fact => MemoryType::Lesson,
                    MemoryType::Lesson => MemoryType::Insight,
                    MemoryType::Insight => MemoryType::User,
                };
                self.memory_panel = Some(next);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_memory_panel_delete_key(
        &mut self,
        panel: MemoryPanelState,
        key: KeyEvent,
        _runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let ws = self.workspace_root.display().to_string();
                let mut next = panel;
                next.confirm_delete(&self.memory_store, &ws)?;
                self.memory_panel = Some(next);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                let mut next = panel;
                next.mode = MemoryPanelMode::Browse;
                self.memory_panel = Some(next);
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn close_memory_panel(&mut self) {
        self.memory_panel = None;
    }
}
