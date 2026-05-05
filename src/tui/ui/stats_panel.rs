use crate::stats::{Granularity, TimeRangeStats};
use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatsChart {
    TokenUsage,
    ModelUsage,
}

#[derive(Clone, Debug)]
pub struct StatsPanelState {
    pub active: bool,
    pub granularity: Granularity,
    pub selected_chart: StatsChart,
    pub cached_stats: Option<TimeRangeStats>,
    pub last_refresh: Option<DateTime<Utc>>,
    pub scroll_offset: usize,
}

impl Default for StatsPanelState {
    fn default() -> Self {
        Self {
            active: false,
            granularity: Granularity::Hour,
            selected_chart: StatsChart::TokenUsage,
            cached_stats: None,
            last_refresh: None,
            scroll_offset: 0,
        }
    }
}

impl StatsPanelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self) {
        self.active = !self.active;
    }

    pub fn next_granularity(&mut self) {
        self.granularity = match self.granularity {
            Granularity::Hour => Granularity::Day,
            Granularity::Day => Granularity::Week,
            Granularity::Week => Granularity::Month,
            Granularity::Month => Granularity::Hour,
        };
        self.cached_stats = None;
    }

    pub fn prev_granularity(&mut self) {
        self.granularity = match self.granularity {
            Granularity::Hour => Granularity::Month,
            Granularity::Day => Granularity::Hour,
            Granularity::Week => Granularity::Day,
            Granularity::Month => Granularity::Week,
        };
        self.cached_stats = None;
    }

    pub fn next_chart(&mut self) {
        self.selected_chart = match self.selected_chart {
            StatsChart::TokenUsage => StatsChart::ModelUsage,
            StatsChart::ModelUsage => StatsChart::TokenUsage,
        };
    }

    pub fn prev_chart(&mut self) {
        self.selected_chart = match self.selected_chart {
            StatsChart::TokenUsage => StatsChart::ModelUsage,
            StatsChart::ModelUsage => StatsChart::TokenUsage,
        };
    }

    pub fn needs_refresh(&self) -> bool {
        if self.cached_stats.is_none() {
            return true;
        }
        if let Some(last) = self.last_refresh {
            let elapsed = Utc::now() - last;
            return elapsed.num_seconds() > 30;
        }
        true
    }
}
