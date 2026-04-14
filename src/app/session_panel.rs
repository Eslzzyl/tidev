use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;
use uuid::Uuid;

use crate::{context::ContextManager, storage::SessionRecord};

use super::App;

#[derive(Clone, Debug)]
pub struct SessionPanelState {
    pub selected_index: usize,
    pub sessions: Vec<SessionRecord>,
}

impl SessionPanelState {
    pub fn new(sessions: Vec<SessionRecord>) -> Self {
        Self {
            selected_index: 0,
            sessions,
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
        self.session_panel = Some(SessionPanelState::new(sessions));
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

    pub(crate) fn handle_session_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(panel) = self.session_panel.clone() else {
            return Ok(());
        };

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
                    self.switch_session(session.session_id)?;
                    self.close_session_panel();
                }
            }
            KeyCode::Esc => {
                self.close_session_panel();
            }
            KeyCode::Tab => {}
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

    pub(crate) fn switch_session(&mut self, session_id: Uuid) -> Result<()> {
        if self.conversation.session_id == session_id {
            self.last_notice = Some("Already on that session".to_string());
            return Ok(());
        }

        let Some(conversation) = self.store.load_conversation(session_id)? else {
            self.last_notice = Some("Session not found".to_string());
            return Ok(());
        };

        let fallback_model = Self::resolve_fallback_model(&self.config, &self.auth)?;
        let active_model =
            Self::resolve_conversation_model(&self.config, &self.auth, &conversation)
                .unwrap_or_else(|_| fallback_model.clone());

        self.pending_request = false;
        self.pending_tool_execution = None;
        self.permission_dialog = None;
        self.running_tool_execution = None;
        self.abort_confirmation_deadline = None;
        self.active_request_id = self.active_request_id.wrapping_add(1);
        self.streaming_markdown = None;
        self.streaming_preview_lines.clear();
        self.context_manager = ContextManager::new();
        self.conversation = conversation;
        self.active_model = active_model;

        if !self.conversation.visible_messages().is_empty() {
            let total_tokens: u32 = self
                .conversation
                .messages
                .iter()
                .filter_map(|m| m.total_tokens)
                .sum();
            if total_tokens > 0 {
                self.context_usage = Some((0, 0, total_tokens));
            }
        }
        self.screen = if self.conversation.visible_messages().is_empty() {
            super::Screen::Welcome
        } else {
            super::Screen::Chat
        };
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.command_palette.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.scroll_messages_to_bottom();

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
