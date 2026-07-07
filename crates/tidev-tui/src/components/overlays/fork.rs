//! ForkConfirmDialog — confirmation prompt before forking a session.

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

pub(crate) struct ForkConfirmDialog {
    selected_message_id: Uuid,
    message_count: usize,
    /// Whether the user pressed Enter (confirm) vs Esc/N (cancel).
    confirmed: bool,
}

impl ForkConfirmDialog {
    pub(crate) fn new(selected_message_id: Uuid, message_count: usize) -> Self {
        Self {
            selected_message_id,
            message_count,
            confirmed: false,
        }
    }

    fn title(&self) -> String {
        "Fork session".to_string()
    }

    fn description(&self) -> String {
        format!(
            "Create a new session from this message? This will copy {} message{} to a new session.",
            self.message_count,
            if self.message_count == 1 { "" } else { "s" }
        )
    }
}

impl Component for ForkConfirmDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            // Confirm fork
            KeyCode::Enter => {
                self.confirmed = true;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::ForkConfirmDialog {
                        message_id: self.selected_message_id,
                        message_count: self.message_count,
                    },
                )))
            }
            // Cancel
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.confirmed = false;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::ForkConfirmDialog {
                        message_id: self.selected_message_id,
                        message_count: self.message_count,
                    },
                )))
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::ForkConfirmDialog { .. })) => {
                if self.confirmed {
                    vec![Action::Session(SessionAction::Fork(
                        self.selected_message_id,
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
        let overlay = centered_rect(60, 10, rect);
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
            Constraint::Min(3),
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
