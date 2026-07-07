//! PermissionDialog — final approve / reject prompt for tool execution.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use anyhow::Result;

use crate::action::{Action, OverlayAction, OverlayKind, PermissionDecision};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{centered_rect, pretty_tool_arguments};

pub(crate) struct PermissionDialog {
    permission_key: String,
    display_name: String,
    arguments: String,
    current_index: usize,
    total: usize,
    decision: Option<PermissionDecision>,
}

impl PermissionDialog {
    pub(crate) fn new(
        permission_key: String,
        display_name: String,
        arguments: String,
        current_index: usize,
        total: usize,
    ) -> Self {
        Self {
            permission_key,
            display_name,
            arguments,
            current_index,
            total,
            decision: None,
        }
    }

    fn title(&self) -> String {
        format!(
            "Approve tool call {} of {} · {}",
            self.current_index, self.total, self.display_name
        )
    }
}

impl Component for PermissionDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            KeyCode::Char('y' | 'Y') => {
                self.decision = Some(PermissionDecision::Allow);
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::PermissionDialog,
                )))
            }
            KeyCode::Char('r' | 'R') => {
                self.decision = Some(PermissionDecision::AllowAndRemember);
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::PermissionDialog,
                )))
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.decision = Some(PermissionDecision::Deny);
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::PermissionDialog,
                )))
            }
            KeyCode::Char('x' | 'X') => {
                self.decision = Some(PermissionDecision::DenyAndRemember);
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::PermissionDialog,
                )))
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::PermissionDialog)) => {
                if let Some(decision) = self.decision.take() {
                    vec![Action::PermissionResponse { decision }]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let preview = pretty_tool_arguments(&self.arguments);
        let preview_height = preview.lines().count().min(8) as u16;
        let overlay = centered_rect(rect.width.min(96), preview_height.saturating_add(10), rect);
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
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);

        // Title bar
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Tool approval ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // Title
        frame.render_widget(
            Paragraph::new(self.title())
                .alignment(ratatui::layout::Alignment::Center)
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[1],
        );

        // Warning text
        frame.render_widget(
            Paragraph::new(
                "This tool can change state. Review the arguments and choose whether to allow it.",
            )
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );

        // Tool arguments preview
        frame.render_widget(
            Paragraph::new(preview)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .wrap(Wrap { trim: false }),
            sections[3],
        );

        // Help / action hints
        frame.render_widget(
            Paragraph::new(
                "Y allow · N deny · R allow and remember · X deny and remember · Esc deny",
            )
            .alignment(ratatui::layout::Alignment::Center)
            .style(
                Style::default()
                    .bg(palette.panel_alt)
                    .fg(palette.accent_soft),
            ),
            sections[4],
        );
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        20
    }

    fn blocks_input(&self) -> bool {
        true
    }
}
