//! ThemePanel component — theme selection panel with dedicated preview.
//!
//! Two panes: the left lists all themes (grouped by Light/Dark) with
//! type-to-search filtering; the right pane shows a self-contained preview
//! of the selected theme built by [`crate::theme::preview`] — it is the only
//! part of the UI that carries the selected theme's colors. The panel chrome
//! and the app behind it keep the current theme. Enter applies the selected
//! theme, Esc/q closes without changing anything.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use tidev_config::ThemeCatalog;
use unicode_width::UnicodeWidthStr;

use crate::action::{Action, OverlayAction, OverlayKind, ThemeAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::markdown::set_syntax_theme_by_key;
use crate::theme::preview::build_preview_lines;
use crate::theme::{resolve_palette, ThemePalette};
use crate::utils::{centered_rect, render_scrollbar};

#[derive(Clone, Debug)]
pub(crate) enum DisplayItem {
    Header(&'static str),
    Theme(String),
}

pub(crate) struct ThemePanel {
    catalog: ThemeCatalog,
    display_items: Vec<DisplayItem>,
    selected_index: usize,
    preview_theme: String,
    original_theme: String,
    query: String,
    confirmed: bool,
    preview_scroll: usize,
    /// Cached preview: (theme, width, lines, palette of the previewed theme).
    cached_preview: Option<(String, usize, Vec<Line<'static>>, ThemePalette)>,
}

impl ThemePanel {
    pub(crate) fn new(catalog: ThemeCatalog, current: String) -> Self {
        let mut panel = Self {
            catalog,
            display_items: Vec::new(),
            selected_index: 0,
            preview_theme: current.clone(),
            original_theme: current,
            query: String::new(),
            confirmed: false,
            preview_scroll: 0,
            cached_preview: None,
        };
        panel.display_items = panel.build_display("");
        panel.selected_index = panel
            .display_items
            .iter()
            .position(|item| matches!(item, DisplayItem::Theme(t) if *t == panel.preview_theme))
            .unwrap_or(0);
        panel
    }

    fn build_display(&self, query: &str) -> Vec<DisplayItem> {
        let q = query.trim().to_lowercase();
        let matches_query = |t: &str| -> bool { q.is_empty() || t.contains(&q) };

        let mut items = Vec::new();

        let light: Vec<_> = self
            .catalog
            .iter()
            .filter(|(id, def)| !def.dark && matches_query(id))
            .map(|(id, _)| id.to_string())
            .collect();
        if !light.is_empty() {
            items.push(DisplayItem::Header("Light"));
            for t in light {
                items.push(DisplayItem::Theme(t));
            }
        }

        let dark: Vec<_> = self
            .catalog
            .iter()
            .filter(|(id, def)| def.dark && matches_query(id))
            .map(|(id, _)| id.to_string())
            .collect();
        if !dark.is_empty() {
            items.push(DisplayItem::Header("Dark"));
            for t in dark {
                items.push(DisplayItem::Theme(t));
            }
        }

        if items.is_empty() {
            return self.build_display("");
        }

        items
    }

    fn rebuild(&mut self) {
        let old_preview = self.preview_theme.clone();
        self.display_items = self.build_display(&self.query);
        self.selected_index = self
            .display_items
            .iter()
            .position(|item| matches!(item, DisplayItem::Theme(t) if *t == old_preview))
            .unwrap_or(0);
        if let Some(DisplayItem::Theme(t)) = self.display_items.get(self.selected_index) {
            self.preview_theme = t.clone();
        }
        self.preview_scroll = 0;
    }

    /// Move selection up (wrapping).
    fn move_up(&mut self) {
        let len = self.display_items.len();
        let mut idx = self.selected_index;
        for _ in 0..len {
            if idx == 0 {
                idx = len;
            }
            idx -= 1;
            if let DisplayItem::Theme(t) = &self.display_items[idx] {
                self.selected_index = idx;
                self.preview_theme = t.clone();
                self.preview_scroll = 0;
                return;
            }
        }
    }

    /// Move selection down (wrapping).
    fn move_down(&mut self) {
        let len = self.display_items.len();
        let mut idx = self.selected_index;
        for _ in 0..len {
            idx = (idx + 1) % len;
            if let DisplayItem::Theme(t) = &self.display_items[idx] {
                self.selected_index = idx;
                self.preview_theme = t.clone();
                self.preview_scroll = 0;
                return;
            }
        }
    }

    fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
    }
    fn scroll_preview_down(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_add(lines);
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

    /// List scroll offset that keeps the selection visible (centered when
    /// it would otherwise fall outside the viewport).
    fn list_scroll(&self, list_height: usize) -> usize {
        if self.selected_index < list_height {
            0
        } else {
            let target = self.selected_index.saturating_sub(list_height / 2);
            target.min(self.display_items.len().saturating_sub(list_height))
        }
    }

    /// Number of themes (excluding headers) currently in the list.
    fn theme_count(&self) -> usize {
        self.display_items
            .iter()
            .filter(|item| matches!(item, DisplayItem::Theme(_)))
            .count()
    }

    /// Position of the selected theme among themes only (1-based).
    fn selected_theme_pos(&self) -> usize {
        self.display_items[..=self.selected_index]
            .iter()
            .filter(|item| matches!(item, DisplayItem::Theme(_)))
            .count()
    }

    /// Rebuild the cached preview lines when the selected theme or the
    /// available width changed.
    ///
    /// `resolve_palette` activates the selected theme's syntax highlighting,
    /// so the code/diff samples render with its colors; afterwards the app's
    /// current syntax theme is restored so only the preview pane carries the
    /// selected theme.
    fn ensure_preview(&mut self, width: usize) {
        let up_to_date = matches!(
            &self.cached_preview,
            Some((theme, w, _, _)) if *theme == self.preview_theme && *w == width
        );
        if !up_to_date {
            let palette = resolve_palette(&self.catalog, &self.preview_theme);
            let def = self.catalog.get(&self.preview_theme);
            let lines = build_preview_lines(&self.preview_theme, palette, def, width);
            if self.preview_theme != self.original_theme
                && let Some(orig) = self
                    .catalog
                    .get(&self.original_theme)
                    .or_else(|| self.catalog.get("dark"))
            {
                set_syntax_theme_by_key(orig.syntax_theme_key());
            }
            self.cached_preview = Some((self.preview_theme.clone(), width, lines, palette));
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
                self.move_up();
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                None
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.move_up();
                }
                None
            }
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.move_down();
                }
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
            KeyCode::Backspace => {
                self.backspace_query();
                None
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.append_query(ch);
                None
            }
            KeyCode::Enter => {
                self.confirmed = true;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::ThemePanel,
                )))
            }
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::ThemePanel,
            ))),
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }

        let overlay = centered_rect(90, 82, area);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let inner_w = inner.width as usize;
        let left_w = (inner_w * 30 / 100).max(20) as u16;
        let in_left = position.x < inner.x + left_w;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                // Scrolls inside the panel are consumed so they never reach
                // the chat behind; scrolls elsewhere fall through (mirrors
                // the PgUp/PgDn pattern for keyboard events).
                if !overlay.contains(position) {
                    return None;
                }
                if in_left {
                    self.move_up();
                } else {
                    self.scroll_preview_up(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::ScrollDown => {
                if !overlay.contains(position) {
                    return None;
                }
                if in_left {
                    self.move_down();
                } else {
                    self.scroll_preview_down(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_left {
                    // Left pane: header row + divider, then the list.
                    let list_y = inner.y + 5;
                    let body_height = inner.height.saturating_sub(4) as usize;
                    let list_height = body_height.saturating_sub(2);
                    if position.y >= list_y {
                        let row = (position.y - list_y) as usize;
                        let idx = self.list_scroll(list_height) + row;
                        if idx < self.display_items.len()
                            && matches!(self.display_items[idx], DisplayItem::Theme(_))
                        {
                            self.selected_index = idx;
                            if let DisplayItem::Theme(t) = &self.display_items[idx] {
                                self.preview_theme = t.clone();
                            }
                            self.preview_scroll = 0;
                        }
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::ThemePanel)) => {
                // The panel never changes the app theme while open — only
                // Enter (confirmed) applies the selected theme.
                if self.confirmed {
                    vec![Action::Theme(ThemeAction::Set(self.preview_theme.clone()))]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(90, 82, rect);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(Clear, overlay);
        frame.render_widget(block, overlay);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // ── Title row ──
        let title = format!(
            " Theme · {}/{} ",
            self.selected_theme_pos(),
            self.theme_count()
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    title,
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", self.preview_theme),
                    Style::default().fg(palette.muted),
                ),
            ]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        // ── Search row ──
        let search_text = if self.query.is_empty() {
            "  Type to search...".to_string()
        } else {
            format!("  {}", self.query)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                search_text,
                Style::default().fg(palette.muted),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
        if !self.query.is_empty() {
            frame.set_cursor_position((
                inner.x + 2 + self.query.as_str().width() as u16,
                inner.y + 1,
            ));
        }

        // ── Divider below search ──
        let divider_y = inner.y + 2;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, divider_y, inner.width, 1),
        );

        // ── Body: left list | separator | right preview ──
        let footer_y = inner.y + inner.height - 1;
        let body = Rect::new(
            inner.x,
            inner.y + 3,
            inner.width,
            inner.height.saturating_sub(4),
        );

        let inner_w = inner.width as usize;
        let left_w = (inner_w * 30 / 100).max(20) as u16;
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
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  Themes",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(left_area.x, left_area.y, left_area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(left_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(left_area.x, left_area.y + 1, left_area.width, 1),
        );

        let list_header_height = 2u16;
        let list_content_y = left_area.y + list_header_height;
        let list_content_height = left_area.height.saturating_sub(list_header_height);
        let list_content_area = Rect::new(
            left_area.x,
            list_content_y,
            left_area.width,
            list_content_height,
        );

        let (list_area, list_scrollbar_area) = if list_content_area.width > 2 {
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

        let display_len = self.display_items.len();
        let list_height = list_area.height as usize;
        let scroll = self.list_scroll(list_height);

        for i in 0..list_area.height {
            let idx = scroll + i as usize;
            if idx >= display_len {
                break;
            }
            let item = &self.display_items[idx];
            let y = list_area.y + i;

            match item {
                DisplayItem::Header(label) => {
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!(" {} ", label),
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD),
                        )))
                        .style(Style::default().bg(palette.panel_alt)),
                        Rect::new(list_area.x, y, list_area.width, 1),
                    );
                }
                DisplayItem::Theme(t) => {
                    let is_selected = idx == self.selected_index;
                    let (text_style, bg_block) = if is_selected {
                        (
                            Style::default()
                                .fg(palette.selection_fg)
                                .add_modifier(Modifier::BOLD),
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
                    frame.render_widget(
                        bg_block,
                        Rect::new(list_area.x, y, list_area.width, 1),
                    );
                    let name = format!("  {}", t.as_str());
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(name, text_style))),
                        Rect::new(list_area.x, y, list_area.width, 1),
                    );
                }
            }
        }

        if let Some(sb_area) = list_scrollbar_area
            && display_len > list_height
        {
            render_scrollbar(
                frame,
                sb_area,
                scroll,
                display_len,
                palette,
                false,
            );
        }

        // ── Right Pane: Preview ──
        // The whole right pane is drawn with the selected theme's palette so
        // it reads as a self-contained preview; the rest of the panel and the
        // app behind it keep the current theme.
        let preview_header_height = 2u16;
        let preview_content_y = right_area.y + preview_header_height;
        let preview_content_height = right_area.height.saturating_sub(preview_header_height);
        let preview_content_area = Rect::new(
            right_area.x,
            preview_content_y,
            right_area.width,
            preview_content_height,
        );

        let (preview_area, preview_scrollbar_area) = if preview_content_area.width > 2 {
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

        if preview_area.width > 0 {
            self.ensure_preview(preview_area.width as usize);
        }
        let preview_palette = self
            .cached_preview
            .as_ref()
            .map(|(_, _, _, p)| *p)
            .unwrap_or(palette);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  Preview",
                Style::default()
                    .fg(preview_palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(preview_palette.panel_alt)),
            Rect::new(right_area.x, right_area.y, right_area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(right_area.width as usize),
                Style::default().fg(preview_palette.border),
            )))
            .style(Style::default().bg(preview_palette.panel_alt)),
            Rect::new(right_area.x, right_area.y + 1, right_area.width, 1),
        );

        if preview_area.width > 0
            && preview_area.height > 0
            && let Some((_, _, lines, _)) = &self.cached_preview
        {
            let total = lines.len();
            self.preview_scroll = self.preview_scroll.min(total.saturating_sub(1));
            let visible: Vec<Line> = lines
                .iter()
                .skip(self.preview_scroll)
                .take(preview_area.height as usize)
                .cloned()
                .collect();
            frame.render_widget(
                Paragraph::new(visible).style(Style::default().bg(preview_palette.panel_alt)),
                preview_area,
            );

            if let Some(sb_area) = preview_scrollbar_area
                && total > preview_area.height as usize
            {
                render_scrollbar(
                    frame,
                    sb_area,
                    self.preview_scroll,
                    total,
                    preview_palette,
                    false,
                );
            }
        }

        // ── Footer hints ──
        let hints =
            "↑/↓: navigate  •  ←/→: scroll preview  •  type: search  •  Enter: apply  •  Esc: close";
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
