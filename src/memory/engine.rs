use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use uuid::Uuid;

use crate::config::ActiveModel;
use crate::llm::LlmClient;

use super::consolidate::{ConsolidationReport, ConsolidationService};
use super::evict::{EvictionReport, EvictionService};
use super::remember::RememberService;
use super::retention::RetentionService;
use super::search_index::fts5_search_memories;
use super::sessions::SessionService;
use super::slots::SlotService;
use super::types::*;
use super::{
    lessons::LessonService,
    reflect::{ReflectReport, ReflectService},
};

// ─── MemoryStore ───────────────────────────────────────────────────

/// Main memory store.
pub struct MemoryStore {
    db_path: PathBuf,
    connection: Arc<Mutex<Connection>>,
    read_connection: Mutex<Connection>,
    llm: RwLock<Option<LlmClient>>,
    active_model: RwLock<Option<ActiveModel>>,
    /// Optional override for summarization model (None = use active_model).
    summarization_model: RwLock<Option<ActiveModel>>,
}

impl MemoryStore {
    /// Open or create the memory store.
    pub fn open(db_path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = db_path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "mmap_size", "268435456")?;
        connection.pragma_update(None, "cache_size", "-64000")?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;

        let read_connection = Connection::open(&path)?;
        read_connection.pragma_update(None, "journal_mode", "WAL")?;
        read_connection.pragma_update(None, "mmap_size", "268435456")?;
        read_connection.pragma_update(None, "cache_size", "-64000")?;
        read_connection.pragma_update(None, "temp_store", "MEMORY")?;
        read_connection.busy_timeout(std::time::Duration::from_secs(5))?;

        let store = Self {
            db_path: path,
            connection: Arc::new(Mutex::new(connection)),
            read_connection: Mutex::new(read_connection),
            llm: RwLock::new(None),
            active_model: RwLock::new(None),
            summarization_model: RwLock::new(None),
        };

        store.rebuild_fts5_if_needed()?;

        Ok(store)
    }

    /// Rebuild the memories_fts index on startup so FTS5 queries work.
    fn rebuild_fts5_if_needed(&self) -> Result<()> {
        let db = self.connection.lock().unwrap();
        let exists: bool = db
            .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'")?
            .exists(rusqlite::params![])?;
        if exists {
            db.execute(
                "INSERT INTO memories_fts(memories_fts) VALUES('rebuild')",
                [],
            )?;
        }
        Ok(())
    }

    /// Open connections reusing a shared write connection (provided by
    /// [`Database`](crate::storage::database::Database)).
    /// Only opens a new read connection; the write connection is shared.
    pub(crate) fn open_with_shared_write(
        db_path: impl AsRef<std::path::Path>,
        connection: Arc<Mutex<Connection>>,
    ) -> Result<Self> {
        let path = db_path.as_ref().to_path_buf();

        let read_connection = Connection::open(&path)?;
        read_connection.pragma_update(None, "journal_mode", "WAL")?;
        read_connection.pragma_update(None, "mmap_size", "268435456")?;
        read_connection.pragma_update(None, "cache_size", "-64000")?;
        read_connection.pragma_update(None, "temp_store", "MEMORY")?;
        read_connection.busy_timeout(std::time::Duration::from_secs(5))?;

        let store = Self {
            db_path: path,
            connection,
            read_connection: Mutex::new(read_connection),
            llm: RwLock::new(None),
            active_model: RwLock::new(None),
            summarization_model: RwLock::new(None),
        };

        store.rebuild_fts5_if_needed()?;

        Ok(store)
    }
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("db_path", &self.db_path)
            .field("llm", &self.llm)
            .finish()
    }
}

impl MemoryStore {
    /// Set the LLM client and models for memory operations.
    /// `active` is the session's chat model; `summarization` is an optional
    /// override (None = inherit from active).
    pub fn set_models(
        &self,
        llm: LlmClient,
        active: ActiveModel,
        summarization: Option<ActiveModel>,
    ) {
        *self.llm.write().unwrap() = Some(llm);
        *self.active_model.write().unwrap() = Some(active);
        *self.summarization_model.write().unwrap() = summarization;
    }

