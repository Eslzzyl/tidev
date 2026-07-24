//! MessagePanel component — browse and search user messages in the current session.
//!
//! Mirrors the old `tidev_tui::ui::message_panel` module with a self-contained
//! Component implementation.

use crate::utils::shorten;
use anyhow::Result;
use chrono::{Local, Utc};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState};
use tidev_types::prompts::SessionMode;
use uuid::Uuid;

use crate::action::{Action, ChatAction, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{centered_rect, strip_system_reminder_tags};
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// MessagePanelMessage
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) struct MessagePanelMessage {
    pub message_id: Uuid,
    pub content: String,
    pub created_at: chrono::DateTime<Utc>,
    pub mode: Option<SessionMode>,
    /// Position of this message in the full conversation (0-based).
    pub original_index: usize,
}

// ---------------------------------------------------------------------------
// Query helper
// ---------------------------------------------------------------------------

fn message_matches_query(query: &str, message: &MessagePanelMessage) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.trim().to_ascii_lowercase();
    let content = message.content.to_ascii_lowercase();
    let id = message.message_id.to_string().to_ascii_lowercase();
    content.contains(&query) || id.contains(&query)
}

// ---------------------------------------------------------------------------
// MessagePanel component
// ---------------------------------------------------------------------------

pub(crate) struct MessagePanel {
    selected_index: usize,
    messages: Vec<MessagePanelMessage>,
    query: String,
    /// When set, closing the panel will also emit a ScrollTo for this message.
    /// Set by Enter / mouse-click, NOT by Esc.
    pending_scroll_id: Option<Uuid>,
}

impl MessagePanel {
    pub(crate) fn new(messages: Vec<MessagePanelMessage>) -> Self {
        Self {
            selected_index: 0,
            messages,
            query: String::new(),
            pending_scroll_id: None,
        }
    }

    /// Indices of messages matching the current query.
    fn matching_indices(&self) -> Vec<usize> {
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                if message_matches_query(&self.query, message) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect()
    }

    fn reset_selection(&mut self) {
        let matches = self.matching_indices();
        self.selected_index = matches.first().copied().unwrap_or(0);
    }

    fn move_selection(&mut self, delta: isize) {
        let matches = self.matching_indices();
        if matches.is_empty() {
            self.selected_index = 0;
            return;
        }
        let len = matches.len() as isize;
        let current = self.selected_index.min(matches.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    fn selected_message(&self) -> Option<&MessagePanelMessage> {
        let matches = self.matching_indices();
        let message_index = *matches.get(self.selected_index)?;
        self.messages.get(message_index)
    }
}

impl Component for MessagePanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

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
                if let Some(message) = self.selected_message() {
                    self.pending_scroll_id = Some(message.message_id);
                    let close = Action::Overlay(OverlayAction::Close(OverlayKind::MessagePanel));
                    Some(close)
                } else {
                    None
                }
            }
            KeyCode::Char('f') => {
                if let Some(message) = self.selected_message() {
                    let message_count = message.original_index + 1;
                    Some(Action::Overlay(OverlayAction::Open(
                        OverlayKind::ForkConfirmDialog {
                            message_id: message.message_id,
                            message_count,
                        },
                    )))
                } else {
                    None
                }
            }
            KeyCode::Char('u') => self.selected_message().map(|message| {
                Action::Overlay(OverlayAction::Open(OverlayKind::UndoConfirmDialog {
                    message_id: message.message_id,
                    content: message.content.clone(),
                }))
            }),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.pending_scroll_id = None;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::MessagePanel,
                )))
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

        let overlay = centered_rect(area.width.min(112), area.height.min(36), area);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
                None
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                // offset for title(1) + instruction(2) + input(3) = 6 header rows
                let header_rows = 6u16;
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < header_rows {
                    return Some(Action::Noop);
                }
                let row = (local_y - header_rows) as usize;
                let matches = self.matching_indices();
                if row < matches.len() {
                    self.selected_index = row;
                    // Same as Enter on selected message
                    if let Some(message) = self.selected_message() {
                        self.pending_scroll_id = Some(message.message_id);
                        let close =
                            Action::Overlay(OverlayAction::Close(OverlayKind::MessagePanel));
                        return Some(close);
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            // When Enter / mouse-click closes the panel, also emit ScrollTo to jump
            // to the message. Esc closes without scrolling (pending_scroll_id is None).
            Action::Overlay(OverlayAction::Close(OverlayKind::MessagePanel)) => {
                let scroll = self
                    .pending_scroll_id
                    .take()
                    .map(|id| Action::Chat(ChatAction::ScrollTo(id)));
                scroll.into_iter().collect()
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(rect.width.min(112), rect.height.min(36), rect);
        frame.render_widget(Clear, overlay);

        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(2), // instruction
            Constraint::Length(3), // search input
            Constraint::Min(8),    // message list / table
            Constraint::Length(1), // footer
        ])
        .split(inner);

        // ── Title ──
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " User messages ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // ── Instruction ──
        frame.render_widget(
            Paragraph::new(
                "Type to filter current session user messages. Enter jumps to the selected message.",
            )
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        // ── Search input ──
        let input_style = Style::default().bg(palette.panel_alt);
        let prefix = " Search user messages: ";
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.muted)),
                Span::styled(&self.query, Style::default().fg(palette.text)),
            ]))
            .style(input_style),
            sections[2],
        );
        frame.set_cursor_position((
            sections[2].x + prefix.width() as u16 + self.query.as_str().width() as u16,
            sections[2].y,
        ));

        // ── Message list ──
        let matches = self.matching_indices();
        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No user messages match this search.")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                sections[3],
            );
        } else {
            let mut rows: Vec<Row> = Vec::new();
            for index in matches.iter() {
                let message = &self.messages[*index];

                // Timestamp column
                let ts_str = format!(
                    "{:<16}",
                    message
                        .created_at
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                );
                let ts_cell = Cell::from(Line::from(vec![Span::styled(
                    ts_str,
                    Style::default().fg(palette.accent_soft),
                )]));

                // Mode column
                let mode_str = match message.mode {
                    Some(SessionMode::Build) => " Build",
                    Some(SessionMode::Plan) => "  Plan",
                    None => "      ",
                };
                let mode_color = message.mode.map_or(palette.muted, |m| match m {
                    SessionMode::Build => palette.mode_build,
                    SessionMode::Plan => palette.mode_plan,
                });
                let mode_cell = Cell::from(Line::from(vec![Span::styled(
                    mode_str,
                    Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
                )]));

                // Content column
                let stripped = strip_system_reminder_tags(&message.content);
                let content_cell = Cell::from(Line::from(vec![Span::styled(
                    shorten(&stripped, 80),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                )]));

                rows.push(Row::new(vec![ts_cell, mode_cell, content_cell]));
            }

            let mut state = TableState::default();
            state.select(Some(
                self.selected_index.min(matches.len().saturating_sub(1)),
            ));

            let table = Table::new(
                rows,
                [
                    Constraint::Length(17),
                    Constraint::Length(6),
                    Constraint::Fill(1),
                ],
            )
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .row_highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

            frame.render_stateful_widget(table, sections[3], &mut state);
        }

        // ── Footer ──
        frame.render_widget(
            Paragraph::new("Enter: jump · Esc: close · Ctrl+P/N: nav")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
