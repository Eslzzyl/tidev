use super::*;

#[derive(Default)]
pub(super) struct UsageTotals {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    total_tokens: u64,
    request_count: u64,
}

impl UsageTotals {
    fn add(&mut self, record: &tidev_core::UsageRecord) {
        self.input_tokens += record.input_tokens;
        self.output_tokens += record.output_tokens;
        self.cache_read_tokens += record.cache_read_tokens;
        self.cache_write_tokens += record.cache_write_tokens;
        self.total_tokens += record.total_tokens;
        self.request_count += 1;
    }

    fn cache_hit_rate(&self) -> f64 {
        if self.input_tokens == 0 {
            0.0
        } else {
            self.cache_read_tokens as f64 / self.input_tokens as f64 * 100.0
        }
    }
}

pub(super) struct UsageGroup {
    provider_id: String,
    provider_display_name: String,
    model_id: String,
    model_display_name: String,
    totals: UsageTotals,
}

pub(super) struct ProviderGroup {
    provider_id: String,
    provider_display_name: String,
    totals: UsageTotals,
}

pub(super) fn stats_session_count(records: &[tidev_core::UsageRecord]) -> u64 {
    records
        .iter()
        .map(|record| record.session_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len() as u64
}

pub(super) fn parse_stats_time(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

pub(super) fn filter_usage_records(
    records: Vec<tidev_core::UsageRecord>,
    query: &StatsQuery,
) -> Vec<tidev_core::UsageRecord> {
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    if start.is_none() && end.is_none() {
        return records;
    }
    records
        .into_iter()
        .filter(|record| {
            let timestamp = parse_stats_time(Some(&record.created_at));
            timestamp.is_some_and(|timestamp| {
                start.is_none_or(|start| timestamp >= start)
                    && end.is_none_or(|end| timestamp < end)
            })
        })
        .collect()
}

pub(super) fn load_stats_records(
    state: &AppState,
    query: &StatsQuery,
) -> Result<Vec<tidev_core::UsageRecord>, ApiError> {
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    let records = state
        .runtime
        .session_manager()
        .store()
        .load_usage_records_in_range(start, end)?;
    Ok(filter_usage_records(records, query))
}

pub(super) fn stats_summary_json(records: &[tidev_core::UsageRecord]) -> serde_json::Value {
    let mut totals = UsageTotals::default();
    for record in records {
        totals.add(record);
    }
    serde_json::json!({
        "total_input_tokens": totals.input_tokens,
        "total_output_tokens": totals.output_tokens,
        "total_cache_read_tokens": totals.cache_read_tokens,
        "total_cache_write_tokens": totals.cache_write_tokens,
        "total_tokens": totals.total_tokens,
        "total_requests": totals.request_count,
        "cache_hit_rate": totals.cache_hit_rate(),
        "total_sessions": stats_session_count(records),
        "first_usage_date": records.first().map(|record| record.created_at.clone()),
    })
}

pub(super) fn stats_bucket(created_at: &str, granularity: &str) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(created_at) else {
        return created_at.to_owned();
    };
    let utc = parsed.with_timezone(&Utc);
    let bucket = match granularity {
        "hour" => utc
            .with_minute(0)
            .and_then(|value| value.with_second(0))
            .and_then(|value| value.with_nanosecond(0)),
        "day" => Some(
            Utc.from_utc_datetime(
                &utc.date_naive()
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is always a valid time"),
            ),
        ),
        "week" => {
            let date =
                utc.date_naive() - Duration::days(utc.weekday().num_days_from_monday() as i64);
            Some(
                Utc.from_utc_datetime(
                    &date
                        .and_hms_opt(0, 0, 0)
                        .expect("midnight is always a valid time"),
                ),
            )
        }
        "month" => Some(
            Utc.from_utc_datetime(
                &NaiveDate::from_ymd_opt(utc.year(), utc.month(), 1)
                    .expect("the first day of a month is always valid")
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is always a valid time"),
            ),
        ),
        _ => Some(utc),
    };
    bucket
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| created_at.to_owned())
}

pub(super) fn valid_granularity(value: Option<&str>) -> String {
    match value {
        Some("hour") | Some("day") | Some("week") | Some("month") => value.unwrap().to_owned(),
        _ => "hour".to_owned(),
    }
}

pub(super) async fn stats_summary(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_summary_json(&records)))
}

