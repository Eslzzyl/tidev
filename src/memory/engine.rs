use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
use uuid::Uuid;

use crate::config::{ActiveModel, EmbeddingActiveModel};
use crate::llm::LlmClient;

use super::compress::CompressionService;
use super::compression_queue::QueueTask;
use super::consolidate::{ConsolidationReport, ConsolidationService};
use super::dedup::DedupMap;
use super::evict::{EvictionReport, EvictionService};
use super::hybrid_search::HybridSearch;
use super::observe::ObservationService;
use super::remember::RememberService;
use super::retention::RetentionService;
use super::search_index::{Bm25Index, fts5_search_memories};
use super::sessions::SessionService;
use super::slots::SlotService;
use super::types::*;
use super::{
    lessons::LessonService,
    reflect::{ReflectReport, ReflectService},
};

// ─── Compression circuit breaker ────────────────────────────────────

/// Consecutive LLM compression failures before tripping the circuit breaker.
const COMPRESSION_CB_THRESHOLD: u32 = 3;

/// How long to pause LLM compression after tripping (seconds).
const COMPRESSION_CB_COOLDOWN_SECS: u64 = 300; // 5 minutes

// ─── MemoryStore ───────────────────────────────────────────────────

/// Main memory store.
pub struct MemoryStore {
    db_path: PathBuf,
    connection: Arc<Mutex<Connection>>,
    read_connection: Mutex<Connection>,
    dedup: Mutex<DedupMap>,
    bm25: RwLock<Bm25Index>,
    llm: RwLock<Option<LlmClient>>,
    active_model: RwLock<Option<ActiveModel>>,
    /// Optional override for compression model (None = use active_model).
    compression_model: RwLock<Option<ActiveModel>>,
    /// Optional override for summarization model (None = use compression_model, then active_model).
    summarization_model: RwLock<Option<ActiveModel>>,
    /// Configured embedding model for vector search.
    embedding_model: RwLock<Option<EmbeddingActiveModel>>,
    hybrid_search: RwLock<HybridSearch>,
    /// Whether automatic compression is enabled.
    compression_enabled: AtomicBool,
    /// Whether to use LLM for compression (default false = synthetic only).
    llm_compression: AtomicBool,
    /// Circuit breaker: consecutive LLM compression failures.
    compression_cb_failures: AtomicU32,
    /// When the circuit breaker was tripped (None = not tripped).
    compression_cb_tripped_at: RwLock<Option<std::time::Instant>>,
    /// Sender to enqueue observations for async compression.
    /// Set after `CompressionQueue::start()`; shared across all Arcs.
    compression_sender: RwLock<Option<mpsc::SyncSender<QueueTask>>>,
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
            dedup: Mutex::new(DedupMap::new()),
            bm25: RwLock::new(Bm25Index::new()),
            llm: RwLock::new(None),
            active_model: RwLock::new(None),
            compression_model: RwLock::new(None),
            summarization_model: RwLock::new(None),
            embedding_model: RwLock::new(None),
            hybrid_search: RwLock::new(HybridSearch::new()),
            compression_enabled: AtomicBool::new(true),
            llm_compression: AtomicBool::new(false),
            compression_cb_failures: AtomicU32::new(0),
            compression_cb_tripped_at: RwLock::new(None),
            compression_sender: RwLock::new(None),
        };

        Ok(store)
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
            dedup: Mutex::new(DedupMap::new()),
            bm25: RwLock::new(Bm25Index::new()),
            llm: RwLock::new(None),
            active_model: RwLock::new(None),
            compression_model: RwLock::new(None),
            summarization_model: RwLock::new(None),
            embedding_model: RwLock::new(None),
            hybrid_search: RwLock::new(HybridSearch::new()),
            compression_enabled: AtomicBool::new(true),
            llm_compression: AtomicBool::new(false),
            compression_cb_failures: AtomicU32::new(0),
            compression_cb_tripped_at: RwLock::new(None),
            compression_sender: RwLock::new(None),
        };

        Ok(store)
    }
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("db_path", &self.db_path)
            .field("bm25", &self.bm25)
            .field("llm", &self.llm)
            .finish()
    }
}

