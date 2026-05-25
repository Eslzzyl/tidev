use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use cron::Schedule as CronExprSchedule;
use std::str::FromStr;

use crate::types::Schedule;

/// Compute the next run time for a given schedule, starting from `from`.
pub fn next_run_for_schedule(schedule: &Schedule, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
    match schedule {
        Schedule::Cron { expr, tz } => {
            let normalized = normalize_expression(expr)?;
            let cron = CronExprSchedule::from_str(&normalized)
                .with_context(|| format!("Invalid cron expression: {expr}"))?;

            if let Some(tz_name) = tz {
                let timezone = chrono_tz::Tz::from_str(tz_name)
                    .with_context(|| format!("Invalid IANA timezone: {tz_name}"))?;
                let localized_from = from.with_timezone(&timezone);
                let next_local = cron.after(&localized_from).next().ok_or_else(|| {
                    log::warn!("cron schedule: no future occurrence for expression: {expr}");
                    anyhow::Error::msg(format!("No future occurrence for expression: {expr}"))
                })?;
                Ok(next_local.with_timezone(&Utc))
            } else {
                // Default to OS local timezone so schedules match user
                // expectations instead of always using UTC.
                let local_from = from.with_timezone(&chrono::Local);
                let next_local = cron.after(&local_from).next().ok_or_else(|| {
                    log::warn!("cron schedule: no future occurrence for expression: {expr}");
                    anyhow::Error::msg(format!("No future occurrence for expression: {expr}"))
                })?;
                Ok(next_local.with_timezone(&Utc))
            }
        }
        Schedule::At { at } => Ok(*at),
        Schedule::Every { every_ms } => {
            if *every_ms == 0 {
                anyhow::bail!("Invalid schedule: every_ms must be > 0");
            }
            let ms = i64::try_from(*every_ms).context("every_ms is too large")?;
            let delta = ChronoDuration::milliseconds(ms);
            from.checked_add_signed(delta).ok_or_else(|| {
                log::error!("cron schedule: every_ms overflowed DateTime arithmetic: {every_ms}");
                anyhow::Error::msg("every_ms overflowed DateTime")
            })
        }
    }
}

/// Validate that a schedule is well-formed and has future occurrences.
pub fn validate_schedule(schedule: &Schedule, now: DateTime<Utc>) -> Result<()> {
    match schedule {
        Schedule::Cron { expr, .. } => {
            let _ = normalize_expression(expr)?;
            let _ = next_run_for_schedule(schedule, now)?;
            Ok(())
        }
        Schedule::At { at } => {
            if *at <= now {
                anyhow::bail!("Invalid schedule: 'at' must be in the future");
            }
            Ok(())
        }
        Schedule::Every { every_ms } => {
            if *every_ms == 0 {
                anyhow::bail!("Invalid schedule: every_ms must be > 0");
            }
            Ok(())
        }
    }
}

/// Extract the cron expression string from a schedule (if it is a Cron variant).
pub fn schedule_cron_expression(schedule: &Schedule) -> Option<String> {
    match schedule {
        Schedule::Cron { expr, .. } => Some(expr.clone()),
        _ => None,
    }
}

/// Normalize a crontab expression to the 6-field format used by the `cron` crate.
///
/// Handles:
/// - 5-field standard crontab → 6-field (seconds=0 prepended)
/// - Weekday field translation: standard crontab uses 0/7=Sun, 1=Mon, …, 6=Sat;
///   `cron` crate uses 1=Sun, 2=Mon, …, 7=Sat.
/// - 6-field expressions are passed through as-is.
pub fn normalize_expression(expression: &str) -> Result<String> {
    let expression = expression.trim();
    let field_count = expression.split_whitespace().count();

    match field_count {
        // Standard crontab syntax: minute hour day month weekday
        // Normalize weekday field from standard crontab semantics
        // (0/7=Sun, 1=Mon, …, 6=Sat) to cron-crate semantics
        // (1=Sun, 2=Mon, …, 7=Sat).
        5 => {
            let mut fields: Vec<&str> = expression.split_whitespace().collect();
            let weekday = fields[4];
            let normalized_weekday = normalize_weekday_field(weekday)?;
            fields[4] = &normalized_weekday;
            Ok(format!(
                "0 {} {} {} {} {}",
                fields[0], fields[1], fields[2], fields[3], fields[4]
            ))
        }
        // Crate-native 6-field with seconds field already present.
        6 => {
            // Only normalize the weekday field (index 5 in 6-field format).
            let mut fields: Vec<&str> = expression.split_whitespace().collect();
            let weekday = fields[5];
            let normalized_weekday = normalize_weekday_field(weekday)?;
            fields[5] = &normalized_weekday;
            Ok(fields.join(" "))
        }
        _ => anyhow::bail!(
            "Invalid cron expression: expected 5 or 6 fields, got {field_count}: {expression}"
        ),
    }
}

