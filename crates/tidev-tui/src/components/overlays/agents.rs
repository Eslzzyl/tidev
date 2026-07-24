//! AgentsPanel component — lists available sub-agent types.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use tidev_types::agent_type::AgentType;

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{centered_rect, render_scrollbar};

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct AgentInfo {
    pub agent_type: AgentType,
    pub display_name: String,
    pub description: String,
    pub read_only: bool,
    pub tools: Vec<String>,
    pub temperature: f32,
}

pub(crate) struct AgentsPanel {
    agents: Vec<AgentInfo>,
    scroll_offset: usize,
}

impl AgentsPanel {
    pub(crate) fn new() -> Self {
        let agents = AgentType::all()
            .iter()
            .map(|at| {
                let tools = at
                    .default_tool_restrictions()
                    .map(|t| t.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_else(|| vec!["all".to_string()]);
                AgentInfo {
                    agent_type: *at,
                    display_name: at.display_name().to_string(),
                    description: at.description().to_string(),
                    read_only: at.is_read_only(),
                    tools,
                    temperature: at.default_temperature(),
                }
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
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_up(3);
                None
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down(3);
                None
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
                " Agents ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        let header = Line::from(vec![
            Span::styled(
                "  Agent",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                "Description",
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
            let tag = if agent.read_only { " [read-only]" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  @{}", agent.display_name),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}{}", agent.description, tag),
                    Style::default().fg(palette.muted),
                ),
            ]));
        }

        let remaining = visible_height.saturating_sub(lines.len());
        if remaining >= 2 {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  ↑/↓ scroll · Esc/q close",
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