impl MemoryStore {
    /// Set the LLM client and models for memory operations.
    /// `active` is the session's chat model; `compression` and `summarization` are optional
    /// overrides (None = inherit from active). Call `set_embedding_model()` separately.
    pub fn set_models(
        &self,
        llm: LlmClient,
        active: ActiveModel,
        compression: Option<ActiveModel>,
        summarization: Option<ActiveModel>,
    ) {
        *self.llm.write().unwrap() = Some(llm);
        *self.active_model.write().unwrap() = Some(active);
        *self.compression_model.write().unwrap() = compression;
        *self.summarization_model.write().unwrap() = summarization;
    }

    /// Enable or disable automatic compression.
    pub fn set_compression_enabled(&self, enabled: bool) {
        self.compression_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Enable or disable LLM-based compression (default: false = synthetic only).
    pub fn set_llm_compression(&self, enabled: bool) {
        self.llm_compression
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set the compression queue sender. Observations will be enqueued
    /// for async compression instead of being scheduled inline.
    pub fn set_compression_sender(&self, sender: mpsc::SyncSender<QueueTask>) {
        *self.compression_sender.write().unwrap() = Some(sender);
    }

    /// Check whether the compression circuit breaker is tripped.
    /// When tripped and still within the cooldown window, LLM compression
    /// is skipped in favour of synthetic compression.
    ///
    /// Also checks the atomic failure counter proactively: if the counter
    /// has already reached [`COMPRESSION_CB_THRESHOLD`] but no worker has
    /// written `tripped_at` yet (a race window between `fetch_add` and
    /// the `write`), this method returns `true` to prevent extra LLM calls.
    fn is_compression_circuit_tripped(&self) -> bool {
        let tripped_at = self.compression_cb_tripped_at.read().unwrap();
        match *tripped_at {
            Some(when) => {
                let elapsed = when.elapsed().as_secs();
                if elapsed >= COMPRESSION_CB_COOLDOWN_SECS {
                    // Cooldown expired — auto-reset
                    drop(tripped_at);
                    *self.compression_cb_tripped_at.write().unwrap() = None;
                    self.compression_cb_failures.store(0, Ordering::Relaxed);
                    crate::log_info!("compression circuit breaker auto-reset after {}s", elapsed);
                    false
                } else {
                    true
                }
            }
            None => {
                // No explicit trip yet, but the atomic counter may already
                // be at the threshold from a concurrent worker that
                // incremented it but hasn't written tripped_at → treat as
                // tripped to prevent extra LLM calls.
                self.compression_cb_failures.load(Ordering::Relaxed)
                    >= COMPRESSION_CB_THRESHOLD
            }
        }
    }

    /// Set the embedding model for vector search.
    /// Ensures the vec0 virtual table exists with matching dimensions.
    pub fn set_embedding_model(&self, model: EmbeddingActiveModel) {
        let dims = model.dimensions;
        *self.embedding_model.write().unwrap() = Some(model);

        // Create vec0 virtual table for the given dimensions
        if let Ok(db) = self.connection.lock() {
            let sql = format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS vec_observations USING vec0(embedding float[{}])",
                dims
            );
            if let Err(e) = db.execute_batch(&sql) {
                crate::log_warn!("failed to create vec_observations table: {}", e);
            }
        }
    }

    /// Resolve the LLM and model to use for compression.
    fn resolve_compression_llm(&self) -> Option<(LlmClient, ActiveModel)> {
        let llm = self.llm.read().unwrap().clone()?;
        let model = self
            .compression_model
            .read()
            .unwrap()
            .clone()
            .or_else(|| self.active_model.read().unwrap().clone())?;
        Some((llm, model))
    }

    /// Resolve the LLM and model to use for summarization.
    fn resolve_summarization_llm(&self) -> Option<(LlmClient, ActiveModel)> {
        let llm = self.llm.read().unwrap().clone()?;
        let model = self
            .summarization_model
            .read()
            .unwrap()
            .clone()
            .or_else(|| self.compression_model.read().unwrap().clone())
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

    /// Search memories. Uses FTS5 for sync path.
    /// For hybrid search (BM25 + vector), use the async `search_hybrid` method.
    pub fn search(&self, workspace_root: &str, query: &str) -> Result<Vec<MemoryEntry>> {
        // Try hybrid search if we're in a tokio context and embedding model is available.
        // Use block_in_place to avoid panicking when called from a tokio worker thread.
        // Resolve models FIRST and drop all read guards before any async work
        // to prevent std::sync::RwLock re-entrancy deadlocks.
        let resolved = {
            let llm = self.llm.read().unwrap().clone();
            let model = self.embedding_model.read().unwrap().clone();
            (llm, model)
        };
        if let (Some(llm), Some(model)) = resolved {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let future = self.search_hybrid_with(
                    llm, model, query, workspace_root, 20,
                );
                let result = tokio::task::block_in_place(move || handle.block_on(future));
                if let Ok(results) = result {
                    if !results.is_empty() {
                        return Ok(results);
                    }
                }
            }
        }
        // Fall back to FTS5 / LIKE
        self.search_fts5_fallback(workspace_root, query)
    }

    /// FTS5 + LIKE fallback (no hybrid search recursion).
    /// Used by both `search` (sync) and `search_hybrid` (async) to avoid
    /// recursive hybrid-search attempts.
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

    /// Search for context-relevant memories using semantic search when available.
    ///
    /// When `query` is provided and embedding is configured, uses hybrid
    /// search (BM25 + vector) via `search_hybrid`. Falls back to FTS5,
    /// then to compound scoring (`select_hot`) when nothing else succeeds.
    ///
    /// This is the synchronous version; it uses `block_in_place` when
    /// called from a tokio context.  Prefer [`search_hot_context_async`]
    /// in async functions to avoid blocking a tokio worker thread.
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
                if let Ok(entries) = self.search(workspace_root, q) {
                    if !entries.is_empty() {
                        return Ok(entries);
                    }
                }
            }
        }
        // Fall back to compound sort (importance × frequency × recency)
        self.select_hot(workspace_root, limit, min_chars)
    }

    /// Async version of [`search_hot_context`].
    ///
    /// Unlike the sync version, this method does **not** use `block_in_place`
    /// / `block_on`.  It resolves the embedding model and calls
    /// [`search_hybrid_with`] (which has a 30-second timeout) via a normal
    /// `.await`, so the tokio worker thread is yielded while waiting for
    /// the embedding API response.
    pub async fn search_hot_context_async(
        &self,
        query: Option<&str>,
        workspace_root: &str,
        limit: usize,
        min_chars: usize,
    ) -> Result<Vec<MemoryEntry>> {
        if let Some(query) = query {
            let q = query.trim();
            if !q.is_empty() {
                let llm = self.llm.read().unwrap().clone();
                let model = self.embedding_model.read().unwrap().clone();
                if let (Some(llm), Some(model)) = (llm, model) {
                    if let Ok(entries) =
                        self.search_hybrid_with(llm, model, q, workspace_root, limit).await
                        && !entries.is_empty()
                    {
                        return Ok(entries);
                    }
                }
                // Fall back to FTS5 / LIKE
                if let Ok(entries) = self.search_fts5_fallback(workspace_root, q)
                    && !entries.is_empty()
                {
                    return Ok(entries);
                }
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
            "SELECT id, session_id, obs_type, title, subtitle,
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
            session_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(Uuid::nil()),
            obs_type: ObservationType::parse_str(&row.get::<_, String>(2)?)
                .unwrap_or(ObservationType::Other),
            title: row.get(3)?,
            subtitle: row.get(4)?,
            facts: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            narrative: row.get(6)?,
            concepts: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
            files: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
            importance: row.get::<_, i64>(9)? as u8,
            confidence: row.get(10)?,
            created_at: row
                .get::<_, String>(11)
                .ok()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
        })
    }

    /// List recent compressed observations across all sessions, newest first.
    /// Only returns observations that have been compressed (have obs_type set).
    pub fn list_recent_observations(
        &self,
        limit: usize,
        min_importance: u8,
    ) -> Result<Vec<CompressedObservation>> {
        let db = self.read_connection.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, session_id, obs_type, title, subtitle,
                    facts, narrative, concepts, files, importance, confidence, created_at
             FROM compressed_observations
             WHERE obs_type IS NOT NULL AND importance >= ?1
             ORDER BY importance DESC, created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![min_importance as i64, limit as i64],
            Self::map_compressed_observation_from_row,
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

    /// Observe a tool hook and create a raw observation.
    /// Returns the observation ID if new, or None if deduplicated.
    pub fn observe(&self, payload: &HookPayload) -> Result<Option<Uuid>> {
        let _t_conn = std::time::Instant::now();
        let id = {
            let db = self.connection.lock().unwrap();
            let mut dedup = self.dedup.lock().unwrap();
            ObservationService::observe(&db, &mut dedup, payload)?
        };
        let _t_conn = _t_conn.elapsed();
        if _t_conn > std::time::Duration::from_millis(100) {
            crate::log_warn!("observe: connection.lock took {:?}", _t_conn);
        }
        match id {
            ObservationResult::New(id) => {
                // Schedule async compression (no DB lock held)
                self.schedule_compression(id);
                // Also add to BM25 index
                let _t_bm25 = std::time::Instant::now();
                if let Ok(raw) = {
                    let db = self.read_connection.lock().unwrap();
                    Self::load_raw_observation(&db, id)
                } {
                    let search_text = format!(
                        "{} {} {} {}",
                        raw.tool_name.unwrap_or_default(),
                        raw.tool_input.unwrap_or_default(),
                        raw.tool_output.unwrap_or_default(),
                        raw.user_prompt.unwrap_or_default(),
                    );
                    self.bm25
                        .write()
                        .unwrap()
                        .add(&id.to_string(), &search_text);
                }
                let _t_bm25 = _t_bm25.elapsed();
                if _t_bm25 > std::time::Duration::from_millis(50) {
                    crate::log_warn!(
                        "observe: bm25.write took {:?}",
                        _t_bm25
                    );
                }
                Ok(Some(id))
            }
            ObservationResult::Deduplicated => Ok(None),
        }
    }

    /// Run compression on an observation. Uses LLM when available, falls
    /// back to synthetic (rule-based) compression otherwise.
    /// Returns an error if compression is disabled via `set_compression_enabled(false)`.
    pub async fn compress(&self, observation_id: Uuid) -> Result<CompressedObservation> {
        if !self
            .compression_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            anyhow::bail!("compression is disabled by configuration");
        }
        let db_path = self.db_path.clone();

        // Only attempt LLM compression when explicitly opted in and available.
        // Default is synthetic (rule-based) compression — zero LLM calls.
        let llm_enabled = self.llm_compression.load(Ordering::Relaxed);
        let use_llm = llm_enabled
            && self.resolve_compression_llm().is_some()
            && !self.is_compression_circuit_tripped();

        let compressed = if use_llm {
            let (llm, model) = self.resolve_compression_llm().unwrap();
            let conn = Connection::open(&db_path)?;
            let compress_result = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                CompressionService::compress(&conn, &llm, &model, observation_id),
            )
            .await;
            match compress_result {
                Ok(Ok(c)) => {
                    // Success — reset circuit breaker
                    self.compression_cb_failures.store(0, Ordering::Relaxed);
                    *self.compression_cb_tripped_at.write().unwrap() = None;
                    c
                }
                Ok(Err(e)) => {
                    crate::log_warn!("LLM compression failed, falling back to synthetic: {}", e);
                    let failures = self.compression_cb_failures.fetch_add(1, Ordering::Relaxed) + 1;
                    if failures >= COMPRESSION_CB_THRESHOLD {
                        crate::log_warn!(
                            "compression circuit breaker tripped after {} consecutive failures",
                            failures,
                        );
                        *self.compression_cb_tripped_at.write().unwrap() =
                            Some(std::time::Instant::now());
                    }
                    let conn = Connection::open(&db_path)?;
                    CompressionService::compress_synthetic(&conn, observation_id)?
                }
                Err(_) => {
                    crate::log_warn!("LLM compression timed out, falling back to synthetic");
                    let failures = self.compression_cb_failures.fetch_add(1, Ordering::Relaxed) + 1;
                    if failures >= COMPRESSION_CB_THRESHOLD {
                        crate::log_warn!(
                            "compression circuit breaker tripped after {} consecutive failures",
                            failures,
                        );
                        *self.compression_cb_tripped_at.write().unwrap() =
                            Some(std::time::Instant::now());
                    }
                    let conn = Connection::open(&db_path)?;
                    CompressionService::compress_synthetic(&conn, observation_id)?
                }
            }
        } else {
            let conn = Connection::open(&db_path)?;
            CompressionService::compress_synthetic(&conn, observation_id)?
        };

        // Update BM25 index
        self.bm25
            .write()
            .unwrap()
            .add(&compressed.id.to_string(), &compressed.to_search_text());

        // Embed and store in vec0 (best-effort)
        let embed_llm = self.llm.read().unwrap().clone();
        let embed_model = self.embedding_model.read().unwrap().clone();
        if let (Some(llm), Some(model)) = (embed_llm, embed_model) {
            let id = compressed.id.to_string();
            let search_text = compressed.to_search_text();
            match llm.embed(&model, &search_text).await {
                Ok(embedding) => {
                    let blob: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
                    if let Ok(db) = self.connection.try_lock() {
                        // Ensure rowid mapping exists
                        let _ = db.execute(
                            "INSERT OR IGNORE INTO vec_obs_map(observation_id) VALUES (?1)",
                            rusqlite::params![&id],
                        );
                        let rowid: i64 = db
                            .query_row(
                                "SELECT rowid FROM vec_obs_map WHERE observation_id = ?1",
                                rusqlite::params![&id],
                                |row| row.get(0),
                            )
                            .unwrap_or(0);
                        if rowid > 0 {
                            let _ = db.execute(
                                "INSERT OR REPLACE INTO vec_observations(rowid, embedding) VALUES (?1, ?2)",
                                rusqlite::params![rowid, blob],
                            );
                        }
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
            .resolve_compression_llm()
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

    fn load_raw_observation(db: &Connection, id: Uuid) -> Result<RawObservation> {
        use super::types::HookType;
        db.query_row(
            "SELECT id, session_id, created_at, hook_type, tool_name, tool_input, tool_output, user_prompt, assistant_response, NULL, NULL
             FROM compressed_observations WHERE id = ?1",
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

    fn schedule_compression(&self, obs_id: Uuid) {
        if let Some(ref sender) = *self.compression_sender.read().unwrap()
            && let Err(e) = sender.try_send(QueueTask::CompressAndEmbed(obs_id))
        {
            match e {
                mpsc::TrySendError::Full(_) => {
                    crate::log_warn!(
                        "compression queue full, dropping observation {}",
                        obs_id
                    );
                }
                mpsc::TrySendError::Disconnected(_) => {}
            }
        }
    }

    /// Enqueue an embedding-only backfill task for an already-compressed
    /// observation that is missing a vector embedding.
    pub fn schedule_embedding_backfill(&self, obs_id: Uuid) {
        if let Some(ref sender) = *self.compression_sender.read().unwrap()
            && let Err(e) = sender.try_send(QueueTask::EmbedBackfill(obs_id))
        {
            match e {
                mpsc::TrySendError::Full(_) => {
                    crate::log_warn!(
                        "compression queue full, dropping embedding backfill for {}",
                        obs_id
                    );
                }
                mpsc::TrySendError::Disconnected(_) => {}
            }
        }
    }

    /// Recover uncompressed observations that may have been left behind
    /// after a previous crash or fast exit (where the async compression
    /// thread was killed before it could run).
    ///
    /// Finds observations where `obs_type IS NULL` (not yet compressed)
    /// and schedules async compression + embedding for each one.
    /// Returns the number of observations scheduled.
    pub fn recover_uncompressed(&self, limit: usize) -> Result<usize> {
        let ids: Vec<Uuid> = {
            let db = self.connection.lock().unwrap();
            let mut stmt = db.prepare(
                "SELECT id FROM compressed_observations
                 WHERE obs_type IS NULL
                 ORDER BY created_at ASC
                 LIMIT ?1",
            )?;
            stmt.query_map(rusqlite::params![limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|s| Uuid::parse_str(&s).ok())
            .collect()
            // db and stmt dropped here → lock released
        };

        let count = ids.len();
        if count == 0 {
            return Ok(0);
        }

        crate::log_info!("recovering {} uncompressed observations", count);

        for id in ids {
            // Enqueue via the compression sender if available.
            // If no queue is configured, fall back to a direct thread spawn
            // (e.g. during early startup before the queue is created).
            let has_sender = self.compression_sender.read().unwrap().is_some();
            if has_sender {
                self.schedule_compression(id);
            } else {
                let store = self.clone();
                std::thread::spawn(move || {
                    let worker_rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            crate::log_warn!(
                                "failed to create runtime for recovery of {}: {}",
                                id,
                                e
                            );
                            return;
                        }
                    };
                    match worker_rt.block_on(store.compress(id)) {
                        Ok(_) => {
                            crate::log_info!("recovered uncompressed observation {}", id);
                        }
                        Err(e) => {
                            crate::log_warn!("recovery compression failed for {}: {}", id, e);
                        }
                    }
                });
            }
        }

        Ok(count)
    }

    /// Backfill embeddings for compressed observations that are missing them.
    ///
    /// This can happen when the vec0 extension failed to load at startup or when
    /// embedding generation temporarily failed.  This method only queries IDs
    /// and enqueues each one through the compression queue; actual embedding
    /// generation runs asynchronously in the worker pool.
    ///
    /// Requires that [`set_compression_sender`] has been called.
    /// Returns the number of observations queued for backfill.
    pub fn backfill_embeddings(&self, limit: usize) -> Result<usize> {
        if self.llm.read().unwrap().is_none()
            || self.embedding_model.read().unwrap().is_none()
        {
            return Ok(0);
        }

        let ids: Vec<Uuid> = {
            let db = self.connection.lock().unwrap();
            let mut stmt = db.prepare(
                "SELECT co.id
                 FROM compressed_observations co
                 LEFT JOIN vec_obs_map m ON m.observation_id = co.id
                 LEFT JOIN vec_observations v ON v.rowid = m.rowid
                 WHERE co.obs_type IS NOT NULL
                   AND v.rowid IS NULL
                 ORDER BY co.created_at ASC
                 LIMIT ?1"
            )?;
            stmt.query_map(rusqlite::params![limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|s| Uuid::parse_str(&s).ok())
            .collect()
        };

        let count = ids.len();
        if count == 0 {
            return Ok(0);
        }

        crate::log_info!(
            "enqueuing {} observations for embedding backfill",
            count
        );

        for id in ids {
            self.schedule_embedding_backfill(id);
        }

        Ok(count)
    }

    /// Generate and store an embedding for a single, already-compressed
    /// observation.  Called by compression queue workers.
    pub async fn backfill_embedding(&self, id: Uuid) -> Result<()> {
        let (title, narrative, facts, concepts, files) = {
            let db = self.connection.lock().unwrap();
            let mut stmt = db.prepare(
                "SELECT co.title, co.narrative, co.facts, co.concepts, co.files
                 FROM compressed_observations co
                 WHERE co.id = ?1"
            )?;
            stmt.query_row(rusqlite::params![id.to_string()], |row| {
                let title: String = row.get(0)?;
                let narrative: String = row.get(1)?;
                let facts_json: String = row.get(2)?;
                let concepts_json: String = row.get(3)?;
                let files_json: String = row.get(4)?;
                let facts: Vec<String> = serde_json::from_str(&facts_json).unwrap_or_default();
                let concepts: Vec<String> =
                    serde_json::from_str(&concepts_json).unwrap_or_default();
                let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();
                Ok((title, narrative, facts, concepts, files))
            })?
        };

        let llm = match self.llm.read().unwrap().as_ref() {
            Some(l) => l.clone(),
            None => anyhow::bail!("no LLM client configured for embedding backfill"),
        };
        let model = match self.embedding_model.read().unwrap().as_ref() {
            Some(m) => m.clone(),
            None => anyhow::bail!("no embedding model configured for backfill"),
        };

        let search_text = format!(
            "{} {} {} {} {}",
            title,
            narrative,
            facts.join(" "),
            concepts.join(" "),
            files.join(" ")
        );

        let embedding = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            llm.embed(&model, &search_text),
        )
        .await
        {
            Ok(Ok(emb)) => emb,
            Ok(Err(e)) => anyhow::bail!("embedding API error: {}", e),
            Err(_) => anyhow::bail!("embedding timed out after 30s"),
        };
        let blob: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        if let Ok(conn) = Connection::open(&self.db_path) {
            let id_str = id.to_string();
            let _ = conn.execute(
                "INSERT OR IGNORE INTO vec_obs_map(observation_id) VALUES (?1)",
                rusqlite::params![&id_str],
            );
            let rowid: i64 = conn
                .query_row(
                    "SELECT rowid FROM vec_obs_map WHERE observation_id = ?1",
                    rusqlite::params![&id_str],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if rowid > 0 {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO vec_observations(rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![rowid, blob],
                );
            }
        }

        crate::log_info!("backfilled embedding for observation {}", id);
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

    // ─── Phase 2: Vector + Hybrid Search ───────────────────────────

    /// Run hybrid search (BM25 + vector). Falls back to FTS5 if no embedding model.
    pub async fn search_hybrid(
        &self,
        query: &str,
        workspace_root: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let llm = self.llm.read().unwrap().clone();
        let model = self.embedding_model.read().unwrap().clone();
        if let (Some(llm), Some(model)) = (llm, model) {
            return self.search_hybrid_with(llm, model, query, workspace_root, limit).await;
        }
        self.search_fts5_fallback(workspace_root, query)
    }

    /// Hybrid search with pre-resolved LLM and embedding model.
    ///
    /// Unlike [`search_hybrid`], this method does **not** acquire
    /// `self.llm`/`self.embedding_model` internally, so it is safe to call
    /// from code paths that already hold those RwLock read guards (see
    /// [`search`] which resolves models before `block_in_place`).
    ///
    /// The embedding API call is wrapped with a 30-second timeout so that
    /// synchronous callers (compose_static_system_prompt → block_in_place) do not
    /// block the tokio worker thread indefinitely.
    pub async fn search_hybrid_with(
        &self,
        llm: LlmClient,
        model: EmbeddingActiveModel,
        query: &str,
        workspace_root: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let query_embedding = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            llm.embed(&model, query),
        )
        .await;

        match query_embedding {
            Ok(Ok(emb)) => {
                let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();

                let bm25_results = self.bm25.read().unwrap().search(query, limit * 2);

                let db = self.read_connection.lock().unwrap();
                let vector_results: Vec<(String, f64)> = match db.prepare(
                    "SELECT m.observation_id, v.distance
                     FROM vec_observations v
                     JOIN vec_obs_map m ON m.rowid = v.rowid
                     WHERE v.embedding MATCH ?1
                     ORDER BY v.distance
                     LIMIT ?2",
                ) {
                    Ok(mut stmt) => match stmt.query_map(
                        rusqlite::params![emb_bytes.as_slice(), (limit * 2) as i64],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
                    ) {
                        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                        Err(_) => vec![],
                    },
                    Err(_) => vec![],
                };

                let hs = self.hybrid_search.read().unwrap();
                let results = hs.fuse(bm25_results, vector_results, limit);

                if !results.is_empty() {
                    let ids: Vec<String> = results.iter().map(|r| r.id.clone()).collect();
                    let mut entries = Vec::new();
                    for id in &ids {
                        let _uuid = Uuid::parse_str(id).unwrap_or(Uuid::nil());
                        if let Ok(entry) = db.query_row(
                            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active, concepts, files, strength, importance, version, parent_id, supersedes, related_ids, is_latest
                              FROM memories WHERE id = ?1 AND workspace_root = ?2 AND active = 1 AND is_latest = 1",
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
            Ok(Err(e)) => crate::log_warn!("hybrid search embedding failed: {}", e),
            Err(_) => crate::log_warn!("hybrid search embedding timed out after 30s"),
        }

        self.search_fts5_fallback(workspace_root, query)
    }

    // ─── Graph: Knowledge Graph Extraction ─────────────────────────

    /// Extract graph entities from a compressed observation.
    ///
    /// Opens its own DB connection (called from spawned threads where
    /// `self.connection` is already held).
    pub fn graph_extract_from_observation(&self, obs: &super::CompressedObservation) -> Result<()> {
        let db_path = self.db_path.clone();
        let db = Connection::open(&db_path).context("failed to open DB for graph extraction")?;
        super::graph::extract_from_observation(&db, obs)
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
            .resolve_compression_llm()
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
