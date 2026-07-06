//! UndoConfirmDialog — confirmation prompt before undoing to a message.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use uuid::Uuid;

use crate::action::{Action, OverlayAction, OverlayKind, SessionAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

pub(crate) struct UndoConfirmDialog {
    selected_message_id: Uuid,
    message_content: String,
    /// Whether the user pressed Enter (confirm) vs Esc/N (cancel).
    confirmed: bool,
}

impl UndoConfirmDialog {
    pub(crate) fn new(selected_message_id: Uuid, message_content: String) -> Self {
        Self {
            selected_message_id,
            message_content,
            confirmed: false,
        }
    }

    fn title(&self) -> String {
        "Undo to message".to_string()
    }

    fn description(&self) -> String {
        let preview = if self.message_content.len() > 50 {
            format!("{}...", &self.message_content[..50])
        } else {
            self.message_content.clone()
        };
        format!(
            "Revert workspace to this message?\n\"{}\"\n\nThis will undo all changes after this message.",
            preview
        )
    }
}

impl Component for UndoConfirmDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            // Confirm undo
            KeyCode::Enter => {
                self.confirmed = true;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::UndoConfirmDialog {
                        message_id: self.selected_message_id,
                        content: self.message_content.clone(),
                    },
                )))
            }
            // Cancel
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.confirmed = false;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::UndoConfirmDialog {
                        message_id: self.selected_message_id,
                        content: self.message_content.clone(),
                    },
                )))
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::UndoConfirmDialog { .. })) => {
                if self.confirmed {
                    vec![Action::Session(SessionAction::Undo)]
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
            Constraint::Length(2),
            Constraint::Min(5),
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
                .alignment(ratatui::layout::Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[2],
        );

        // Help text
        frame.render_widget(
            Paragraph::new("Enter to confirm · Esc or N to cancel")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[3],
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
