use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::config::ActiveModel;
use crate::llm::LlmClient;
use crate::session::{Message, MessageRole};

use crate::memory::types::SessionSummary;
use crate::memory::xml::{clean_llm_xml_response, get_xml_tag_ci, get_xml_children_ci};

// ─── LLM Prompts (translated from agentmemory/src/prompts/summary.ts) ──

pub const SUMMARY_SYSTEM: &str = "You are a session summarizer for an AI coding agent's memory system. Given all compressed observations from a coding session, produce a concise session summary.

Output EXACTLY this XML format with no additional text:

<summary>
  <title>Short session title (max 100 chars)</title>
  <narrative>3-5 sentence narrative of what was accomplished</narrative>
  <decisions>
    <decision>Key technical decision made</decision>
  </decisions>
  <files>
    <file>path/to/modified/file</file>
  </files>
  <concepts>
    <concept>key concept from session</concept>
  </concepts>
</summary>

Rules:
- Focus on outcomes, not individual tool calls
- Highlight decisions and their rationale
- List all files that were created or modified
- Concepts should be searchable terms for future context retrieval";

fn build_summary_prompt(observations: &[CompressedView]) -> String {
    let lines: Vec<String> = observations
        .iter()
        .enumerate()
        .map(|(i, obs)| {
            format!(
                "[{}] {}: {}\n{}\nFiles: {}\nConcepts: {}",
                i + 1,
                obs.obs_type,
                obs.title,
                obs.narrative,
                obs.files.join(", "),
                obs.concepts.join(", ")
            )
        })
        .collect();

    format!(
        "Session observations ({} total):\n\n{}",
        observations.len(),
        lines.join("\n\n---\n\n")
    )
}

struct CompressedView {
    obs_type: String,
    title: String,
    narrative: String,
    files: Vec<String>,
    concepts: Vec<String>,
}

/// Session management service.
pub struct SessionService;

