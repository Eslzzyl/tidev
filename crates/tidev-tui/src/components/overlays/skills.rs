//! SkillsPanel component — skill browsing panel.

use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::markdown::MarkdownRender;
use crate::markdown::render_markdown_text_with_width_and_cwd;
use crate::utils::{centered_rect, render_scrollbar};

#[derive(Clone, Debug)]
pub(crate) struct SkillItem {
    pub name: String,
    pub content: String,
    pub is_bundled: bool,
}

pub(crate) struct SkillsPanel {
    all_skills: Vec<SkillItem>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    query: String,
    list_scroll: usize,
    preview_scroll: usize,
    query_active: bool,
    cached_preview: Option<(String, Arc<MarkdownRender>)>,
    preview_content_width: usize,
}

impl SkillsPanel {
    pub(crate) fn new(skills: Vec<SkillItem>) -> Self {
        let filtered_indices: Vec<usize> = (0..skills.len()).collect();
        Self {
            all_skills: skills,
            filtered_indices,
            selected_index: 0,
            query: String::new(),
            list_scroll: 0,
            preview_scroll: 0,
            query_active: false,
            cached_preview: None,
            preview_content_width: 60,
        }
    }

    fn is_empty(&self) -> bool {
        self.all_skills.is_empty()
    }
    fn filtered_count(&self) -> usize {
        self.filtered_indices.len()
    }

    fn selected_skill(&self) -> Option<&SkillItem> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.all_skills.get(idx))
    }

    fn append_to_query(&mut self, ch: char) {
        self.query.push(ch);
        self.refilter();
    }
    fn backspace_query(&mut self) {
        if !self.query.is_empty() {
            self.query.pop();
            self.refilter();
        }
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.filtered_indices = (0..self.all_skills.len()).collect();
        } else {
            self.filtered_indices = self
                .all_skills
                .iter()
                .enumerate()
                .filter(|(_, s)| s.name.to_ascii_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_index = self
            .selected_index
            .min(self.filtered_indices.len().saturating_sub(1));
        self.list_scroll = 0;
        self.preview_scroll = 0;
        self.cached_preview = None;
    }

    fn ensure_list_scroll_visible(&mut self) {
        if self.selected_index < self.list_scroll {
            self.list_scroll = self.selected_index;
        }
    }

    fn move_up(&mut self, _step: usize) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else {
            self.selected_index = self.filtered_indices.len().saturating_sub(1);
        }
        self.preview_scroll = 0;
        self.ensure_list_scroll_visible();
    }

    fn move_down(&mut self, _step: usize) {
        if self.selected_index + 1 < self.filtered_indices.len() {
            self.selected_index += 1;
        } else {
            self.selected_index = 0;
        }
        self.preview_scroll = 0;
    }

    fn page_up(&mut self, step: usize) {
        for _ in 0..step {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            }
        }
        self.preview_scroll = 0;
    }

    fn page_down(&mut self, step: usize) {
        for _ in 0..step {
            if self.selected_index + 1 < self.filtered_indices.len() {
                self.selected_index += 1;
            }
        }
        self.preview_scroll = 0;
    }

    fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
    }
    fn scroll_preview_down(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_add(lines);
    }
}