pub(super) fn stats_timeseries_json(
    records: &[tidev_core::UsageRecord],
    granularity: &str,
) -> serde_json::Value {
    let mut buckets: HashMap<String, UsageTotals> = HashMap::new();
    for record in records {
        buckets
            .entry(stats_bucket(&record.created_at, granularity))
            .or_default()
            .add(record);
    }
    let mut entries = buckets.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let entries = entries
        .into_iter()
        .map(|(time_bucket, totals)| {
            serde_json::json!({
                "time_bucket": time_bucket,
                "input_tokens": totals.input_tokens,
                "output_tokens": totals.output_tokens,
                "cache_read_tokens": totals.cache_read_tokens,
                "cache_write_tokens": totals.cache_write_tokens,
                "total_tokens": totals.total_tokens,
                "request_count": totals.request_count,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "granularity": granularity,
        "entries": entries,
        "summary": stats_summary_json(records),
    })
}

pub(super) async fn stats_timeseries(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    let granularity = valid_granularity(query.granularity.as_deref());
    Ok(Json(stats_timeseries_json(&records, &granularity)))
}

pub(super) fn stats_models_json(records: &[tidev_core::UsageRecord]) -> serde_json::Value {
    let mut groups: HashMap<(String, String), UsageGroup> = HashMap::new();
    for record in records {
        let key = (record.provider_id.clone(), record.model_id.clone());
        let group = groups.entry(key).or_insert_with(|| UsageGroup {
            provider_id: record.provider_id.clone(),
            provider_display_name: record.provider_display_name.clone(),
            model_id: record.model_id.clone(),
            model_display_name: record.model_display_name.clone(),
            totals: UsageTotals::default(),
        });
        group.totals.add(record);
    }
    let mut entries = groups.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.totals.total_tokens.cmp(&left.totals.total_tokens));
    serde_json::json!({
        "entries": entries.into_iter().map(|group| serde_json::json!({
            "provider_id": group.provider_id,
            "provider_display_name": group.provider_display_name,
            "model_id": group.model_id,
            "model_display_name": group.model_display_name,
            "input_tokens": group.totals.input_tokens,
            "output_tokens": group.totals.output_tokens,
            "cache_read_tokens": group.totals.cache_read_tokens,
            "cache_write_tokens": group.totals.cache_write_tokens,
            "total_tokens": group.totals.total_tokens,
            "request_count": group.totals.request_count,
        })).collect::<Vec<_>>(),
    })
}

pub(super) async fn stats_models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_models_json(&records)))
}

pub(super) fn stats_providers_json(records: &[tidev_core::UsageRecord]) -> serde_json::Value {
    let mut groups: HashMap<String, ProviderGroup> = HashMap::new();
    for record in records {
        let group = groups
            .entry(record.provider_id.clone())
            .or_insert_with(|| ProviderGroup {
                provider_id: record.provider_id.clone(),
                provider_display_name: record.provider_display_name.clone(),
                totals: UsageTotals::default(),
            });
        group.totals.add(record);
    }
    let mut entries = groups.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.totals.total_tokens.cmp(&left.totals.total_tokens));
    serde_json::json!({
        "entries": entries.into_iter().map(|group| serde_json::json!({
            "provider_id": group.provider_id,
            "provider_display_name": group.provider_display_name,
            "input_tokens": group.totals.input_tokens,
            "output_tokens": group.totals.output_tokens,
            "cache_read_tokens": group.totals.cache_read_tokens,
            "cache_write_tokens": group.totals.cache_write_tokens,
            "total_tokens": group.totals.total_tokens,
            "request_count": group.totals.request_count,
        })).collect::<Vec<_>>(),
    })
}

pub(super) async fn stats_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_providers_json(&records)))
}

pub(super) fn stats_sessions_json(
    records: &[tidev_core::UsageRecord],
    limit: i64,
    offset: i64,
) -> serde_json::Value {
    let mut groups: HashMap<String, (tidev_core::UsageRecord, UsageTotals)> = HashMap::new();
    for record in records {
        let entry = groups
            .entry(record.session_id.clone())
            .or_insert_with(|| (record.clone(), UsageTotals::default()));
        entry.1.add(record);
    }
    let mut entries = groups.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.total_tokens.cmp(&left.1.total_tokens));
    let total = entries.len();
    let offset = offset.max(0) as usize;
    let limit = limit.clamp(1, 200) as usize;
    let entries = entries
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(record, totals)| {
            serde_json::json!({
                "session_id": record.session_id,
                "title": record.title,
                "provider_id": record.provider_id,
                "model_id": record.model_id,
                "model_display_name": record.model_display_name,
                "message_count": totals.request_count,
                "input_tokens": totals.input_tokens,
                "output_tokens": totals.output_tokens,
                "cache_read_tokens": totals.cache_read_tokens,
                "cache_write_tokens": totals.cache_write_tokens,
                "total_tokens": totals.total_tokens,
                "created_at": record.session_created_at,
                "updated_at": record.session_updated_at,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "entries": entries, "total": total })
}

