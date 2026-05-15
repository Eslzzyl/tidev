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
use super::slots::SlotService;
use super::retention::RetentionService;
use super::evict::{EvictionService, EvictionReport};
use super::vector_index::VectorIndex;
use super::hybrid_search::HybridSearch;
use super::embed::OpenAIEmbedder;

// ─── MemoryStore ───────────────────────────────────────────────────

/// Main memory store.
pub struct MemoryStore {
    db_path: PathBuf,
    connection: Mutex<Connection>,
    read_connection: Mutex<Connection>,
    dedup: Mutex<DedupMap>,
    bm25: RwLock<Bm25Index>,
    llm: RwLock<Option<LlmClient>>,
    active_model: RwLock<Option<ActiveModel>>,
    vector_index: RwLock<VectorIndex>,
    hybrid_search: RwLock<HybridSearch>,
    embedder: RwLock<Option<OpenAIEmbedder>>,
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

        // Phase 4: Add embedding column for existing databases
        let _ = connection.execute(
            "ALTER TABLE compressed_observations ADD COLUMN embedding BLOB",
            [],
        );

        let store = Self {
            db_path: path,
            connection: Mutex::new(connection),
            read_connection: Mutex::new(read_connection),
            dedup: Mutex::new(DedupMap::new()),
            bm25: RwLock::new(Bm25Index::new()),
            llm: RwLock::new(None),
            active_model: RwLock::new(None),
            vector_index: RwLock::new(VectorIndex::new(1536)),
            hybrid_search: RwLock::new(HybridSearch::new()),
            embedder: RwLock::new(None),
        };

        // Phase 4: Load persisted embeddings into vector index on startup
        store.load_embeddings_from_db();

        Ok(store)
    }
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("db_path", &self.db_path)
            .field("bm25", &self.bm25)
            .field("llm", &self.llm)
            .field("vector_index", &self.vector_index)
            .finish()
    }
}

impl MemoryStore {
    pub fn set_llm(&self, llm: LlmClient, model: ActiveModel) {
        *self.llm.write().unwrap() = Some(llm);
        *self.active_model.write().unwrap() = Some(model);
    }

    /// Set the OpenAI embedder for vector search.
    pub fn set_embedder(&self, embedder: OpenAIEmbedder) {
        let dims = embedder.dimensions();
        *self.embedder.write().unwrap() = Some(embedder);
        // Re-create vector index with correct dimensions, then load persisted
        *self.vector_index.write().unwrap() = VectorIndex::new(dims);
        self.load_embeddings_from_db();
    }

