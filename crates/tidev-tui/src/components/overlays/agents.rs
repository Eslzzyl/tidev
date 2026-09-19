//! AgentsPanel component — lists available sub-agent types.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use tidev_core::agent_type::AgentType;

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::i18n::{TextKey, agent_type_name};
use crate::utils::{centered_rect, render_scrollbar};

#[derive(Clone, Debug)]
pub(crate) struct AgentInfo {
    pub agent_type: AgentType,
    pub read_only: bool,
}

pub(crate) struct AgentsPanel {
    agents: Vec<AgentInfo>,
    scroll_offset: usize,
}

impl AgentsPanel {
    pub(crate) fn new() -> Self {
        let agents = AgentType::all()
            .iter()
            .map(|at| AgentInfo {
                agent_type: *at,
                read_only: at.is_read_only(),
            })
            .collect();

        Self {
            agents,
            scroll_offset: 0,
        }
    }

    fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }
}

impl Component for AgentsPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::AgentsPanel,
            ))),
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_up(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_down(1);
                None
            }
            KeyCode::PageUp => {
                self.scroll_up(10);
                None
            }
            KeyCode::PageDown => {
                self.scroll_down(10);
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
        // Scrolls inside the panel are consumed so they never reach the chat
        // behind; scrolls elsewhere fall through (mirrors the PgUp/PgDn
        // pattern for keyboard events).
        let overlay = centered_rect(70, 24, area);
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

    fn update(&mut self, _action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        vec![]
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(70, 24, rect);
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
            Constraint::Min(0),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(" {} ", ctx.ui_text.text(TextKey::AgentsTitle)),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        let header = Line::from(vec![
            Span::styled(
                format!("  {}", ctx.ui_text.text(TextKey::Agents)),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                ctx.ui_text.text(TextKey::AgentDescription),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(header).style(Style::default().bg(palette.panel_alt)),
            sections[1],
        );

        let divider = Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(palette.muted),
        ));
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().bg(palette.panel_alt)),
            sections[2],
        );

        let content_area = sections[3];
        let (content_area, scrollbar_area) = if content_area.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area);
            (chunks[0], Some(chunks[2]))
        } else if content_area.width > 1 {
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (content_area, None)
        };

        let mut lines: Vec<Line<'_>> = Vec::new();
        let scroll = self.scroll_offset;
        let visible_height = content_area.height as usize;
        for agent in self.agents.iter().skip(scroll).take(visible_height) {
            let tag = if agent.read_only {
                ctx.ui_text.text(TextKey::AgentsReadOnly)
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  @{}", agent_type_name(&ctx.ui_text, agent.agent_type)),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  {}{}",
                        agent_description(&ctx.ui_text, agent.agent_type),
                        tag
                    ),
                    Style::default().fg(palette.muted),
                ),
            ]));
        }

        let remaining = visible_height.saturating_sub(lines.len());
        if remaining >= 2 {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", ctx.ui_text.text(TextKey::AgentsFooter)),
                Style::default().fg(palette.muted),
            )));
        }

        while lines.len() < visible_height {
            lines.push(Line::from(""));
        }

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(palette.panel_alt)),
            content_area,
        );

        if let Some(sb_area) = scrollbar_area {
            render_scrollbar(
                frame,
                sb_area,
                self.scroll_offset,
                self.agents.len() + 2,
                palette,
                false,
            );
        }
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

fn agent_description(ui_text: &crate::i18n::UiText, agent_type: AgentType) -> String {
    match agent_type {
        AgentType::General => ui_text.text(TextKey::AgentGeneralDescription),
        AgentType::Explorer => ui_text.text(TextKey::AgentExplorerDescription),
        AgentType::Librarian => ui_text.text(TextKey::AgentLibrarianDescription),
        AgentType::Oracle => ui_text.text(TextKey::AgentOracleDescription),
        AgentType::Fixer => ui_text.text(TextKey::AgentFixerDescription),
    }
}
