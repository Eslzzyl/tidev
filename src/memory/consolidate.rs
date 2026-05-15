use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

use crate::config::ActiveModel;
use crate::llm::LlmClient;
use crate::session::{Message, MessageRole};
use crate::memory::types::*;
use crate::memory::remember::RememberService;
use crate::memory::retention::RetentionService;

// ─── LLM Prompts ──────────────────────────────────────────────────────

pub const SEMANTIC_MERGE_SYSTEM: &str = r#"You are a memory consolidation engine for a coding AI assistant. Given session summaries from past work sessions, extract stable factual knowledge about the project.

Output EXACTLY this XML format with no additional text:

<facts>
  <fact confidence="0.0-1.0">Concise factual statement about the project</fact>
</facts>

Rules:
- Extract only facts that appear in 2+ sessions or are highly confident (>0.8)
- Confidence reflects how well-supported the fact is across sessions
- Combine overlapping information into single concise facts
- Focus on: architecture decisions, project conventions, tool preferences, key constraints, dependency choices
- Skip ephemeral details (specific error messages, temporary state, one-off debugging steps)
- Each fact must be a complete, standalone statement"#;

pub const PROCEDURAL_EXTRACTION_SYSTEM: &str = r#"You are a procedural memory extractor. Given workflow patterns and repeated tasks observed across sessions, extract reusable procedures.

Output EXACTLY this XML format with no additional text:

<procedures>
  <procedure name="short descriptive name" trigger="when to use this procedure">
    <step>Step 1 description</step>
    <step>Step 2 description</step>
  </procedure>
</procedures>

Rules:
- Only extract procedures observed 2+ times or that are clearly repeatable
- Steps should be concrete and actionable (specific tool commands, file paths, etc.)
- Trigger condition should be specific enough to match automatically
- Name should be short (max 60 chars)"#;

// ─── Report ───────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct ConsolidationReport {
    pub semantic_facts_added: usize,
    pub procedural_patterns_added: usize,
    pub skipped_reason: Option<String>,
}

// ─── Core Service ─────────────────────────────────────────────────────

pub struct ConsolidationService;

impl ConsolidationService {
    /// Run the full consolidation pipeline:
    ///   Tier 1: extract cross-session semantic facts from summaries
    ///   Tier 2: extract reusable procedures from pattern/workflow memories
    pub async fn run(
        db_path: &Path,
        llm: &LlmClient,
        model: &ActiveModel,
        project: &str,
    ) -> Result<ConsolidationReport> {
        let mut report = ConsolidationReport::default();

        // Tier 1
        match Self::consolidate_semantic(db_path, llm, model, project).await {
            Ok(n) => report.semantic_facts_added = n,
            Err(e) => crate::log_warn!("semantic consolidation failed: {}", e),
        }

        // Tier 2
        match Self::extract_procedural(db_path, llm, model, project).await {
            Ok(n) => report.procedural_patterns_added = n,
            Err(e) => crate::log_warn!("procedural extraction failed: {}", e),
        }

        if report.semantic_facts_added == 0 && report.procedural_patterns_added == 0 {
            report.skipped_reason = Some("no new facts or patterns extracted".to_string());
        }

        Ok(report)
    }

    // ─── Tier 1: Semantic Consolidation ───────────────────────────────

