use std::collections::HashMap;
use anyhow::Result;
use rusqlite::Connection;

use crate::memory::types::MemoryType;
use crate::memory::remember::RememberService;

// ─── Pattern Types ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CoChangePattern {
    pub file_a: String,
    pub file_b: String,
    pub co_occurrence_count: usize,
    pub sessions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ErrorRepeatPattern {
    pub error_fragment: String,
    pub occurrence_count: usize,
    pub sessions: Vec<String>,
    pub sample_titles: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PatternReport {
    pub co_change_added: usize,
    pub error_repeat_added: usize,
}

// ─── Pattern Mining Service ───────────────────────────────────────────

pub struct PatternMiningService;

impl PatternMiningService {
    /// Run all statistical pattern mining passes.
    ///
    /// Reads from `compressed_observations`, writes results as
    /// `MemoryEntry(type=pattern)` with tags distinguishing the pattern type.
    pub fn run(db: &Connection, project: &str) -> Result<PatternReport> {
        let mut report = PatternReport::default();

        // Tier 0a: co-change patterns (file co-occurrence)
        if let Ok(n) = Self::mine_co_change(db, project) {
            report.co_change_added = n;
        }

        // Tier 0b: error repeat patterns (recurring errors)
        if let Ok(n) = Self::mine_error_repeats(db, project) {
            report.error_repeat_added = n;
        }

        Ok(report)
    }

    /// Mine file co-change patterns.
    ///
    /// For each observation with at least 2 files, count how often each
    /// pair co-occurs across observations. Pairs with ≥2 co-occurrences
    /// are stored as pattern memories.
    fn mine_co_change(db: &Connection, project: &str) -> Result<usize> {
        // Load all observations with file lists
        let mut stmt = db.prepare(
            "SELECT files, session_id FROM compressed_observations
             WHERE files IS NOT NULL AND files != '[]'",
        )?;

        let rows = stmt.query_map([], |row| {
            let files_json: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            Ok((files_json, session_id))
        })?;

        // Count co-occurrence pairs: (file_a, file_b) → (count, set<session>)
        #[derive(Default)]
        struct PairStats {
            count: usize,
            sessions: Vec<String>,
        }
        let mut co_map: HashMap<(String, String), PairStats> = HashMap::new();

        for row in rows {
            let (files_json, session_id) = row?;
            let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();
            if files.len() < 2 {
                continue;
            }

            // Sort to ensure canonical pair ordering
            let mut sorted = files.clone();
            sorted.sort();

            for i in 0..sorted.len() {
                for j in (i + 1)..sorted.len() {
                    let key = (sorted[i].clone(), sorted[j].clone());
                    let entry = co_map.entry(key).or_default();
                    entry.count += 1;
                    if !entry.sessions.contains(&session_id) {
                        entry.sessions.push(session_id.clone());
                    }
                }
            }
        }

        // Write patterns with ≥2 co-occurrences
        let mut added = 0;
        for ((file_a, file_b), stats) in &co_map {
            let unique_sessions = stats.sessions.len();
            if stats.count < 2 || unique_sessions < 2 {
                continue; // need meaningful co-occurrence
            }

            let title = format!("co_change: {} ↔ {}", file_a, file_b);
            let content = format!(
                "Files `{}` and `{}` are frequently modified together ({} times across {} sessions). Consider updating both when changing shared interfaces.",
                file_a, file_b, stats.count, unique_sessions
            );
            let tags = vec![
                "pattern".to_string(),
                "co_change".to_string(),
                format!("freq:{}", stats.count),
            ];

            if let Err(e) = RememberService::remember(
                db, project, MemoryType::Pattern,
                &title, &content, &[], &[file_a.clone(), file_b.clone()],
                &tags, None,
            ) {
                crate::log_warn!("failed to save co-change pattern: {}", e);
                continue;
            }
            added += 1;
        }

        Ok(added)
    }

    /// Mine error repeat patterns.
    ///
    /// Look for observations of type `error` with similar titles/narratives.
    /// Errors appearing ≥2 times across ≥2 sessions are stored.
    fn mine_error_repeats(db: &Connection, project: &str) -> Result<usize> {
        let mut stmt = db.prepare(
            "SELECT title, narrative, session_id FROM compressed_observations
             WHERE obs_type = 'error'
               AND title IS NOT NULL
               AND title != ''",
        )?;

        let rows = stmt.query_map([], |row| {
            let title: String = row.get(0)?;
            let narrative: String = row.get(1)?;
            let session_id: String = row.get(2)?;
            Ok((title, narrative, session_id))
        })?;

        // Group similar errors by checking if titles share significant words
        let mut error_map: HashMap<String, ErrorStats> = HashMap::new();

        for row in rows {
            let (title, _narrative, session_id) = row?;

            // Normalize: lowercase, take first ~80 chars as a key fragment
            let normalized: String = title
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace())
                .collect();
            let fragment = if normalized.len() > 80 {
                normalized[..80].to_string()
            } else {
                normalized.clone()
            };

            if fragment.len() < 10 {
                continue; // skip too-short fragments
            }

            let entry = error_map.entry(fragment).or_default();
            entry.count += 1;
            if !entry.sessions.contains(&session_id) {
                entry.sessions.push(session_id.clone());
            }
            if !entry.titles.contains(&title) {
                entry.titles.push(title);
            }
        }

        let mut added = 0;
        for (_fragment, stats) in &error_map {
            let unique_sessions = stats.sessions.len();
            if stats.count < 2 || unique_sessions < 2 {
                continue;
            }

            let sample = stats.titles.first().map(|s| s.as_str()).unwrap_or("");
            let title = format!("error_repeat: {}", sample);
            let content = format!(
                "Recurring error pattern observed {} times across {} sessions: \"{}\"",
                stats.count, unique_sessions, sample
            );
            let tags = vec![
                "pattern".to_string(),
                "error_repeat".to_string(),
                format!("freq:{}", stats.count),
            ];

            if let Err(e) = RememberService::remember(
                db, project, MemoryType::Pattern,
                &title, &content, &[], &[],
                &tags, None,
            ) {
                crate::log_warn!("failed to save error-repeat pattern: {}", e);
                continue;
            }
            added += 1;
        }

        Ok(added)
    }
}

// ─── Internal Helpers ─────────────────────────────────────────────────

#[derive(Default)]
struct ErrorStats {
    count: usize,
    sessions: Vec<String>,
    titles: Vec<String>,
}
