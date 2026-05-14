use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{
    Mutex,
    RwLock,
};
use uuid::Uuid;

use crate::llm::LlmClient;
use crate::config::ActiveModel;

use super::types::*;
use super::dedup::DedupMap;
use super::search_index::{Bm25Index, fts5_search_memories};
use super::observe::ObservationService;
use super::compress::CompressionService;
use super::remember::RememberService;
use super::sessions::SessionService;
use super::audit::{AuditService, AuditQuery};

// ─── MemoryStore ───────────────────────────────────────────────────

/// Main memory store — replaces the old `MemoryStore` with complete
/// agentmemory-style functionality. All old public methods preserved.
#[derive(Debug)]
pub struct MemoryStore {
    db_path: PathBuf,
    connection: Mutex<Connection>,
    read_connection: Mutex<Connection>,
    dedup: Mutex<DedupMap>,
    bm25: RwLock<Bm25Index>,
    llm: RwLock<Option<LlmClient>>,
    active_model: RwLock<Option<ActiveModel>>,
}

impl MemoryStore {
    /// Open or create the memory store.
    pub fn open(db_path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = db_path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;

        let read_connection = Connection::open(&path)?;
        read_connection.pragma_update(None, "foreign_keys", "ON")?;
        read_connection.pragma_update(None, "journal_mode", "WAL")?;
        read_connection.busy_timeout(std::time::Duration::from_secs(5))?;

        Ok(Self {
            db_path: path,
            connection: Mutex::new(connection),
            read_connection: Mutex::new(read_connection),
            dedup: Mutex::new(DedupMap::new()),
            bm25: RwLock::new(Bm25Index::new()),
            llm: RwLock::new(None),
            active_model: RwLock::new(None),
        })
    }

    /// Clone for sharing — opens a new connection.
    pub fn try_clone(&self) -> Result<Self> {
        Self::open(&self.db_path)
    }

    /// Set the LLM client for compression and summarization.
    pub fn set_llm(&self, llm: LlmClient, model: ActiveModel) {
        *self.llm.write().unwrap() = Some(llm);
        *self.active_model.write().unwrap() = Some(model);
    }

    // ─── Backward-compatible Old API ────────────────────────────────

    /// Store a new memory.
    pub fn add(&self, entry: &MemoryEntry) -> Result<()> {
        let db = self.connection.lock().unwrap();
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
        Ok(())
    }

