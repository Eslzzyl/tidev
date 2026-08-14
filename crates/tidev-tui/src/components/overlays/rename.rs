//! RenameDialog — inline text-input dialog for renaming a session.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use uuid::Uuid;

use crate::action::{Action, OverlayAction, OverlayKind, SessionAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{centered_rect, paste_from_clipboard, wrapped_input_tail};

pub(crate) struct RenameDialog {
    session_id: Uuid,
    original_title: String,
    /// Current text in the edit buffer.
    buffer: String,
    /// Whether the user pressed Enter (confirm) vs Esc (cancel).
    confirmed: bool,
}

impl RenameDialog {
    pub(crate) fn new(session_id: Uuid, original_title: String) -> Self {
        Self {
            session_id,
            original_title: original_title.clone(),
            buffer: original_title,
            confirmed: false,
        }
    }

    fn title(&self) -> String {
        "Rename session".to_string()
    }

    fn description(&self) -> String {
        format!("Current title: {}", self.original_title)
    }
}

impl Component for RenameDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            // Cancel
            KeyCode::Esc => {
                self.confirmed = false;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::RenameDialog,
                )))
            }
            // Confirm (Enter without Shift or Alt)
            KeyCode::Enter
                if !key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.confirmed = true;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::RenameDialog,
                )))
            }
            // Paste (Ctrl+V / Cmd+V)
            KeyCode::Char('v')
                if (key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::SUPER))
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                if let Some(text) = paste_from_clipboard() {
                    self.buffer.push_str(&text);
                }
                None
            }
            // Edit buffer
            KeyCode::Char(c) if !c.is_control() => {
                self.buffer.push(c);
                None
            }
            KeyCode::Backspace => {
                self.buffer.pop();
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::RenameDialog)) => {
                if self.confirmed && !self.buffer.is_empty() {
                    vec![Action::Session(SessionAction::Rename(
                        self.session_id,
                        self.buffer.clone(),
                    ))]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(60, 12, rect);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

        // Title
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                self.title(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // Description
        frame.render_widget(
            Paragraph::new(self.description())
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[1],
        );

        // Help text
        frame.render_widget(
            Paragraph::new("Press Enter to save, Esc to cancel")
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );

        // Input field
        let (visible_lines, cursor) = wrapped_input_tail(&self.buffer, sections[4]);
        let input_text = if self.buffer.is_empty() {
            "New session title...".to_string()
        } else {
            visible_lines.join("\n")
        };
        frame.render_widget(
            Paragraph::new(input_text)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .wrap(Wrap { trim: false }),
            sections[4],
        );
        frame.set_cursor_position(cursor);

        // Bottom hint
        frame.render_widget(
            Paragraph::new("Type a new name, then press Enter")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[5],
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

    fn wants_terminal_cursor(&self) -> bool {
        true
    }

    fn handle_paste(&mut self, text: &str) -> Option<Action> {
        if !text.is_empty() {
            self.buffer.push_str(text);
        }
        None
    }
}
