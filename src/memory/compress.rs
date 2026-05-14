use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::llm::LlmClient;
use crate::config::ActiveModel;
use crate::session::{Message, MessageRole};

use crate::memory::types::{CompressedObservation, ObservationType, RawObservation};

// ─── LLM Prompts (translated from agentmemory/src/prompts/compression.ts) ──

/// System prompt for observation compression.
pub const COMPRESSION_SYSTEM: &str = "You are a memory compression engine for an AI coding agent. Your job is to extract the essential information from a tool usage observation and compress it into structured data.

Output EXACTLY this XML format with no additional text:

<observation>
  <type>one of: file_read, file_write, file_edit, command_run, search, web_fetch, conversation, error, decision, discovery, subagent, notification, task, other</type>
  <title>Short descriptive title (max 80 chars)</title>
  <subtitle>One-line context (optional)</subtitle>
  <facts>
    <fact>Specific factual detail 1</fact>
    <fact>Specific factual detail 2</fact>
  </facts>
  <narrative>2-3 sentence summary of what happened and why it matters</narrative>
  <concepts>
    <concept>technical concept or pattern</concept>
  </concepts>
  <files>
    <file>path/to/file</file>
  </files>
  <importance>1-10 scale, 10 being critical architectural decision</importance>
</observation>

Rules:
- Be concise but preserve ALL technically relevant details
- File paths must be exact
- Importance: 1-3 for routine reads, 4-6 for edits/commands, 7-9 for architectural decisions, 10 for breaking changes
- Concepts should be reusable search terms (e.g., \"React hooks\", \"SQL migration\", \"auth middleware\")
- Strip any secrets, tokens, or credentials from the output";

/// Build the compression user prompt from a raw observation.
pub fn build_compression_prompt(raw: &RawObservation) -> String {
    let mut parts = Vec::new();

    parts.push(format!("Timestamp: {}", raw.timestamp.to_rfc3339()));
    parts.push(format!("Hook: {}", raw.hook_type.as_str()));

    if let Some(ref name) = raw.tool_name {
        parts.push(format!("Tool: {}", name));
    }
    if let Some(ref input) = raw.tool_input {
        parts.push(format!("Input:\n{}", truncate(input, 4000)));
    }
    if let Some(ref output) = raw.tool_output {
        parts.push(format!("Output:\n{}", truncate(output, 4000)));
    }
    if let Some(ref prompt) = raw.user_prompt {
        parts.push(format!("User prompt:\n{}", truncate(prompt, 2000)));
    }

    parts.join("\n\n")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}\n[...truncated]", &s[..max])
    } else {
        s.to_string()
    }
}

// ─── XML Parsing (translated from agentmemory/src/functions/compress.ts) ──

/// Valid observation types (from agentmemory's VALID_TYPES set).
const VALID_TYPES: &[&str] = &[
    "file_read", "file_write", "file_edit", "command_run",
    "search", "web_fetch", "conversation", "error",
    "decision", "discovery", "subagent", "notification",
    "task", "image", "other",
];

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

    let mut result = Vec::new();
    let mut pos = 0;
    while let Some(cs) = section[pos..].find(&child_start) {
        let content_start = pos + cs + child_start.len();
        if let Some(ce) = section[content_start..].find(&child_end) {
            let value = section[content_start..content_start + ce].trim().to_string();
            if !value.is_empty() {
                result.push(value);
            }
            pos = content_start + ce + child_end.len();
        } else {
            break;
        }
    }

    result
}

/// Parse compressed observation from LLM XML response.
fn parse_compression_xml(xml: &str) -> Result<(ObservationType, String, Option<String>, Vec<String>, String, Vec<String>, Vec<String>, u8)> {
    let raw_type = get_xml_tag(xml, "type")
        .ok_or_else(|| anyhow::anyhow!("missing <type> in compression XML"))?;
    let title = get_xml_tag(xml, "title")
        .ok_or_else(|| anyhow::anyhow!("missing <title> in compression XML"))?;

    let obs_type = if VALID_TYPES.contains(&raw_type.as_str()) {
        ObservationType::parse_str(&raw_type).unwrap_or(ObservationType::Other)
    } else {
        ObservationType::Other
    };

    let subtitle = get_xml_tag(xml, "subtitle");
    let facts = get_xml_children(xml, "facts", "fact");
    let narrative = get_xml_tag(xml, "narrative").unwrap_or_default();
    let concepts = get_xml_children(xml, "concepts", "concept");
    let files = get_xml_children(xml, "files", "file");
    let importance = get_xml_tag(xml, "importance")
        .and_then(|v| v.parse::<u8>().ok())
        .map(|v| v.max(1).min(10))
        .unwrap_or(5);

    Ok((obs_type, title, subtitle, facts, narrative, concepts, files, importance))
}

