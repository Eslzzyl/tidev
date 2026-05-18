use crate::tui::App;
use crate::tui::render::render::centered_rect;
use ratatui::{
    layout::{Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::Line,
    widgets::{Block, Clear, List, ListItem, ListState},
};

impl App {
    pub(crate) fn render_panel_launcher(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.panel_launcher.visible {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(56);
        let height = (self.panel_launcher.filtered.len() as u16 + 3) // entries + search bar + border
            .min(18)
            .saturating_add(2); // border
        let rect = centered_rect(width, height, area);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        // --- Search bar line ---
        let search_text = if self.panel_launcher.query.is_empty() {
            "  Type to filter panels...".to_string()
        } else {
            format!("  {}", self.panel_launcher.query)
        };
        let search_style = Style::default().fg(palette.muted);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(ratatui::text::Span::styled(
                search_text,
                search_style,
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        // --- Divider ---
        let divider_y = inner.y + 1;
        frame.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(ratatui::text::Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, divider_y, inner.width, 1),
        );

        // --- List ---
        let list_y = inner.y + 2;
        let list_height = inner.height.saturating_sub(2);
        if list_height == 0 {
            return;
        }

        let list_area = Rect::new(inner.x, list_y, inner.width, list_height);

        let items: Vec<ListItem<'_>> = self
            .panel_launcher
            .filtered
            .iter()
            .map(|entry| ListItem::new(Line::from(ratatui::text::Span::raw(entry.description))))
            .collect();

        let mut state = ListState::default();
        state.select(Some(self.panel_launcher.selected_index));

        let block = Block::default().style(Style::default().bg(palette.panel_alt));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, rect);
        frame.render_widget(block, rect);
        frame.render_stateful_widget(list, list_area, &mut state);
    }
}