impl SessionService {
    /// Generate and store a session summary using LLM.
    /// Opens its own DB connection to avoid !Send issues.
    pub async fn summarize_session(
        db_path: &std::path::Path,
        llm: &LlmClient,
        model: &ActiveModel,
        session_id: Uuid,
        project: &str,
    ) -> Result<SessionSummary> {
        // 1. Load compressed observations (sync, connection dropped before await)
        let views = {
            let db = Connection::open(db_path)?;
            let mut stmt = db.prepare(
                "SELECT obs_type, title, narrative, files, concepts
                 FROM compressed_observations
                 WHERE session_id = ?1
                 ORDER BY created_at ASC",
            )?;

            let views: Vec<CompressedView> = stmt
                .query_map(rusqlite::params![session_id.to_string()], |row| {
                    let files_json: String = row.get(3)?;
                    let concepts_json: String = row.get(4)?;
                    Ok(CompressedView {
                        obs_type: row.get(0)?,
                        title: row.get(1)?,
                        narrative: row.get(2)?,
                        files: serde_json::from_str(&files_json).unwrap_or_default(),
                        concepts: serde_json::from_str(&concepts_json).unwrap_or_default(),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            views
        };

        if views.is_empty() {
            // No compressed observations — return empty summary
            let summary = SessionSummary {
                session_id,
                project: project.to_string(),
                created_at: Utc::now(),
                title: None,
                narrative: None,
                key_decisions: vec![],
                files_modified: vec![],
                concepts: vec![],
                observation_count: 0,
            };
            let store_db = Connection::open(db_path)?;
            Self::store_summary(&store_db, &summary)?;
            return Ok(summary);
        }

        // ─── STRICTER_SUFFIX for retry (like compress.rs) ────────────
        const STRICTER_SUFFIX: &str = r"
IMPORTANT: Your response MUST contain valid XML tags. Do NOT output any text outside the XML tags. Do NOT wrap XML in markdown code fences. The first non-whitespace character MUST be '<'.";

        // Helper: build LLM messages with optional stricter suffix
        let make_messages = |strict: bool| -> Vec<Message> {
            let system = if strict {
                format!("{}{}", SUMMARY_SYSTEM, STRICTER_SUFFIX)
            } else {
                SUMMARY_SYSTEM.to_string()
            };
            let prompt = build_summary_prompt(&views);
            vec![
                Message::new(MessageRole::System, system),
                Message::new(MessageRole::User, prompt),
            ]
        };

        // Try LLM call with up to 1 retry on parse failure
        let mut response = String::new();
        let mut parse_ok = false;

        for attempt in 0..2 {
            let messages = make_messages(attempt > 0);
            response = match llm
                .complete_with_messages(model.clone(), messages, vec![])
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    crate::log_warn!("LLM summarization failed (attempt {}): {}", attempt, e);
                    if attempt == 0 {
                        continue; // retry
                    }
                    break; // give up, will use synthetic fallback
                }
            };

            // Attempt to parse the response
            let cleaned = clean_llm_xml_response(&response);
            let has_title = get_xml_tag_ci(&cleaned, "title").is_some();
            let has_narrative = get_xml_tag_ci(&cleaned, "narrative").is_some();
            if has_title && has_narrative {
                parse_ok = true;
                break;
            }

            if attempt == 0 {
                crate::log_warn!(
                    "LLM summarization response unparseable (attempt 0), retrying with stricter prompt"
                );
            }
        }

        let summary = if parse_ok {
            let cleaned = clean_llm_xml_response(&response);
            let title = get_xml_tag_ci(&cleaned, "title");
            let narrative = get_xml_tag_ci(&cleaned, "narrative");
            let decisions = get_xml_children_ci(&cleaned, "decisions", "decision");
            let files = get_xml_children_ci(&cleaned, "files", "file");
            let concepts = get_xml_children_ci(&cleaned, "concepts", "concept");

            SessionSummary {
                session_id,
                project: project.to_string(),
                created_at: Utc::now(),
                title,
                narrative,
                key_decisions: decisions,
                files_modified: files,
                concepts,
                observation_count: views.len() as i64,
            }
        } else {
            let fb = Self::parse_summary_free_text(&response, &views, session_id, project);
            if let Some(ref fb_title) = fb.title {
                crate::log_info!(
                    "LLM summarization response unparseable, used free-text fallback (title=\"{}\")",
                    fb_title
                );
                fb
            } else {
                crate::log_warn!("LLM summarization failed or response unparseable, using synthetic fallback");
                let title = views.first().map(|v| v.title.clone()).unwrap_or_default();
                let mut file_set: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                let mut concept_set: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                let mut type_counts: std::collections::BTreeMap<String, usize> =
                    std::collections::BTreeMap::new();
                for v in &views {
                    for f in &v.files {
                        file_set.insert(f.clone());
                    }
                    for c in &v.concepts {
                        concept_set.insert(c.clone());
                    }
                    *type_counts.entry(v.obs_type.clone()).or_default() += 1;
                }
                let obs_summary: String = {
                    let mut parts: Vec<String> = type_counts
                        .into_iter()
                        .map(|(t, c)| format!("{}×{}", c, t))
                        .collect();
                    parts.sort();
                    parts.join(", ")
                };
                let narrative = if !obs_summary.is_empty() {
                    format!(
                        "Session with {} observations ({}).",
                        views.len(),
                        obs_summary
                    )
                } else {
                    format!("Session with {} observations.", views.len())
                };
                SessionSummary {
                    session_id,
                    project: project.to_string(),
                    created_at: Utc::now(),
                    title: Some(title),
                    narrative: Some(narrative),
                    key_decisions: vec![],
                    files_modified: file_set.into_iter().collect(),
                    concepts: concept_set.into_iter().collect(),
                    observation_count: views.len() as i64,
                }
            }
        };

        // 4. Persist (sync, new connection)
        let db = Connection::open(db_path)?;
        Self::store_summary(&db, &summary)?;

        Ok(summary)
    }

    /// Fallback: extract summary fields from free-form text when the LLM
    /// cannot produce structured XML. Returns a SessionSummary with fields
    /// extracted heuristically; returns `None` title if nothing useful.
    fn parse_summary_free_text(
        text: &str,
        views: &[CompressedView],
        session_id: Uuid,
        project: &str,
    ) -> SessionSummary {
        let lines: Vec<&str> = text.lines().collect();

        // Title: first non-empty line under 100 chars
        let title = lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .map(|l| {
                let t = l.trim();
                if t.len() <= 100 {
                    t.to_string()
                } else {
                    format!("{}…", &t[..97])
                }
            });

        // Narrative: skip first "title" line, take the rest
        let narrative = if let Some(t) = &title {
            let rest: Vec<&str> = lines
                .iter()
                .filter(|l| l.trim() != t.as_str() && !l.trim().is_empty())
                .copied()
                .collect();
            if rest.is_empty() {
                None
            } else {
                let joined = rest.join("\n").trim().to_string();
                if joined.len() > 1000 {
                    Some(format!("{}…", &joined[..1000]))
                } else {
                    Some(joined)
                }
            }
        } else {
            None
        };

        // Decisions: bullet or numbered lines
        let decisions: Vec<String> = lines
            .iter()
            .filter(|l| {
                let t = l.trim();
                (t.starts_with('-') || t.starts_with('*'))
                    && t.len() > 2
            })
            .map(|l| {
                l.trim()
                    .trim_start_matches('-')
                    .trim_start_matches('*')
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        // Files: extract path-like patterns
        let files = extract_paths_from_text(text);

        // Concepts: extract technical keywords
        let concepts = extract_concepts_from_text(text);

        SessionSummary {
            session_id,
            project: project.to_string(),
            created_at: Utc::now(),
            title,
            narrative,
            key_decisions: decisions,
            files_modified: files,
            concepts,
            observation_count: views.len() as i64,
        }
    }

    fn store_summary(db: &Connection, summary: &SessionSummary) -> Result<()> {
        db.execute(
            "INSERT OR REPLACE INTO session_summaries (session_id, project, created_at, title, narrative, key_decisions, files_modified, concepts, observation_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                summary.session_id.to_string(),
                summary.project,
                summary.created_at.to_rfc3339(),
                summary.title,
                summary.narrative,
                serde_json::to_string(&summary.key_decisions)?,
                serde_json::to_string(&summary.files_modified)?,
                serde_json::to_string(&summary.concepts)?,
                summary.observation_count,
            ],
        )?;

        Ok(())
    }

    /// Load a session summary.
    pub fn load_summary(db: &Connection, session_id: Uuid) -> Result<Option<SessionSummary>> {
        let mut stmt = db.prepare(
            "SELECT session_id, project, created_at, title, narrative, key_decisions, files_modified, concepts, observation_count
             FROM session_summaries WHERE session_id = ?1",
        )?;

        let result = stmt.query_row(rusqlite::params![session_id.to_string()], |row| {
            let decisions_json: String = row.get(5)?;
            let files_json: String = row.get(6)?;
            let concepts_json: String = row.get(7)?;
            Ok(SessionSummary {
                session_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(session_id),
                project: row.get(1)?,
                created_at: row
                    .get::<_, String>(2)
                    .ok()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                title: row.get(3)?,
                narrative: row.get(4)?,
                key_decisions: serde_json::from_str(&decisions_json).unwrap_or_default(),
                files_modified: serde_json::from_str(&files_json).unwrap_or_default(),
                concepts: serde_json::from_str(&concepts_json).unwrap_or_default(),
                observation_count: row.get(8)?,
            })
        });

        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// Extract file paths from unstructured text.
fn extract_paths_from_text(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for word in text.split_whitespace() {
        let w = word.trim_matches(|c: char| c == ',' || c == '"' || c == '\'' || c == '`' || c == '(' || c == ')');
        if (w.contains('/') || w.contains('\\'))
            && w.contains('.')
            && !w.starts_with("http")
            && !w.starts_with("https")
        {
            if !w.starts_with('<') && !w.starts_with('{') && !w.starts_with('(') && !paths.contains(&w.to_string()) {
                paths.push(w.to_string());
            }
        }
    }
    paths.truncate(8);
    paths
}

/// Extract technical concepts from unstructured text.
fn extract_concepts_from_text(text: &str) -> Vec<String> {
    let keywords = &[
        "Rust", "rust", "Cargo", "Go", "golang", "TypeScript", "JavaScript",
        "Node", "Python", "React", "Vue", "SQLite", "Postgres", "MySQL",
        "Docker", "Kubernetes", "API", "CLI", "TUI", "Git", "Linux", "macOS",
        "SSH", "HTTP", "TLS", "compression", "memory", "caching", "logging",
        "error", "configuration", "refactoring", "migration", "testing",
    ];
    let text_lower = text.to_lowercase();
    let mut found: Vec<String> = Vec::new();
    for &kw in keywords {
        if text_lower.contains(&kw.to_lowercase()) {
            let formatted = match kw {
                "javascript" => "JavaScript".to_string(),
                "typescript" => "TypeScript".to_string(),
                "golang" => "Go".to_string(),
                "rust" => "Rust".to_string(),
                _ => {
                    let mut c = kw.chars();
                    match c.next() {
                        Some(first) => first.to_uppercase().to_string() + c.as_str(),
                        None => kw.to_string(),
                    }
                }
            };
            if !found.contains(&formatted) {
                found.push(formatted);
            }
        }
    }
    found
}