// ─── Compression Service ──────────────────────────────────────────────

/// Handle LLM compression of observations.
/// Replicates agentmemory's `mem::compress` function.
pub struct CompressionService;

impl CompressionService {
    /// Compress a raw observation using the LLM.
    pub async fn compress(
        db: &Connection,
        llm: &LlmClient,
        model: &ActiveModel,
        observation_id: Uuid,
    ) -> Result<CompressedObservation> {
        // 1. Load raw observation from DB
        let raw = Self::load_raw_observation(db, observation_id)
            .context("failed to load observation for compression")?;

        // 2. Build prompt and call LLM
        let prompt = build_compression_prompt(&raw);
        let messages = vec![
            Message::new(MessageRole::System, COMPRESSION_SYSTEM.to_string()),
            Message::new(MessageRole::User, prompt),
        ];

        let response = llm
            .complete_with_messages(model.clone(), messages, vec![])
            .await
            .context("LLM compression failed")?;

        // 3. Parse XML response
        let (obs_type, title, subtitle, facts, narrative, concepts, files, importance) =
            parse_compression_xml(&response)?;

        // 4. Build compressed observation
        let compressed = CompressedObservation {
            id: Uuid::new_v4(),
            observation_id,
            session_id: raw.session_id,
            obs_type,
            title,
            subtitle,
            facts,
            narrative,
            concepts,
            files,
            importance,
            confidence: None,
            created_at: Utc::now(),
        };

        // 5. Persist to DB
        db.execute(
            "INSERT INTO compressed_observations (id, observation_id, session_id, obs_type, title, subtitle, facts, narrative, concepts, files, importance, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                compressed.id.to_string(),
                compressed.observation_id.to_string(),
                compressed.session_id.to_string(),
                compressed.obs_type.as_str(),
                compressed.title,
                compressed.subtitle,
                serde_json::to_string(&compressed.facts)?,
                compressed.narrative,
                serde_json::to_string(&compressed.concepts)?,
                serde_json::to_string(&compressed.files)?,
                compressed.importance as i64,
                compressed.confidence,
                compressed.created_at.to_rfc3339(),
            ],
        )?;

        // 6. Update FTS index by updating the content table trigger will handle it
        // The FTS5 content-sync trigger automatically updates observations_fts

        Ok(compressed)
    }

    /// Synthetic compression (no LLM fallback) — simplified rule-based version.
    #[allow(dead_code)]
    pub fn compress_synthetic(
        db: &Connection,
        observation_id: Uuid,
    ) -> Result<CompressedObservation> {
        let raw = Self::load_raw_observation(db, observation_id)?;

        let title = raw
            .tool_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let compressed = CompressedObservation {
            id: Uuid::new_v4(),
            observation_id,
            session_id: raw.session_id,
            obs_type: ObservationType::Other,
            title,
            subtitle: None,
            facts: vec![],
            narrative: String::new(),
            concepts: vec![],
            files: vec![],
            importance: 5,
            confidence: None,
            created_at: Utc::now(),
        };

        db.execute(
            "INSERT INTO compressed_observations (...) VALUES (...)",
            rusqlite::params![],
        )?;

        Ok(compressed)
    }

    fn load_raw_observation(db: &Connection, id: Uuid) -> Result<RawObservation> {
        db.query_row(
            "SELECT id, session_id, timestamp, hook_type, tool_name, tool_input, tool_output, user_prompt, assistant_response, modality, image_data
             FROM observations WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| {
                Ok(RawObservation {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(id),
                    session_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(id),
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or(chrono::Utc::now()),
                    hook_type: HookType::parse_str(&row.get::<_, String>(3)?).unwrap_or(HookType::PostToolUse),
                    tool_name: row.get(4)?,
                    tool_input: row.get(5)?,
                    tool_output: row.get(6)?,
                    user_prompt: row.get(7)?,
                    assistant_response: row.get(8)?,
                    modality: Modality::Text,
                    image_data: None,
                })
            },
        ).context("observation not found")
    }
}

use crate::memory::types::{HookType, Modality};
