//! Global instruction file panel.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::action::{Action, InstructionsAction, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::i18n::TextKey;
use crate::utils::{centered_rect, render_scrollbar};

pub(crate) struct InstructionsPanel {
    path: String,
    content: String,
    exists: bool,
    scroll_offset: u16,
    error: Option<String>,
}

impl InstructionsPanel {
    pub(crate) fn new() -> Self {
        Self {
            path: String::new(),
            content: String::new(),
            exists: false,
            scroll_offset: 0,
            error: None,
        }
    }

    fn scroll_up(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    fn scroll_down(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    fn reload(&mut self, ctx: &UpdateContext) {
        self.path = ctx.runtime.global_instruction_path().display().to_string();
        match ctx.runtime.load_global_instructions() {
            Ok(content) => {
                self.exists = content.is_some();
                self.content = content.unwrap_or_default();
                self.error = None;
                self.scroll_offset = 0;
            }
            Err(error) => {
                self.error = Some(error.to_string());
            }
        }
    }
}

impl Component for InstructionsPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::InstructionsPanel,
            ))),
            KeyCode::Char('e') => Some(Action::Instructions(InstructionsAction::Edit)),
            KeyCode::Char('r') => Some(Action::Instructions(InstructionsAction::Reload)),
            KeyCode::Char('d') => Some(Action::Instructions(InstructionsAction::Delete)),
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_up(1);
                Some(Action::Consumed)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_down(1);
                Some(Action::Consumed)
            }
            KeyCode::PageUp => {
                self.scroll_up(10);
                Some(Action::Consumed)
            }
            KeyCode::PageDown => {
                self.scroll_down(10);
                Some(Action::Consumed)
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }
        let overlay = centered_rect(80, 30, area);
        if !overlay.contains(position) {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_up(3);
                Some(Action::Consumed)
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down(3);
                Some(Action::Consumed)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        if matches!(
            action,
            Action::Overlay(OverlayAction::Open(OverlayKind::InstructionsPanel))
                | Action::Instructions(InstructionsAction::Reload)
        ) {
            self.reload(ctx);
        }
        vec![]
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(80, 30, rect);
        frame.render_widget(Clear, overlay);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.panel_alt)),
            overlay,
        );

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(" {} ", ctx.ui_text.text(TextKey::GlobalInstructionsTitle)),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )])),
            sections[0],
        );
        frame.render_widget(
            Paragraph::new(self.path.clone()).style(Style::default().fg(palette.muted)),
            sections[1],
        );
        let status = if let Some(error) = &self.error {
            format!(
                "{}: {error}",
                ctx.ui_text.text(TextKey::GlobalInstructionsError)
            )
        } else if self.exists {
            ctx.ui_text.text(TextKey::GlobalInstructionsExists)
        } else {
            ctx.ui_text.text(TextKey::GlobalInstructionsMissing)
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(palette.muted)),
            sections[2],
        );

        let content = if self.content.is_empty() {
            ctx.ui_text.text(TextKey::GlobalInstructionsEmpty)
        } else {
            self.content.clone()
        };
        let text = Text::from(content);
        let line_count = text.lines.len().max(1);
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(palette.text))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll_offset, 0)),
            sections[3],
        );
        render_scrollbar(
            frame,
            sections[3],
            self.scroll_offset as usize,
            line_count,
            palette,
            false,
        );

        frame.render_widget(
            Paragraph::new(ctx.ui_text.text(TextKey::GlobalInstructionsHelp))
                .style(Style::default().fg(palette.muted)),
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
