//! SessionPanel component — session list with search, multi-select,
//! and embedded sub-dialogs (delete confirm, cleanup, export confirm).
//!
//! Mirrors the old `tidev_tui::ui::session_panel` and associated
//! render/input modules with a self-contained Component implementation.

use std::path::Path;

use anyhow::Result;
use chrono::{Duration as ChronoDuration, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState};
use tidev_core::SessionRecord;
use unicode_width::UnicodeWidthStr;
use crate::utils::shorten;
use uuid::Uuid;

use crate::action::{Action, OverlayAction, OverlayKind, SessionAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

// ---------------------------------------------------------------------------
// Enums & types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SessionViewMode {
    CurrentWorkspace,
    AllSessions,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OperationMode {
    Select,
    MultiSelect,
}

#[derive(Clone, Debug)]
pub(crate) struct CleanupPreview {
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
pub(crate) enum SessionPanelDialog {
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

// ---------------------------------------------------------------------------
// SessionPanel component
// ---------------------------------------------------------------------------

pub(crate) struct SessionPanel {
    selected_index: usize,
    sessions: Vec<SessionRecord>,
    view_mode: SessionViewMode,
    operation_mode: OperationMode,
    selected_indices: Vec<usize>,
    dialog: SessionPanelDialog,
    query: String,
    /// Session ID of the currently active session (for marking in the list).
    current_session_id: Uuid,
}

impl SessionPanel {
    pub(crate) fn new(
        sessions: Vec<SessionRecord>,
        view_mode: SessionViewMode,
        current_session_id: Uuid,
    ) -> Self {
        Self {
            selected_index: 0,
            sessions,
            view_mode,
            operation_mode: OperationMode::Select,
            selected_indices: Vec::new(),
            dialog: SessionPanelDialog::None,
            query: String::new(),
            current_session_id,
        }
    }

    // ── Query helpers ──

    fn matching_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return (0..self.sessions.len()).collect();
        }
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let title = s.title.to_ascii_lowercase();
                let provider = s.provider_display_name.to_ascii_lowercase();
                let model = s.model_display_name.to_ascii_lowercase();
                let sid = s.session_id.to_string().to_ascii_lowercase();
                let ws = s.workspace_root.to_ascii_lowercase();
                (title.contains(&query)
                    || provider.contains(&query)
                    || model.contains(&query)
                    || sid.contains(&query)
                    || ws.contains(&query))
                .then_some(i)
            })
            .collect()
    }

    fn reset_selection(&mut self) {
        let matches = self.matching_indices();
        if matches.is_empty() {
            self.selected_index = 0;
            return;
        }
        // Try to stay on the current session
        if let Some(pos) = matches
            .iter()
            .position(|&i| self.sessions[i].session_id == self.current_session_id)
        {
            self.selected_index = pos;
            return;
        }
        self.selected_index = self.selected_index.min(matches.len().saturating_sub(1));
    }

    fn move_selection(&mut self, delta: isize) {
        let matches = self.matching_indices();
        if matches.is_empty() {
            self.selected_index = 0;
            return;
        }
        let len = matches.len() as isize;
        let current = self
            .selected_index
            .min(matches.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    fn selected_session(&self) -> Option<&SessionRecord> {
        let matches = self.matching_indices();
        let session_index = *matches.get(self.selected_index)?;
        self.sessions.get(session_index)
    }

    // ── Multi-select ──

    fn toggle_selection(&mut self) {
        if self.operation_mode != OperationMode::MultiSelect {
            return;
        }
        let matches = self.matching_indices();
        if let Some(&session_index) = matches.get(self.selected_index) {
            if let Some(pos) = self.selected_indices.iter().position(|&i| i == session_index) {
                self.selected_indices.remove(pos);
            } else {
                self.selected_indices.push(session_index);
            }
        }
    }

    fn is_selected(&self, session_index: usize) -> bool {
        self.selected_indices.contains(&session_index)
    }

    fn selected_count(&self) -> usize {
        self.selected_indices.len()
    }

    fn get_selected_session_ids(&self) -> Vec<Uuid> {
        if self.operation_mode == OperationMode::MultiSelect && !self.selected_indices.is_empty()
        {
            self.selected_indices
                .iter()
                .filter_map(|&i| self.sessions.get(i).map(|s| s.session_id))
                .collect()
        } else {
            self.selected_session()
                .map(|s| vec![s.session_id])
                .unwrap_or_default()
        }
    }

    fn get_selected_session_titles(&self) -> Vec<String> {
        if self.operation_mode == OperationMode::MultiSelect && !self.selected_indices.is_empty()
        {
            self.selected_indices
                .iter()
                .filter_map(|&i| self.sessions.get(i).map(|s| s.title.clone()))
                .collect()
        } else {
            self.selected_session()
                .map(|s| vec![s.title.clone()])
                .unwrap_or_default()
        }
    }
}

impl Component for SessionPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        // ── Sub-dialog key handling ──
        match &self.dialog {
            SessionPanelDialog::DeleteConfirm { .. } => {
                return match key.code {
                    KeyCode::Enter => {
                        Some(Action::Overlay(OverlayAction::Close(OverlayKind::SessionPanel)))
                    }
                    KeyCode::Esc => {
                        self.dialog = SessionPanelDialog::None;
                        None
                    }
                    _ => None,
                };
            }
            SessionPanelDialog::Cleanup { .. } => {
                return match key.code {
                    KeyCode::Char('1') => {
                        if let SessionPanelDialog::Cleanup {
                            ref mut selected_duration,
                            ref mut cleanup_workspace,
                            ..
                        } = self.dialog
                        {
                            *selected_duration = Some(ChronoDuration::weeks(1));
                            *cleanup_workspace = false;
                        }
                        None
                    }
                    KeyCode::Char('2') => {
                        if let SessionPanelDialog::Cleanup {
                            ref mut selected_duration,
                            ref mut cleanup_workspace,
                            ..
                        } = self.dialog
                        {
                            *selected_duration = Some(ChronoDuration::days(30));
                            *cleanup_workspace = false;
                        }
                        None
                    }
                    KeyCode::Char('3') => {
                        if let SessionPanelDialog::Cleanup {
                            ref mut selected_duration,
                            ref mut cleanup_workspace,
                            ..
                        } = self.dialog
                        {
                            *selected_duration = Some(ChronoDuration::days(90));
                            *cleanup_workspace = false;
                        }
                        None
                    }
                    KeyCode::Char('4') => {
                        if let SessionPanelDialog::Cleanup {
                            ref mut selected_duration,
                            ref mut cleanup_workspace,
                            ..
                        } = self.dialog
                        {
                            *selected_duration = Some(ChronoDuration::days(365));
                            *cleanup_workspace = false;
                        }
                        None
                    }
                    KeyCode::Char('5') => {
                        if let SessionPanelDialog::Cleanup {
                            ref mut cleanup_workspace,
                            ..
                        } = self.dialog
                        {
                            *cleanup_workspace = !*cleanup_workspace;
                        }
                        None
                    }
                    KeyCode::Enter => {
                        Some(Action::Overlay(OverlayAction::Close(OverlayKind::SessionPanel)))
                    }
                    KeyCode::Esc => {
                        self.dialog = SessionPanelDialog::None;
                        None
                    }
                    _ => None,
                };
            }
            SessionPanelDialog::ExportConfirm { .. } => {
                return match key.code {
                    KeyCode::Enter => {
                        Some(Action::Overlay(OverlayAction::Close(OverlayKind::SessionPanel)))
                    }
                    KeyCode::Esc => {
                        self.dialog = SessionPanelDialog::None;
                        None
                    }
                    _ => None,
                };
            }
            SessionPanelDialog::None => {}
        }

        // ── Main panel key handling ──
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-1);
                None
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(1);
                None
            }
            KeyCode::Enter => {
                self.selected_session()
                    .map(|s| Action::Session(SessionAction::Select(s.session_id)))
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.operation_mode = if self.operation_mode == OperationMode::MultiSelect {
                    OperationMode::Select
                } else {
                    OperationMode::MultiSelect
                };
                self.selected_indices.clear();
                None
            }
            KeyCode::Char(' ') => {
                self.toggle_selection();
                None
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                let ids = self.get_selected_session_ids();
                let titles = self.get_selected_session_titles();
                if !ids.is_empty() {
                    self.dialog = SessionPanelDialog::DeleteConfirm {
                        session_ids: ids,
                        session_titles: titles,
                    };
                }
                None
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                let preview = CleanupPreview::from_sessions(self.sessions.clone());
                self.dialog = SessionPanelDialog::Cleanup {
                    preview,
                    selected_duration: None,
                    cleanup_workspace: false,
                };
                None
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                let ids = self.get_selected_session_ids();
                let titles = self.get_selected_session_titles();
                if !ids.is_empty() {
                    self.dialog = SessionPanelDialog::ExportConfirm {
                        session_ids: ids,
                        session_titles: titles,
                    };
                }
                None
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                self.view_mode = if self.view_mode == SessionViewMode::CurrentWorkspace {
                    SessionViewMode::AllSessions
                } else {
                    SessionViewMode::CurrentWorkspace
                };
                // Reload sessions via action broadcast
                Some(Action::Session(SessionAction::Reload))
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.view_mode = if self.view_mode == SessionViewMode::CurrentWorkspace {
                    SessionViewMode::AllSessions
                } else {
                    SessionViewMode::CurrentWorkspace
                };
                Some(Action::Session(SessionAction::Reload))
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                Some(Action::Overlay(OverlayAction::Close(OverlayKind::SessionPanel)))
            }
            KeyCode::Backspace => {
                if !self.query.is_empty() {
                    self.query.pop();
                    self.reset_selection();
                }
                None
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.query.push(ch);
                self.reset_selection();
                None
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
                None
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
                None
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Session(SessionAction::Reload) => {
                let session_store = ctx.runtime.session_manager().store();
                let sessions = match self.view_mode {
                    SessionViewMode::CurrentWorkspace => {
                        let workspace_root = ctx
                            .runtime
                            .workspace_root()
                            .display()
                            .to_string();
                        session_store
                            .list_sessions_for_workspace(&workspace_root, 1000, 0)
                            .unwrap_or_default()
                    }
                    SessionViewMode::AllSessions => {
                        session_store.list_sessions(1000, 0).unwrap_or_default()
                    }
                };
                self.sessions = sessions;
                self.selected_index = 0;
                self.selected_indices.clear();
                self.reset_selection();
                vec![]
            }
            Action::Overlay(OverlayAction::Close(OverlayKind::SessionPanel)) => {
                // Determine what action to perform based on dialog state
                match &self.dialog {
                    SessionPanelDialog::DeleteConfirm {
                        session_ids, ..
                    } => {
                        // Execute deletion (sync)
                        let ids = session_ids.clone();
                        if let Err(e) =
                            ctx.runtime.session_manager().store().delete_sessions(&ids)
                        {
                            log::error!("Failed to delete sessions: {e}");
                        }
                        vec![]
                    }
                    SessionPanelDialog::Cleanup {
                        selected_duration,
                        cleanup_workspace,
                        ..
                    } => {
                        if *cleanup_workspace {
                            let ws = ctx.runtime.workspace_root();
                            if let Err(e) = ctx
                                .runtime
                                .session_manager()
                                .store()
                                .delete_sessions_in_workspace(Path::new(ws))
                            {
                                log::error!("Failed to cleanup workspace: {e}");
                            }
                        } else if let Some(duration) = selected_duration {
                            if let Err(e) = ctx
                                .runtime
                                .session_manager()
                                .store()
                                .delete_sessions_older_than(*duration)
                            {
                                log::error!("Failed to cleanup sessions: {e}");
                            }
                        }
                        vec![]
                    }
                    SessionPanelDialog::ExportConfirm {
                        session_ids, ..
                    } => {
                        let export_dir = ctx.runtime.paths().data_dir.join("export");
                        for session_id in session_ids {
                            if let Err(e) = ctx
                                .runtime
                                .session_manager()
                                .store()
                                .export_session_to_jsonl(*session_id, &export_dir)
                            {
                                log::error!("Failed to export session {session_id}: {e}");
                            }
                        }
                        vec![]
                    }
                    SessionPanelDialog::None => {
                        vec![]
                    }
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;

        // ── If sub-dialog active, render it ──
        match &self.dialog {
            SessionPanelDialog::None => {} // render main panel below
            _ => {
                self.render_dialog(frame, rect, ctx);
                return;
            }
        }

        // ── Main panel ──
        let overlay = centered_rect(rect.width.min(112), rect.height.min(36), rect);
        frame.render_widget(Clear, overlay);

        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let view_mode_text = match self.view_mode {
            SessionViewMode::CurrentWorkspace => "Current Workspace",
            SessionViewMode::AllSessions => "All Sessions",
        };
        let title_text = format!(" Sessions: {} ", view_mode_text);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

        // Title
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                &title_text,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // Instruction
        frame.render_widget(
            Paragraph::new(
                "Type to filter by title, model, provider, or session id.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        // Search input
        let input_style = Style::default().bg(palette.panel_alt);
        let prefix = " Search sessions: ";
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.muted)),
                Span::styled(&self.query, Style::default().fg(palette.text)),
            ]))
            .style(input_style),
            sections[2],
        );
        frame.set_cursor_position((
            sections[2].x + UnicodeWidthStr::width(prefix) as u16 + self.query.as_str().width() as u16,
            sections[2].y,
        ));

        // Session list
        let matches = self.matching_indices();
        let is_multi_select = self.operation_mode == OperationMode::MultiSelect;

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No sessions match this search.")
                    .alignment(Alignment::Center)
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.muted),
                    ),
                sections[3],
            );
        } else {
            // Compute minimum width for the right column
            let max_right_width = matches
                .iter()
                .map(|&idx| {
                    let session = &self.sessions[idx];
                    let pm = format!(
                        "{} / {}",
                        shorten(&session.provider_display_name, 12),
                        shorten(&session.model_display_name, 14)
                    );
                    let time = session
                        .updated_at
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string();
                    let mut w = pm.chars().count() + 2 + time.chars().count();
                    if session.session_id == self.current_session_id {
                        w += 9; // "  current"
                    }
                    if session.parent_session_id.is_some() {
                        w += 7; // "  child"
                    }
                    w
                })
                .max()
                .unwrap_or(35)
                .max(30) as u16;

            let mut rows: Vec<Row> = Vec::new();
            let mut current_workspace = String::new();

            for &index in matches.iter() {
                let session = &self.sessions[index];

                // Workspace separator in AllSessions mode
                if self.view_mode == SessionViewMode::AllSessions
                    && session.workspace_root != current_workspace
                {
                    if !current_workspace.is_empty() {
                        rows.push(Row::new(vec![Cell::from(""), Cell::from("")]));
                    }
                    current_workspace = session.workspace_root.clone();
                    rows.push(Row::new(vec![
                        Cell::from(Line::from(vec![Span::styled(
                            format!("[ {} ]", session.workspace_root),
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD),
                        )])),
                        Cell::from(""),
                    ]));
                }

                let is_current = session.session_id == self.current_session_id;
                let updated_at = session
                    .updated_at
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string();
                let is_sel = self.is_selected(index);

                let checkbox = if is_multi_select {
                    if is_sel { "[✓] " } else { "[ ] " }
                } else {
                    ""
                };

                // Left cell: checkbox + title
                let left_line = Line::from(vec![
                    Span::raw(checkbox),
                    Span::styled(
                        shorten(
                            &session.title,
                            sections[3]
                                .width
                                .saturating_sub(max_right_width + 4)
                                as usize,
                        ),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]);

                // Right cell: provider/model + time + badges
                let provider_model = format!(
                    "{} / {}",
                    shorten(&session.provider_display_name, 12),
                    shorten(&session.model_display_name, 14)
                );
                let mut right_spans: Vec<Span> = vec![
                    Span::styled(provider_model, Style::default().fg(palette.accent_soft)),
                    Span::raw("  "),
                    Span::styled(updated_at, Style::default().fg(palette.muted)),
                ];

                if is_current {
                    right_spans.push(Span::raw("  "));
                    right_spans.push(Span::styled(
                        "current",
                        Style::default().fg(palette.success),
                    ));
                }
                if session.parent_session_id.is_some() {
                    right_spans.push(Span::raw("  "));
                    right_spans.push(Span::styled(
                        "child",
                        Style::default().fg(palette.accent_soft),
                    ));
                }

                let right_line = Line::from(right_spans).alignment(Alignment::Right);

                rows.push(Row::new(vec![
                    Cell::from(left_line),
                    Cell::from(right_line),
                ]));
            }

            let mut state = TableState::default();
            state.select(Some(
                self.selected_index.min(matches.len().saturating_sub(1)),
            ));

            let table = Table::new(
                rows,
                [Constraint::Fill(1), Constraint::Min(max_right_width)],
            )
            .style(
                Style::default()
                    .bg(palette.panel_alt)
                    .fg(palette.text),
            )
            .row_highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

            frame.render_stateful_widget(table, sections[3], &mut state);
        }

        // Footer
        let help_text = if self.operation_mode == OperationMode::MultiSelect {
            "Enter/D: switch/delete · Space: select · Ctrl+A: exit multi-select · Tab: switch view · C: cleanup · E: export"
        } else {
            "Enter: switch · D: delete · C: cleanup · Ctrl+A: multi-select · Tab: switch view · W: all sessions · E: export"
        };
        frame.render_widget(
            Paragraph::new(help_text)
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.muted),
                ),
            sections[4],
        );
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        10
    }

    fn blocks_input(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Sub-dialog rendering (inline methods)
// ---------------------------------------------------------------------------

impl SessionPanel {
    fn render_dialog(&self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;

        match &self.dialog {
            SessionPanelDialog::None => {}
            SessionPanelDialog::DeleteConfirm {
                session_ids,
                session_titles,
            } => {
                let overlay = centered_rect(60, 20, rect);
                frame.render_widget(Clear, overlay);
                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(inner);

                // Title
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Delete session(s) ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Message
                frame.render_widget(
                    Paragraph::new(format!("Delete {} session(s)?", session_ids.len()))
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.text),
                        ),
                    sections[1],
                );

                // Session titles
                let mut content = String::new();
                for title in session_titles.iter().take(5) {
                    content.push_str(&format!("  • {}\n", title));
                }
                if session_titles.len() > 5 {
                    content.push_str(&format!(
                        "  ... and {} more\n",
                        session_titles.len() - 5
                    ));
                }
                frame.render_widget(
                    Paragraph::new(content)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.muted),
                        ),
                    sections[2],
                );

                // Footer
                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[3],
                );
            }
            SessionPanelDialog::ExportConfirm {
                session_ids,
                session_titles,
            } => {
                let overlay = centered_rect(60, 20, rect);
                frame.render_widget(Clear, overlay);
                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(inner);

                // Title
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Export session(s) ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Message
                frame.render_widget(
                    Paragraph::new(format!(
                        "Export {} session(s) to JSONL?",
                        session_ids.len()
                    ))
                    .alignment(Alignment::Center)
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.text),
                    ),
                    sections[1],
                );

                // Session titles
                let mut content = String::new();
                for title in session_titles.iter().take(5) {
                    content.push_str(&format!("  • {}\n", title));
                }
                if session_titles.len() > 5 {
                    content.push_str(&format!(
                        "  ... and {} more\n",
                        session_titles.len() - 5
                    ));
                }
                frame.render_widget(
                    Paragraph::new(content)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.muted),
                        ),
                    sections[2],
                );

                // Footer
                frame.render_widget(
                    Paragraph::new("Enter: export · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[3],
                );
            }
            SessionPanelDialog::Cleanup {
                preview,
                selected_duration,
                cleanup_workspace,
            } => {
                let overlay = centered_rect(70, 25, rect);
                frame.render_widget(Clear, overlay);
                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Min(6),
                    Constraint::Length(3),
                ])
                .split(inner);

                // Title
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Cleanup Sessions ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Status message
                let (title_text, hint_text) = if *cleanup_workspace {
                    (
                        "Delete all sessions in current workspace".to_string(),
                        "5: current workspace (selected)".to_string(),
                    )
                } else {
                    let duration_text = match selected_duration {
                        Some(d) if *d <= ChronoDuration::weeks(1) => "1 week",
                        Some(d) if *d <= ChronoDuration::days(30) => "1 month",
                        Some(d) if *d <= ChronoDuration::days(90) => "3 months",
                        Some(d) if *d <= ChronoDuration::days(365) => "1 year",
                        None => "Select duration",
                        _ => "Custom",
                    };
                    (
                        format!("Delete sessions older than: {}", duration_text),
                        "1: 1 week · 2: 1 month · 3: 3 months · 4: 1 year · 5: current workspace"
                            .to_string(),
                    )
                };

                frame.render_widget(
                    Paragraph::new(title_text)
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.text),
                        ),
                    sections[1],
                );
                frame.render_widget(
                    Paragraph::new(hint_text)
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[2],
                );

                // Preview counts
                let mut preview_text = String::new();
                preview_text.push_str(&format!(
                    "Total: {} session(s)\n",
                    preview.total_count
                ));
                for (ws, count) in &preview.workspace_counts {
                    preview_text.push_str(&format!("  • {}: {}\n", ws, count));
                }
                frame.render_widget(
                    Paragraph::new(preview_text)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.muted),
                        ),
                    sections[4],
                );

                // Footer
                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[5],
                );
            }
        }
    }
}
