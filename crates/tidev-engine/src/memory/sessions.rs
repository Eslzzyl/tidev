use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::config::ActiveModel;
use crate::context::ContextManager;
use crate::llm::LlmClient;
use crate::memory::types::SessionSummary;
use crate::memory::xml::{clean_llm_xml_response, get_xml_children_ci, get_xml_tag_ci};
use tidev_types::prompts::SessionMode;
use tidev_session::session::{Conversation, Message, MessageRole};
use crate::storage::load_session_messages;
use crate::tooling::ToolDefinition;

pub const SUMMARY_INSTRUCTION: &str =
    "Please summarize this session. Output EXACTLY this XML format with no additional text:

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

/// Session management service.
pub struct SessionService;

impl SessionService {
    /// Generate and store a session summary using LLM.
    ///
    /// Builds the request messages using the same `build_request_messages` logic
    /// as normal requests and compaction, so the prefix is identical (maximizing
    /// prompt cache hits). Only messages after the last compact are included,
    /// with the context summary prepended as context.
    ///
    /// `tools` must be the same filtered tool list used during normal conversation
    /// turns for the target model, so the full LLM request (messages + tools)
    /// is byte-for-byte identical up to the appended summary instruction.
    pub async fn summarize_session(
        db_path: &std::path::Path,
        llm: &LlmClient,
        model: &ActiveModel,
        session_id: Uuid,
        project: &str,
        tools: &[ToolDefinition],
    ) -> Result<SessionSummary> {
        // 1. Load session context state + messages (sync, connection dropped before await)
        let (messages, context_summary, context_retained_from, system_prompt) = {
            let db = Connection::open(db_path)?;

            // Load context state from session record
            let (summary, retained_from, system_prompt) = {
                let mut stmt = db.prepare(
                    "SELECT context_summary, context_retained_from, system_prompt FROM sessions WHERE id = ?1"
                )?;
                let result = stmt
                    .query_row(params![session_id.to_string()], |row| {
                        let summary: String = row.get(0)?;
                        let retained: i64 = row.get(1)?;
                        let sp: String = row.get(2)?;
                        Ok((
                            if summary.is_empty() {
                                None
                            } else {
                                Some(summary)
                            },
                            retained as usize,
                            if sp.is_empty() { None } else { Some(sp) },
                        ))
                    })
                    .optional()?;
                result.unwrap_or((None, 0, None))
            };

            let messages = load_session_messages(&db, session_id)?;
            (messages, summary, retained_from, system_prompt)
        };

        let msg_count = messages.len();

        if msg_count == 0 {
            let summary = SessionSummary {
                session_id,
                project: project.to_string(),
                created_at: Utc::now(),
                title: None,
                narrative: None,
                key_decisions: vec![],
                files_modified: vec![],
                concepts: vec![],
            };
            let store_db = Connection::open(db_path)?;
            Self::store_summary(&store_db, &summary)?;
            return Ok(summary);
        }

        // 2. Build request messages using the same logic as normal requests / compact.
        //    This ensures the prefix is byte-for-byte identical, maximizing cache hits.
        let context_manager = ContextManager::from_state(context_summary, context_retained_from);
        let mut conv = Conversation::new(session_id, "", "", "", "", "", "");
        conv.messages = messages;

        let mut llm_messages = context_manager.build_request_messages(&conv, SessionMode::Build);
        llm_messages.push(Message::new(
            MessageRole::User,
            SUMMARY_INSTRUCTION.to_string(),
        ));

        // Try LLM call with up to 1 retry on parse failure
        const STRICTER_SUFFIX: &str = r"
IMPORTANT: Your response MUST contain valid XML tags. Do NOT output any text outside the XML tags. Do NOT wrap XML in markdown code fences. The first non-whitespace character MUST be '<'.";

        let mut response = String::new();
        let mut parse_ok = false;

        for attempt in 0..2 {
            let mut attempt_messages = llm_messages.clone();
            if attempt > 0
                && let Some(last) = attempt_messages.last_mut()
            {
                last.content = format!("{}{}", last.content, STRICTER_SUFFIX);
            }

            let llm_model = if let Some(ref sp) = system_prompt {
                let mut m = model.clone();
                m.system_prompt = sp.clone();
                m
            } else {
                model.clone()
            };
            response = match llm
                .complete_with_messages(llm_model, attempt_messages, tools.to_vec())
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("LLM summarization failed (attempt {}): {}", attempt, e);
                    if attempt == 0 {
                        continue;
                    }
                    break;
                }
            };