    /// Resolve the LLM and model to use for summarization/consolidation/reflection.
    fn resolve_summarization_llm(&self) -> Option<(LlmClient, ActiveModel)> {
        let llm = self.llm.read().unwrap().clone()?;
        let model = self
            .summarization_model
            .read()
            .unwrap()
            .clone()
            .or_else(|| self.active_model.read().unwrap().clone())?;
        Some((llm, model))
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

    /// Search memories using FTS5 + LIKE fallback.
    pub fn search(&self, workspace_root: &str, query: &str) -> Result<Vec<MemoryEntry>> {
        self.search_fts5_fallback(workspace_root, query)
    }

    fn search_fts5_fallback(&self, workspace_root: &str, query: &str) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        if query.trim().is_empty() {
            // Just return latest memories
            return self.get_or_load(workspace_root);
        }
        let fts_results = fts5_search_memories(&db, query, workspace_root, 20).unwrap_or_default();
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
               FROM memories WHERE workspace_root = ?1 AND active = 1 AND is_latest = 1
               AND (title LIKE ?2 OR content LIKE ?3 OR tags LIKE ?4)
               ORDER BY usage_count DESC LIMIT 20",
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
        // Auto-update retention score
        if let Ok(entry) = self.read_by_id(&db, &id, workspace_root) {
            let age_days = (chrono::Utc::now() - entry.created_at).num_days() as f64;
            let _ = RetentionService::compute_and_store(
                &db,
                &id.to_string(),
                "memory",
                entry.importance as f64,
                age_days,
                entry.usage_count + 1,
            );
        }
        Ok(())
    }

