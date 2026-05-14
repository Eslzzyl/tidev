use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::memory::types::RetentionScore;

/// Retention scoring using exponential temporal decay + access frequency.
/// Replicates agentmemory's `mem::retention`.
pub struct RetentionService;

impl RetentionService {
    /// Compute retention score for an entity.
    /// Formula: score = importance * exp(-lambda * age_days) + access_boost
    /// Where:
    ///   - lambda = 0.1 (decay rate, from agentmemory)
    ///   - access_boost = 0.3 * min(access_count / 10, 1.0)
    pub fn compute(
        importance: f64,
        age_days: f64,
        access_count: i64,
    ) -> f64 {
        let lambda = 0.1;
        let base = importance * (-lambda * age_days).exp();
        let access_boost = 0.3 * (access_count as f64 / 10.0).min(1.0);
        (base + access_boost).clamp(0.0, 10.0)
    }

    /// Compute and store retention score.
    pub fn compute_and_store(
        db: &Connection,
        entity_id: &str,
        entity_type: &str,
        importance: f64,
        age_days: f64,
        access_count: i64,
    ) -> Result<RetentionScore> {
        let score = Self::compute(importance, age_days, access_count);
        let now = Utc::now();

        db.execute(
            "INSERT OR REPLACE INTO retention_scores (entity_id, entity_type, importance, access_frequency, age_days, score, computed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                entity_id,
                entity_type,
                importance,
                access_count as f64,
                age_days,
                score,
                now.to_rfc3339(),
            ],
        )?;

        Ok(RetentionScore {
            entity_id: entity_id.to_string(),
            entity_type: entity_type.to_string(),
            importance,
            access_frequency: access_count as f64,
            age_days,
            score,
            computed_at: now,
        })
    }

    /// Get retention tier label based on score.
    pub fn tier(score: f64) -> &'static str {
        if score >= 7.0 {
            "hot"
        } else if score >= 4.0 {
            "warm"
        } else if score >= 1.5 {
            "cold"
        } else {
            "stale"
        }
    }
}
