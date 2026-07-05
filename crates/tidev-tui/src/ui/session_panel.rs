use anyhow::{Context, Result};
use chrono::Duration as ChronoDuration;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;
use uuid::Uuid;

use tidev_core::SessionRecord;

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
        self.ui.command_palette.clear();
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;

        let sessions = self.runtime.session_manager().store()
            .list_sessions(1000, 0)?;
        self.ui.session_panel = Some(SessionPanelState::new(
            sessions,
            SessionViewMode::CurrentWorkspace,
        ));
        self.ui.composer.clear();
        self.ui
            .composer
            .set_placeholder("Search sessions by title, model, or id");
        self.ui.composer.set_text(initial_query);
        self.reset_session_panel_selection();
        Ok(())
    }

    pub(crate) fn close_session_panel(&mut self) {
        if self.ui.session_panel.take().is_some() {
            self.ui.composer.clear();
            self.ui
                .composer
                .set_placeholder("Ask tidev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_session_panel_selection(&mut self) {
        let current_session_id = self.ui.chat_context.session_id;
        let query = self.ui.composer.text().to_string();
        if let Some(panel) = &mut self.ui.session_panel {
            panel.reset_selection(&query, current_session_id);
        }
    }

    pub(crate) fn handle_session_panel_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<()> {
        let Some(panel) = self.ui.session_panel.clone() else {
            return Ok(());
        };

        match (&panel.dialog, key.code) {
            (SessionPanelDialog::None, _) => self.handle_session_panel_main_key(panel, key),
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
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                let query = self.ui.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.ui.session_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let query = self.ui.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.ui.session_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let query = self.ui.composer.text().to_string();
                if let Some(session) = panel.selected_session(&query).cloned() {
                    self.switch_session(session.session_id)?;
                    self.close_session_panel();
                }
            }
            KeyCode::Esc => {
                if panel.operation_mode == OperationMode::MultiSelect {
                    if let Some(p) = &mut self.ui.session_panel {
                        p.clear_selection();
                    }
                } else {
                    self.close_session_panel();
                }
            }
            KeyCode::Tab => self.toggle_session_view_mode()?,
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.ui.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.ui.session_panel = Some(next_panel);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.ui.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.ui.session_panel = Some(next_panel);
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_multi_select_mode()?
            }
            KeyCode::Char(' ') => {
                if let Some(p) = &mut self.ui.session_panel {
                    p.toggle_selection();
                }
                self.ui.session_panel = Some(panel);
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
                let previous_query = self.ui.composer.text().to_string();
                let _ = self.ui.composer.handle_key_with_history(key, false);
                if self.ui.composer.text() != previous_query {
                    self.reset_session_panel_selection();
                }
            }
        }

        Ok(())
    }

    pub(crate) fn toggle_session_view_mode(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.ui.session_panel {
            let new_mode = if panel.view_mode == SessionViewMode::CurrentWorkspace {
                SessionViewMode::AllSessions
            } else {
                SessionViewMode::CurrentWorkspace
            };

            let sessions = if new_mode == SessionViewMode::AllSessions {
                self.runtime
                    .session_manager()
                    .store()
                    .list_sessions(1000, 0)
                    .unwrap_or_default()
            } else {
                self.runtime.session_manager().store()
                    .list_sessions(1000, 0)
                    .unwrap_or_default()
            };

            *panel = SessionPanelState::new(sessions, new_mode);
        }
        Ok(())
    }

    pub(crate) fn switch_to_all_sessions_view(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.ui.session_panel
            && panel.view_mode == SessionViewMode::CurrentWorkspace
        {
            let sessions = self
                .runtime
                .session_manager()
                .store()
                .list_sessions(1000, 0)
                .unwrap_or_default();
            *panel = SessionPanelState::new(sessions, SessionViewMode::AllSessions);
        }
        Ok(())
    }

    pub(crate) fn toggle_multi_select_mode(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.ui.session_panel {
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
        if let Some(panel) = &mut self.ui.session_panel {
            let query = self.ui.composer.text().to_string();
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
        if let Some(panel) = self.ui.session_panel.take()
            && let SessionPanelDialog::DeleteConfirm { session_ids, .. } = panel.dialog
        {
            self.runtime
                .session_manager()
                .store()
                .delete_sessions(&session_ids)?;
            let count = session_ids.len();
            self.ui.last_notice = Some(format!("Deleted {} session(s)", count));
        }

        self.close_session_panel();
        self.open_session_panel(String::new())?;
        Ok(())
    }

    pub(crate) fn open_cleanup_dialog(&mut self) -> Result<()> {
        let sessions = self.runtime.session_manager().store()
            .get_sessions_older_than_preview(ChronoDuration::days(1))
            .unwrap_or_default();
        let preview = CleanupPreview::from_sessions(sessions);

        if let Some(panel) = &mut self.ui.session_panel {
            panel.dialog = SessionPanelDialog::Cleanup {
                preview,
                selected_duration: None,
                cleanup_workspace: false,
            };
        }
        Ok(())
    }

    pub(crate) fn select_cleanup_duration(&mut self, duration: ChronoDuration) -> Result<()> {
        if let Some(panel) = &mut self.ui.session_panel
            && let SessionPanelDialog::Cleanup { .. } = &panel.dialog
        {
            let sessions = self.runtime.session_manager().store()
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
        if let Some(panel) = &mut self.ui.session_panel
            && let SessionPanelDialog::Cleanup {
                preview: _,
                selected_duration,
                ..
            } = &panel.dialog
        {
            let sessions = self.runtime.session_manager().store()
                .get_current_workspace_sessions_count(Path::new(&self.runtime.workspace_root()))
                .unwrap_or(0);

            let new_preview = CleanupPreview {
                sessions: vec![],
                workspace_counts: vec![(
                    self.runtime.workspace_root().to_string_lossy().to_string(),
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
        if let Some(panel) = self.ui.session_panel.take()
            && let SessionPanelDialog::Cleanup {
                preview: _,
                selected_duration,
                cleanup_workspace,
            } = panel.dialog
        {
            if cleanup_workspace {
                let deleted = self.runtime.session_manager().store()
                    .delete_sessions_in_workspace(Path::new(&self.runtime.workspace_root()))?;
                let count = deleted.len();
                self.ui.last_notice =
                    Some(format!("Deleted {} session(s) in current workspace", count));
            } else if let Some(duration) = selected_duration {
                let deleted = self
                    .runtime
                    .session_manager()
                    .store()
                    .delete_sessions_older_than(duration)?;
                let count = deleted.len();
                self.ui.last_notice = Some(format!("Deleted {} old session(s)", count));
            }
        }

        self.close_session_panel();
        self.open_session_panel(String::new())?;
        Ok(())
    }

    pub(crate) fn open_export_dialog(&mut self) -> Result<()> {
        if let Some(panel) = &mut self.ui.session_panel {
            let query = self.ui.composer.text().to_string();
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
        if let Some(panel) = self.ui.session_panel.take()
            && let SessionPanelDialog::ExportConfirm { session_ids, .. } = panel.dialog
        {
            let export_dir = self.runtime.paths().data_dir.join("export");

            log::info!("Export dir: {}", export_dir.display());

            for session_id in &session_ids {
                match self
                    .runtime
                    .session_manager()
                    .store()
                    .export_session_to_jsonl(*session_id, &export_dir)
                {
                    Ok(path) => log::info!("Exported: {}", path.display()),
                    Err(e) => log::error!("Export failed: {}", e),
                }
            }

            let count = session_ids.len();
            self.ui.last_notice = Some(format!(
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
        if let Some(panel) = &mut self.ui.session_panel {
            panel.dialog = SessionPanelDialog::None;
        }
        Ok(())
    }

    pub(crate) fn switch_session(
        &mut self,
        session_id: Uuid,
    ) -> Result<()> {
        if self.ui.chat_context.session_id == session_id {
            self.ui.last_notice = Some("Already on that session".to_string());
            return Ok(());
        }

        let fallback_model =
            Self::resolve_fallback_model(&self.runtime.config(), &self.runtime.auth())?;
        self.cache_active_session_runtime();

        if let Err(error) = self.restore_or_load_session(session_id, &fallback_model) {
            self.ui.last_notice = Some(error.to_string());
            return Ok(());
        }

        self.ui.chat_context.session_id = session_id;

        self.ui.screen = if self.ui.chat_context.visible_messages().is_empty() {
            super::Screen::Welcome
        } else {
            super::Screen::Chat
        };
        self.clear_mouse_selection();
        self.ui.connect_dialog = None;
        self.ui.theme_panel = None;
        self.ui.model_panel = None;
        self.ui.session_panel = None;
        self.ui.command_palette.clear();

        if let Some(dialog) = self.ui.question_dialog.as_ref() {
            self.ui.composer.set_text(dialog.current_answer_text());
            self.ui
                .composer
                .set_placeholder(dialog.answer_placeholder());
        } else {
            self.ui
                .composer
                .set_placeholder("Ask tidev about your code, task, or question...");
        }

        if self.ui.pending_assistant_turns.remove(&session_id) {
            log::info!(
                "switch_session: session {} has pending assistant turn, starting now",
                session_id
            );
            if !self.ui.pending_request {
                self.spawn_agent_loop()?;
            }
        }

        Ok(())
    }

    /// Load or restore a session into the UI.
    ///
    /// Checks the in-memory cache first; if not found, loads from the
    /// database via [`SessionManager`] and builds a fresh [`ChatContext`].
    pub(crate) fn restore_or_load_session(
        &mut self,
        session_id: Uuid,
        _fallback_model: &tidev_config::auth::ActiveModel,
    ) -> Result<()> {
        // 1. Try the in-memory cache.
        if let Some(cached) = self.ui.cached_sessions.remove(&session_id) {
            self.ui.chat_context = crate::chat_context::ChatContext::new(
                session_id,
                String::new(), // title not cached yet
                self.runtime.workspace_root().to_string_lossy().to_string(),
                cached.messages,
                None,
                cached.provider_id,
                cached.model_id,
                String::new(),
                String::new(),
            );
            return Ok(());
        }

        // 2. Load from the database.
        let record = self
            .runtime
            .session_manager()
            .load_session(session_id)?
            .context("session not found")?;
        let messages = self
            .runtime
            .session_manager()
            .load_messages(session_id)?;

        self.ui.chat_context = crate::chat_context::ChatContext::new(
            session_id,
            record.title,
            record.workspace_root,
            messages,
            record.parent_session_id,
            record.provider_id,
            record.model_id,
            record.model_display_name,
            record.provider_display_name,
        );
        Ok(())
    }

    /// Start (or resume) the agent loop for the current session.
    ///
    /// Used when switching to a session that has a pending assistant turn
    /// (e.g. after a subagent returned its result while the user was away).
    pub(crate) fn spawn_agent_loop(&mut self) -> Result<()> {
        if self.ui.pending_request {
            return Ok(());
        }

        self.ui.pending_request = true;
        self.ui.last_notice = Some(match self.ui.mode {
            tidev_types::prompts::SessionMode::Plan => "Planning...".to_string(),
            tidev_types::prompts::SessionMode::Build => "Thinking...".to_string(),
        });

        let session_id = self.ui.chat_context.session_id;
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = runtime.continue_session(session_id).await {
                log::error!("spawn_agent_loop: continue_session failed: {e}");
            }
        });

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
