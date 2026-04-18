use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    style::Color,
    text::{Line, Span},
    widgets::{Bar, BarChart, Block, Borders, Clear, Paragraph},
};

use crate::{
    stats::{Granularity, TimeRangeStats, UsageSummary},
    theme::ThemePalette,
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
                    StatsChart::InputOutput => {
                        self.render_input_output_chart(frame, area, stats, palette);
                    }
                    StatsChart::ModelUsage => {
                        self.render_model_usage_chart(frame, area, stats, palette);
                    }
                    StatsChart::CacheHitRate => {
                        self.render_cache_stats(frame, area, stats, palette);
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

        let bars: Vec<Bar> = stats
            .entries
            .iter()
            .map(|entry| {
                let label = stats.granularity.bucket_label(&entry.time_bucket);
                Bar::with_label(label, entry.total_tokens as u64)
                    .style(Color::Cyan)
                    .value_style(Style::default().fg(Color::White))
            })
            .collect();

        let chart = BarChart::vertical(bars)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title(" Total Token Usage ")
                    .style(Style::default().bg(palette.panel)),
            )
            .bar_width(6)
            .bar_gap(1);

        frame.render_widget(chart, layout[0]);

        self.render_summary_block(frame, layout[1], &stats.summary, palette);
    }

    fn render_input_output_chart(
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
            bars.push(
                Bar::with_label(format!("I-{}", label), entry.input_tokens as u64)
                    .style(Color::Blue),
            );
            bars.push(
                Bar::with_label(format!("O-{}", label), entry.output_tokens as u64)
                    .style(Color::Green),
            );
        }

        let chart = BarChart::vertical(bars)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title(" Input vs Output Tokens (Blue=Input, Green=Output) ")
                    .style(Style::default().bg(palette.panel)),
            )
            .bar_width(4)
            .bar_gap(0);

        frame.render_widget(chart, layout[0]);

        let mut lines = vec![Line::from("")];
        lines.push(Line::from(vec![
            Span::styled(
                "Input Tokens:  ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_number(stats.summary.total_input_tokens),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                "Output Tokens: ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_number(stats.summary.total_output_tokens),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

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
                    format_number(entry.total_tokens),
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

    fn render_cache_stats(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        stats: &TimeRangeStats,
        palette: ThemePalette,
    ) {
        let layout =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

        let mut lines = vec![Line::from("")];
        lines.push(Line::from(Span::styled(
            "Cache Statistics",
            Style::default()
                .bg(palette.panel)
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Cache Read Tokens:  ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_number(stats.summary.total_cache_read_tokens),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                "Cache Write Tokens: ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_number(stats.summary.total_cache_write_tokens),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Cache Hit Rate: ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format!("{:.1}%", stats.summary.cache_hit_rate()),
                Style::default()
                    .bg(palette.panel)
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

        let paragraph = Paragraph::new(lines).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, layout[0]);

        self.render_summary_block(frame, layout[1], &stats.summary, palette);
    }

    fn render_summary_block(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        summary: &UsageSummary,
        palette: ThemePalette,
    ) {
        let mut lines = vec![Line::from("")];
        lines.push(Line::from(Span::styled(
            "Summary",
            Style::default()
                .bg(palette.panel)
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Total Tokens:   ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_number(summary.total_tokens),
                Style::default()
                    .bg(palette.panel)
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                "Total Requests: ",
                Style::default().bg(palette.panel).fg(palette.muted),
            ),
            Span::styled(
                format_number(summary.total_requests),
                Style::default()
                    .bg(palette.panel)
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

        let paragraph = Paragraph::new(lines).style(Style::default().bg(palette.panel));
        frame.render_widget(paragraph, area);
    }

    fn render_stats_footer(&self, frame: &mut Frame<'_>, area: Rect, palette: ThemePalette) {
        let stats_panel = self.stats_panel.as_ref();

        let chart_labels = ["Tokens", "I/O", "Models", "Cache"];
        let charts = [
            StatsChart::TokenUsage,
            StatsChart::InputOutput,
            StatsChart::ModelUsage,
            StatsChart::CacheHitRate,
        ];

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

fn format_number(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn shorten_model_id(id: &str, max_len: usize) -> String {
    if id.len() <= max_len {
        id.to_string()
    } else {
        format!("{}...", &id[..max_len.saturating_sub(3)])
    }
}
