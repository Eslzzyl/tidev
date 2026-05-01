use anyhow::Result;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent};
use uuid::Uuid;

use crate::memory::types::{MemoryEntry, MemoryType, MemoryStore};

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

#[derive(Clone, Debug)]
pub struct MemoryPanelState {
    pub mode: MemoryPanelMode,
    pub selected_index: usize,
    pub memories: Vec<MemoryEntry>,
    pub filter_type: Option<MemoryType>,
    /// For Add/Edit mode
    pub edit_title: String,
    pub edit_content: String,
    pub edit_type: MemoryType,
    pub edit_tags: String,
    pub edit_id: Option<Uuid>,
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
            edit_title: String::new(),
            edit_content: String::new(),
            edit_type: MemoryType::Project,
            edit_tags: String::new(),
            edit_id: None,
        }
    }

    pub fn load(&mut self, store: &MemoryStore, workspace_root: &str) -> Result<()> {
        self.memories = store.get_or_load(workspace_root)?;
        self.selected_index = self.selected_index.min(self.memories.len().saturating_sub(1));
        Ok(())
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        self.memories
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                self.filter_type.is_none_or(|t| m.memory_type == t)
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
    }

    pub fn start_add(&mut self) {
        self.mode = MemoryPanelMode::Add;
        self.edit_title.clear();
        self.edit_content.clear();
        self.edit_type = MemoryType::Project;
        self.edit_tags.clear();
        self.edit_id = None;
    }

    pub fn start_edit(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else { return; };
        self.mode = MemoryPanelMode::Edit;
        self.edit_title = entry.title;
        self.edit_content = entry.content;
        self.edit_type = entry.memory_type;
        self.edit_tags = entry.tags.join(", ");
        self.edit_id = Some(entry.id);
    }

    pub fn confirm_save(&mut self, store: &MemoryStore, workspace_root: &str) -> Result<()> {
        let now = Utc::now();
        let tags: Vec<String> = self.edit_tags.split(',')
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
            Some(MemoryType::Reference) => None,
        };
        self.selected_index = 0;
    }
}

// ─── App methods ──────────────────────────────────────────────

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
            KeyCode::Esc => {
                self.close_memory_panel();
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let mut next = panel;
                next.start_add();
                self.memory_panel = Some(next);
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                let mut next = panel;
                next.start_edit();
                self.memory_panel = Some(next);
            }
            KeyCode::Char('d') | KeyCode::Char('D')
                if panel.selected_entry().is_some() => {
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
            _ => {}
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
                    MemoryType::Reference => MemoryType::User,
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
