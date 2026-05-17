use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashMap;

use crate::memory::remember::RememberService;
use crate::memory::types::MemoryType;
use crate::storage::compression::decompress_text;

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
    /// Reads from tool result messages, writes results as
    /// `MemoryEntry(type=pattern)` with tags distinguishing the pattern type.
    pub fn run(db: &Connection, project: &str) -> Result<PatternReport> {
        let mut report = PatternReport::default();

        if let Ok(n) = Self::mine_co_change(db, project) {
            report.co_change_added = n;
        }

        if let Ok(n) = Self::mine_error_repeats(db, project) {
            report.error_repeat_added = n;
        }

        Ok(report)
    }

    /// Mine file co-change patterns from tool result messages.
    ///
    /// Reads tool results for write/edit/edit_and_apply, extracts file paths
    /// from `file_diffs` (JSON) and `content` text, then counts co-occurrence
    /// pairs across sessions.
    fn mine_co_change(db: &Connection, project: &str) -> Result<usize> {
        let file_tools = ["write", "edit", "edit_and_apply", "create", "delete", "rename"];

        let mut stmt = db.prepare(
            "SELECT content, file_diffs, session_id
             FROM messages
             WHERE role = 'tool'
               AND tool_name IN (?,?,?,?,?,?)",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![
                file_tools[0], file_tools[1], file_tools[2],
                file_tools[3], file_tools[4], file_tools[5]
            ],
            |row| {
                let content_blob: Vec<u8> = row.get(0)?;
                let diffs_blob: Option<Vec<u8>> = row.get(1)?;
                let session_id: String = row.get(2)?;
                Ok((content_blob, diffs_blob, session_id))
            },
        )?;

        // Collect files per session
        let mut session_files: HashMap<String, Vec<String>> = HashMap::new();

        for row in rows {
            let (content_blob, diffs_blob, session_id) = row?;
            let content = decompress_text(&content_blob);
            let mut files: Vec<String> = Vec::new();

            // Try file_diffs first
            if let Some(diffs_blob) = diffs_blob {
                let diffs_text = decompress_text(&diffs_blob);
                if let Ok(diffs_json) = serde_json::from_str::<Vec<serde_json::Value>>(&diffs_text) {
                    for entry in &diffs_json {
                        if let Some(path) = entry.get("path").and_then(|v| v.as_str())
                            && !files.contains(&path.to_string()) {
                                files.push(path.to_string());
                            }
                    }
                }
            }

            // Fallback: extract paths from content text
            if files.is_empty() {
                files = extract_file_paths(&content);
            }

            if files.len() >= 2 {
                let sorted = {
                    let mut s = files.clone();
                    s.sort();
                    s
                };
                let entry = session_files.entry(session_id).or_default();
                for f in sorted {
                    if !entry.contains(&f) {
                        entry.push(f);
                    }
                }
            }
        }

        // Build co-occurrence pairs across sessions
        #[derive(Default)]
        struct PairStats {
            count: usize,
            sessions: Vec<String>,
        }
        let mut co_map: HashMap<(String, String), PairStats> = HashMap::new();

        for (session_id, files) in &session_files {
            let sorted = {
                let mut s = files.clone();
                s.sort();
                s
            };
            for i in 0..sorted.len() {
                for j in (i + 1)..sorted.len() {
                    let key = (sorted[i].clone(), sorted[j].clone());
                    let entry = co_map.entry(key).or_default();
                    entry.count += 1;
                    if !entry.sessions.contains(session_id) {
                        entry.sessions.push(session_id.clone());
                    }
                }
            }
        }

        // Write patterns with ≥2 co-occurrences across ≥2 sessions
        let mut added = 0;
        for ((file_a, file_b), stats) in &co_map {
            let unique_sessions = stats.sessions.len();
            if stats.count < 2 || unique_sessions < 2 {
                continue;
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
                db,
                project,
                MemoryType::Pattern,
                &title,
                &content,
                &[],
                &[file_a.clone(), file_b.clone()],
                &tags,
                None,
            ) {
                crate::log_warn!("failed to save co-change pattern: {}", e);
                continue;
            }
            added += 1;
        }

        Ok(added)
    }

    /// Mine error repeat patterns from tool result messages.
    ///
    /// Reads tool results where content indicates errors (non-zero exit,
    /// error/fail/panic markers). Groups similar errors across sessions.
    fn mine_error_repeats(db: &Connection, project: &str) -> Result<usize> {
        let mut stmt = db.prepare(
            "SELECT content, session_id
             FROM messages
             WHERE role = 'tool'
               AND (content LIKE '%error%' OR content LIKE '%fail%' OR content LIKE '%panic%'
                    OR content LIKE '%exit code%' OR content LIKE '%non-zero%'
                    OR content LIKE '%Error%' OR content LIKE '%FAILED%')",
        )?;

        let rows = stmt.query_map([], |row| {
            let content_blob: Vec<u8> = row.get(0)?;
            let session_id: String = row.get(1)?;
            Ok((content_blob, session_id))
        })?;

        #[derive(Default)]
        struct ErrorStats {
            count: usize,
            sessions: Vec<String>,
            titles: Vec<String>,
        }
        let mut error_map: HashMap<String, ErrorStats> = HashMap::new();

        for row in rows {
            let (content_blob, session_id) = row?;
            let content = decompress_text(&content_blob);

            // Normalize: lowercase, take first ~80 chars as a key fragment
            let normalized: String = content
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
                continue;
            }

            let entry = error_map.entry(fragment).or_default();
            entry.count += 1;
            if !entry.sessions.contains(&session_id) {
                entry.sessions.push(session_id.clone());
            }
            let title = content
                .lines()
                .next()
                .unwrap_or(&content[..content.len().min(80)])
                .to_string();
            if !entry.titles.contains(&title) {
                entry.titles.push(title);
            }
        }

        let mut added = 0;
        for stats in error_map.values() {
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
                db,
                project,
                MemoryType::Pattern,
                &title,
                &content,
                &[],
                &[],
                &tags,
                None,
            ) {
                crate::log_warn!("failed to save error-repeat pattern: {}", e);
                continue;
            }
            added += 1;
        }

        Ok(added)
    }
}

/// Extract file paths from tool result text content.
/// Looks for common patterns like paths in backticks, quotes, or bare paths.
fn extract_file_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for word in text.split_whitespace() {
        let w = word.trim_matches(|c: char| {
            c == ',' || c == '"' || c == '\'' || c == '`' || c == '(' || c == ')' || c == '.'
        });
        if (w.contains('/') || w.contains('\\'))
            && !w.starts_with("http")
            && !w.starts_with("https")
            && !paths.contains(&w.to_string())
        {
            paths.push(w.to_string());
        }
    }
    paths.truncate(20);
    paths
}