    /// Select hot (frequently used) memories.
    pub fn select_hot(
        &self,
        workspace_root: &str,
        limit: usize,
        min_chars: usize,
    ) -> Result<Vec<MemoryEntry>> {
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

    /// Search for context-relevant memories using FTS5 + LIKE, with
    /// compound scoring fallback.
    pub fn search_hot_context(
        &self,
        query: Option<&str>,
        workspace_root: &str,
        limit: usize,
        min_chars: usize,
    ) -> Result<Vec<MemoryEntry>> {
        if let Some(query) = query {
            let q = query.trim();
            if !q.is_empty() {
                // Try search (hybrid → FTS5 → LIKE) with the query
                if let Ok(entries) = self.search(workspace_root, q)
                    && !entries.is_empty() {
                        return Ok(entries);
                    }
            }
        }
        // Fall back to compound sort (importance × frequency × recency)
        self.select_hot(workspace_root, limit, min_chars)
    }

    pub async fn search_hot_context_async(
        &self,
        query: Option<&str>,
        workspace_root: &str,
        limit: usize,
        min_chars: usize,
    ) -> Result<Vec<MemoryEntry>> {
        if let Some(query) = query {
            let q = query.trim();
            if !q.is_empty()
                && let Ok(entries) = self.search_fts5_fallback(workspace_root, q)
                    && !entries.is_empty()
                {
                    return Ok(entries);
                }
        }
        self.select_hot(workspace_root, limit, min_chars)
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

    fn map_session_summary_from_row(row: &rusqlite::Row) -> rusqlite::Result<SessionSummary> {
        Ok(SessionSummary {
            session_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
            project: row.get(1)?,
            created_at: row
                .get::<_, String>(2)
                .ok()
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
            &db,
            workspace_root,
            memory_type,
            title,
            content,
            concepts,
            files,
            tags,
            source_session_id,
        )?;

        // Auto-compute retention score for new memory
        let age_days = (chrono::Utc::now() - entry.created_at).num_days() as f64;
        let _ = RetentionService::compute_and_store(
            &db,
            &entry.id.to_string(),
            "memory",
            entry.importance as f64,
            age_days,
            entry.usage_count,
        );

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
        let (llm, model) = self
            .resolve_summarization_llm()
            .ok_or_else(|| anyhow::anyhow!("LLM client not configured for summarization"))?;

        let db_path = self.db_path.clone();
        SessionService::summarize_session(&db_path, &llm, &model, session_id, project).await
    }

    /// Run the consolidation pipeline (semantic + procedural).
    pub async fn run_consolidation(&self, project: &str) -> Result<ConsolidationReport> {
        let (llm, model) = self
            .resolve_summarization_llm()
            .ok_or_else(|| anyhow::anyhow!("LLM client not configured for consolidation"))?;

        let db_path = self.db_path.clone();
        let project = project.to_string();
        ConsolidationService::run(&db_path, &llm, &model, &project).await
    }

    /// Load consolidated facts for prompt injection.
    pub fn load_consolidated_facts(&self, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        ConsolidationService::load_consolidated_facts(&db, project, limit)
    }

    /// Load consolidated procedures for prompt injection.
    pub fn load_consolidated_procedures(
        &self,
        project: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        ConsolidationService::load_consolidated_procedures(&db, project, limit)
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


    /// Get a key-value from the `meta` table.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let db = self.read_connection.lock().unwrap();
        let mut stmt = db.prepare("SELECT value FROM meta WHERE key = ?1")?;
        let mut rows = stmt.query(rusqlite::params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get::<_, String>(0)?)),
            None => Ok(None),
        }
    }

    /// Set a key-value in the `meta` table.
    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        let db = self.connection.lock().unwrap();
        db.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    // ─── Phase 2: Slots ────────────────────────────────────────────

    /// Ensure default slots exist.
    pub fn ensure_default_slots(&self, project: &str) -> Result<()> {
        let db = self.connection.lock().unwrap();
        SlotService::ensure_defaults(&db, project)
    }

    /// List memory slots.
    pub fn list_slots(
        &self,
        scope: Option<SlotScope>,
        project: Option<&str>,
    ) -> Result<Vec<MemorySlot>> {
        let db = self.read_connection.lock().unwrap();
        SlotService::list(&db, scope, project)
    }

    /// Get a single slot.
    pub fn get_slot(
        &self,
        label: &str,
        scope: SlotScope,
        project: &str,
    ) -> Result<Option<MemorySlot>> {
        let db = self.read_connection.lock().unwrap();
        SlotService::get(&db, label, scope, project)
    }

    /// Set a slot.
    pub fn set_slot(&self, slot: &MemorySlot) -> Result<()> {
        let db = self.connection.lock().unwrap();
        SlotService::set(&db, slot)
    }

    /// Append content to a slot.
    pub fn append_slot(
        &self,
        label: &str,
        scope: SlotScope,
        project: &str,
        content: &str,
    ) -> Result<MemorySlot> {
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
    pub fn compute_retention(
        &self,
        entity_id: &str,
        entity_type: &str,
        importance: f64,
        age_days: f64,
        access_count: i64,
    ) -> Result<RetentionScore> {
        let db = self.connection.lock().unwrap();
        RetentionService::compute_and_store(
            &db,
            entity_id,
            entity_type,
            importance,
            age_days,
            access_count,
        )
    }

    // ─── Phase 2: Eviction ─────────────────────────────────────────

    /// Run eviction rules.
    pub fn run_eviction(&self) -> Result<EvictionReport> {
        let db = self.connection.lock().unwrap();
        EvictionService::run_eviction(&db)
    }

    /// Search the knowledge graph for context related to the query.
    ///
    /// Synchronous, no LLM — safe to call from compose_static_system_prompt.
    pub fn search_graph_context(
        &self,
        query: Option<&str>,
        max_depth: usize,
        max_results: usize,
    ) -> Result<Vec<super::graph_retrieval::GraphEntityPath>> {
        let q = query.unwrap_or("");
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let db_path = self.db_path.clone();
        let db = Connection::open(&db_path).context("failed to open DB for graph search")?;
        super::graph_retrieval::GraphRetrieval::search_related(&db, q, max_depth, max_results)
    }

    // ─── Lessons ─────────────────────────────────────────────────────

    /// Save a lesson.
    pub fn save_lesson(
        &self,
        project: &str,
        content: &str,
        context: &str,
        confidence: f64,
        tags: &[String],
    ) -> Result<MemoryEntry> {
        let db = self.connection.lock().unwrap();
        LessonService::save_lesson(&db, project, content, context, confidence, tags)
    }

    /// Recall lessons by query.
    pub fn recall_lessons(
        &self,
        project: &str,
        query: &str,
        limit: usize,
        min_confidence: f64,
    ) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        LessonService::recall_lessons(&db, project, query, limit, min_confidence)
    }

    /// List recent lessons.
    pub fn list_lessons(&self, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        LessonService::list_lessons(&db, project, limit)
    }

    /// Reinforce a lesson.
    pub fn reinforce_lesson(&self, id: &Uuid) -> Result<()> {
        let db = self.connection.lock().unwrap();
        LessonService::reinforce_lesson(&db, id)
    }

    /// Decay lessons (reduce strength over time).
    pub fn decay_lessons(&self, project: &str) -> Result<usize> {
        let db = self.connection.lock().unwrap();
        LessonService::decay_lessons(&db, project)
    }

    /// Delete a lesson.
    pub fn delete_lesson(&self, id: &Uuid) -> Result<()> {
        let db = self.connection.lock().unwrap();
        LessonService::delete_lesson(&db, id)
    }

    // ─── Reflect: Insight Synthesis ───────────────────────────────────

    /// Run the reflection pipeline (clustering facts + LLM insight synthesis).
    pub async fn run_reflect(&self, project: &str) -> Result<ReflectReport> {
        let (llm, model) = self
            .resolve_summarization_llm()
            .ok_or_else(|| anyhow::anyhow!("LLM client not configured for reflection"))?;

        let db_path = self.db_path.clone();
        let project = project.to_string();
        ReflectService::run(&db_path, &llm, &model, &project).await
    }

    /// Load insights for prompt injection.
    pub fn load_insights(&self, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let db = self.read_connection.lock().unwrap();
        ReflectService::load_insights(&db, project, limit)
    }

    /// Reinforce an insight.
    pub fn reinforce_insight(&self, id: &Uuid) -> Result<()> {
        let db = self.connection.lock().unwrap();
        ReflectService::reinforce_insight(&db, id)
    }
}

impl Clone for MemoryStore {
    fn clone(&self) -> Self {
        Self::open(&self.db_path).expect("failed to clone MemoryStore")
    }
}