    async fn consolidate_semantic(
        db_path: &Path,
        llm: &LlmClient,
        model: &ActiveModel,
        project: &str,
    ) -> Result<usize> {
        // 1. Load summaries (sync, connection dropped before await)
        let summaries = {
            let db = Connection::open(db_path)?;
            Self::load_summaries(&db, project)?
        };

        if summaries.len() < 5 {
            return Ok(0); // need at least 5 summaries for meaningful consolidation
        }

        // 2. Load cursor — skip already-consolidated summaries
        let last_id = {
            let db = Connection::open(db_path)?;
            Self::load_cursor(&db, "semantic")?
        };

        let new_summaries: Vec<_> = summaries
            .iter()
            .filter(|s| s.session_id.to_string() > last_id)
            .collect();

        if new_summaries.is_empty() {
            return Ok(0);
        }

        // 3. Build prompt and call LLM (no DB connection held)
        let prompt = build_semantic_prompt(&new_summaries);
        let messages = vec![
            Message::new(MessageRole::System, SEMANTIC_MERGE_SYSTEM.to_string()),
            Message::new(MessageRole::User, prompt),
        ];
        let response = llm
            .complete_with_messages(model.clone(), messages, vec![])
            .await
            .context("semantic consolidation LLM call failed")?;

        // 4. Parse XML
        let facts = parse_facts_xml(&response);

        // 5. Write facts + update cursor (sync, new connection)
        let db = Connection::open(db_path)?;
        for fact in &facts {
            let tags = vec!["consolidated".to_string()];
            if let Err(e) = RememberService::remember(
                &db,
                project,
                MemoryType::Fact,
                &fact.title,
                &fact.content,
                &[],  // concepts — empty to save tokens; could be populated later
                &[],  // files
                &tags,
                None, // source_session_id — multiple sources, leave generic
            ) {
                crate::log_warn!("failed to remember consolidated fact: {}", e);
                continue;
            }
            // Auto-compute retention score
            let _ = RetentionService::compute_and_store(
                &db,
                &Uuid::new_v4().to_string(),
                "memory",
                5.0,
                0.0,
                1,
            );
        }

        // Update cursor to the last summary we processed
        if let Some(last) = new_summaries.last() {
            Self::save_cursor(&db, "semantic", &last.session_id.to_string())?;
        }

        Ok(facts.len())
    }

    // ─── Tier 2: Procedural Extraction ───────────────────────────────

    async fn extract_procedural(
        db_path: &Path,
        llm: &LlmClient,
        model: &ActiveModel,
        project: &str,
    ) -> Result<usize> {
        // 1. Load pattern/workflow memories (sync)
        let patterns = {
            let db = Connection::open(db_path)?;
            Self::load_pattern_memories(&db, project)?
        };

        if patterns.len() < 3 {
            return Ok(0); // need patterns to extract procedures from
        }

        // 2. Load cursor
        let last_id = {
            let db = Connection::open(db_path)?;
            Self::load_cursor(&db, "procedural")?
        };

        let new_patterns: Vec<_> = patterns
            .iter()
            .filter(|m| m.id.to_string() > last_id)
            .collect();

        if new_patterns.is_empty() {
            return Ok(0);
        }

        // 3. Call LLM (no DB connection held)
        let prompt = build_procedural_prompt(&new_patterns);
        let messages = vec![
            Message::new(MessageRole::System, PROCEDURAL_EXTRACTION_SYSTEM.to_string()),
            Message::new(MessageRole::User, prompt),
        ];
        let response = llm
            .complete_with_messages(model.clone(), messages, vec![])
            .await
            .context("procedural extraction LLM call failed")?;

        // 4. Parse XML
        let procedures = parse_procedures_xml(&response);

        // 5. Write procedures (sync, new connection)
        let db = Connection::open(db_path)?;
        for proc in &procedures {
            let steps_text = proc.steps.join("\n");
            let tags = vec!["consolidated".to_string(), "procedure".to_string()];
            if let Err(e) = RememberService::remember(
                &db,
                project,
                MemoryType::Pattern,
                &proc.name,
                &steps_text,
                &[],
                &[],
                &tags,
                None,
            ) {
                crate::log_warn!("failed to remember procedure: {}", e);
                continue;
            }
        }

        // Update cursor
        if let Some(last) = new_patterns.last() {
            Self::save_cursor(&db, "procedural", &last.id.to_string())?;
        }

        Ok(procedures.len())
    }

    // ─── Cursor Helpers (via meta table) ──────────────────────────────

    fn load_cursor(db: &Connection, tier: &str) -> Result<String> {
        let key = format!("consolidation_cursor_{}", tier);
        let result: Option<String> = db
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .ok();
        Ok(result.unwrap_or_default())
    }

