use axum::{Json, extract::Query, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::stats::{
    Granularity, ProviderUsageEntry, SessionUsageEntry, UsageStatsService, UsageSummary,
};
use crate::web::{error::AppError, state::AppState};

/// ─── Request query types ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TimeSeriesQuery {
    pub granularity: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Deserialize)]
pub struct TimeRangeQuery {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// ─── Response types ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SummaryResponse {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_tokens: i64,
    pub total_requests: i64,
    pub cache_hit_rate: f64,
    pub total_sessions: i64,
    pub first_usage_date: Option<String>,
}

impl From<UsageSummary> for SummaryResponse {
    fn from(s: UsageSummary) -> Self {
        let cache_hit_rate = if s.total_input_tokens == 0 {
            0.0
        } else {
            (s.total_cache_read_tokens as f64 / s.total_input_tokens as f64) * 100.0
        };
        Self {
            total_input_tokens: s.total_input_tokens,
            total_output_tokens: s.total_output_tokens,
            total_cache_read_tokens: s.total_cache_read_tokens,
            total_cache_write_tokens: s.total_cache_write_tokens,
            total_tokens: s.total_tokens,
            total_requests: s.total_requests,
            cache_hit_rate,
            total_sessions: s.total_sessions,
            first_usage_date: s.first_usage_date,
        }
    }
}

#[derive(Serialize)]
pub struct TimeSeriesEntry {
    pub time_bucket: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Serialize)]
pub struct TimeSeriesResponse {
    pub granularity: String,
    pub entries: Vec<TimeSeriesEntry>,
    pub summary: SummaryResponse,
}

#[derive(Serialize)]
pub struct ModelUsageEntry {
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

impl From<crate::stats::ModelUsageEntry> for ModelUsageEntry {
    fn from(e: crate::stats::ModelUsageEntry) -> Self {
        Self {
            provider_id: e.provider_id,
            provider_display_name: e.provider_display_name,
            model_id: e.model_id,
            model_display_name: e.model_display_name,
            input_tokens: e.input_tokens,
            output_tokens: e.output_tokens,
            cache_read_tokens: e.cache_read_tokens,
            cache_write_tokens: e.cache_write_tokens,
            total_tokens: e.total_tokens,
            request_count: e.request_count,
        }
    }
}

#[derive(Serialize)]
pub struct ModelUsageResponse {
    pub entries: Vec<ModelUsageEntry>,
}

#[derive(Serialize)]
pub struct ProviderUsageResponse {
    pub entries: Vec<ProviderUsageEntry>,
}

#[derive(Serialize)]
pub struct SessionUsageResponse {
    pub entries: Vec<SessionUsageEntry>,
    pub total: i64,
}

// ─── Helper to parse time range ─────────────────────────────────────────

fn parse_time_range(query: &TimeRangeQuery) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = query
        .end
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let start = query
        .start
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| end - chrono::Duration::days(7));

    (start, end)
}

// ─── Shared helpers ────────────────────────────────────────────────────

/// Build a (provider_id, model_id) → model_display_name lookup from config.
fn build_model_lookup(
    config: &crate::config::AppConfig,
) -> HashMap<String, HashMap<String, String>> {
    let mut lookup: HashMap<String, HashMap<String, String>> = HashMap::new();
    for m in config.available_models() {
        lookup
            .entry(m.provider_id.clone())
            .or_default()
            .insert(m.model_id.clone(), m.model_display_name);
    }
    lookup
}

/// Build a provider_id → provider_display_name lookup from config.
fn build_provider_lookup(config: &crate::config::AppConfig) -> HashMap<String, String> {
    let mut lookup = HashMap::new();
    for m in config.available_models() {
        lookup
            .entry(m.provider_id.clone())
            .or_insert(m.provider_display_name.clone());
    }
    lookup
}

