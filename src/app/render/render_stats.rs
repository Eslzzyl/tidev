use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    style::Color,
    text::{Line, Span},
    widgets::{Bar, BarChart, Block, Borders, Clear, Paragraph},
};

use crate::{
    stats::{Granularity, TimeRangeStats},
    theme::ThemePalette,
    utils::format_token_count,
};

use super::App;
use crate::app::ui::stats_panel::StatsChart;

fn centered_stats_rect(area: Rect) -> Rect {
    let width = area.width.min(100).min(area.width.saturating_sub(4));
    let height = area.height.min(32).min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

impl App {
    pub(super) fn render_stats_panel(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let overlay = centered_stats_rect(area);

        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .style(Style::default().bg(palette.panel))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(" Usage Statistics ");

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
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(inner);

        self.render_stats_header(frame, layout[0], palette);
        self.render_stats_content(frame, layout[1], palette);
        self.render_stats_footer(frame, layout[2], palette);
    }

    fn render_stats_header(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let stats_panel = self.stats_panel.as_ref();

        let granularity_labels = ["Hour", "Day", "Week", "Month"];
        let granularities = [
            Granularity::Hour,
            Granularity::Day,
            Granularity::Week,
            Granularity::Month,
        ];

        let mut spans = vec![Span::styled(
            "Time Range: ",
            Style::default().bg(palette.panel).fg(palette.muted),
        )];

        for (i, (label, gran)) in granularity_labels
            .iter()
            .zip(granularities.iter())
            .enumerate()
        {
            let is_selected = stats_panel.map(|p| p.granularity == *gran).unwrap_or(false);

            let style = if is_selected {
                Style::default()
                    .bg(palette.panel)
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(palette.panel).fg(palette.text)
            };

            if i > 0 {
                spans.push(Span::styled(
                    " | ",
                    Style::default().bg(palette.panel).fg(palette.muted),
                ));
            }
            spans.push(Span::styled(format!("[{}]", label), style));
        }

        let paragraph = Paragraph::new(Line::from(spans)).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }

    fn render_stats_content(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let stats_panel = self.stats_panel.as_ref();

        if let Some(panel) = stats_panel {
            if let Some(stats) = &panel.cached_stats {
                match panel.selected_chart {
                    StatsChart::TokenUsage => {
                        self.render_token_usage_chart(frame, area, stats, palette);
                    }
                    StatsChart::ModelUsage => {
                        self.render_model_usage_chart(frame, area, stats, palette);
                    }
                }
            } else {
                let paragraph = Paragraph::new(Line::from(Span::styled(
                    "Loading statistics...",
                    Style::default().bg(palette.panel).fg(palette.muted),
                )))
                .style(Style::default().bg(palette.panel));
                frame.render_widget(paragraph, area);
            }
        }
    }

    fn render_token_usage_chart(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        stats: &TimeRangeStats,
        palette: ThemePalette,
    ) {
        if stats.entries.is_empty() {
            let paragraph = Paragraph::new(Line::from(Span::styled(
                "No data available for this time range",
                Style::default().bg(palette.panel).fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel));
            frame.render_widget(paragraph, area);
            return;
        }

        let layout =
            Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

        let mut bars = Vec::new();
        for entry in &stats.entries {
            let label = stats.granularity.bucket_label(&entry.time_bucket);
            // Input
            bars.push(
                Bar::default()
                    .value(entry.input_tokens as u64)
                    .style(Color::Blue),
            );
            // Cache Read (shows label with formatted token count)
            let cache_label = format!(
                "{}\n{}",
                label,
                format_token_count(entry.cache_read_tokens as u64)
            );
            bars.push(
                Bar::with_label(cache_label, entry.cache_read_tokens as u64).style(Color::Cyan),
            );
            // Output
            bars.push(
                Bar::default()
                    .value(entry.output_tokens as u64)
                    .style(Color::Green),
            );
        }

        let chart = BarChart::vertical(bars)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title(" Token Components (Blue:Input, Cyan:Cached, Green:Output) ")
                    .style(Style::default().bg(palette.panel)),
            )
            .bar_width(6)
            .bar_gap(0)
            .group_gap(1);

        frame.render_widget(chart, layout[0]);

        let mut lines = vec![Line::from("")];

        // Total
        lines.push(Line::from(vec![
            Span::styled(
                "Total Tokens:      ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_token_count(stats.summary.total_tokens as u64),
                Style::default()
                    .bg(palette.panel)
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // Input
        lines.push(Line::from(vec![
            Span::styled(
                "Input Tokens:      ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_token_count(stats.summary.total_input_tokens as u64),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // Cache
        lines.push(Line::from(vec![
            Span::styled(
                "Cached Tokens:     ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_token_count(stats.summary.total_cache_read_tokens as u64),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({:.1}%)", stats.summary.cache_hit_rate()),
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
        ]));

        // Output
        lines.push(Line::from(vec![
            Span::styled(
                "Output Tokens:     ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_token_count(stats.summary.total_output_tokens as u64),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        if let Some(usage) = &self.context_usage
            && let Some(tps) = usage.tokens_per_second
        {
            lines.push(Line::from(vec![
                Span::styled(
                    "Last Speed:        ",
                    Style::default().bg(palette.panel).fg(palette.muted),
                ),
                Span::styled(
                    format!("{:.1} t/s", tps),
                    Style::default()
                        .bg(palette.panel)
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        // Requests
        lines.push(Line::from(vec![
            Span::styled(
                "Total Requests:    ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_token_count(stats.summary.total_requests as u64),
                Style::default()
                    .bg(palette.panel)
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let paragraph = Paragraph::new(lines).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, layout[1]);
    }

    fn render_model_usage_chart(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        stats: &TimeRangeStats,
        palette: ThemePalette,
    ) {
        if stats.model_usage.is_empty() {
            let paragraph = Paragraph::new(Line::from(Span::styled(
                "No model usage data available",
                Style::default().bg(palette.panel).fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel));
            frame.render_widget(paragraph, area);
            return;
        }

        let layout =
            Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

        let bars: Vec<Bar> = stats
            .model_usage
            .iter()
            .take(8)
            .map(|entry| {
                let label = shorten_model_id(&entry.model_id, 10);
                Bar::with_label(label, entry.total_tokens as u64)
                    .style(Color::Magenta)
                    .value_style(Style::default().fg(Color::White))
            })
            .collect();

        let chart = BarChart::horizontal(bars)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title(" Token Usage by Model ")
                    .style(Style::default().bg(palette.panel)),
            )
            .bar_width(2)
            .bar_gap(1);

        frame.render_widget(chart, layout[0]);

        let mut lines = vec![Line::from("")];
        lines.push(Line::from(Span::styled(
            "Top Models by Tokens:",
            Style::default()
                .bg(palette.panel)
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        for entry in stats.model_usage.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}: ", shorten_model_id(&entry.model_id, 20)),
                    Style::default().bg(palette.panel).fg(palette.text),
                ),
                Span::styled(
                    format_token_count(entry.total_tokens as u64),
                    Style::default().bg(palette.panel).fg(palette.accent_soft),
                ),
                Span::styled(
                    format!(" ({} requests)", entry.request_count),
                    Style::default().bg(palette.panel).fg(palette.muted),
                ),
            ]));
        }

        let paragraph = Paragraph::new(lines).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, layout[1]);
    }

    fn render_stats_footer(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let stats_panel = self.stats_panel.as_ref();

        let chart_labels = ["Tokens", "Models"];
        let charts = [StatsChart::TokenUsage, StatsChart::ModelUsage];

        let mut spans = vec![Span::styled(
            "Chart: ",
            Style::default().bg(palette.panel).fg(palette.muted),
        )];

        for (i, (label, chart)) in chart_labels.iter().zip(charts.iter()).enumerate() {
            let is_selected = stats_panel
                .map(|p| p.selected_chart == *chart)
                .unwrap_or(false);

            let style = if is_selected {
                Style::default()
                    .bg(palette.panel)
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(palette.panel).fg(palette.text)
            };

            if i > 0 {
                spans.push(Span::styled(
                    " | ",
                    Style::default().bg(palette.panel).fg(palette.muted),
                ));
            }
            spans.push(Span::styled(format!("[{}]", label), style));
        }

        spans.push(Span::styled(
            "    [Tab] Next  [Esc] Close",
            Style::default().bg(palette.panel).fg(palette.muted),
        ));

        let paragraph = Paragraph::new(Line::from(spans)).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }
}

fn shorten_model_id(id: &str, max_len: usize) -> String {
    if id.len() <= max_len {
        id.to_string()
    } else {
        format!("{}...", &id[..max_len.saturating_sub(3)])
    }
}
