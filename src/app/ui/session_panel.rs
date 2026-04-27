use anyhow::Result;
use chrono::Duration as ChronoDuration;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;
use uuid::Uuid;

use crate::storage::SessionRecord;

use super::App;

#[derive(Clone, Debug, PartialEq)]
pub enum SessionViewMode {
    CurrentWorkspace,
    AllSessions,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OperationMode {
    Select,
    MultiSelect,
}

#[derive(Clone, Debug)]
pub struct CleanupPreview {
    #[allow(dead_code)]
    pub sessions: Vec<SessionRecord>,
    pub workspace_counts: Vec<(String, usize)>,
    pub total_count: usize,
}

impl CleanupPreview {
    pub fn from_sessions(sessions: Vec<SessionRecord>) -> Self {
        use std::collections::HashMap;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for session in &sessions {
            *counts.entry(session.workspace_root.clone()).or_insert(0) += 1;
        }
        let workspace_counts: Vec<_> = counts.into_iter().collect();
        let total_count = sessions.len();
        Self {
            sessions,
            workspace_counts,
            total_count,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SessionPanelDialog {
    None,
    DeleteConfirm {
        session_ids: Vec<Uuid>,
        session_titles: Vec<String>,
    },
    Cleanup {
        preview: CleanupPreview,
        selected_duration: Option<ChronoDuration>,
        cleanup_workspace: bool,
    },
    ExportConfirm {
        session_ids: Vec<Uuid>,
        session_titles: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub struct SessionPanelState {
    pub selected_index: usize,
    pub sessions: Vec<SessionRecord>,
    pub view_mode: SessionViewMode,
    pub operation_mode: OperationMode,
    pub selected_indices: Vec<usize>,
    pub dialog: SessionPanelDialog,
}

impl SessionPanelState {
    pub fn new(sessions: Vec<SessionRecord>, view_mode: SessionViewMode) -> Self {
        Self {
            selected_index: 0,
            sessions,
            view_mode,
            operation_mode: OperationMode::Select,
            selected_indices: Vec::new(),
            dialog: SessionPanelDialog::None,
        }
    }

    pub fn matching_indices(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_ascii_lowercase();
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| session_matches_query(&query, session).then_some(index))
            .collect()
    }

    pub fn reset_selection(&mut self, query: &str, current_session_id: Uuid) {
        let matches = self.matching_indices(query);
        if matches.is_empty() {
            self.selected_index = 0;
            return;
        }

        if let Some(index) = matches
            .iter()
            .position(|candidate| self.sessions[*candidate].session_id == current_session_id)
        {
            self.selected_index = index;
            return;
        }

        self.selected_index = self.selected_index.min(matches.len().saturating_sub(1));
    }

    pub fn move_selection(&mut self, query: &str, delta: isize) {
        let matches = self.matching_indices(query);
        if matches.is_empty() {
            self.selected_index = 0;
            return;
        }

        let len = matches.len() as isize;
        let current = self.selected_index.min(matches.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    pub fn selected_session(&self, query: &str) -> Option<&SessionRecord> {
        let matches = self.matching_indices(query);
        let session_index = *matches.get(self.selected_index)?;
        self.sessions.get(session_index)
    }

    pub fn toggle_selection(&mut self) {
        if self.operation_mode != OperationMode::MultiSelect {
            return;
        }

        let matches = self.matching_indices("");
        if let Some(&session_index) = matches.get(self.selected_index) {
            if let Some(pos) = self
                .selected_indices
                .iter()
                .position(|&i| i == session_index)
            {
                self.selected_indices.remove(pos);
            } else {
                self.selected_indices.push(session_index);
            }
        }
    }

    pub fn is_selected(&self, session_index: usize) -> bool {
        self.selected_indices.contains(&session_index)
    }

    pub fn selected_count(&self) -> usize {
        self.selected_indices.len()
    }

    pub fn clear_selection(&mut self) {
        self.selected_indices.clear();
        self.operation_mode = OperationMode::Select;
    }

    pub fn get_selected_session_ids(&self, query: &str) -> Vec<Uuid> {
        if self.operation_mode == OperationMode::MultiSelect && !self.selected_indices.is_empty() {
            let matches = self.matching_indices(query);
            self.selected_indices
                .iter()
                .filter_map(|&idx| matches.contains(&idx).then_some(idx))
                .filter_map(|idx| self.sessions.get(idx))
                .map(|s| s.session_id)
                .collect()
        } else if let Some(session) = self.selected_session(query) {
            vec![session.session_id]
        } else {
            Vec::new()
        }
    }

    pub fn get_selected_session_titles(&self, query: &str) -> Vec<String> {
        if self.operation_mode == OperationMode::MultiSelect && !self.selected_indices.is_empty() {
            let matches = self.matching_indices(query);
            self.selected_indices
                .iter()
                .filter_map(|&idx| matches.contains(&idx).then_some(idx))
                .filter_map(|idx| self.sessions.get(idx))
                .map(|s| s.title.clone())
                .collect()
        } else if let Some(session) = self.selected_session(query) {
            vec![session.title.clone()]
        } else {
            Vec::new()
        }
    }
}

impl App {
    pub(crate) fn open_session_panel(&mut self, initial_query: String) -> Result<()> {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.mcp_panel = None;

        let sessions = self
            .store
            .load_sessions_for_workspace(Path::new(&self.workspace_root))?;
        self.session_panel = Some(SessionPanelState::new(
            sessions,
            SessionViewMode::CurrentWorkspace,
        ));
        self.composer.clear();
        self.composer
            .set_placeholder("Search sessions by title, model, or id");
        self.composer.set_text(initial_query);
        self.reset_session_panel_selection();
        Ok(())
    }

    pub(crate) fn close_session_panel(&mut self) {
        if self.session_panel.take().is_some() {
            self.composer.clear();
            self.composer
                .set_placeholder("Ask TiDev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_session_panel_selection(&mut self) {
        let current_session_id = self.conversation.session_id;
        let query = self.composer.text().to_string();
        if let Some(panel) = &mut self.session_panel {
            panel.reset_selection(&query, current_session_id);
        }
    }

    pub(crate) fn handle_session_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        let Some(panel) = self.session_panel.clone() else {
            return Ok(());
        };

        match (&panel.dialog, key.code) {
            (SessionPanelDialog::None, _) => {
                self.handle_session_panel_main_key(panel, key, runtime)
            }
            (SessionPanelDialog::DeleteConfirm { .. }, KeyCode::Enter) => {
                self.confirm_delete_session()
            }
            (SessionPanelDialog::DeleteConfirm { .. }, KeyCode::Esc)
            | (SessionPanelDialog::Cleanup { .. }, KeyCode::Esc)
            | (SessionPanelDialog::ExportConfirm { .. }, KeyCode::Esc) => {
                self.close_session_panel_dialog()
            }
            (SessionPanelDialog::Cleanup { .. }, KeyCode::Enter) => self.confirm_cleanup_sessions(),
            (SessionPanelDialog::Cleanup { .. }, KeyCode::Char('1')) => {
                self.select_cleanup_duration(ChronoDuration::weeks(1))
            }
            (SessionPanelDialog::Cleanup { .. }, KeyCode::Char('2')) => {
                self.select_cleanup_duration(ChronoDuration::days(30))
            }
            (SessionPanelDialog::Cleanup { .. }, KeyCode::Char('3')) => {
                self.select_cleanup_duration(ChronoDuration::days(90))
            }
            (SessionPanelDialog::Cleanup { .. }, KeyCode::Char('4')) => {
                self.select_cleanup_duration(ChronoDuration::days(365))
            }
            (SessionPanelDialog::Cleanup { .. }, KeyCode::Char('5')) => {
                self.select_cleanup_workspace()
            }
            (SessionPanelDialog::ExportConfirm { .. }, KeyCode::Enter) => {
                self.confirm_export_session()
            }
            _ => Ok(()),
        }
    }

    fn handle_session_panel_main_key(
        &mut self,
        panel: SessionPanelState,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.session_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.session_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let query = self.composer.text().to_string();
                if let Some(session) = panel.selected_session(&query).cloned() {
                    self.switch_session(session.session_id, runtime)?;
                    self.close_session_panel();
                }
            }
            KeyCode::Esc => {
                if panel.operation_mode == OperationMode::MultiSelect {
                    if let Some(p) = &mut self.session_panel {
                        p.clear_selection();
                    }
                } else {
                    self.close_session_panel();
                }
            }
            KeyCode::Tab => self.toggle_session_view_mode()?,
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.session_panel = Some(next_panel);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.session_panel = Some(next_panel);
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_multi_select_mode()?
            }
            KeyCode::Char(' ') => {
                if let Some(p) = &mut self.session_panel {
                    p.toggle_selection();
                }
                self.session_panel = Some(panel);
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.open_delete_confirmation()?;
                return Ok(());
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.open_cleanup_dialog()?;
                return Ok(());
            }
            KeyCode::Char('w') | KeyCode::Char('W') => self.switch_to_all_sessions_view()?,
            KeyCode::Char('e') | KeyCode::Char('E') => {
                self.open_export_dialog()?;
                return Ok(());
            }
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    self.reset_session_panel_selection();
                }
            }
        }

        Ok(())
    }

    pub(crate) fn toggle_session_view_mode(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel {
            let new_mode = if panel.view_mode == SessionViewMode::CurrentWorkspace {
                SessionViewMode::AllSessions
            } else {
                SessionViewMode::CurrentWorkspace
            };

            let sessions = if new_mode == SessionViewMode::AllSessions {
                self.store.load_all_sessions().unwrap_or_default()
            } else {
                self.store
                    .load_sessions_for_workspace(Path::new(&self.workspace_root))
                    .unwrap_or_default()
            };

            *panel = SessionPanelState::new(sessions, new_mode);
        }
        Ok(())
    }

    pub(crate) fn switch_to_all_sessions_view(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel
            && panel.view_mode == SessionViewMode::CurrentWorkspace
        {
            let sessions = self.store.load_all_sessions().unwrap_or_default();
            *panel = SessionPanelState::new(sessions, SessionViewMode::AllSessions);
        }
        Ok(())
    }

    pub(crate) fn toggle_multi_select_mode(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel {
            if panel.operation_mode == OperationMode::Select {
                panel.operation_mode = OperationMode::MultiSelect;
                panel.selected_indices.clear();
            } else {
                panel.clear_selection();
            }
        }
        Ok(())
    }

    pub(crate) fn open_delete_confirmation(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel {
            let query = self.composer.text().to_string();
            let session_ids = panel.get_selected_session_ids(&query);
            let session_titles = panel.get_selected_session_titles(&query);

            if !session_ids.is_empty() {
                panel.dialog = SessionPanelDialog::DeleteConfirm {
                    session_ids,
                    session_titles,
                };
            }
        }
        Ok(())
    }

    pub(crate) fn confirm_delete_session(&mut self) -> Result<()> {
        if let Some(panel) = self.session_panel.take()
            && let SessionPanelDialog::DeleteConfirm { session_ids, .. } = panel.dialog
        {
            self.store.delete_sessions(&session_ids)?;
            let count = session_ids.len();
            self.last_notice = Some(format!("Deleted {} session(s)", count));
        }

        self.close_session_panel();
        self.open_session_panel(String::new())?;
        Ok(())
    }

    pub(crate) fn open_cleanup_dialog(&mut self) -> Result<()> {
        let sessions = self
            .store
            .get_sessions_older_than_preview(ChronoDuration::days(1))
            .unwrap_or_default();
        let preview = CleanupPreview::from_sessions(sessions);

        if let Some(panel) = &mut self.session_panel {
            panel.dialog = SessionPanelDialog::Cleanup {
                preview,
                selected_duration: None,
                cleanup_workspace: false,
            };
        }
        Ok(())
    }

    pub(crate) fn select_cleanup_duration(&mut self, duration: ChronoDuration) -> Result<()> {
        if let Some(panel) = &mut self.session_panel
            && let SessionPanelDialog::Cleanup { .. } = &panel.dialog
        {
            let sessions = self
                .store
                .get_sessions_older_than_preview(duration)
                .unwrap_or_default();
            let new_preview = CleanupPreview::from_sessions(sessions);

            panel.dialog = SessionPanelDialog::Cleanup {
                preview: new_preview,
                selected_duration: Some(duration),
                cleanup_workspace: false,
            };
        }
        Ok(())
    }

    pub(crate) fn select_cleanup_workspace(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel
            && let SessionPanelDialog::Cleanup {
                preview: _,
                selected_duration,
                ..
            } = &panel.dialog
        {
            let sessions = self
                .store
                .get_current_workspace_sessions_count(Path::new(&self.workspace_root))
                .unwrap_or(0);

            let new_preview = CleanupPreview {
                sessions: vec![],
                workspace_counts: vec![(
                    self.workspace_root.to_string_lossy().to_string(),
                    sessions as usize,
                )],
                total_count: sessions as usize,
            };

            panel.dialog = SessionPanelDialog::Cleanup {
                preview: new_preview,
                selected_duration: *selected_duration,
                cleanup_workspace: true,
            };
        }
        Ok(())
    }

    pub(crate) fn confirm_cleanup_sessions(&mut self) -> Result<()> {
        if let Some(panel) = self.session_panel.take()
            && let SessionPanelDialog::Cleanup {
                preview: _,
                selected_duration,
                cleanup_workspace,
            } = panel.dialog
        {
            if cleanup_workspace {
                let deleted = self
                    .store
                    .delete_sessions_in_workspace(Path::new(&self.workspace_root))?;
                let count = deleted.len();
                self.last_notice =
                    Some(format!("Deleted {} session(s) in current workspace", count));
            } else if let Some(duration) = selected_duration {
                let deleted = self.store.delete_sessions_older_than(duration)?;
                let count = deleted.len();
                self.last_notice = Some(format!("Deleted {} old session(s)", count));
            }
        }

        self.close_session_panel();
        self.open_session_panel(String::new())?;
        Ok(())
    }

    pub(crate) fn open_export_dialog(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel {
            let query = self.composer.text().to_string();
            let session_ids = panel.get_selected_session_ids(&query);
            let session_titles = panel.get_selected_session_titles(&query);

            if !session_ids.is_empty() {
                panel.dialog = SessionPanelDialog::ExportConfirm {
                    session_ids,
                    session_titles,
                };
            }
        }
        Ok(())
    }

    pub(crate) fn confirm_export_session(&mut self) -> Result<()> {
        if let Some(panel) = self.session_panel.take()
            && let SessionPanelDialog::ExportConfirm { session_ids, .. } = panel.dialog
        {
            let export_dir = self.paths.data_dir.join("export");

            crate::log_info!("Export dir: {}", export_dir.display());

            for session_id in &session_ids {
                match self.store.export_session_to_jsonl(*session_id, &export_dir) {
                    Ok(path) => crate::log_info!("Exported: {}", path.display()),
                    Err(e) => crate::log_error!("Export failed: {}", e),
                }
            }

            let count = session_ids.len();
            self.last_notice = Some(format!(
                "Exported {} session(s) to {}",
                count,
                export_dir.display()
            ));
        }

        self.close_session_panel_dialog()?;
        self.open_session_panel(String::new())?;
        Ok(())
    }

    pub(crate) fn close_session_panel_dialog(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.session_panel {
            panel.dialog = SessionPanelDialog::None;
        }
        Ok(())
    }

    pub(crate) fn switch_session(
        &mut self,
        session_id: Uuid,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        if self.conversation.session_id == session_id {
            self.last_notice = Some("Already on that session".to_string());
            return Ok(());
        }

        let fallback_model = Self::resolve_fallback_model(&self.config, &self.auth)?;
        self.cache_active_session_runtime();

        if let Err(error) = self.restore_or_load_session(session_id, &fallback_model) {
            self.last_notice = Some(error.to_string());
            return Ok(());
        }

        self.screen = if self.conversation.visible_messages().is_empty() {
            super::Screen::Welcome
        } else {
            super::Screen::Chat
        };
        self.clear_mouse_selection();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.command_palette.clear();
        self.composer.clear();

        if let Some(dialog) = self.question_dialog.as_ref() {
            self.composer.set_text(dialog.current_answer_text());
            self.composer.set_placeholder(dialog.answer_placeholder());
        } else {
            self.composer
                .set_placeholder("Ask TiDev about your code, task, or question...");
        }

        if self.pending_assistant_turns.remove(&session_id) {
            crate::log_info!(
                "switch_session: session {} has pending assistant turn, starting now",
                session_id
            );
            if !self.pending_request {
                self.start_assistant_turn(runtime)?;
            }
        }

        Ok(())
    }
}

fn session_matches_query(query: &str, session: &SessionRecord) -> bool {
    if query.is_empty() {
        return true;
    }

    let title = session.title.to_ascii_lowercase();
    let provider = session.provider_display_name.to_ascii_lowercase();
    let model = session.model_display_name.to_ascii_lowercase();
    let session_id = session.session_id.to_string().to_ascii_lowercase();
    let workspace_root = session.workspace_root.to_ascii_lowercase();

    title.contains(query)
        || provider.contains(query)
        || model.contains(query)
        || session_id.contains(query)
        || workspace_root.contains(query)
}