pub(super) async fn stats_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_sessions_json(
        &records,
        query.limit.unwrap_or(10),
        query.offset.unwrap_or(0),
    )))
}

const STATS_ACTIVITY_DAY_COUNT: i64 = 365;

pub(super) fn stats_activity_level(value: u64, scale: u64) -> u8 {
    if value == 0 || scale == 0 {
        return 0;
    }
    let normalized = ((value as f64 + 1.0).ln() / (scale as f64 + 1.0).ln()).min(1.0);
    (normalized * 4.0).ceil().clamp(1.0, 4.0) as u8
}

pub(super) fn stats_activity_response(
    days: Vec<tidev_core::UsageActivityDay>,
    end_date: chrono::NaiveDate,
) -> StatsActivityResponse {
    let start_date = end_date - Duration::days(STATS_ACTIVITY_DAY_COUNT - 1);
    let mut days_by_date = days
        .into_iter()
        .map(|day| (day.date.clone(), day))
        .collect::<HashMap<_, _>>();
    let mut values = days_by_date
        .values()
        .map(|day| day.request_count)
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    values.sort_unstable();
    let scale = values
        .get(((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1))
        .copied()
        .unwrap_or(0)
        .max(1);

    let mut total_requests = 0;
    let mut total_tokens = 0;
    let mut cells = Vec::with_capacity(STATS_ACTIVITY_DAY_COUNT as usize);
    for offset in 0..STATS_ACTIVITY_DAY_COUNT {
        let date = start_date + Duration::days(offset);
        let date_key = date.format("%Y-%m-%d").to_string();
        let day = days_by_date.remove(&date_key);
        let request_count = day.as_ref().map_or(0, |day| day.request_count);
        let token_count = day.as_ref().map_or(0, |day| day.total_tokens);
        total_requests += request_count;
        total_tokens += token_count;
        cells.push(StatsActivityCell {
            date: date_key,
            request_count,
            total_tokens: token_count,
            level: stats_activity_level(request_count, scale),
        });
    }

    StatsActivityResponse {
        start_date: start_date.format("%Y-%m-%d").to_string(),
        end_date: end_date.format("%Y-%m-%d").to_string(),
        total_requests,
        total_tokens,
        cells,
    }
}

pub(super) async fn stats_activity(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatsActivityResponse>, ApiError> {
    let end_date = Utc::now().date_naive();
    let start_date = end_date - Duration::days(STATS_ACTIVITY_DAY_COUNT - 1);
    let start = Utc.from_utc_datetime(
        &start_date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid"),
    );
    let end = Utc.from_utc_datetime(
        &(end_date + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid"),
    );
    let days = state
        .runtime
        .session_manager()
        .store()
        .load_usage_activity_days(start, end)?;
    Ok(Json(stats_activity_response(days, end_date)))
}

const STATS_MODEL_MIX_LIMIT: u64 = 5;

pub(super) fn stats_insight_granularity(query: &StatsQuery) -> String {
    let granularity = valid_granularity(query.granularity.as_deref());
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    let range_exceeds_31_days = start
        .zip(end)
        .is_some_and(|(start, end)| end - start > Duration::days(31));
    if granularity == "hour" && (start.is_none() || end.is_none() || range_exceeds_31_days) {
        "day".to_owned()
    } else {
        granularity
    }
}

pub(super) fn stats_rhythm_response(
    cells: Vec<tidev_core::UsageRhythmCell>,
) -> StatsRhythmResponse {
    let mut cells_by_time = cells
        .into_iter()
        .map(|cell| ((cell.weekday, cell.hour), cell))
        .collect::<HashMap<_, _>>();
    let mut values = cells_by_time
        .values()
        .map(|cell| cell.request_count)
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    values.sort_unstable();
    let scale = values
        .get(((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1))
        .copied()
        .unwrap_or(0)
        .max(1);

    let mut response_cells = Vec::with_capacity(7 * 24);
    for weekday in 0..7 {
        for hour in 0..24 {
            let cell = cells_by_time.remove(&(weekday, hour));
            let request_count = cell.as_ref().map_or(0, |cell| cell.request_count);
            response_cells.push(StatsRhythmCell {
                weekday,
                hour,
                request_count,
                total_tokens: cell.as_ref().map_or(0, |cell| cell.total_tokens),
                level: stats_activity_level(request_count, scale),
            });
        }
    }
    StatsRhythmResponse {
        cells: response_cells,
    }
}

pub(super) fn stats_model_mix_response(
    buckets: Vec<tidev_core::UsageModelMixBucket>,
) -> StatsModelMixResponse {
    #[derive(Debug)]
    struct SeriesMeta {
        provider_display_name: String,
        model_display_name: String,
        is_other: bool,
        total_tokens: u64,
    }

    let mut series_by_key = BTreeMap::<String, SeriesMeta>::new();
    let mut shares_by_time = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for bucket in buckets {
        let key = if bucket.is_other {
            "other".to_owned()
        } else {
            format!("{}:{}", bucket.provider_id, bucket.model_id)
        };
        let series = series_by_key.entry(key.clone()).or_insert(SeriesMeta {
            provider_display_name: bucket.provider_display_name.clone(),
            model_display_name: bucket.model_display_name.clone(),
            is_other: bucket.is_other,
            total_tokens: 0,
        });
        series.total_tokens += bucket.total_tokens;
        *shares_by_time
            .entry(bucket.time_bucket)
            .or_default()
            .entry(key)
            .or_default() += bucket.total_tokens;
    }

    let mut series = series_by_key.into_iter().collect::<Vec<_>>();
    series.sort_by(|(left_key, left), (right_key, right)| {
        left.is_other
            .cmp(&right.is_other)
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| left_key.cmp(right_key))
    });
    let series = series
        .into_iter()
        .map(|(key, series)| StatsModelMixSeries {
            key,
            provider_display_name: series.provider_display_name,
            model_display_name: series.model_display_name,
            is_other: series.is_other,
        })
        .collect::<Vec<_>>();
    let points = shares_by_time
        .into_iter()
        .map(|(time_bucket, totals)| {
            let total_tokens = totals.values().sum::<u64>();
            let shares = totals
                .into_iter()
                .map(|(key, value)| {
                    let share = if total_tokens == 0 {
                        0.0
                    } else {
                        value as f64 / total_tokens as f64 * 100.0
                    };
                    (key, share)
                })
                .collect();
            StatsModelMixPoint {
                time_bucket,
                shares,
            }
        })
        .collect();
    StatsModelMixResponse { series, points }
}

pub(super) async fn stats_insights(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<StatsInsightsResponse>, ApiError> {
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    let granularity = stats_insight_granularity(&query);
    let store = state.runtime.session_manager().store();
    let active_sessions = store
        .load_usage_active_sessions(start, end, &granularity)?
        .into_iter()
        .map(|entry| StatsActiveSessionPoint {
            time_bucket: entry.time_bucket,
            active_sessions: entry.active_sessions,
        })
        .collect();
    let rhythm = stats_rhythm_response(store.load_usage_rhythm(start, end)?);
    let model_mix = stats_model_mix_response(store.load_usage_model_mix(
        start,
        end,
        &granularity,
        STATS_MODEL_MIX_LIMIT,
    )?);
    let request_size_distribution = store
        .load_usage_request_size_distribution(start, end)?
        .into_iter()
        .map(|bucket| StatsRequestSizeBucket {
            lower_bound: bucket.lower_bound,
            upper_bound: bucket.upper_bound,
            request_count: bucket.request_count,
            total_tokens: bucket.total_tokens,
        })
        .collect();

    Ok(Json(StatsInsightsResponse {
        granularity,
        active_sessions,
        rhythm,
        model_mix,
        request_size_distribution,
    }))
}

pub(super) async fn stats_overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    let granularity = valid_granularity(query.granularity.as_deref());
    Ok(Json(serde_json::json!({
        "summary": stats_summary_json(&records),
        "timeseries": stats_timeseries_json(&records, &granularity),
        "models": stats_models_json(&records),
        "providers": stats_providers_json(&records),
        "sessions": stats_sessions_json(
            &records,
            query.limit.unwrap_or(10),
            query.offset.unwrap_or(0),
        ),
    })))
}
