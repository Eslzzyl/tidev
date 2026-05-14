use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::llm::LlmClient;
use crate::config::ActiveModel;
use crate::session::{Message, MessageRole};

use crate::memory::types::{SessionSummary};

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

    format!("Session observations ({} total):\n\n{}", observations.len(), lines.join("\n\n---\n\n"))
}

struct CompressedView {
    obs_type: String,
    title: String,
    narrative: String,
    files: Vec<String>,
    concepts: Vec<String>,
}

// ─── XML Parsing ──────────────────────────────────────────────────────

fn get_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let start = format!("<{}>", tag);
    let end = format!("</{}>", tag);
    let s = xml.find(&start)?;
    let e = xml[s + start.len()..].find(&end)?;
    let value = xml[s + start.len()..s + start.len() + e].trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn get_xml_children(xml: &str, parent: &str, child: &str) -> Vec<String> {
    let parent_start = format!("<{}>", parent);
    let parent_end = format!("</{}>", parent);
    let mut result = Vec::new();
    let s = match xml.find(&parent_start) {
        Some(pos) => pos,
        None => return vec![],
    };
    let e = match xml[s..].find(&parent_end) {
        Some(pos) => pos,
        None => return vec![],
    };
    let section = &xml[s + parent_start.len()..s + e];
    let child_start = format!("<{}>", child);
    let child_end = format!("</{}>", child);
    let mut pos = 0;
    while let Some(cs) = section[pos..].find(&child_start) {
        let content_start = pos + cs + child_start.len();
        if let Some(ce) = section[content_start..].find(&child_end) {
            let value = section[content_start..content_start + ce].trim().to_string();
            if !value.is_empty() { result.push(value); }
            pos = content_start + ce + child_end.len();
        } else { break; }
    }
    result
}

/// Session management service.
pub struct SessionService;

impl SessionService {
    /// Generate and store a session summary using LLM.
    pub async fn summarize_session(
        db: &Connection,
        llm: &LlmClient,
        model: &ActiveModel,
        session_id: Uuid,
        project: &str,
    ) -> Result<SessionSummary> {
        // 1. Load compressed observations for this session
        let mut stmt = db.prepare(
            "SELECT obs_type, title, narrative, files, concepts
             FROM compressed_observations
             WHERE session_id = ?1
             ORDER BY created_at ASC",
        )?;

        let views: Vec<CompressedView> = stmt.query_map(
            rusqlite::params![session_id.to_string()],
            |row| {
                let files_json: String = row.get(3)?;
                let concepts_json: String = row.get(4)?;
                Ok(CompressedView {
                    obs_type: row.get(0)?,
                    title: row.get(1)?,
                    narrative: row.get(2)?,
                    files: serde_json::from_str(&files_json).unwrap_or_default(),
                    concepts: serde_json::from_str(&concepts_json).unwrap_or_default(),
                })
            },
        )?.filter_map(|r| r.ok()).collect();

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
            Self::store_summary(db, &summary)?;
            return Ok(summary);
        }

        // 2. Build prompt and call LLM
        let prompt = build_summary_prompt(&views);
        let messages = vec![
            Message::new(MessageRole::System, SUMMARY_SYSTEM.to_string()),
            Message::new(MessageRole::User, prompt),
        ];

        let response = llm
            .complete_with_messages(model.clone(), messages, vec![])
            .await
            .context("LLM summarization failed")?;

        // 3. Parse XML response
        let title = get_xml_tag(&response, "title");
        let narrative = get_xml_tag(&response, "narrative");
        let decisions = get_xml_children(&response, "decisions", "decision");
        let files = get_xml_children(&response, "files", "file");
        let concepts = get_xml_children(&response, "concepts", "concept");

        let summary = SessionSummary {
            session_id,
            project: project.to_string(),
            created_at: Utc::now(),
            title,
            narrative,
            key_decisions: decisions,
            files_modified: files,
            concepts,
            observation_count: views.len() as i64,
        };

        // 4. Persist
        Self::store_summary(db, &summary)?;

        Ok(summary)
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

        let result = stmt.query_row(
            rusqlite::params![session_id.to_string()],
            |row| {
                let decisions_json: String = row.get(5)?;
                let files_json: String = row.get(6)?;
                let concepts_json: String = row.get(7)?;
                Ok(SessionSummary {
                    session_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(session_id),
                    project: row.get(1)?,
                    created_at: row.get::<_, String>(2).ok()
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
            },
        );

        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