impl Component for SkillsPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        if self.query_active {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.query_active = false;
                }
                KeyCode::Backspace => {
                    self.backspace_query();
                }
                KeyCode::Char(c) => {
                    self.append_to_query(c);
                }
                _ => {}
            }
            return None;
        }

        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            return Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::SkillsPanel,
            )));
        }

        match key.code {
            KeyCode::Char('/') | KeyCode::Char('s') => {
                self.query_active = true;
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up(10);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down(10);
                None
            }
            KeyCode::PageUp => {
                self.page_up(10);
                None
            }
            KeyCode::PageDown => {
                self.page_down(10);
                None
            }
            KeyCode::Home => {
                self.selected_index = 0;
                self.list_scroll = 0;
                None
            }
            KeyCode::End if !self.filtered_indices.is_empty() => {
                self.selected_index = self.filtered_indices.len() - 1;
                None
            }
            KeyCode::Left => {
                self.scroll_preview_up(5);
                None
            }
            KeyCode::Right => {
                self.scroll_preview_down(5);
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

        let overlay = centered_rect(85, 80, area);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Scrolls inside the panel are consumed so they never reach the chat
        // behind; scrolls elsewhere fall through (mirrors the PgUp/PgDn
        // pattern for keyboard events).
        if !overlay.contains(position) {
            return None;
        }

        let inner_w = inner.width as usize;
        let split_x = (inner_w * 35 / 100) as u16;
        let in_left = position.x < inner.x + split_x;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if in_left {
                    self.move_up(10);
                } else {
                    self.scroll_preview_up(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::ScrollDown => {
                if in_left {
                    self.move_down(10);
                } else {
                    self.scroll_preview_down(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_left {
                    let header_rows = 4u16;
                    let list_area = Rect::new(inner.x, inner.y, split_x, inner.height);
                    if position.y >= list_area.y + header_rows {
                        let row = (position.y - list_area.y - header_rows) as usize;
                        let idx = self.list_scroll + row;
                        if idx < self.filtered_indices.len() {
                            self.selected_index = idx;
                            self.preview_scroll = 0;
                        }
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, _action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        vec![]
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;

        let overlay = centered_rect(85, 80, rect);
        frame.render_widget(Clear, overlay);
        let panel_block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let skills_title = if self.is_empty() {
            " Skills ".to_string()
        } else {
            format!(
                " Skills · {}/{} ",
                self.selected_index + 1,
                self.filtered_count()
            )
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                &skills_title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        let body = Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        );

        if self.is_empty() {
            let empty_text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No skills discovered",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Create .opencode/skills/SKILL.md to add skills",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press Esc or q to close",
                    Style::default().fg(palette.muted),
                )),
            ];
            frame.render_widget(
                Paragraph::new(empty_text).style(Style::default().bg(palette.panel_alt)),
                body,
            );
            return;
        }

        let inner_w = inner.width as usize;
        let left_w = (inner_w * 35 / 100).max(20) as u16;
        let layout = Layout::horizontal([
            Constraint::Length(left_w),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .split(body);
        let left_area = layout[0];
        let right_area = layout[2];

        // Vertical separator between panes
        let sep_area = layout[1];
        let sep_lines: Vec<Line> = (0..body.height)
            .map(|_| Line::from(Span::styled("│", Style::default().fg(palette.border))))
            .collect();
        frame.render_widget(
            Paragraph::new(sep_lines).style(Style::default().bg(palette.panel_alt)),
            sep_area,
        );

        // ── Left Pane: List ──
        let filter_text = if self.query_active {
            format!("  Search: {}", self.query)
        } else if self.query.is_empty() {
            "  Search... (/)".to_string()
        } else {
            format!("  Search: {}", self.query)
        };
        let filter_style = if self.query_active {
            Style::default().fg(palette.accent)
        } else {
            Style::default().fg(palette.muted)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(filter_text, filter_style)]))
                .style(Style::default().bg(palette.panel_alt)),
            Rect::new(left_area.x, left_area.y, left_area.width, 1),
        );
        if self.query_active {
            frame.set_cursor_position((
                left_area.x + 11 + self.query.as_str().width() as u16,
                left_area.y,
            ));
        }

        // Name column header
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  Name",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(left_area.x, left_area.y + 1, left_area.width, 1),
        );

        // Separator
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(left_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(left_area.x, left_area.y + 2, left_area.width, 1),
        );

        let list_header_height = 3u16;
        let list_content_y = left_area.y + list_header_height;
        let list_content_height = left_area.height.saturating_sub(list_header_height);
        let list_content_area = Rect::new(
            left_area.x,
            list_content_y,
            left_area.width,
            list_content_height,
        );

        let (list_content_area, list_scrollbar_area) = if list_content_area.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(list_content_area);
            (chunks[0], Some(chunks[2]))
        } else if list_content_area.width > 1 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                .split(list_content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (list_content_area, None)
        };

        let mut list_lines: Vec<Line<'_>> = Vec::new();
        let visible_items = list_content_height as usize;
        let end = (self.list_scroll + visible_items).min(self.filtered_indices.len());
        for i in self.list_scroll..end {
            let idx = self.filtered_indices[i];
            let skill = &self.all_skills[idx];
            let is_selected = i == self.selected_index;
            let prefix = if skill.is_bundled {
                Span::styled(" ", Style::default().fg(palette.muted))
            } else {
                Span::raw("  ")
            };
            let name_style = if is_selected {
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            list_lines.push(Line::from(vec![
                prefix,
                Span::styled(&skill.name, name_style),
            ]));
        }

        while list_lines.len() < list_content_height as usize {
            list_lines.push(Line::from(""));
        }

        frame.render_widget(
            Paragraph::new(list_lines).style(Style::default().bg(palette.panel_alt)),
            list_content_area,
        );

        if let Some(sb_area) = list_scrollbar_area
            && self.filtered_indices.len() > list_content_height as usize
        {
            render_scrollbar(
                frame,
                sb_area,
                self.list_scroll,
                self.filtered_indices.len(),
                palette,
                false,
            );
        }

        // ── Right Pane: Preview ──
        let preview_header_y = right_area.y + 1;
        frame.render_widget(
            Paragraph::new(vec![Line::from(vec![Span::styled(
                "  Preview",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )])])
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(right_area.x, preview_header_y, right_area.width, 1),
        );

        let preview_divider_y = preview_header_y + 1;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(right_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(right_area.x, preview_divider_y, right_area.width, 1),
        );

        let preview_content_y = preview_divider_y + 1;
        let preview_content_height = right_area.height.saturating_sub(4);
        let preview_content_area = Rect::new(
            right_area.x,
            preview_content_y,
            right_area.width,
            preview_content_height,
        );
        self.preview_content_width = preview_content_area.width.saturating_sub(2) as usize;
        let (preview_content_area, preview_scrollbar_area) = if preview_content_area.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(preview_content_area);
            (chunks[0], Some(chunks[2]))
        } else if preview_content_area.width > 1 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                .split(preview_content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (preview_content_area, None)
        };

        // Populate preview cache from SkillItem.content if it doesn't match the selected skill
        let needs_render = match &self.cached_preview {
            Some((name, _)) => self.selected_skill().is_none_or(|s| *name != s.name),
            None => true,
        };
        if needs_render && let Some(skill) = self.selected_skill() {
            let rendered = render_markdown_text_with_width_and_cwd(
                &skill.content,
                Some(self.preview_content_width),
                None,
            );
            self.cached_preview = Some((skill.name.clone(), rendered));
        }

        if let Some((_, rendered)) = &self.cached_preview {
            let total_preview_lines = rendered.lines.len();
            let max_scroll = total_preview_lines.saturating_sub(preview_content_height as usize);
            self.preview_scroll = self.preview_scroll.min(max_scroll);
            let scroll = self.preview_scroll;
            let visible_lines: Vec<Line<'_>> = rendered
                .lines
                .iter()
                .skip(scroll)
                .take(preview_content_height as usize)
                .cloned()
                .collect();

            frame.render_widget(
                Paragraph::new(visible_lines).style(Style::default().bg(palette.panel_alt)),
                preview_content_area,
            );

            if let Some(sb_area) = preview_scrollbar_area
                && total_preview_lines > preview_content_height as usize
            {
                render_scrollbar(
                    frame,
                    sb_area,
                    self.preview_scroll,
                    total_preview_lines,
                    palette,
                    false,
                );
            }
        }

        let footer_y = inner.y + inner.height - 1;
        let hints = if self.query_active {
            "Enter: confirm search  •  Esc: cancel"
        } else {
            "↑/↓: navigate  •  ←/→: scroll preview  •  /: search  •  Esc: close"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {}", hints),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, footer_y, inner.width, 1),
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