/// Correct provider_id and populate display names for a list of models.
///
/// model_id in usage_stats is stored as "provider:model" (Anthropic/OpenAI)
/// or just "model" (Gemini).  The provider_id column may not always match
/// (e.g. when a subagent uses a model from a different provider), so we
/// prefer the prefix extracted from model_id when it matches a known provider.
fn populate_model_entries(
    models: &mut [crate::stats::ModelUsageEntry],
    model_lookup: &HashMap<String, HashMap<String, String>>,
    provider_lookup: &HashMap<String, String>,
) {
    for m in models.iter_mut() {
        // Determine the best candidate provider: prefer the prefix from model_id
        let (lookup_provider, lookup_model) = match m.model_id.split_once(':') {
            Some((p, rest)) if !rest.is_empty() && model_lookup.contains_key(p) => {
                m.provider_id = p.to_string();
                (p.to_string(), rest.to_string())
            }
            _ => (m.provider_id.clone(), m.model_id.clone()),
        };

        // Model display name
        if let Some(display_name) = model_lookup
            .get(&lookup_provider)
            .and_then(|providers| providers.get(&lookup_model))
        {
            m.model_display_name = display_name.clone();
        }
        if m.model_display_name.is_empty() {
            let prefix = format!("{}:", &lookup_provider);
            m.model_display_name = lookup_model
                .strip_prefix(&prefix)
                .unwrap_or(&lookup_model)
                .to_string();
        }

        // Provider display name
        if let Some(display_name) = provider_lookup.get(&m.provider_id) {
            m.provider_display_name = display_name.clone();
        }
        if m.provider_display_name.is_empty() {
            m.provider_display_name = m.provider_id.clone();
        }
    }
}

// ─── Route handlers ─────────────────────────────────────────────────────

