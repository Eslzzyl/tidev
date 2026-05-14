use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::collections::HashSet;
use uuid::Uuid;

use crate::memory::types::{MemoryEntry, MemoryType};

/// Jaccard similarity between two strings (from agentmemory's `jaccardSimilarity`).
/// Filters out words shorter than 3 characters.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .collect();
    let set_b: HashSet<&str> = b
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }

    let intersection = set_a.intersection(&set_b).count();
    intersection as f64 / (set_a.len() + set_b.len() - intersection) as f64
}

/// Remember a memory with Jaccard dedup and versioning.
/// Replicates agentmemory's `mem::remember` function.
pub struct RememberService;

impl RememberService {
    /// Save a new memory. If a similar memory (>0.7 Jaccard) exists,
    /// the new one supersedes it (version chain).
    pub fn remember(
        db: &Connection,
        workspace_root: &str,
        memory_type: MemoryType,
        title: &str,
        content: &str,
        concepts: &[String],
        files: &[String],
        tags: &[String],
        source_session_id: Option<Uuid>,
    ) -> Result<MemoryEntry> {
        // 1. Load existing active memories for Jaccard dedup
        let existing = Self::load_active_memories(db, workspace_root)?;

        // 2. Find superseded memory by Jaccard similarity > 0.7
        let mut supersedes: Vec<Uuid> = Vec::new();
        let mut parent_id: Option<Uuid> = None;
        let mut version: i64 = 1;
        let lower_content = content.to_lowercase();

        for mem in &existing {
            let sim = jaccard_similarity(&lower_content, &mem.content.to_lowercase());
            if sim > 0.7 {
                supersedes.push(mem.id);
                parent_id = Some(mem.id);
                version = mem.version + 1;
                // Mark old version as not latest
                Self::mark_not_latest(db, &mem.id)?;
            }
        }

        // 3. Create new entry
        let now = Utc::now();
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            workspace_root: workspace_root.to_string(),
            memory_type,
            title: title.to_string(),
            content: content.to_string(),
            tags: tags.to_vec(),
            source_session_id,
            created_at: now,
            updated_at: now,
            usage_count: 0,
            active: true,
            concepts: concepts.to_vec(),
            files: files.to_vec(),
            strength: 0.0,
            importance: 5,
            version,
            parent_id,
            supersedes,
            related_ids: vec![],
            is_latest: true,
        };

        // 4. Persist to DB
        db.execute(
            "INSERT INTO memories (id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                entry.id.to_string(),
                entry.workspace_root,
                entry.memory_type.as_str(),
                entry.title,
                entry.content,
                serde_json::to_string(&entry.tags)?,
                entry.source_session_id.map(|id| id.to_string()),
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.usage_count,
                entry.active as i64,
                serde_json::to_string(&entry.concepts)?,
                serde_json::to_string(&entry.files)?,
                entry.strength,
                entry.importance as i64,
                entry.version,
                entry.parent_id.map(|id| id.to_string()),
                serde_json::to_string(&entry.supersedes.iter().map(|id| id.to_string()).collect::<Vec<_>>())?,
                serde_json::to_string(&entry.related_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())?,
                entry.is_latest as i64,
            ],
        )?;

        Ok(entry)
    }

    /// Mark a memory as not latest (superseded by a new version).
    pub fn mark_not_latest(db: &Connection, id: &Uuid) -> Result<()> {
        db.execute(
            "UPDATE memories SET is_latest = 0 WHERE id = ?1",
            rusqlite::params![id.to_string()],
        )?;
        Ok(())
    }

    /// Load all active memories for a workspace.
    pub fn load_active_memories(db: &Connection, workspace_root: &str) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1 AND active = 1
             ORDER BY created_at DESC",
        )?;

        let _uuid_nil = Uuid::nil();
        let entries = stmt.query_map(rusqlite::params![workspace_root], |row| {
            map_memory_entry_from_row(row)
        })?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry?);
        }
        Ok(result)
    }

    /// Load all active latest memories for a workspace.
    pub fn load_latest_memories(db: &Connection, workspace_root: &str) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1 AND active = 1 AND is_latest = 1
             ORDER BY usage_count DESC, updated_at DESC",
        )?;

        let entries = stmt.query_map(rusqlite::params![workspace_root], |row| {
            map_memory_entry_from_row(row)
        })?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry?);
        }
        Ok(result)
    }

    /// Get version chain for a memory.
    pub fn get_version_chain(db: &Connection, id: &Uuid) -> Result<Vec<MemoryEntry>> {
        // Walk the parent chain
        let mut chain = Vec::new();
        let mut current_id = *id;
        loop {
            let entry = db.query_row(
                "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
                 FROM memories WHERE id = ?1",
                rusqlite::params![current_id.to_string()],
                map_memory_entry_from_row,
            )?;
            chain.push(entry);

            match chain.last().unwrap().parent_id {
                Some(pid) => current_id = pid,
                None => break,
            }
        }
        Ok(chain)
    }

    /// Delete (soft-delete) a memory.
    pub fn delete(db: &Connection, id: &Uuid) -> Result<()> {
        db.execute(
            "UPDATE memories SET active = 0 WHERE id = ?1",
            rusqlite::params![id.to_string()],
        )?;
        Ok(())
    }
}

/// Row mapper for MemoryEntry.
pub fn map_memory_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
    let uuid_nil = Uuid::nil();
    let tags_json: String = row.get(5)?;
    let concepts_json: String = row.get(11)?;
    let files_json: String = row.get(12)?;
    let supersedes_json: String = row.get(17)?;
    let related_ids_json: String = row.get(18)?;

    let parse_ids = |s: &str| -> Vec<Uuid> {
        serde_json::from_str::<Vec<String>>(s)
            .unwrap_or_default()
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect()
    };

    Ok(MemoryEntry {
        id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(uuid_nil),
        workspace_root: row.get(1)?,
        memory_type: MemoryType::parse_str(&row.get::<_, String>(2)?).unwrap_or(MemoryType::Fact),
        title: row.get(3)?,
        content: row.get(4)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        source_session_id: row.get::<_, Option<String>>(6)?.and_then(|s| Uuid::parse_str(&s).ok()),
        created_at: row.get::<_, String>(7).ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
        updated_at: row.get::<_, String>(8).ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
        usage_count: row.get(9)?,
        active: row.get::<_, i64>(10)? != 0,
        concepts: serde_json::from_str(&concepts_json).unwrap_or_default(),
        files: serde_json::from_str(&files_json).unwrap_or_default(),
        strength: row.get(13)?,
        importance: row.get::<_, i64>(14)? as u8,
        version: row.get(15)?,
        parent_id: row.get::<_, Option<String>>(16)?.and_then(|s| Uuid::parse_str(&s).ok()),
        supersedes: parse_ids(&supersedes_json),
        related_ids: parse_ids(&related_ids_json),
        is_latest: row.get::<_, i64>(19)? != 0,
    })
}