    fn save_cursor(db: &Connection, tier: &str, value: &str) -> Result<()> {
        let key = format!("consolidation_cursor_{}", tier);
        db.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    // ─── Data Loaders ─────────────────────────────────────────────────

    fn load_summaries(db: &Connection, project: &str) -> Result<Vec<SessionSummary>> {
        let mut stmt = db.prepare(
            "SELECT session_id, project, created_at, title, narrative,
                    key_decisions, files_modified, concepts, observation_count
             FROM session_summaries
             WHERE project = ?1 AND title IS NOT NULL
             ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(rusqlite::params![project], |row| {
            let decisions_json: String = row.get(5)?;
            let files_json: String = row.get(6)?;
            let concepts_json: String = row.get(7)?;
            Ok(SessionSummary {
                session_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
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
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    fn load_pattern_memories(db: &Connection, project: &str) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type IN ('pattern', 'workflow')
               AND active = 1 AND is_latest = 1
             ORDER BY created_at DESC",
        )?;

        let rows = stmt.query_map(rusqlite::params![project], |row| {
            map_memory_entry(row)
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load consolidated facts for prompt injection.
    pub fn load_consolidated_facts(db: &Connection, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type = 'fact'
               AND active = 1 AND is_latest = 1
               AND tags LIKE '%consolidated%'
             ORDER BY strength DESC, usage_count DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![project, limit as i64], |row| {
            map_memory_entry(row)
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load consolidated procedures for prompt injection.
    pub fn load_consolidated_procedures(db: &Connection, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type = 'pattern'
               AND active = 1 AND is_latest = 1
               AND tags LIKE '%consolidated%'
             ORDER BY strength DESC, usage_count DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![project, limit as i64], |row| {
            map_memory_entry(row)
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

// ─── Prompt Builders ──────────────────────────────────────────────────

fn build_semantic_prompt(summaries: &[&SessionSummary]) -> String {
    let mut parts = vec![format!(
        "Here are {} session summaries from the project. Extract stable, cross-session factual knowledge.\n",
        summaries.len()
    )];

    for (i, s) in summaries.iter().enumerate() {
        let title = s.title.as_deref().unwrap_or("Untitled");
        let narrative = s.narrative.as_deref().unwrap_or("");
        let decisions = s.key_decisions.join("; ");
        let concepts = s.concepts.join(", ");

        parts.push(format!(
            "[Session {}] Title: {}
Narrative: {}
Decisions: {}
Concepts: {}",
            i + 1,
            title,
            narrative,
            decisions,
            concepts,
        ));
    }

    parts.join("\n\n---\n\n")
}

fn build_procedural_prompt(memories: &[&MemoryEntry]) -> String {
    let mut parts = vec![format!(
        "Here are {} remembered patterns and workflows. Extract reusable procedures.\n",
        memories.len()
    )];

    for (i, m) in memories.iter().enumerate() {
        parts.push(format!(
            "[{}] Type: {} | Title: {}
Content: {}",
            i + 1,
            m.memory_type.as_str(),
            m.title,
            m.content,
        ));
    }

    parts.join("\n\n---\n\n")
}

// ─── XML Parsing ──────────────────────────────────────────────────────

struct FactEntry {
    title: String,
    content: String,
    confidence: f64,
}

struct ProcedureEntry {
    name: String,
    trigger: String,
    steps: Vec<String>,
}

fn parse_facts_xml(xml: &str) -> Vec<FactEntry> {
    let mut facts = Vec::new();

    // Find <facts>...</facts>
    let facts_start = xml.find("<facts>");
    let facts_end = xml.find("</facts>");
    let (start, end) = match (facts_start, facts_end) {
        (Some(s), Some(e)) => (s + "<facts>".len(), e),
        _ => return facts,
    };
    let inner = &xml[start..end];

    // Parse individual <fact confidence="...">...</fact>
    let mut pos = 0;
    while let Some(fs) = inner[pos..].find("<fact") {
        let tag_end = inner[pos + fs..].find('>').map(|i| pos + fs + i + 1);
        let content_start = match tag_end {
            Some(i) => i,
            None => break,
        };
        let content_end = match inner[content_start..].find("</fact>") {
            Some(i) => content_start + i,
            None => break,
        };

        // Extract confidence attribute
        let attr_section = &inner[pos + fs..pos + fs + 80.min(inner.len() - pos - fs)];
        let confidence = attr_section
            .find("confidence=\"")
            .and_then(|c| {
                let val_start = c + "confidence=\"".len();
                attr_section[val_start..].find('"').map(|e| {
                    attr_section[val_start..val_start + e]
                        .parse::<f64>()
                        .unwrap_or(0.5)
                })
            })
            .unwrap_or(0.5);

        let content = inner[content_start..content_end].trim().to_string();
        if !content.is_empty() {
            // Use first 80 chars as title
            let title = if content.len() > 80 {
                format!("{}...", &content[..77])
            } else {
                content.clone()
            };
            facts.push(FactEntry {
                title,
                content,
                confidence,
            });
        }

        pos = content_end + "</fact>".len();
    }

    facts
}

fn parse_procedures_xml(xml: &str) -> Vec<ProcedureEntry> {
    let mut procedures = Vec::new();

    let outer_start = xml.find("<procedures>");
    let outer_end = xml.find("</procedures>");
    let (start, end) = match (outer_start, outer_end) {
        (Some(s), Some(e)) => (s + "<procedures>".len(), e),
        _ => return procedures,
    };
    let inner = &xml[start..end];

    let mut pos = 0;
    while let Some(ps) = inner[pos..].find("<procedure") {
        let tag_end = inner[pos + ps..].find('>').map(|i| pos + ps + i + 1);
        let block_start = match tag_end {
            Some(i) => i,
            None => break,
        };
        let block_end = match inner[block_start..].find("</procedure>") {
            Some(i) => block_start + i,
            None => break,
        };

        // Extract attributes
        let attr_section = &inner[pos + ps..pos + ps + 200.min(inner.len() - pos - ps)];
        let name = extract_attr(attr_section, "name").unwrap_or_default();
        let trigger = extract_attr(attr_section, "trigger").unwrap_or_default();

        // Parse steps
        let block = &inner[block_start..block_end];
        let mut steps = Vec::new();
        let mut sp = 0;
        while let Some(ss) = block[sp..].find("<step>") {
            let se = block[sp + ss + "<step>".len()..].find("</step>");
            match se {
                Some(e) => {
                    let step = block[sp + ss + "<step>".len()..sp + ss + "<step>".len() + e]
                        .trim()
                        .to_string();
                    if !step.is_empty() {
                        steps.push(step);
                    }
                    sp += ss + "<step>".len() + e + "</step>".len();
                }
                None => break,
            }
        }

        if !name.is_empty() && !steps.is_empty() {
            procedures.push(ProcedureEntry {
                name,
                trigger,
                steps,
            });
        }

        pos = block_end + "</procedure>".len();
    }

    procedures
}

fn extract_attr(s: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = s.find(&pattern)?;
    let val_start = start + pattern.len();
    let end = s[val_start..].find('"')?;
    Some(s[val_start..val_start + end].to_string())
}

// ─── Row Mapper ───────────────────────────────────────────────────────

fn map_memory_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let id_str: String = row.get(0)?;
    let memory_type_str: String = row.get(2)?;
    let tags_json: String = row.get(5)?;
    let source_session_id: Option<String> = row.get(6)?;
    let created_at_str: String = row.get(7)?;
    let updated_at_str: String = row.get(8)?;
    let usage_count: i64 = row.get(9)?;
    let active: i64 = row.get(10)?;
    let concepts_json: String = row.get(11)?;
    let files_json: String = row.get(12)?;
    let strength: f64 = row.get(13)?;
    let importance: i64 = row.get(14)?;
    let version: i64 = row.get(15)?;
    let parent_id: Option<String> = row.get(16)?;
    let supersedes_json: String = row.get(17)?;
    let related_ids_json: String = row.get(18)?;
    let is_latest: i64 = row.get(19)?;

    let parse_uuid = |s: &str| Uuid::parse_str(s).ok();
    let parse_time = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc))
    };

    Ok(MemoryEntry {
        id: parse_uuid(&id_str).unwrap_or(Uuid::nil()),
        workspace_root: row.get(1)?,
        memory_type: MemoryType::parse_str(&memory_type_str).unwrap_or(MemoryType::Fact),
        title: row.get(3)?,
        content: row.get(4)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        source_session_id: source_session_id.and_then(|s| parse_uuid(&s)),
        created_at: parse_time(&created_at_str).unwrap_or_else(Utc::now),
        updated_at: parse_time(&updated_at_str).unwrap_or_else(Utc::now),
        usage_count: usage_count,
        active: active != 0,
        concepts: serde_json::from_str(&concepts_json).unwrap_or_default(),
        files: serde_json::from_str(&files_json).unwrap_or_default(),
        strength,
        importance: importance as u8,
        version: version,
        parent_id: parent_id.and_then(|s| parse_uuid(&s)),
        supersedes: serde_json::from_str(&supersedes_json).unwrap_or_default(),
        related_ids: serde_json::from_str(&related_ids_json).unwrap_or_default(),
        is_latest: is_latest != 0,
    })
}