/// GET /api/stats/summary
pub async fn get_summary(State(state): State<AppState>) -> Result<Json<SummaryResponse>, AppError> {
    let conn = rusqlite::Connection::open_with_flags(
        &state.database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let service = UsageStatsService::new(conn);
    let summary = service.get_enhanced_summary()?;
    Ok(Json(SummaryResponse::from(summary)))
}

/// GET /api/stats/timeseries?granularity=day&start=...&end=...
pub async fn get_timeseries(
    State(state): State<AppState>,
    Query(query): Query<TimeSeriesQuery>,
) -> Result<Json<TimeSeriesResponse>, AppError> {
    let granularity = Granularity::parse(query.granularity.as_deref().unwrap_or("day"));

    // Check if we should use default range before moving query
    let use_default_range = query.start.is_none() && query.end.is_none();

    let time_range = TimeRangeQuery {
        start: query.start,
        end: query.end,
    };
    let (start, end) = if use_default_range {
        granularity.default_range()
    } else {
        parse_time_range(&time_range)
    };

    let conn = rusqlite::Connection::open_with_flags(
        &state.database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let service = UsageStatsService::new(conn);
    let stats = service.get_time_range_stats(granularity, start, end)?;

    let entries: Vec<TimeSeriesEntry> = stats
        .entries
        .into_iter()
        .map(|e| TimeSeriesEntry {
            time_bucket: e.time_bucket,
            input_tokens: e.input_tokens,
            output_tokens: e.output_tokens,
            cache_read_tokens: e.cache_read_tokens,
            cache_write_tokens: e.cache_write_tokens,
            total_tokens: e.total_tokens,
            request_count: e.request_count,
        })
        .collect();

    Ok(Json(TimeSeriesResponse {
        granularity: granularity.as_str().to_string(),
        entries,
        summary: SummaryResponse::from(stats.summary),
    }))
}

/// GET /api/stats/models?start=...&end=...
pub async fn get_model_usage(
    State(state): State<AppState>,
    Query(query): Query<TimeRangeQuery>,
) -> Result<Json<ModelUsageResponse>, AppError> {
    let (start, end) = parse_time_range(&query);

    let config = state.config.read().await;
    let model_lookup = build_model_lookup(&config);
    let provider_lookup = build_provider_lookup(&config);
    drop(config);

    let conn = rusqlite::Connection::open_with_flags(
        &state.database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let service = UsageStatsService::new(conn);
    let mut models = service.get_model_usage_stats(start, end)?;

    populate_model_entries(&mut models, &model_lookup, &provider_lookup);

    let entries: Vec<ModelUsageEntry> = models.into_iter().map(|m| m.into()).collect();
    Ok(Json(ModelUsageResponse { entries }))
}

/// GET /api/stats/providers?start=...&end=...
///
/// Aggregates from corrected model-level data rather than querying usage_stats
/// directly, because the provider_id column in usage_stats may be wrong
/// (it records the session's active provider, not the model's actual provider).
pub async fn get_provider_usage(
    State(state): State<AppState>,
    Query(query): Query<TimeRangeQuery>,
) -> Result<Json<ProviderUsageResponse>, AppError> {
    let (start, end) = parse_time_range(&query);

    let config = state.config.read().await;
    let model_lookup = build_model_lookup(&config);
    let provider_lookup = build_provider_lookup(&config);
    drop(config);

    let conn = rusqlite::Connection::open_with_flags(
        &state.database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let service = UsageStatsService::new(conn);
    let mut models = service.get_model_usage_stats(start, end)?;

    // Correct provider_id and populate display names
    populate_model_entries(&mut models, &model_lookup, &provider_lookup);

    // Aggregate by provider_id
    let mut map: HashMap<String, crate::stats::ProviderUsageEntry> = HashMap::new();
    for m in models {
        let entry =
            map.entry(m.provider_id.clone())
                .or_insert_with(|| crate::stats::ProviderUsageEntry {
                    provider_id: m.provider_id.clone(),
                    provider_display_name: m.provider_display_name.clone(),
                    ..Default::default()
                });
        entry.input_tokens += m.input_tokens;
        entry.output_tokens += m.output_tokens;
        entry.cache_read_tokens += m.cache_read_tokens;
        entry.cache_write_tokens += m.cache_write_tokens;
        entry.total_tokens += m.total_tokens;
        entry.request_count += m.request_count;
    }

    let mut entries: Vec<_> = map.into_values().collect();
    entries.sort_by_key(|b| std::cmp::Reverse(b.total_tokens));

    Ok(Json(ProviderUsageResponse { entries }))
}

/// GET /api/stats/sessions?limit=20&offset=0
pub async fn get_session_usage(
    State(state): State<AppState>,
    Query(query): Query<SessionsQuery>,
) -> Result<Json<SessionUsageResponse>, AppError> {
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0);

    let config = state.config.read().await;
    let model_lookup = build_model_lookup(&config);
    drop(config);

    let conn = rusqlite::Connection::open_with_flags(
        &state.database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let service = UsageStatsService::new(conn);
    let (mut entries, total) = service.get_session_usage_stats(limit, offset)?;

    // Populate model display names for sessions
    for entry in &mut entries {
        let (lookup_provider, lookup_model) = match entry.model_id.split_once(':') {
            Some((p, rest)) if !rest.is_empty() && model_lookup.contains_key(p) => {
                entry.provider_id = p.to_string();
                (p.to_string(), rest.to_string())
            }
            _ => (entry.provider_id.clone(), entry.model_id.clone()),
        };

        if let Some(display_name) = model_lookup
            .get(&lookup_provider)
            .and_then(|providers| providers.get(&lookup_model))
        {
            entry.model_display_name = display_name.clone();
        }

        if entry.model_display_name.is_empty() {
            let prefix = format!("{}:", &lookup_provider);
            entry.model_display_name = lookup_model
                .strip_prefix(&prefix)
                .unwrap_or(&lookup_model)
                .to_string();
        }
    }

    Ok(Json(SessionUsageResponse { entries, total }))
}
