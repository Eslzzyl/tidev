use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::theme::ThemePalette;

use super::App;
use crate::app::ui::balance_panel::{BalancePanelState, ProviderTab};

fn centered_balance_rect(area: Rect) -> Rect {
    let width = area.width.min(60).min(area.width.saturating_sub(4));
    let height = area.height.min(20).min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

impl App {
    pub(super) fn render_balance_panel(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let overlay = centered_balance_rect(area);

        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .style(Style::default().bg(palette.panel))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(" Balance ");

        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        if inner.width < 20 || inner.height < 10 {
            return;
        }

        let layout = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

        self.render_balance_tabs(frame, layout[0], palette);
        self.render_balance_content(frame, layout[1], palette);
        self.render_balance_footer(frame, layout[2], palette);
    }

    fn render_balance_tabs(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let guard = match self.balance_panel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let panel = guard.as_ref();

        let tabs = ProviderTab::all();
        let mut spans = vec![Span::styled(
            "Provider: ",
            Style::default().bg(palette.panel).fg(palette.muted),
        )];

        for (i, tab) in tabs.iter().enumerate() {
            let is_selected = panel.map(|p| p.selected_provider == *tab).unwrap_or(false);

            let style = if is_selected {
                Style::default().bg(palette.panel).fg(palette.text)
            } else {
                Style::default().bg(palette.panel).fg(palette.muted)
            };

            let label = tab.label();

            if i > 0 {
                spans.push(Span::styled("  ", Style::default().bg(palette.panel)));
            }
            spans.push(Span::styled(format!("[{}]", label), style));
        }

        spans.push(Span::styled("     ", Style::default().bg(palette.panel)));

        let paragraph = Paragraph::new(Line::from(spans)).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }

    fn render_balance_content(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let guard = match self.balance_panel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let panel = guard.as_ref();

        if area.width < 20 {
            return;
        }

        match panel.map(|p| p.selected_provider) {
            Some(ProviderTab::DeepSeek) => {
                self.render_deepseek_balance(frame, area, palette, panel);
            }
            Some(ProviderTab::SiliconFlow) => {
                self.render_siliconflow_balance(frame, area, palette, panel);
            }
            None => {
                let spans = vec![Span::styled(
                    "No balance data",
                    Style::default().bg(palette.panel).fg(palette.muted),
                )];
                let paragraph = Paragraph::new(Line::from(spans));
                frame.render_widget(paragraph, area);
            }
        }
    }

    fn render_deepseek_balance(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        palette: ThemePalette,
        panel: Option<&BalancePanelState>,
    ) {
        let mut lines = Vec::new();

        if let Some(panel) = panel {
            if panel.loading {
                lines.push(Line::from(vec![Span::styled(
                    "Loading...",
                    Style::default().bg(palette.panel).fg(palette.muted),
                )]));
            } else if let Some(error) = &panel.error {
                lines.push(Line::from(vec![Span::styled(
                    format!("Error: {}", error),
                    Style::default().bg(palette.panel).fg(palette.error),
                )]));
            } else if let Some(balance) = &panel.deepseek_balance {
                // Header
                lines.push(Line::from(vec![Span::styled(
                    format!("DeepSeek Account (Available: {})", balance.is_available),
                    Style::default().bg(palette.panel).fg(palette.text),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    "─────────────────────────────────",
                    Style::default().bg(palette.panel).fg(palette.border),
                )]));

                for info in &balance.balance_infos {
                    lines.push(Line::from(vec![Span::styled(
                        format!("Currency: {}", info.currency),
                        Style::default().bg(palette.panel).fg(palette.text),
                    )]));
                    lines.push(Line::from(vec![Span::styled(
                        format!("  Total Balance: {} {}", info.total_balance, info.currency),
                        Style::default().bg(palette.panel).fg(palette.text),
                    )]));
                    lines.push(Line::from(vec![Span::styled(
                        format!("  Granted:      {} {}", info.granted_balance, info.currency),
                        Style::default().bg(palette.panel).fg(palette.muted),
                    )]));
                    lines.push(Line::from(vec![Span::styled(
                        format!(
                            "  Topped Up:    {} {}",
                            info.topped_up_balance, info.currency
                        ),
                        Style::default().bg(palette.panel).fg(palette.muted),
                    )]));
                }
            }
        }

        let paragraph = Paragraph::new(lines).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }

    fn render_siliconflow_balance(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        palette: ThemePalette,
        panel: Option<&BalancePanelState>,
    ) {
        let mut lines = Vec::new();

        if let Some(panel) = panel {
            if panel.loading {
                lines.push(Line::from(vec![Span::styled(
                    "Loading...",
                    Style::default().bg(palette.panel).fg(palette.muted),
                )]));
            } else if let Some(error) = &panel.error {
                lines.push(Line::from(vec![Span::styled(
                    format!("Error: {}", error),
                    Style::default().bg(palette.panel).fg(palette.error),
                )]));
            } else if let Some(balance) = &panel.siliconflow_balance {
                // Header
                lines.push(Line::from(vec![Span::styled(
                    "SiliconFlow Account",
                    Style::default().bg(palette.panel).fg(palette.text),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    "─────────────────────────────────",
                    Style::default().bg(palette.panel).fg(palette.border),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    format!("Total Balance: {}", balance.data.total_balance),
                    Style::default().bg(palette.panel).fg(palette.text),
                )]));
            }
        }

        let paragraph = Paragraph::new(lines).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }

    fn render_balance_footer(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let guard = match self.balance_panel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let panel = guard.as_ref();

        let refresh_hint = if panel.map(|p| p.loading).unwrap_or(false) {
            "[r] Refresh (loading...)"
        } else {
            "[r] Refresh"
        };

        let spans = vec![
            Span::styled(
                refresh_hint,
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                "                    [Esc] Close",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
        ];

        let paragraph = Paragraph::new(Line::from(spans)).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }
}
