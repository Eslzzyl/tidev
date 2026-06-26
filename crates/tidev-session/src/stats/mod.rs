use chrono::{DateTime, Datelike, Duration, Local, Timelike, Utc};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Hour,
    Day,
    Week,
    Month,
}

impl Granularity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "hour" => Self::Hour,
            "day" => Self::Day,
            "week" => Self::Week,
            "month" => Self::Month,
            _ => Self::Day,
        }
    }

    pub fn time_bucket(&self, dt: &DateTime<Utc>) -> String {
        match self {
            Self::Hour => {
                let d = dt.with_second(0).unwrap().with_nanosecond(0).unwrap();
                d.to_rfc3339()
            }
            Self::Day => {
                let d = dt
                    .with_hour(0)
                    .unwrap()
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
                d.to_rfc3339()
            }
            Self::Week => {
                let weekday = dt.weekday().num_days_from_monday();
                let d = (*dt - Duration::days(weekday as i64))
                    .with_hour(0)
                    .unwrap()
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
                d.to_rfc3339()
            }
            Self::Month => {
                let d = dt
                    .with_day(1)
                    .unwrap()
                    .with_hour(0)
                    .unwrap()
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
                d.to_rfc3339()
            }
        }
    }

    pub fn bucket_label(&self, bucket: &str) -> String {
        let dt = match DateTime::parse_from_rfc3339(bucket) {
            Ok(dt) => dt.with_timezone(&Local),
            Err(_) => return bucket.to_string(),
        };

        match self {
            Self::Hour => dt.format("%Y-%m-%d %H:00").to_string(),
            Self::Day => dt.format("%Y-%m-%d").to_string(),
            Self::Week => dt.format("Week of %Y-%m-%d").to_string(),
            Self::Month => dt.format("%Y-%m").to_string(),
        }
    }

    pub fn default_range(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc::now();
        let start = match self {
            Self::Hour => now - Duration::hours(24),
            Self::Day => now - Duration::days(7),
            Self::Week => now - Duration::weeks(4),
            Self::Month => now - Duration::days(90),
        };
        (start, now)
    }

    pub fn bucket_count(&self, start: &DateTime<Utc>, end: &DateTime<Utc>) -> usize {
        let total_seconds = (end.timestamp() - start.timestamp()).max(0) as u64;
        match self {
            Self::Hour => (total_seconds / 3600).max(1) as usize,
            Self::Day => (total_seconds / 86400).max(1) as usize,
            Self::Week => (total_seconds / (86400 * 7)).max(1) as usize,
            Self::Month => (total_seconds / (86400 * 30)).max(1) as usize,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatsEntry {
    pub time_bucket: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Clone, Debug, Default)]
pub struct ModelUsageEntry {
    pub provider_id: String,
    pub model_id: String,
    pub model_display_name: String,
    pub provider_display_name: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProviderUsageEntry {
    pub provider_id: String,
    pub provider_display_name: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Clone, Debug, Default)]
pub struct SessionUsageEntry {
    pub session_id: String,
    pub title: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default)]
pub struct UsageSummary {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_tokens: i64,
    pub total_requests: i64,
}

impl UsageSummary {
    pub fn cache_hit_rate(&self) -> f64 {
        let total_cache = self.total_cache_read_tokens + self.total_cache_write_tokens;
        if total_cache == 0 {
            return 0.0;
        }
        self.total_cache_read_tokens as f64 / total_cache as f64
    }
}

#[derive(Clone, Debug)]
pub struct TimeRangeStats {
    pub granularity: Granularity,
    pub entries: Vec<StatsEntry>,
    pub summary: UsageSummary,
    pub model_usage: Vec<ModelUsageEntry>,
}
