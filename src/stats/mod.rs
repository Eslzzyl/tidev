use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use rusqlite::{params, Connection};

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
            Self::Hour => format!(
                "{:04}-{:02}-{:02}T{:02}:00",
                dt.year(),
                dt.month(),
                dt.day(),
                dt.hour()
            ),
            Self::Day => format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day()),
            Self::Week => {
                let iso_week = dt.iso_week();
                format!("{:04}-W{:02}", iso_week.year(), iso_week.week())
            }
            Self::Month => format!("{:04}-{:02}", dt.year(), dt.month()),
        }
    }

    pub fn bucket_label(&self, bucket: &str) -> String {
        match self {
            Self::Hour => {
                if let Some(hour_part) = bucket.split('T').nth(1) {
                    format!("{}:00", hour_part.trim_end_matches(":00"))
                } else {
                    bucket.to_string()
                }
            }
            Self::Day => {
                if let Some(day_part) = bucket.split('-').nth(2) {
                    day_part.to_string()
                } else {
                    bucket.to_string()
                }
            }
            Self::Week => {
                if let Some(week_part) = bucket.split('W').nth(1) {
                    format!("W{}", week_part)
                } else {
                    bucket.to_string()
                }
            }
            Self::Month => {
                let parts: Vec<&str> = bucket.split('-').collect();
                if parts.len() >= 2 {
                    let month_names = [
                        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
                        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
                    ];
                    if let Ok(month) = parts[1].parse::<usize>()
                        && (1..=12).contains(&month)
                    {
                        return month_names[month - 1].to_string();
                    }
                }
                bucket.to_string()
            }
        }
    }

    pub fn default_range(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc::now();
        let start = match self {
            Self::Hour => now - Duration::hours(24),
            Self::Day => now - Duration::days(7),
            Self::Week => now - Duration::weeks(4),
            Self::Month => now - Duration::days(365),
        };
        (start, now)
    }

    pub fn bucket_count(&self) -> usize {
        match self {
            Self::Hour => 24,
            Self::Day => 7,
            Self::Week => 4,
            Self::Month => 12,
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
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
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
        if self.total_input_tokens == 0 {
            0.0
        } else {
            self.total_cache_read_tokens as f64 / self.total_input_tokens as f64 * 100.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimeRangeStats {
    pub granularity: Granularity,
    pub entries: Vec<StatsEntry>,
    pub summary: UsageSummary,
    pub model_usage: Vec<ModelUsageEntry>,
}

pub struct UsageStatsService {
    connection: Connection,
}

impl UsageStatsService {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub fn record_usage(
        &self,
        provider_id: &str,
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> Result<()> {
        let now = Utc::now();
        let time_bucket = Granularity::Hour.time_bucket(&now);
        let now_text = now.to_rfc3339();
        let total_tokens = input_tokens as i64 + output_tokens as i64;

        self.connection.execute(
            r#"
            INSERT INTO usage_stats (
                provider_id, model_id, time_bucket, granularity,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                total_tokens, request_count, created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'hour', ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9)
            ON CONFLICT(provider_id, model_id, time_bucket, granularity) DO UPDATE SET
                input_tokens = input_tokens + excluded.input_tokens,
                output_tokens = output_tokens + excluded.output_tokens,
                cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                total_tokens = total_tokens + excluded.total_tokens,
                request_count = request_count + 1,
                updated_at = excluded.updated_at
            "#,
            params![
                provider_id,
                model_id,
                time_bucket,
                input_tokens as i64,
                output_tokens as i64,
                cache_read_tokens as i64,
                cache_write_tokens as i64,
                total_tokens,
                now_text,
            ],
        )?;

        Ok(())
    }

    pub fn get_time_range_stats(
        &self,
        granularity: Granularity,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<TimeRangeStats> {
        let mut entries = Vec::new();
        let mut summary = UsageSummary::default();

        let granularity_str = granularity.as_str();

        let mut stmt = self.connection.prepare(
            r#"
            SELECT
                time_bucket,
                SUM(input_tokens) as input_tokens,
                SUM(output_tokens) as output_tokens,
                SUM(cache_read_tokens) as cache_read_tokens,
                SUM(cache_write_tokens) as cache_write_tokens,
                SUM(total_tokens) as total_tokens,
                SUM(request_count) as request_count
            FROM usage_stats
            WHERE granularity = ?1
            GROUP BY time_bucket
            HAVING time_bucket >= ?2 AND time_bucket <= ?3
            ORDER BY time_bucket ASC
            "#,
        )?;

        let start_bucket = granularity.time_bucket(&start);
        let end_bucket = granularity.time_bucket(&end);

        let rows = stmt.query_map(params![granularity_str, start_bucket, end_bucket], |row| {
            Ok(StatsEntry {
                time_bucket: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cache_read_tokens: row.get(3)?,
                cache_write_tokens: row.get(4)?,
                total_tokens: row.get(5)?,
                request_count: row.get(6)?,
            })
        })?;

        for row in rows {
            let entry = row?;
            summary.total_input_tokens += entry.input_tokens;
            summary.total_output_tokens += entry.output_tokens;
            summary.total_cache_read_tokens += entry.cache_read_tokens;
            summary.total_cache_write_tokens += entry.cache_write_tokens;
            summary.total_tokens += entry.total_tokens;
            summary.total_requests += entry.request_count;
            entries.push(entry);
        }

        let model_usage = self.get_model_usage_stats(start, end)?;

        Ok(TimeRangeStats {
            granularity,
            entries,
            summary,
            model_usage,
        })
    }

    pub fn get_model_usage_stats(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<ModelUsageEntry>> {
        let mut entries = Vec::new();

        let start_text = start.to_rfc3339();
        let end_text = end.to_rfc3339();

        let mut stmt = self.connection.prepare(
            r#"
            SELECT
                provider_id,
                model_id,
                SUM(input_tokens) as input_tokens,
                SUM(output_tokens) as output_tokens,
                SUM(cache_read_tokens) as cache_read_tokens,
                SUM(cache_write_tokens) as cache_write_tokens,
                SUM(total_tokens) as total_tokens,
                SUM(request_count) as request_count
            FROM usage_stats
            WHERE created_at >= ?1 AND created_at <= ?2
            GROUP BY provider_id, model_id
            ORDER BY total_tokens DESC
            "#,
        )?;

        let rows = stmt.query_map(params![start_text, end_text], |row| {
            Ok(ModelUsageEntry {
                provider_id: row.get(0)?,
                model_id: row.get(1)?,
                input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                cache_read_tokens: row.get(4)?,
                cache_write_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                request_count: row.get(7)?,
            })
        })?;

        for row in rows {
            entries.push(row?);
        }

        Ok(entries)
    }

    pub fn get_all_time_summary(&self) -> Result<UsageSummary> {
        let mut stmt = self.connection.prepare(
            r#"
            SELECT
                SUM(input_tokens),
                SUM(output_tokens),
                SUM(cache_read_tokens),
                SUM(cache_write_tokens),
                SUM(total_tokens),
                SUM(request_count)
            FROM usage_stats
            "#,
        )?;

        let row = stmt.query_row([], |row| {
            Ok(UsageSummary {
                total_input_tokens: row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                total_output_tokens: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                total_cache_read_tokens: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                total_cache_write_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                total_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                total_requests: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            })
        })?;

        Ok(row)
    }
}