    /// Load persisted embeddings from DB into the in-memory vector index.
    fn load_embeddings_from_db(&self) {
        let db = match self.read_connection.lock() {
            Ok(db) => db,
            Err(_) => return,
        };
        let mut stmt = match db.prepare(
            "SELECT id, session_id, embedding FROM compressed_observations WHERE embedding IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(_) => return, // column may not exist yet
        };
        let rows = match stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let blob: Vec<u8> = row.get(2)?;
            Ok((id, session_id, blob))
        }) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut count = 0;
        let mut vi = self.vector_index.write().unwrap();
        for row in rows.flatten() {
            let (id, session_id, blob) = row;
            // Each f32 is 4 bytes
            if blob.len() % 4 != 0 {
                continue;
            }
            let embedding: Vec<f32> = blob
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            if vi.add(&id, &session_id, embedding).is_ok() {
                count += 1;
            }
        }
        if count > 0 {
            crate::log_info!("loaded {} embeddings into vector index", count);
        }
    }

    /// Clone for sharing — opens a new connection.
    pub fn try_clone(&self) -> Result<Self> {
        Self::open(&self.db_path)
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
        let _ = AuditService::record(&db, "add", "memory", &entry.id.to_string(), None, None, None);
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
        let _ = AuditService::record(&db, "update", "memory", &entry.id.to_string(), None, None, None);
        Ok(())
    }

    /// Soft-delete a memory.
    pub fn delete(&self, workspace_root: &str, id: Uuid) -> Result<()> {
        let db = self.connection.lock().unwrap();
        db.execute(
            "UPDATE memories SET active = 0 WHERE id = ?1 AND workspace_root = ?2",
            rusqlite::params![id.to_string(), workspace_root],
        )?;
        let _ = AuditService::record(&db, "delete", "memory", &id.to_string(), None, None, None);
        Ok(())
    }

    /// Search memories. Uses FTS5 for sync path.
    /// For hybrid search (BM25 + vector), use the async `search_hybrid` method.
    pub fn search(&self, workspace_root: &str, query: &str) -> Result<Vec<MemoryEntry>> {
        // Try hybrid search if we're in a tokio context and embedder is available
        if self.embedder.read().unwrap().is_some() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let future = self.search_hybrid(query, workspace_root, 20);
                if let Ok(results) = handle.block_on(future) {
                    if !results.is_empty() {
                        return Ok(results);
                    }
                }
            }
        }
        // Fall back to FTS5
        let db = self.read_connection.lock().unwrap();
        if query.trim().is_empty() {
            // Just return latest memories
            return self.get_or_load(workspace_root);
        }
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
        // Final fallback: LIKE search
        let pattern = format!("%{}%", query);
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
             FROM memories WHERE workspace_root = ?1 AND active = 1
             AND (title LIKE ?2 OR content LIKE ?3 OR tags LIKE ?4)
             ORDER BY usage_count DESC LIMIT 20",
        )?;
        let entries = stmt.query_map(
            rusqlite::params![workspace_root, pattern, pattern, pattern],
            |row| super::remember::map_memory_entry_from_row(row),
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
             ORDER BY
                 importance * 0.5 +
                 LEAST(usage_count / 20.0, 1.0) * 0.3 +
                 CASE WHEN updated_at >= datetime('now', '-7 days') THEN 0.2 ELSE 0.0 END
             DESC
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

    // ─── Phase 1: Context Injection Helpers ──────────────────────────

    /// Load recent compressed observations for a session, ordered by
    /// descending importance then recency.
    pub fn load_recent_compressed_observations(
        &self,
        session_id: &Uuid,
        limit: usize,
        min_importance: u8,
    ) -> Result<Vec<CompressedObservation>> {
        let db = self.read_connection.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, observation_id, session_id, obs_type, title, subtitle,
                    facts, narrative, concepts, files, importance, confidence, created_at
             FROM compressed_observations
             WHERE session_id = ?1 AND importance >= ?2
             ORDER BY importance DESC, created_at DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![session_id.to_string(), min_importance as i64, limit as i64],
            Self::map_compressed_observation_from_row,
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load session summaries from sessions other than the current one,
    /// newest first.
    pub fn load_other_session_summaries(
        &self,
        exclude_session_id: &Uuid,
        limit: usize,
    ) -> Result<Vec<SessionSummary>> {
        let db = self.read_connection.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT session_id, project, created_at, title, narrative,
                    key_decisions, files_modified, concepts, observation_count
             FROM session_summaries
             WHERE session_id != ?1 AND title IS NOT NULL
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![exclude_session_id.to_string(), limit as i64],
            Self::map_session_summary_from_row,
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    fn map_compressed_observation_from_row(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<CompressedObservation> {
        Ok(CompressedObservation {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
            observation_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(Uuid::nil()),
            session_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or(Uuid::nil()),
            obs_type: ObservationType::parse_str(&row.get::<_, String>(3)?)
                .unwrap_or(ObservationType::Other),
            title: row.get(4)?,
            subtitle: row.get(5)?,
            facts: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
            narrative: row.get(7)?,
            concepts: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
            files: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
            importance: row.get::<_, i64>(10)? as u8,
            confidence: row.get(11)?,
            created_at: row.get::<_, String>(12).ok()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
        })
    }

    fn map_session_summary_from_row(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<SessionSummary> {
        Ok(SessionSummary {
            session_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
            project: row.get(1)?,
            created_at: row.get::<_, String>(2).ok()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
            title: row.get(3)?,
            narrative: row.get(4)?,
            key_decisions: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            files_modified: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
            concepts: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
            observation_count: row.get(8)?,
        })
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

    /// Run compression on an observation. Uses LLM when available, falls
    /// back to synthetic (rule-based) compression otherwise.
    pub async fn compress(&self, observation_id: Uuid) -> Result<CompressedObservation> {
        let db_path = self.db_path.clone();

        // Try LLM compression if configured
        let llm_available = {
            let llm = self.llm.read().unwrap().is_some();
            let model = self.active_model.read().unwrap().is_some();
            llm && model
        };

        let compressed = if llm_available {
            let (llm, model) = {
                let llm = self.llm.read().unwrap().clone();
                let model = self.active_model.read().unwrap().clone();
                (llm, model)
            };
            let conn = Connection::open(&db_path)?;
            match CompressionService::compress(&conn, &llm.unwrap(), &model.unwrap(), observation_id).await {
                Ok(c) => c,
                Err(e) => {
                    crate::log_warn!("LLM compression failed, falling back to synthetic: {}", e);
                    let conn = Connection::open(&db_path)?;
                    CompressionService::compress_synthetic(&conn, observation_id)?
                }
            }
        } else {
            let conn = Connection::open(&db_path)?;
            CompressionService::compress_synthetic(&conn, observation_id)?
        };

        // Update indexes (in-memory BM25 + vector)
        self.bm25.write().unwrap().add(
            &compressed.id.to_string(),
            &compressed.to_search_text(),
            &compressed.session_id.to_string(),
        );

        // Add to vector index (async, best-effort) + persist to DB
        if let Some(ref embedder) = *self.embedder.read().unwrap() {
            let id = compressed.id.to_string();
            let session_id = compressed.session_id.to_string();
            let search_text = compressed.to_search_text();
            match embedder.embed(&search_text).await {
                Ok(embedding) => {
                    // 1. Add to in-memory index
                    if let Err(e) = self.vector_index.write().unwrap().add(&id, &session_id, embedding.clone()) {
                        crate::log_warn!("vector index add failed: {}", e);
                    }
                    // 2. Persist to DB
                    let blob: Vec<u8> = embedding.iter()
                        .flat_map(|f| f.to_le_bytes())
                        .collect();
                    if let Ok(db) = self.connection.lock() {
                        let _ = db.execute(
                            "UPDATE compressed_observations SET embedding = ?1 WHERE id = ?2",
                            rusqlite::params![blob, &id],
                        );
                    }
                }
                Err(e) => crate::log_warn!("embedding failed: {}", e),
            }
        }

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
        let (llm, model) = {
            let llm = self.llm.read().unwrap().clone();
            let model = self.active_model.read().unwrap().clone();
            (llm, model)
        };
        let llm = llm.ok_or_else(|| anyhow::anyhow!("LLM client not configured for summarization"))?;
        let model = model.ok_or_else(|| anyhow::anyhow!("Active model not configured for summarization"))?;

        let db_path = self.db_path.clone();
        SessionService::summarize_session(&db_path, &llm, &model, session_id, project).await
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

    // ─── Phase 2: Slots ────────────────────────────────────────────

    /// Ensure default slots exist.
    pub fn ensure_default_slots(&self, project: &str) -> Result<()> {
        let db = self.connection.lock().unwrap();
        SlotService::ensure_defaults(&db, project)
    }

    /// List memory slots.
    pub fn list_slots(&self, scope: Option<SlotScope>, project: Option<&str>) -> Result<Vec<MemorySlot>> {
        let db = self.read_connection.lock().unwrap();
        SlotService::list(&db, scope, project)
    }

    /// Get a single slot.
    pub fn get_slot(&self, label: &str, scope: SlotScope, project: &str) -> Result<Option<MemorySlot>> {
        let db = self.read_connection.lock().unwrap();
        SlotService::get(&db, label, scope, project)
    }

    /// Set a slot.
    pub fn set_slot(&self, slot: &MemorySlot) -> Result<()> {
        let db = self.connection.lock().unwrap();
        SlotService::set(&db, slot)
    }

    /// Append content to a slot.
    pub fn append_slot(&self, label: &str, scope: SlotScope, project: &str, content: &str) -> Result<MemorySlot> {
        let db = self.connection.lock().unwrap();
        SlotService::append(&db, label, scope, project, content)
    }

    /// Delete a slot.
    pub fn delete_slot(&self, label: &str, scope: SlotScope, project: &str) -> Result<()> {
        let db = self.connection.lock().unwrap();
        SlotService::delete(&db, label, scope, project)
    }

    /// Render pinned slots for prompt injection.
    pub fn render_pinned_slots(&self, project: &str) -> Result<String> {
        let db = self.read_connection.lock().unwrap();
        SlotService::render_pinned(&db, project)
    }

    // ─── Phase 2: Retention ────────────────────────────────────────

    /// Compute and store retention score for a memory.
    pub fn compute_retention(&self, entity_id: &str, entity_type: &str, importance: f64, age_days: f64, access_count: i64) -> Result<RetentionScore> {
        let db = self.connection.lock().unwrap();
        RetentionService::compute_and_store(&db, entity_id, entity_type, importance, age_days, access_count)
    }

    // ─── Phase 2: Eviction ─────────────────────────────────────────

    /// Run eviction rules.
    pub fn run_eviction(&self) -> Result<EvictionReport> {
        let db = self.connection.lock().unwrap();
        EvictionService::run_eviction(&db)
    }

    // ─── Phase 2: Vector + Hybrid Search ───────────────────────────

    /// Get a reference to the vector index.
    pub fn vector_index(&self) -> &RwLock<VectorIndex> {
        &self.vector_index
    }

    /// Run hybrid search (BM25 + vector). Falls back to FTS5 if no embedder.
    pub async fn search_hybrid(&self, query: &str, workspace_root: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        // If embedder is available, try hybrid search
        if let Some(embedder) = self.embedder.read().unwrap().as_ref() {
            let query_embedding = embedder.embed(query).await;
            match query_embedding {
                Ok(emb) => {
                    let hs = self.hybrid_search.read().unwrap();
                    let bm25 = self.bm25.read().unwrap();
                    let vector = self.vector_index.read().unwrap();
                    let results = hs.search(query, limit, &bm25, &vector, Some(&emb));

                    if !results.is_empty() {
                        let db = self.read_connection.lock().unwrap();
                        let ids: Vec<String> = results.iter().map(|r| r.id.clone()).collect();
                        let mut entries = Vec::new();
                        for id in &ids {
                            // Try to look up as a memory
                            let _uuid = Uuid::parse_str(id).unwrap_or(Uuid::nil());
                            if let Ok(entry) = db.query_row(
                                "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
                                 FROM memories WHERE id = ?1 AND workspace_root = ?2 AND active = 1",
                                rusqlite::params![id, workspace_root],
                                |row| super::remember::map_memory_entry_from_row(row),
                            ) {
                                entries.push(entry);
                            }
                        }
                        if !entries.is_empty() {
                            return Ok(entries);
                        }
                    }
                }
                Err(e) => crate::log_warn!("hybrid search embedding failed: {}", e),
            }
        }

        // Fall back to FTS5
        self.search(workspace_root, query)
    }
}

impl Clone for MemoryStore {
    fn clone(&self) -> Self {
        Self::open(&self.db_path).expect("failed to clone MemoryStore")
    }
}