/// Normalize a single weekday field value from standard crontab to cron-crate
/// semantics.
fn normalize_weekday_field(field: &str) -> Result<String> {
    // If the field contains named days (MON, TUE, etc.), pass through unchanged.
    if field.chars().any(|c| c.is_ascii_alphabetic()) {
        return Ok(field.to_string());
    }

    // Process each comma-separated item individually.
    let items: Vec<&str> = field.split(',').collect();
    let mut result = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        result.push(normalize_weekday_item(item)?);
    }
    if result.is_empty() {
        anyhow::bail!("Empty weekday field after normalization");
    }
    Ok(result.join(","))
}

/// Normalize a single weekday item (which may be a wildcard, a number, a range,
/// or a range with step).
fn normalize_weekday_item(item: &str) -> Result<String> {
    // Wildcards and question marks pass through unchanged.
    if item == "*" || item == "?" {
        return Ok(item.to_string());
    }

    // Check for range with step: e.g. "1-5/2"
    if let Some((range_part, step)) = item.split_once('/') {
        let normalized_range = normalize_weekday_range(range_part)?;
        return Ok(format!("{normalized_range}/{step}"));
    }

    // Check for simple range: e.g. "1-5"
    if let Some((start, end)) = item.split_once('-') {
        let start_val = parse_weekday_number(start)?;
        let end_val = parse_weekday_number(end)?;
        let new_start = crontab_to_cron_weekday(start_val);
        let new_end = crontab_to_cron_weekday(end_val);
        // Handle wrap-around ranges like 5-1 (Fri-Sun → 6-1 in cron crate)
        if new_start <= new_end {
            return Ok(format!("{new_start}-{new_end}"));
        }
        // Wrap-around: split into two ranges.
        // e.g. 5-1 (Fri-Sun) → 6-7,1
        return Ok(format!("{new_start}-7,1-{new_end}"));
    }

    // Simple number.
    let val = parse_weekday_number(item)?;
    Ok(crontab_to_cron_weekday(val).to_string())
}

/// Normalize a weekday range (without step).
fn normalize_weekday_range(range: &str) -> Result<String> {
    if let Some((start, end)) = range.split_once('-') {
        let start_val = parse_weekday_number(start)?;
        let end_val = parse_weekday_number(end)?;
        let new_start = crontab_to_cron_weekday(start_val);
        let new_end = crontab_to_cron_weekday(end_val);
        if new_start <= new_end {
            return Ok(format!("{new_start}-{new_end}"));
        }
        Ok(format!("{new_start}-7,1-{new_end}"))
    } else {
        // Single number passed as range.
        let val = parse_weekday_number(range)?;
        Ok(crontab_to_cron_weekday(val).to_string())
    }
}

/// Parse a weekday number from a string.
fn parse_weekday_number(s: &str) -> Result<u8> {
    let s = s.trim();
    let val: u8 = s
        .parse()
        .with_context(|| format!("Invalid weekday number: {s}"))?;
    Ok(val)
}

/// Convert a standard crontab weekday number to a cron-crate weekday number.
///
/// Standard crontab: 0/7 = Sunday, 1 = Monday, …, 6 = Saturday
/// cron crate:      1 = Sunday, 2 = Monday, …, 7 = Saturday
fn crontab_to_cron_weekday(val: u8) -> u8 {
    match val {
        0 | 7 => 1, // Sun
        1 => 2,     // Mon
        2 => 3,     // Tue
        3 => 4,     // Wed
        4 => 5,     // Thu
        5 => 6,     // Fri
        6 => 7,     // Sat
        _ => val,   // passthrough for values outside expected range
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn next_run_for_schedule_cron() {
        let now = Utc::now();
        let schedule = Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
        };
        let next = next_run_for_schedule(&schedule, now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn next_run_for_schedule_at() {
        let now = Utc::now();
        let at = now + ChronoDuration::minutes(10);
        let schedule = Schedule::At { at };
        let next = next_run_for_schedule(&schedule, now).unwrap();
        assert_eq!(next, at);
    }

    #[test]
    fn next_run_for_schedule_every() {
        let now = Utc::now();
        let schedule = Schedule::Every { every_ms: 5000 };
        let next = next_run_for_schedule(&schedule, now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn next_run_for_schedule_supports_timezone() {
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 0, 0, 0).unwrap();
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("America/Los_Angeles".into()),
        };

        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 17, 0, 0).unwrap());
    }

    #[test]
    fn normalize_weekday_field_translates_standard_crontab_values() {
        assert_eq!(normalize_weekday_field("0").unwrap(), "1"); // Sun
        assert_eq!(normalize_weekday_field("1").unwrap(), "2"); // Mon
        assert_eq!(normalize_weekday_field("5").unwrap(), "6"); // Fri
        assert_eq!(normalize_weekday_field("6").unwrap(), "7"); // Sat
        assert_eq!(normalize_weekday_field("7").unwrap(), "1"); // Sun (alias)
    }

    #[test]
    fn normalize_5_field_expression() {
        let result = normalize_expression("*/5 9 * * 1-5").unwrap();
        assert_eq!(result, "0 */5 9 * * 2-6");
    }

    #[test]
    fn validate_schedule_rejects_invalid() {
        let now = Utc::now();
        let past = now - ChronoDuration::hours(1);
        let schedule = Schedule::At { at: past };
        assert!(validate_schedule(&schedule, now).is_err());
    }
}