    /// Update an existing memory.
    pub fn update(&self, entry: &MemoryEntry) -> Result<()> {
        let db = self.connection.lock().unwrap();
        db.execute(
            "UPDATE memories SET title=?1, content=?2, tags=?3, memory_type=?4, updated_at=?5, concepts=?6, files=?7, strength=?8, importance=?9, version=?10, is_latest=?11 WHERE id=?12",
            rusqlite::params![
                entry.title,
                entry.content,
                serde_json::to_string(&entry.tags)?,
                entry.memory_type.as_str(),
                Utc::now().to_rfc3339(),
                serde_json::to_string(&entry.concepts)?,
                serde_json::to_string(&entry.files)?,
                entry.strength,
                entry.importance as i64,
                entry.version,
                entry.is_latest as i64,
                entry.id.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Soft-delete a memory.
    pub fn delete(&self, workspace_root: &str, id: Uuid) -> Result<()> {
        let db = self.connection.lock().unwrap();
        db.execute(
            "UPDATE memories SET active = 0 WHERE id = ?1 AND workspace_root = ?2",
            rusqlite::params![id.to_string(), workspace_root],
        )?;
        Ok(())
    }

    /// Search memories by keyword.
    pub fn search(&self, workspace_root: &str, query: &str) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();

        if query.trim().is_empty() {
            return self.get_or_load(workspace_root);
        }

        // Try FTS5 search first
        let fts_results = fts5_search_memories(&db, query, workspace_root, 20)
            .unwrap_or_default();

        if !fts_results.is_empty() {
            let mut entries = Vec::new();
            for (id, _title, _score) in &fts_results {
                if let Ok(entry) = self.read_by_id(&db, id, workspace_root) {
                    entries.push(entry);
                }
            }
            if !entries.is_empty() {
                return Ok(entries);
            }
        }

        // Fallback: LIKE search
        let pattern = format!("%{}%", query);
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1 AND active = 1
             AND (title LIKE ?2 OR content LIKE ?3 OR tags LIKE ?4)
             ORDER BY usage_count DESC
             LIMIT 20",
        )?;

        let entries = stmt.query_map(
            rusqlite::params![workspace_root, pattern, pattern, pattern],
            super::remember::map_memory_entry_from_row,
        )?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry?);
        }
        Ok(result)
    }

    /// Get all active memories for a workspace.
    pub fn get_or_load(&self, workspace_root: &str) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        RememberService::load_latest_memories(&db, workspace_root)
    }

    /// Read a specific memory.
    pub fn read(&self, workspace_root: &str, id: Uuid) -> Result<MemoryEntry> {
        let db = self.read_connection.lock().unwrap();
        self.read_by_id(&db, &id, workspace_root)
    }

    /// Record a usage event for a memory.
    pub fn record_usage(&self, workspace_root: &str, id: Uuid) -> Result<()> {
        let db = self.connection.lock().unwrap();
        db.execute(
            "UPDATE memories SET usage_count = usage_count + 1 WHERE id = ?1 AND workspace_root = ?2",
            rusqlite::params![id.to_string(), workspace_root],
        )?;
        Ok(())
    }

    /// Select hot (frequently used) memories.
    pub fn select_hot(&self, workspace_root: &str, limit: usize, min_chars: usize) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1 AND active = 1 AND is_latest = 1 AND LENGTH(content) >= ?2
             ORDER BY usage_count DESC
             LIMIT ?3",
        )?;

        let entries = stmt.query_map(
            rusqlite::params![workspace_root, min_chars as i64, limit as i64],
            super::remember::map_memory_entry_from_row,
        )?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry?);
        }
        Ok(result)
    }

    /// Format memories for inclusion in the system prompt.
    pub fn format_for_prompt(entries: &[MemoryEntry]) -> String {
        if entries.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();
        parts.push("## Workspace Memories\n".to_string());

        for entry in entries {
            let tags_str = if entry.tags.is_empty() {
                String::new()
            } else {
                format!(" [tags: {}]", entry.tags.join(", "))
            };
            parts.push(format!(
                "- **[{}]** {}: {}{}",
                entry.memory_type.short_label(),
                entry.title,
                entry.content,
                tags_str,
            ));
        }

        parts.join("\n")
    }

    // ─── New AgentMemory-Style API ──────────────────────────────────

    /// Observe a tool hook and create a raw observation.
    /// Returns the observation ID if new, or None if deduplicated.
    pub fn observe(&self, payload: &HookPayload) -> Result<Option<Uuid>> {
        let db = self.connection.lock().unwrap();
        let mut dedup = self.dedup.lock().unwrap();
        match ObservationService::observe(&db, &mut dedup, payload)? {
            ObservationResult::New(id) => {
                // Schedule async compression
                self.schedule_compression(id);
                // Also add to BM25 index
                if let Ok(raw) = Self::load_raw_observation(&db, id) {
                    let search_text = format!(
                        "{} {} {} {}",
                        raw.tool_name.unwrap_or_default(),
                        raw.tool_input.unwrap_or_default(),
                        raw.tool_output.unwrap_or_default(),
                        raw.user_prompt.unwrap_or_default(),
                    );
                    self.bm25.write().unwrap().add(
                        &id.to_string(),
                        &search_text,
                        &raw.session_id.to_string(),
                    );
                }
                Ok(Some(id))
            }
            ObservationResult::Deduplicated => Ok(None),
        }
    }

    /// Run LLM compression on an observation (async).
    pub async fn compress(&self, observation_id: Uuid) -> Result<CompressedObservation> {
        let llm_guard = self.llm.read().unwrap();
        let llm = llm_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("LLM client not configured for compression"))?;
        let model = self.active_model.read().unwrap().clone()
            .ok_or_else(|| anyhow::anyhow!("Active model not configured for compression"))?;
        let db = self.connection.lock().unwrap();
        let compressed = CompressionService::compress(&db, llm, &model, observation_id).await?;

        // Add to BM25 index
        self.bm25.write().unwrap().add(
            &compressed.id.to_string(),
            &compressed.to_search_text(),
            &compressed.session_id.to_string(),
        );

        Ok(compressed)
    }

    /// Remember with Jaccard dedup (new API).
    pub fn remember(
        &self,
        workspace_root: &str,
        memory_type: MemoryType,
        title: &str,
        content: &str,
        concepts: &[String],
        files: &[String],
        tags: &[String],
        source_session_id: Option<Uuid>,
    ) -> Result<MemoryEntry> {
        let db = self.connection.lock().unwrap();
        let entry = RememberService::remember(
            &db, workspace_root, memory_type, title, content,
            concepts, files, tags, source_session_id,
        )?;

        // Audit log
        AuditService::record(
            &db,
            "remember",
            "memory",
            &entry.id.to_string(),
            None, None, source_session_id,
        )?;

        Ok(entry)
    }

    /// Get version chain for a memory.
    pub fn get_version_chain(&self, id: &Uuid) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        RememberService::get_version_chain(&db, id)
    }

    /// Generate session summary.
    pub async fn summarize_session(
        &self,
        session_id: Uuid,
        project: &str,
    ) -> Result<SessionSummary> {
        let llm_guard = self.llm.read().unwrap();
        let llm = llm_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("LLM client not configured for summarization"))?;
        let model = self.active_model.read().unwrap().clone()
            .ok_or_else(|| anyhow::anyhow!("Active model not configured for summarization"))?;
        let db = self.connection.lock().unwrap();
        SessionService::summarize_session(&db, llm, &model, session_id, project).await
    }

    /// Query audit log.
    pub fn audit_query(&self, query: &AuditQuery) -> Result<Vec<AuditEntry>> {
        let db = self.read_connection.lock().unwrap();
        AuditService::query(&db, query)
    }

    // ─── Internal Helpers ───────────────────────────────────────────

    fn read_by_id(&self, db: &Connection, id: &Uuid, workspace_root: &str) -> Result<MemoryEntry> {
        db.query_row(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
             FROM memories WHERE id = ?1 AND workspace_root = ?2",
            rusqlite::params![id.to_string(), workspace_root],
            super::remember::map_memory_entry_from_row,
        ).context("memory not found")
    }

    fn load_raw_observation(db: &Connection, id: Uuid) -> Result<RawObservation> {
        use super::types::HookType;
        db.query_row(
            "SELECT id, session_id, timestamp, hook_type, tool_name, tool_input, tool_output, user_prompt, assistant_response, modality, image_data
             FROM observations WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| {
                Ok(RawObservation {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(id),
                    session_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(id),
                    timestamp: row.get::<_, String>(2).ok()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now),
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

    fn schedule_compression(&self, _obs_id: Uuid) {
        // In Phase 1, this is a placeholder.
        // The hook system will call compress() explicitly from the runtime.
        // In a full implementation, this would spawn a tokio task with 500ms delay.
    }
}

impl Clone for MemoryStore {
    fn clone(&self) -> Self {
        Self::open(&self.db_path).expect("failed to clone MemoryStore")
    }
}