            let cleaned = clean_llm_xml_response(&response);
            let has_title = get_xml_tag_ci(&cleaned, "title").is_some();
            let has_narrative = get_xml_tag_ci(&cleaned, "narrative").is_some();
            if has_title && has_narrative {
                parse_ok = true;
                break;
            }

            if attempt == 0 {
                log::warn!(
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
            }
        } else {
            let fb = Self::parse_summary_free_text(&response, session_id, project);
            if let Some(ref fb_title) = fb.title {
                log::info!(
                    "LLM summarization response unparseable, used free-text fallback (title=\"{}\")",
                    fb_title
                );
                fb
            } else {
                log::warn!(
                    "LLM summarization failed or response unparseable, using synthetic fallback"
                );
                SessionSummary {
                    session_id,
                    project: project.to_string(),
                    created_at: Utc::now(),
                    title: Some(format!("Session {}", &session_id.to_string()[..8])),
                    narrative: Some(format!("Session with {} messages.", msg_count)),
                    key_decisions: vec![],
                    files_modified: vec![],
                    concepts: vec![],
                }
            }
        };

        // Persist
        let db = Connection::open(db_path)?;
        Self::store_summary(&db, &summary)?;

        Ok(summary)
    }

    fn parse_summary_free_text(text: &str, session_id: Uuid, project: &str) -> SessionSummary {
        let lines: Vec<&str> = text.lines().collect();

        let title = lines.iter().find(|l| !l.trim().is_empty()).map(|l| {
            let t = l.trim();
            if t.len() <= 100 {
                t.to_string()
            } else {
                format!("{}…", &t[..97])
            }
        });

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

        let decisions: Vec<String> = lines
            .iter()
            .filter(|l| {
                let t = l.trim();
                (t.starts_with('-') || t.starts_with('*')) && t.len() > 2
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

        let files = extract_paths_from_text(text);
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
        }
    }

    fn store_summary(db: &Connection, summary: &SessionSummary) -> Result<()> {
        db.execute(
            "INSERT OR REPLACE INTO session_summaries (session_id, project, created_at, title, narrative, key_decisions, files_modified, concepts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                summary.session_id.to_string(),
                summary.project,
                summary.created_at.to_rfc3339(),
                summary.title,
                summary.narrative,
                serde_json::to_string(&summary.key_decisions)?,
                serde_json::to_string(&summary.files_modified)?,
                serde_json::to_string(&summary.concepts)?,
            ],
        )?;

        Ok(())
    }

    /// Load a session summary.
    pub fn load_summary(db: &Connection, session_id: Uuid) -> Result<Option<SessionSummary>> {
        let mut stmt = db.prepare(
            "SELECT session_id, project, created_at, title, narrative, key_decisions, files_modified, concepts
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
        let w = word.trim_matches(|c: char| {
            c == ',' || c == '"' || c == '\'' || c == '`' || c == '(' || c == ')'
        });
        if (w.contains('/') || w.contains('\\'))
            && w.contains('.')
            && !w.starts_with("http")
            && !w.starts_with("https")
            && !w.starts_with('<')
            && !w.starts_with('{')
            && !w.starts_with('(')
            && !paths.contains(&w.to_string())
        {
            paths.push(w.to_string());
        }
    }
    paths.truncate(8);
    paths
}

/// Extract technical concepts from unstructured text.
fn extract_concepts_from_text(text: &str) -> Vec<String> {
    let keywords = &[
        "Rust",
        "rust",
        "Cargo",
        "Go",
        "golang",
        "TypeScript",
        "JavaScript",
        "Node",
        "Python",
        "React",
        "Vue",
        "SQLite",
        "Postgres",
        "MySQL",
        "Docker",
        "Kubernetes",
        "API",
        "CLI",
        "TUI",
        "Git",
        "Linux",
        "macOS",
        "SSH",
        "HTTP",
        "TLS",
        "compression",
        "memory",
        "caching",
        "logging",
        "error",
        "configuration",
        "refactoring",
        "migration",
        "testing",
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
