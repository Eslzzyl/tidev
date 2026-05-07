use crate::tui::App;
use crate::tui::render::render::spans_with_highlights;
use ratatui::{
    layout::{Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

impl App {
    pub(crate) fn render_command_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.command_palette.visible || self.command_palette.suggestions.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72);
        let height = (self.command_palette.suggestions.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let items = self
            .command_palette
            .suggestions
            .iter()
            .map(|suggestion| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        suggestion.spec.label(),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        suggestion.spec.description,
                        Style::default().fg(palette.muted),
                    ),
                ]))
            })
            .collect::<Vec<_>>();

        let mut state = ListState::default();
        state.select(Some(self.command_palette.selected_index));

        let panel = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(format!("Commands · /{}", self.command_palette.query));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, rect);
        frame.render_widget(panel, rect);
        frame.render_stateful_widget(list, inner, &mut state);
    }

    pub(crate) fn render_at_mention_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.at_mention.visible || self.at_mention.suggestions.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72);
        let height = (self.at_mention.suggestions.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let items = self
            .at_mention
            .suggestions
            .iter()
            .map(|suggestion| {
                let path_style = Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD);
                let highlight_style = Style::default()
                    .fg(palette.accent_soft)
                    .add_modifier(Modifier::BOLD);
                let mut path_spans = vec![Span::styled("@", path_style)];
                path_spans.extend(spans_with_highlights(
                    &suggestion.path,
                    &suggestion.matched_indices,
                    path_style,
                    highlight_style,
                ));
                let mut spans = path_spans;
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    suggestion.display.clone(),
                    Style::default().fg(palette.muted),
                ));

                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>();

        let mut state = ListState::default();
        state.select(Some(self.at_mention.selected_index));

        let panel = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(format!("Files · @{}", self.at_mention.query));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, rect);
        frame.render_widget(panel, rect);
        frame.render_stateful_widget(list, inner, &mut state);
    }

    pub(crate) fn render_snippet_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.snippet_state.visible || self.snippet_state.snippets.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72);
        let height = (self.snippet_state.snippets.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let items = self
            .snippet_state
            .snippets
            .iter()
            .map(|snippet| {
                let text_style = Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD);
                let highlight_style = Style::default()
                    .fg(palette.accent_soft)
                    .add_modifier(Modifier::BOLD);

                let spans = spans_with_highlights(
                    &snippet.text,
                    &snippet.matched_indices,
                    text_style,
                    highlight_style,
                );

                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>();

        let mut state = ListState::default();
        state.select(Some(self.snippet_state.selected_index));

        let panel = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(format!("Snippets · {}", self.snippet_state.query));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, rect);
        frame.render_widget(panel, rect);
        frame.render_stateful_widget(list, inner, &mut state);
    }

    pub(crate) fn render_shell_completion_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.shell_completion.visible || self.shell_completion.candidates.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72);
        let height = (self.shell_completion.candidates.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let items: Vec<ListItem> = self
            .shell_completion
            .candidates
            .iter()
            .map(|cmd| {
                ListItem::new(Line::from(Span::styled(
                    cmd.clone(),
                    Style::default().fg(palette.text),
                )))
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(self.shell_completion.selected_index));

        let panel = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(format!(
                "Commands ({})",
                self.shell_completion.candidates.len()
            ));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, rect);
        frame.render_widget(panel, rect);
        frame.render_stateful_widget(list, inner, &mut state);
    }
}
