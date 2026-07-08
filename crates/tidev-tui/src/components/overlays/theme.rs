//! ThemePanel component — theme selection panel.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;
use crate::theme::ThemeName;

use crate::action::{Action, OverlayAction, OverlayKind, ThemeAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

#[derive(Clone, Debug)]
pub(crate) enum DisplayItem {
    Header(&'static str),
    Theme(ThemeName),
}

pub(crate) struct ThemePanel {
    display_items: Vec<DisplayItem>,
    selected_index: usize,
    preview_theme: ThemeName,
    original_theme: ThemeName,
    query: String,
}

impl ThemePanel {
    pub(crate) fn new(current: ThemeName) -> Self {
        let display_items = Self::build_display("");
        let selected_index = display_items
            .iter()
            .position(|item| matches!(item, DisplayItem::Theme(t) if *t == current))
            .unwrap_or(0);

        Self {
            display_items,
            selected_index,
            preview_theme: current,
            original_theme: current,
            query: String::new(),
        }
    }

    fn build_display(query: &str) -> Vec<DisplayItem> {
        let all = ThemeName::all();
        let q = query.trim().to_lowercase();
        let matches_query = |t: &ThemeName| -> bool { q.is_empty() || t.as_str().contains(&q) };

        let mut items = Vec::new();

        let light: Vec<_> = all
            .iter()
            .filter(|t| !t.is_dark() && matches_query(t))
            .collect();
        if !light.is_empty() {
            items.push(DisplayItem::Header("Light"));
            for t in light {
                items.push(DisplayItem::Theme(*t));
            }
        }

        let dark: Vec<_> = all
            .iter()
            .filter(|t| t.is_dark() && matches_query(t))
            .collect();
        if !dark.is_empty() {
            items.push(DisplayItem::Header("Dark"));
            for t in dark {
                items.push(DisplayItem::Theme(*t));
            }
        }

        if items.is_empty() {
            return Self::build_display("");
        }

        items
    }

    fn rebuild(&mut self) {
        let old_preview = self.preview_theme;
        self.display_items = Self::build_display(&self.query);
        self.selected_index = self
            .display_items
            .iter()
            .position(|item| matches!(item, DisplayItem::Theme(t) if *t == old_preview))
            .unwrap_or(0);
        if let Some(DisplayItem::Theme(t)) = self.display_items.get(self.selected_index) {
            self.preview_theme = *t;
        }
    }

    fn move_up(&mut self) {
        let mut idx = self.selected_index;
        loop {
            if idx == 0 { return; }
            idx -= 1;
            if matches!(self.display_items[idx], DisplayItem::Theme(_)) {
                self.selected_index = idx;
                if let DisplayItem::Theme(t) = self.display_items[idx] {
                    self.preview_theme = t;
                }
                return;
            }
        }
    }

    fn move_down(&mut self) {
        let len = self.display_items.len();
        let mut idx = self.selected_index;
        loop {
            if idx + 1 >= len { return; }
            idx += 1;
            if matches!(self.display_items[idx], DisplayItem::Theme(_)) {
                self.selected_index = idx;
                if let DisplayItem::Theme(t) = self.display_items[idx] {
                    self.preview_theme = t;
                }
                return;
            }
        }
    }

    fn append_query(&mut self, ch: char) {
        self.query.push(ch);
        self.rebuild();
    }

    fn backspace_query(&mut self) {
        if !self.query.is_empty() {
            self.query.pop();
            self.rebuild();
        }
    }
}

impl Component for ThemePanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = self.preview_theme;
                self.move_up();
                if self.preview_theme != prev {
                    Some(Action::Theme(ThemeAction::Preview(self.preview_theme)))
                } else {
                    None
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let prev = self.preview_theme;
                self.move_down();
                if self.preview_theme != prev {
                    Some(Action::Theme(ThemeAction::Preview(self.preview_theme)))
                } else {
                    None
                }
            }
            KeyCode::Backspace => {
                self.backspace_query();
                Some(Action::Theme(ThemeAction::Preview(self.preview_theme)))
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.append_query(ch);
                Some(Action::Theme(ThemeAction::Preview(self.preview_theme)))
            }
            KeyCode::Enter => {
                Some(Action::Theme(ThemeAction::Set(self.preview_theme)))
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                Some(Action::Overlay(OverlayAction::Close(OverlayKind::ThemePanel)))
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }

        let overlay = centered_rect(36, 22, area);
        let inner = overlay.inner(Margin { horizontal: 1, vertical: 1 });

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let prev = self.preview_theme;
                self.move_up();
                if self.preview_theme != prev {
                    Some(Action::Theme(ThemeAction::Preview(self.preview_theme)))
                } else {
                    None
                }
            }
            MouseEventKind::ScrollDown => {
                let prev = self.preview_theme;
                self.move_down();
                if self.preview_theme != prev {
                    Some(Action::Theme(ThemeAction::Preview(self.preview_theme)))
                } else {
                    None
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let header_rows = 2u16;
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < header_rows {
                    return Some(Action::Noop);
                }
                let row = (local_y - header_rows) as usize;
                let list_height = inner.height.saturating_sub(2) as usize;
                let scroll = if self.selected_index < list_height {
                    0
                } else {
                    let target = self.selected_index.saturating_sub(list_height / 2);
                    target.min(self.display_items.len().saturating_sub(list_height))
                };
                let idx = scroll + row;
                if idx < self.display_items.len()
                    && matches!(self.display_items[idx], DisplayItem::Theme(_))
                {
                    self.selected_index = idx;
                    if let DisplayItem::Theme(t) = self.display_items[idx] {
                        self.preview_theme = t;
                    }
                    Some(Action::Theme(ThemeAction::Set(self.preview_theme)))
                } else {
                    Some(Action::Noop)
                }
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::ThemePanel)) => {
                if self.preview_theme != self.original_theme {
                    self.preview_theme = self.original_theme;
                    vec![Action::Theme(ThemeAction::Preview(self.original_theme))]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(36, 22, rect);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(Clear, overlay);
        frame.render_widget(block, overlay);
        let inner = overlay.inner(Margin { horizontal: 1, vertical: 1 });

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Theme ",
                Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        let search_text = if self.query.is_empty() {
            "  Type to search...".to_string()
        } else {
            format!("  {}", self.query)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(search_text, Style::default().fg(palette.muted))]))
                .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
        if !self.query.is_empty() {
            frame.set_cursor_position((
                inner.x + 2 + self.query.as_str().width() as u16,
                inner.y + 1,
            ));
        }

        let divider_y = inner.y + 2;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, divider_y, inner.width, 1),
        );

        let list_y = inner.y + 3;
        let list_height = inner.height.saturating_sub(3);
        if list_height == 0 { return; }
        let list_area = Rect::new(inner.x, list_y, inner.width, list_height);

        let (content_area, _scrollbar_area) = if list_area.width > 2 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                .split(list_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (list_area, None)
        };

        let display_len = self.display_items.len();
        let scroll = if self.selected_index < list_height as usize {
            0
        } else {
            let target = self.selected_index.saturating_sub(list_height as usize / 2);
            target.min(display_len.saturating_sub(list_height as usize))
        };

        for i in 0..list_height {
            let idx = scroll + i as usize;
            if idx >= display_len { break; }
            let item = &self.display_items[idx];
            let y = content_area.y + i;

            match item {
                DisplayItem::Header(label) => {
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!(" {} ", label),
                            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
                        )))
                        .style(Style::default().bg(palette.panel_alt)),
                        Rect::new(content_area.x, y, content_area.width, 1),
                    );
                }
                DisplayItem::Theme(t) => {
                    let is_selected = idx == self.selected_index;
                    let (text_style, bg_block) = if is_selected {
                        (
                            Style::default().fg(palette.selection_fg).add_modifier(Modifier::BOLD),
                            Paragraph::new(Line::from(Span::styled(
                                "█",
                                Style::default().bg(palette.selection_bg),
                            )))
                            .style(Style::default().bg(palette.selection_bg)),
                        )
                    } else {
                        (
                            Style::default().fg(palette.text),
                            Paragraph::new(Line::from(Span::styled(
                                " ",
                                Style::default().bg(palette.panel_alt),
                            )))
                            .style(Style::default().bg(palette.panel_alt)),
                        )
                    };
                    frame.render_widget(bg_block, Rect::new(content_area.x, y, content_area.width, 1));
                    let name = if *t == palette.name {
                        format!("  {} ◀", t.as_str())
                    } else {
                        format!("  {}", t.as_str())
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(name, text_style))),
                        Rect::new(content_area.x, y, content_area.width, 1),
                    );
                }
            }
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
