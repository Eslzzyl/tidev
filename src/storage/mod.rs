pub mod compression;
pub mod database;
pub mod migration;
pub mod schema;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rusqlite::{
    Connection, OptionalExtension, named_params, params, params_from_iter, types::Type,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    session::{Conversation, Message, MessageRole, ToolCall},
    stats::{Granularity, ModelUsageEntry, StatsEntry, TimeRangeStats, UsageSummary},
    tooling::TodoItem,
};

use rayon::prelude::*;

use self::compression::{compress_text, decompress_text};
use schema::{EXPORT_SCHEMA_SQL, SCHEMA_SQL, SESSION_SELECT_COLUMNS};

/// Build a struct literal from a SQLite row where each field maps directly
/// to a column index with no custom conversion.
///
/// # Examples
///
/// ```ignore
/// fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
///     Ok(map_row!(Self, row,
///         id: 0,
///         name: 1,
///         content: 2,
///     ))
/// }
/// ```
macro_rules! map_row {
    ($struct:tt, $row:expr, $($field:ident: $idx:expr),+ $(,)?) => {
        $struct {
            $($field: $row.get($idx)?),+
        }
    };
}

pub struct SessionStore {
    /// Shared write connection (behind Mutex for thread-safety).
    write_conn: Arc<Mutex<Connection>>,
    /// Connection for read operations (SELECT).
    /// A separate connection avoids WAL contention and prepares for
    /// a future RwLock / connection-pool migration.
    read_conn: Connection,
    path: PathBuf,
}

impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self {
            write_conn: Arc::clone(&self.write_conn),
            read_conn: Connection::open(&self.path)
                .expect("failed to clone SessionStore read_conn"),
            path: self.path.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub session_id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub workspace_root: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: String,
    pub ended_at: Option<DateTime<Utc>>,
    pub context_summary: Option<String>,
    pub context_retained_from: usize,
    pub system_prompt: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSessionCount {
    pub workspace_root: String,
    pub session_count: i64,
}

/// Token statistics for a session.
#[derive(Debug, Clone)]
pub struct SessionTokenStats {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Raw row data from `messages` table, before decompression/parsing.
///
/// Used inside `load_messages` to decouple SQLite row iteration from
/// the (potentially parallel) decompression and JSON-deserialization step.
struct RawMessageRow {
    id: String,
    role: String,
    content: Vec<u8>,
    attachments: String,
    reasoning: Option<Vec<u8>>,
    tool_calls: Vec<u8>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    metadata: Vec<u8>,
    created_at: String,
    completed_at: Option<String>,
    streaming: bool,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    total_tokens: Option<u32>,
    cache_read_tokens: Option<u32>,
    cache_write_tokens: Option<u32>,
    model_id: Option<String>,
    tokens_per_second: Option<f32>,
    snapshot_hash: Option<String>,
    patch_files: Option<Vec<u8>>,
    file_diffs: Option<Vec<u8>>,
    mode: Option<String>,
    rtk_rewritten: bool,
    thinking_level: Option<String>,
}

impl RawMessageRow {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            role: row.get(1)?,
            content: row.get(2)?,
            attachments: row.get(3)?,
            reasoning: row.get(4)?,
            tool_calls: read_blob_maybe_text_bytes(row, 5)?,
            tool_call_id: row.get(6)?,
            tool_name: row.get(7)?,
            metadata: read_blob_maybe_text_bytes(row, 8)?,
            created_at: row.get(9)?,
            completed_at: row.get(10)?,
            streaming: row.get::<_, i64>(11)? != 0,
            input_tokens: row.get(12)?,
            output_tokens: row.get(13)?,
            total_tokens: row.get(14)?,
            cache_read_tokens: row.get(15)?,
            cache_write_tokens: row.get(16)?,
            model_id: row.get(17)?,
            tokens_per_second: row.get(18)?,
            snapshot_hash: row.get(19)?,
            patch_files: read_opt_blob_maybe_text(row, 20)?,
            file_diffs: read_opt_blob_maybe_text(row, 21)?,
            mode: row.get(22)?,
            rtk_rewritten: row.get::<_, i64>(23)? != 0,
            thinking_level: row.get(24)?,
        })
    }

    /// Decompress `content`/`reasoning`, parse JSON fields, build `Message`.
    fn into_message(self) -> Result<Message> {
        use crate::config::reasoning::ThinkingLevelType;
        use crate::prompts::SessionMode;
        use crate::session::ToolMetadata;

        let content = decompress_text(&self.content);
        let reasoning = match self.reasoning {
            Some(b) => decompress_text(&b),
            None => String::new(),
        };
        let attachments: Vec<crate::session::MessageAttachment> =
            serde_json::from_str(&self.attachments).unwrap_or_default();
        let tool_calls: Vec<ToolCall> =
            serde_json::from_str(&decompress_text(&self.tool_calls)).unwrap_or_default();
        let metadata: ToolMetadata =
            serde_json::from_str(&decompress_text(&self.metadata)).unwrap_or_default();
        let mode: Option<SessionMode> = self.mode.and_then(|m| serde_json::from_str(&m).ok());
        let completed_at = self.completed_at.and_then(|s| parse_datetime(&s).ok());
        let thinking_level = self
            .thinking_level
            .filter(|s| !s.is_empty())
            .map(|s| ThinkingLevelType::from_string(&s));

        let mut message = Message::persisted(
            Uuid::parse_str(&self.id).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            MessageRole::from_db_value(&self.role),
            content,
            parse_datetime(&self.created_at).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            self.streaming,
        );
        message.attachments = attachments;
        message.reasoning = reasoning;
        message.tool_calls = tool_calls;
        message.tool_call_id = self.tool_call_id;
        message.tool_name = self.tool_name;
        message.metadata = metadata;
        message.completed_at = completed_at;
        message.input_tokens = self.input_tokens;
        message.output_tokens = self.output_tokens;
        message.total_tokens = self.total_tokens;
        message.cache_read_tokens = self.cache_read_tokens;
        message.cache_write_tokens = self.cache_write_tokens;
        message.model_id = self.model_id;
        message.tokens_per_second = self.tokens_per_second;
        message.snapshot_hash = self.snapshot_hash;
        message.patch_files = self.patch_files.map(|b| decompress_text(&b));
        message.file_diffs = self.file_diffs.map(|b| decompress_text(&b));
        message.mode = mode;
        message.rtk_rewritten = self.rtk_rewritten;
        message.thinking_level = thinking_level;
        Ok(message)
    }
}

impl SessionStore {
    /// Open (or create) the database, run schema initialisation, and return a
    /// fully initialised store.
    ///
    /// This is the backward-compatible public API.  New code should prefer
    /// [`Database::open`](super::database::Database::open) followed by
    /// [`Database::create_session_store`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let store = Self::open_existing(path)?;
        {
            let conn = store.write_conn.lock().unwrap();
            conn.execute_batch(SCHEMA_SQL)
                .context("failed to initialise database schema")?;
            migration::run_pending(&conn)
                .context("failed to run database migrations")?;
        }
        Ok(store)
    }

    /// Open connections to an existing database **without** running schema
    /// initialisation.  The caller guarantees that the schema already exists
    /// (e.g. because [`Database::open`] was called first).
    pub(crate) fn open_existing(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }

        // ── Write connection ─────────────────────────────────────────────
        let write_conn = Connection::open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        write_conn.pragma_update(None, "foreign_keys", "ON")?;
        write_conn.pragma_update(None, "journal_mode", "WAL")?;
        write_conn.busy_timeout(Duration::from_secs(5))?;
        // Performance pragmas
        write_conn.pragma_update(None, "mmap_size", "268435456")?; // 256MB
        write_conn.pragma_update(None, "cache_size", "-64000")?; // 64MB
        write_conn.pragma_update(None, "synchronous", "NORMAL")?; // WAL-safe
        write_conn.pragma_update(None, "temp_store", "MEMORY")?; // in-memory temp

        // ── Read connection ──────────────────────────────────────────────
        let read_conn = Connection::open(&path)
            .with_context(|| format!("failed to open {} for reads", path.display()))?;
        read_conn.pragma_update(None, "journal_mode", "WAL")?;
        read_conn.busy_timeout(Duration::from_secs(5))?;
        read_conn.pragma_update(None, "mmap_size", "268435456")?;
        read_conn.pragma_update(None, "cache_size", "-64000")?;
        read_conn.pragma_update(None, "temp_store", "MEMORY")?;

        Ok(Self {
            write_conn: Arc::new(Mutex::new(write_conn)),
            read_conn,
            path,
        })
    }

    /// Open connections reusing a shared write connection (provided by
    /// [`Database`](super::database::Database)).
    /// Only opens a new read connection; the write connection is shared.
    pub(crate) fn open_with_shared_write(
        path: impl AsRef<Path>,
        write_conn: Arc<Mutex<Connection>>,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let read_conn = Connection::open(&path)
            .with_context(|| format!("failed to open {} for reads", path.display()))?;
        read_conn.pragma_update(None, "journal_mode", "WAL")?;
        read_conn.busy_timeout(Duration::from_secs(5))?;
        read_conn.pragma_update(None, "mmap_size", "268435456")?;
        read_conn.pragma_update(None, "cache_size", "-64000")?;
        read_conn.pragma_update(None, "temp_store", "MEMORY")?;

        Ok(Self {
            write_conn,
            read_conn,
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &self,
        session_id: Uuid,
        workspace_root: &Path,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
    ) -> Result<SessionRecord> {
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let session_id_text = session_id.to_string();
        let workspace_root = workspace_root.display().to_string();

        self.write_execute(
            "INSERT INTO sessions (id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, status, context_summary, context_retained_from) VALUES (:id, :provider_id, :provider_display_name, :model_id, :model_display_name, :title, :created_at, :updated_at, :status, :context_summary, :context_retained_from)",
            named_params! {
                ":id": session_id_text.clone(),
                ":provider_id": provider_id,
                ":provider_display_name": provider_display_name,
                ":model_id": model_id,
                ":model_display_name": model_display_name,
                ":title": title,
                ":created_at": now_text,
                ":updated_at": now_text,
                ":status": "active",
                ":context_summary": "",
                ":context_retained_from": 0_i64,
            },
        )?;

        self.write_execute(
            "INSERT INTO session_workspaces (session_id, workspace_root) VALUES (:session_id, :workspace_root)",
            named_params! {
                ":session_id": session_id_text,
                ":workspace_root": workspace_root.clone(),
            },
        )?;

        Ok(SessionRecord {
            session_id,
            parent_session_id: None,
            workspace_root,
            provider_id: provider_id.to_string(),
            provider_display_name: provider_display_name.to_string(),
            model_id: model_id.to_string(),
            model_display_name: model_display_name.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
            status: "active".to_string(),
            ended_at: None,
            context_summary: None,
            context_retained_from: 0,
            system_prompt: String::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session_with_parent(
        &self,
        session_id: Uuid,
        parent_session_id: Uuid,
        workspace_root: &Path,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
    ) -> Result<SessionRecord> {
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let session_id_text = session_id.to_string();
        let parent_session_id_text = parent_session_id.to_string();
        let workspace_root = workspace_root.display().to_string();

        self.write_execute(
            "INSERT INTO sessions (id, parent_session_id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, status, context_summary, context_retained_from) VALUES (:id, :parent_session_id, :provider_id, :provider_display_name, :model_id, :model_display_name, :title, :created_at, :updated_at, :status, :context_summary, :context_retained_from)",
            named_params! {
                ":id": session_id_text.clone(),
                ":parent_session_id": parent_session_id_text,
                ":provider_id": provider_id,
                ":provider_display_name": provider_display_name,
                ":model_id": model_id,
                ":model_display_name": model_display_name,
                ":title": title,
                ":created_at": now_text,
                ":updated_at": now_text,
                ":status": "active",
                ":context_summary": "",
                ":context_retained_from": 0_i64,
            },
        )?;

        self.write_execute(
            "INSERT INTO session_workspaces (session_id, workspace_root) VALUES (:session_id, :workspace_root)",
            named_params! {
                ":session_id": session_id_text,
                ":workspace_root": workspace_root.clone(),
            },
        )?;

        Ok(SessionRecord {
            session_id,
            parent_session_id: Some(parent_session_id),
            workspace_root,
            provider_id: provider_id.to_string(),
            provider_display_name: provider_display_name.to_string(),
            model_id: model_id.to_string(),
            model_display_name: model_display_name.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
            status: "active".to_string(),
            ended_at: None,
            context_summary: None,
            context_retained_from: 0,
            system_prompt: String::new(),
        })
    }

    pub fn update_session_context_state(
        &self,
        session_id: Uuid,
        summary: Option<&str>,
        retained_from: usize,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_conn.lock().unwrap().execute(
            "UPDATE sessions SET context_summary = ?1, context_retained_from = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                summary.unwrap_or(""),
                retained_from as i64,
                now,
                session_id.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn update_session_title(&self, session_id: Uuid, title: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_conn.lock().unwrap().execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, session_id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_session_model(
        &self,
        session_id: Uuid,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_conn.lock().unwrap().execute(
            "UPDATE sessions SET provider_id = ?1, provider_display_name = ?2, model_id = ?3, model_display_name = ?4, updated_at = ?5 WHERE id = ?6",
            params![
                provider_id,
                provider_display_name,
                model_id,
                model_display_name,
                now,
                session_id.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn update_session_system_prompt(
        &self,
        session_id: Uuid,
        system_prompt: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_conn.lock().unwrap().execute(
            "UPDATE sessions SET system_prompt = ?1, updated_at = ?2 WHERE id = ?3",
            params![system_prompt, now, session_id.to_string()],
        )?;
        Ok(())
    }

    pub fn load_session_system_prompt(&self, session_id: Uuid) -> Result<String> {
        let result = self
            .read_conn
            .query_row(
                "SELECT system_prompt FROM sessions WHERE id = ?1",
                params![session_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(result.unwrap_or_default())
    }

    pub fn append_instruction_source(&self, session_id: Uuid, source: &str) -> Result<()> {
        self.write_execute(
            "INSERT OR IGNORE INTO session_instruction_sources (session_id, source) VALUES (:session_id, :source)",
            named_params! {
                ":session_id": session_id.to_string(),
                ":source": source,
            },
        )?;
        Ok(())
    }

    pub fn load_instruction_sources(&self, session_id: Uuid) -> Result<Vec<String>> {
        self.read_query(
            "SELECT source FROM session_instruction_sources WHERE session_id = :session_id",
            named_params! { ":session_id": session_id.to_string() },
            |row| row.get(0),
        )
    }

    pub fn append_message(&self, session_id: Uuid, message: &Message) -> Result<()> {
        let tool_calls = compress_text(
            &serde_json::to_string(&message.tool_calls)
                .context("failed to serialize tool calls")?,
        );
        let attachments = serde_json::to_string(&message.attachments)
            .context("failed to serialize attachments")?;
        let metadata = compress_text(
            &serde_json::to_string(&message.metadata).context("failed to serialize metadata")?,
        );
        let mode = message
            .mode
            .map(|m| serde_json::to_string(&m).unwrap_or_default());
        let thinking_level = message.thinking_level.as_ref().map(|t| t.to_string());
        let compressed_content = compress_text(&message.content);
        let reasoning = if message.reasoning.is_empty() {
            None
        } else {
            Some(compress_text(&message.reasoning))
        };
        self.write_execute(
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, rtk_rewritten, thinking_level) VALUES (:id, :session_id, :role, :content, :attachments, :reasoning, :tool_calls, :tool_call_id, :tool_name, :metadata, :created_at, :completed_at, :streaming, :input_tokens, :output_tokens, :total_tokens, :cache_read_tokens, :cache_write_tokens, :model_id, :tokens_per_second, :snapshot_hash, :patch_files, :file_diffs, :mode, :rtk_rewritten, :thinking_level)",
            named_params! {
                ":id": message.id.to_string(),
                ":session_id": session_id.to_string(),
                ":role": message.role.db_value(),
                ":content": compressed_content,
                ":attachments": attachments,
                ":reasoning": reasoning,
                ":tool_calls": tool_calls,
                ":tool_call_id": message.tool_call_id,
                ":tool_name": message.tool_name,
                ":metadata": metadata,
                ":created_at": message.created_at.to_rfc3339(),
                ":completed_at": message.completed_at.map(|t| t.to_rfc3339()),
                ":streaming": if message.streaming { 1_i64 } else { 0_i64 },
                ":input_tokens": message.input_tokens,
                ":output_tokens": message.output_tokens,
                ":total_tokens": message.total_tokens,
                ":cache_read_tokens": message.cache_read_tokens,
                ":cache_write_tokens": message.cache_write_tokens,
                ":model_id": message.model_id,
                ":tokens_per_second": message.tokens_per_second,
                ":snapshot_hash": message.snapshot_hash,
                ":patch_files": message.patch_files.as_deref().map(compress_text),
                ":file_diffs": message.file_diffs.as_deref().map(compress_text),
                ":mode": mode,
                ":rtk_rewritten": if message.rtk_rewritten { 1_i64 } else { 0_i64 },
                ":thinking_level": thinking_level,
            },
        )?;

        self.touch_session(session_id)?;
        Ok(())
    }

    /// Append multiple messages in a single transaction.
    ///
    /// This is significantly faster than calling [`append_message`] N times
    /// because all inserts share one SQLite transaction instead of N.
    /// Use this for tool-heavy turns where 10+ tool results are persisted
    /// sequentially.
    pub fn append_messages(&self, session_id: Uuid, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let guard = self.write_conn.lock().unwrap();

        let result = (|| -> Result<()> {
            guard.execute_batch("BEGIN TRANSACTION")?;

            let mut stmt = guard.prepare(
                "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, rtk_rewritten, thinking_level) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            ).context("failed to prepare batch insert statement")?;

            for message in messages {
                let tool_calls = compress_text(
                    &serde_json::to_string(&message.tool_calls)
                        .context("failed to serialize tool calls")?,
                );
                let attachments = serde_json::to_string(&message.attachments)
                    .context("failed to serialize attachments")?;
                let metadata = compress_text(
                    &serde_json::to_string(&message.metadata)
                        .context("failed to serialize metadata")?,
                );
                let mode = message
                    .mode
                    .map(|m| serde_json::to_string(&m).unwrap_or_default());
                let thinking_level = message.thinking_level.as_ref().map(|t| t.to_string());
                let compressed_content = compress_text(&message.content);
                let reasoning = if message.reasoning.is_empty() {
                    None
                } else {
                    Some(compress_text(&message.reasoning))
                };

                stmt.execute(params![
                    message.id.to_string(),
                    session_id.to_string(),
                    message.role.db_value(),
                    compressed_content,
                    attachments,
                    reasoning,
                    tool_calls,
                    message.tool_call_id,
                    message.tool_name,
                    metadata,
                    message.created_at.to_rfc3339(),
                    message.completed_at.map(|t| t.to_rfc3339()),
                    if message.streaming { 1_i64 } else { 0_i64 },
                    message.input_tokens,
                    message.output_tokens,
                    message.total_tokens,
                    message.cache_read_tokens,
                    message.cache_write_tokens,
                    message.model_id,
                    message.tokens_per_second,
                    message.snapshot_hash,
                    message.patch_files.as_deref().map(compress_text),
                    message.file_diffs.as_deref().map(compress_text),
                    mode,
                    if message.rtk_rewritten { 1_i64 } else { 0_i64 },
                    thinking_level,
                ])
                .context("failed to insert message in batch")?;
            }

            drop(stmt);
            guard.execute_batch("COMMIT")?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                drop(guard);
                self.touch_session(session_id)?;
                Ok(())
            }
            Err(e) => {
                let _ = guard.execute_batch("ROLLBACK");
                drop(guard);
                Err(e)
            }
        }
    }

    pub fn delete_messages(&self, session_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        for message_id in message_ids {
            self.write_conn.lock().unwrap().execute(
                "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
                params![session_id.to_string(), message_id.to_string()],
            )?;
        }

        self.touch_session(session_id)?;
        Ok(())
    }

    /// Delete all tool result messages with the given `tool_call_id`.
    ///
    /// Used to clean up stale ShellOutput-persisted messages (e.g. sandbox
    /// denial output) before persisting a retry result.
    pub fn delete_messages_by_tool_call_id(
        &self,
        session_id: Uuid,
        tool_call_id: &str,
    ) -> Result<()> {
        self.write_execute(
            "DELETE FROM messages WHERE session_id = :session_id AND tool_call_id = :tool_call_id",
            named_params! {
                ":session_id": session_id.to_string(),
                ":tool_call_id": tool_call_id,
            },
        )?;
        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn remember_tool_permission(
        &self,
        session_id: Uuid,
        tool_name: &str,
        allowed: bool,
    ) -> Result<()> {
        self.write_execute(
            "INSERT INTO tool_permissions (session_id, tool_name, allowed, created_at) VALUES (:session_id, :tool_name, :allowed, :created_at) ON CONFLICT(session_id, tool_name) DO UPDATE SET allowed = excluded.allowed, created_at = excluded.created_at",
            named_params! {
                ":session_id": session_id.to_string(),
                ":tool_name": tool_name,
                ":allowed": if allowed { 1_i64 } else { 0_i64 },
                ":created_at": Utc::now().to_rfc3339(),
            },
        )?;

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn load_tool_permission(&self, session_id: Uuid, tool_name: &str) -> Result<Option<bool>> {
        self.read_query_opt(
            "SELECT allowed FROM tool_permissions WHERE session_id = :session_id AND tool_name = :tool_name LIMIT 1",
            named_params! {
                ":session_id": session_id.to_string(),
                ":tool_name": tool_name,
            },
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )
    }

    pub fn copy_tool_permissions(&self, from_session_id: Uuid, to_session_id: Uuid) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            "INSERT INTO tool_permissions (session_id, tool_name, allowed, created_at)
             SELECT ?1, tool_name, allowed, ?2
             FROM tool_permissions
             WHERE session_id = ?3
             ON CONFLICT(session_id, tool_name)
             DO UPDATE SET allowed = excluded.allowed, created_at = excluded.created_at",
            params![
                to_session_id.to_string(),
                Utc::now().to_rfc3339(),
                from_session_id.to_string(),
            ],
        )?;

        self.touch_session(to_session_id)?;
        Ok(())
    }

    pub fn replace_todos(&self, session_id: Uuid, todos: &[TodoItem]) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            "DELETE FROM todos WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;

        if !todos.is_empty() {
            let guard = self.write_conn.lock().unwrap();
            let mut stmt = guard.prepare(
                "INSERT INTO todos (session_id, position, content, status) VALUES (?1, ?2, ?3, ?4)"
            )?;

            for (position, todo) in todos.iter().enumerate() {
                stmt.execute(params![
                    session_id.to_string(),
                    position as i64,
                    &todo.content,
                    &todo.status,
                ])?;
            }
        }

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn load_todos(&self, session_id: Uuid) -> Result<Vec<TodoItem>> {
        let mut statement = self.read_conn.prepare(
            "SELECT content, status FROM todos WHERE session_id = ?1 ORDER BY position ASC",
        )?;

        let rows = statement.query_map(params![session_id.to_string()], |row| {
            Ok(TodoItem {
                content: row.get::<_, String>(0)?,
                status: row.get::<_, String>(1)?,
            })
        })?;

        let mut todos = Vec::new();
        for row in rows {
            todos.push(row?);
        }

        Ok(todos)
    }

    pub fn load_latest_session(&self) -> Result<Option<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.parent_session_id IS NULL ORDER BY s.updated_at DESC LIMIT 1"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let record = statement.query_row([], Self::session_from_row).optional()?;

        Ok(record)
    }

    pub fn load_conversation(&self, session_id: Uuid) -> Result<Option<Conversation>> {
        let record = self.load_session_record(session_id)?;

        let Some(record) = record else {
            return Ok(None);
        };

        let messages = self.load_messages(session_id)?;
        let revert_message_id = self.load_revert_message_id(session_id)?;
        Ok(Some(Conversation {
            session_id: record.session_id,
            parent_session_id: record.parent_session_id,
            workspace_root: record.workspace_root,
            provider_id: record.provider_id,
            provider_display_name: record.provider_display_name,
            model_id: record.model_id,
            model_display_name: record.model_display_name,
            title: record.title,
            created_at: record.created_at,
            updated_at: record.updated_at,
            context_summary: record.context_summary,
            context_retained_from: record.context_retained_from,
            messages,
            revert_message_id,
        }))
    }

    pub fn load_revert_message_id(&self, session_id: Uuid) -> Result<Option<Uuid>> {
        let message_id = self
            .read_query_opt(
                "SELECT message_id FROM session_reverts WHERE session_id = :session_id LIMIT 1",
                named_params! { ":session_id": session_id.to_string() },
                |row| row.get::<_, String>(0),
            )?
            .map(|value| {
                Uuid::parse_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })
            })
            .transpose()?;

        Ok(message_id)
    }

    pub fn set_revert_message_id(
        &self,
        session_id: Uuid,
        message_id: Option<Uuid>,
        redo_snapshot: Option<&str>,
    ) -> Result<()> {
        let compressed_snapshot = redo_snapshot.map(compress_text);
        match message_id {
            Some(message_id) => {
                self.write_conn.lock().unwrap().execute(
                    "INSERT INTO session_reverts (session_id, message_id, redo_snapshot, created_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(session_id) DO UPDATE SET message_id = excluded.message_id, redo_snapshot = excluded.redo_snapshot, created_at = excluded.created_at",
                    params![
                        session_id.to_string(),
                        message_id.to_string(),
                        compressed_snapshot,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
            }
            None => {
                self.write_conn.lock().unwrap().execute(
                    "DELETE FROM session_reverts WHERE session_id = ?1",
                    params![session_id.to_string()],
                )?;
            }
        }

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn clear_revert_message_id(&self, session_id: Uuid) -> Result<()> {
        self.set_revert_message_id(session_id, None, None)
    }

    pub fn load_redo_snapshot(&self, session_id: Uuid) -> Result<Option<String>> {
        let mut statement = self
            .read_conn
            .prepare("SELECT redo_snapshot FROM session_reverts WHERE session_id = ?1 LIMIT 1")?;

        let snapshot = statement
            .query_row(params![session_id.to_string()], |row| {
                read_opt_blob_maybe_text(row, 0)
            })
            .optional()?
            .flatten()
            .map(|bytes| decompress_text(&bytes));

        Ok(snapshot)
    }

    /// Get token statistics for a session.
    pub fn get_session_token_stats(&self, session_id: Uuid) -> Result<SessionTokenStats> {
        let mut statement = self.read_conn.prepare(
            r#"
            SELECT
                COALESCE(SUM(input_tokens), 0) as input_tokens,
                COALESCE(SUM(output_tokens), 0) as output_tokens
            FROM messages
            WHERE session_id = ?1
            "#,
        )?;

        let stats = statement.query_row(params![session_id.to_string()], |row| {
            Ok(SessionTokenStats {
                input_tokens: row.get::<_, i64>(0)? as u32,
                output_tokens: row.get::<_, i64>(1)? as u32,
            })
        })?;

        Ok(stats)
    }

    pub fn update_message_snapshot(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        snapshot_hash: &str,
    ) -> Result<()> {
        self.write_execute(
            "UPDATE messages SET snapshot_hash = :snapshot_hash WHERE session_id = :session_id AND id = :id",
            named_params! {
                ":snapshot_hash": snapshot_hash,
                ":session_id": session_id.to_string(),
                ":id": message_id.to_string(),
            },
        )?;
        Ok(())
    }

    pub fn update_message_patch(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        patch_files: &str,
    ) -> Result<()> {
        let compressed = compress_text(patch_files);
        self.write_execute(
            "UPDATE messages SET patch_files = :compressed WHERE session_id = :session_id AND id = :id",
            named_params! {
                ":compressed": compressed,
                ":session_id": session_id.to_string(),
                ":id": message_id.to_string(),
            },
        )?;
        Ok(())
    }

    pub fn update_message_file_diffs(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        file_diffs: &str,
    ) -> Result<()> {
        let compressed = compress_text(file_diffs);
        self.write_execute(
            "UPDATE messages SET file_diffs = :compressed WHERE session_id = :session_id AND id = :id",
            named_params! {
                ":compressed": compressed,
                ":session_id": session_id.to_string(),
                ":id": message_id.to_string(),
            },
        )?;
        Ok(())
    }

    pub fn update_message_content(&self, message_id: Uuid, content: &str) -> Result<()> {
        let compressed = compress_text(content);
        self.write_execute(
            "UPDATE messages SET content = :content WHERE id = :id",
            named_params! {
                ":content": compressed,
                ":id": message_id.to_string(),
            },
        )?;
        Ok(())
    }

    pub fn load_session_record(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.id = ?1 LIMIT 1"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let record = statement
            .query_row(params![session_id.to_string()], |row| {
                Self::session_from_row(row)
            })
            .optional()?;

        Ok(record)
    }

    pub fn set_gateway_chat_session(
        &self,
        platform: &str,
        chat_key: &str,
        session_id: Uuid,
    ) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            "INSERT INTO gateway_chat_sessions (platform, chat_key, session_id, updated_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(platform, chat_key) DO UPDATE SET session_id = excluded.session_id, updated_at = excluded.updated_at",
            params![
                platform,
                chat_key,
                session_id.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn load_gateway_chat_session(
        &self,
        platform: &str,
        chat_key: &str,
    ) -> Result<Option<Uuid>> {
        let mut statement = self.read_conn.prepare(
            "SELECT session_id FROM gateway_chat_sessions WHERE platform = ?1 AND chat_key = ?2 LIMIT 1",
        )?;

        let value = statement
            .query_row(params![platform, chat_key], |row| row.get::<_, String>(0))
            .optional()?;

        value
            .map(|raw| Uuid::parse_str(&raw).context("invalid stored gateway session id"))
            .transpose()
    }

    pub fn clear_gateway_chat_session(&self, platform: &str, chat_key: &str) -> Result<()> {
        self.write_execute(
            "DELETE FROM gateway_chat_sessions WHERE platform = :platform AND chat_key = :chat_key",
            named_params! {
                ":platform": platform,
                ":chat_key": chat_key,
            },
        )?;
        Ok(())
    }

    pub fn set_gateway_chat_model(
        &self,
        platform: &str,
        chat_key: &str,
        provider_id: &str,
        model_id: &str,
    ) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            "INSERT INTO gateway_chat_models (platform, chat_key, provider_id, model_id, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(platform, chat_key) DO UPDATE SET provider_id = excluded.provider_id, model_id = excluded.model_id, updated_at = excluded.updated_at",
            params![
                platform,
                chat_key,
                provider_id,
                model_id,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn load_gateway_chat_model(
        &self,
        platform: &str,
        chat_key: &str,
    ) -> Result<Option<(String, String)>> {
        let mut statement = self.read_conn.prepare(
            "SELECT provider_id, model_id FROM gateway_chat_models WHERE platform = ?1 AND chat_key = ?2 LIMIT 1",
        )?;

        let value = statement
            .query_row(params![platform, chat_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;

        Ok(value)
    }

    pub fn clear_gateway_chat_model(&self, platform: &str, chat_key: &str) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            "DELETE FROM gateway_chat_models WHERE platform = ?1 AND chat_key = ?2",
            params![platform, chat_key],
        )?;
        Ok(())
    }

    /// List all gateway chat sessions for a given platform.
    /// Returns a list of (chat_key, session_id) tuples sorted by updated_at descending.
    pub fn list_gateway_chat_sessions(&self, platform: &str) -> Result<Vec<(String, Uuid)>> {
        let mut statement = self.read_conn.prepare(
            "SELECT chat_key, session_id, updated_at FROM gateway_chat_sessions WHERE platform = ?1 ORDER BY updated_at DESC",
        )?;

        let rows = statement.query_map(params![platform], |row| {
            let chat_key = row.get::<_, String>(0)?;
            let session_id_str = row.get::<_, String>(1)?;
            Ok((chat_key, session_id_str))
        })?;

        let mut sessions: Vec<(String, Uuid)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for row in rows {
            let (chat_key, session_id_str) = row?;
            if seen.contains(&chat_key) {
                continue;
            }
            seen.insert(chat_key.clone());
            let session_id = Uuid::parse_str(&session_id_str)
                .context("invalid session_id in gateway_chat_sessions")?;
            sessions.push((chat_key, session_id));
        }

        Ok(sessions)
    }

    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        // Phase 1: collect raw rows (no decompression, no JSON parsing)
        let raw_rows: Vec<RawMessageRow> = self.read_query(
            "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, rtk_rewritten, thinking_level FROM messages WHERE session_id = :session_id ORDER BY created_at ASC, rowid ASC",
            named_params! { ":session_id": session_id.to_string() },
            RawMessageRow::from_row,
        )?;

        // Phase 2: decompress + parse in parallel (if enough rows)
        let messages: Vec<Message> = if raw_rows.len() >= 16 {
            raw_rows
                .into_par_iter()
                .map(|r| r.into_message())
                .collect::<Result<Vec<_>>>()?
        } else {
            raw_rows
                .into_iter()
                .map(|r| r.into_message())
                .collect::<Result<Vec<_>>>()?
        };

        Ok(messages)
    }

    pub fn load_sessions_for_workspace(&self, workspace_root: &Path) -> Result<Vec<SessionRecord>> {
        let workspace_root = workspace_root.display().to_string();
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE sw.workspace_root = ?1 AND s.parent_session_id IS NULL ORDER BY s.updated_at DESC, s.created_at DESC"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let rows = statement.query_map(params![workspace_root], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn load_child_sessions(&self, parent_session_id: Uuid) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.parent_session_id = ?1 ORDER BY s.updated_at DESC, s.created_at DESC"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let rows = statement.query_map(
            params![parent_session_id.to_string()],
            Self::session_from_row,
        )?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn load_all_sessions(&self) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.parent_session_id IS NULL ORDER BY s.updated_at DESC, s.created_at DESC"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let rows = statement.query_map([], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn delete_session(&self, session_id: Uuid) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
    }

    pub fn delete_sessions(&self, session_ids: &[Uuid]) -> Result<()> {
        if session_ids.is_empty() {
            return Ok(());
        }

        let ids: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
        self.delete_sessions_by_ids(&ids)
    }

    fn delete_sessions_by_ids(&self, session_ids: &[String]) -> Result<()> {
        if session_ids.is_empty() {
            return Ok(());
        }

        let placeholders: Vec<&str> = session_ids.iter().map(|_| "?").collect();
        let sql = format!(
            "DELETE FROM sessions WHERE id IN ({})",
            placeholders.join(",")
        );

        let params: Vec<String> = session_ids.to_vec();
        self.write_conn
            .lock()
            .unwrap()
            .execute(&sql, params_from_iter(params))?;

        Ok(())
    }

    pub fn delete_sessions_older_than(
        &self,
        duration: ChronoDuration,
    ) -> Result<Vec<SessionRecord>> {
        let cutoff = Utc::now() - duration;
        let cutoff_text = cutoff.to_rfc3339();

        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.updated_at < ?1 AND s.parent_session_id IS NULL ORDER BY s.updated_at DESC"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let rows = statement.query_map(params![cutoff_text], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        let session_ids: Vec<String> = records.iter().map(|r| r.session_id.to_string()).collect();
        self.delete_sessions_by_ids(&session_ids)?;

        Ok(records)
    }

    pub fn delete_sessions_in_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<SessionRecord>> {
        let workspace_root = workspace_root.display().to_string();

        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE sw.workspace_root = ?1 AND s.parent_session_id IS NULL ORDER BY s.updated_at DESC"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let rows = statement.query_map(params![workspace_root], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        let session_ids: Vec<String> = records.iter().map(|r| r.session_id.to_string()).collect();
        self.delete_sessions_by_ids(&session_ids)?;

        Ok(records)
    }

    pub fn get_session_counts_by_workspace(&self) -> Result<Vec<WorkspaceSessionCount>> {
        self.read_query(
            "SELECT sw.workspace_root, COUNT(*) as cnt FROM session_workspaces sw INNER JOIN sessions s ON s.id = sw.session_id WHERE s.parent_session_id IS NULL GROUP BY sw.workspace_root ORDER BY cnt DESC",
            [],
            |row| Ok(map_row!(WorkspaceSessionCount, row,
                workspace_root: 0,
                session_count: 1,
            )),
        )
    }

    pub fn get_sessions_older_than_preview(
        &self,
        duration: ChronoDuration,
    ) -> Result<Vec<SessionRecord>> {
        let cutoff = Utc::now() - duration;
        let cutoff_text = cutoff.to_rfc3339();

        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.updated_at < ?1 AND s.parent_session_id IS NULL ORDER BY sw.workspace_root, s.updated_at DESC"
        );
        let mut statement = self.read_conn.prepare(&sql)?;

        let rows = statement.query_map(params![cutoff_text], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn get_current_workspace_sessions_count(&self, workspace_root: &Path) -> Result<i64> {
        let workspace_root = workspace_root.display().to_string();
        self.read_query_row(
            "SELECT COUNT(*) FROM session_workspaces sw INNER JOIN sessions s ON s.id = sw.session_id WHERE sw.workspace_root = :workspace_root AND s.parent_session_id IS NULL",
            named_params! { ":workspace_root": workspace_root },
            |row| row.get(0),
        )
    }

    fn touch_session(&self, session_id: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "UPDATE sessions SET updated_at = :updated_at WHERE id = :id",
            named_params! {
                ":updated_at": now,
                ":id": session_id.to_string(),
            },
        )?;
        Ok(())
    }

    /// Find user sessions (parent_session_id IS NULL) that have been inactive
    /// long enough (updated_at < cutoff) and still have status 'active'.
    /// Excludes the current foreground session.
    pub fn find_inactive_sessions(
        &self,
        cutoff: &str,
        exclude_session_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        let mut stmt = self.read_conn.prepare(
            "SELECT s.id FROM sessions s
             WHERE s.parent_session_id IS NULL
               AND s.status = 'active'
               AND s.updated_at < ?1
               AND s.id != ?2
                AND EXISTS (
                    SELECT 1 FROM messages m
                    WHERE m.session_id = s.id
                )
             ORDER BY s.updated_at ASC",
        )?;
        let ids = stmt
            .query_map(params![cutoff, exclude_session_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                Uuid::parse_str(&id_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Set session status and optionally ended_at timestamp.
    pub fn set_session_status(&self, session_id: Uuid, status: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let ended_at = if status == "completed" {
            Some(&now)
        } else {
            None
        };
        self.write_execute(
            "UPDATE sessions SET status = :status, ended_at = :ended_at, updated_at = :updated_at WHERE id = :id",
            named_params! {
                ":status": status,
                ":ended_at": ended_at,
                ":updated_at": now,
                ":id": session_id.to_string(),
            },
        )?;
        Ok(())
    }

    fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
        let id = row.get::<_, String>(0)?;
        let parent_session_id = row.get::<_, Option<String>>(1)?;
        let provider_id = row.get::<_, String>(2)?;
        let provider_display_name = row.get::<_, String>(3)?;
        let model_id = row.get::<_, String>(4)?;
        let model_display_name = row.get::<_, String>(5)?;
        let title = row.get::<_, String>(6)?;
        let created_at = row.get::<_, String>(7)?;
        let updated_at = row.get::<_, String>(8)?;
        let status = row.get::<_, String>(9)?;
        let ended_at = row.get::<_, Option<String>>(10)?;
        let context_summary = row.get::<_, String>(11)?;
        let context_retained_from = row.get::<_, i64>(12)? as usize;
        let system_prompt = row.get::<_, String>(13)?;
        let workspace_root = row.get::<_, String>(14)?;

        let parent_session_id = parent_session_id
            .map(|value| {
                Uuid::parse_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(1, Type::Text, Box::new(error))
                })
            })
            .transpose()?;

        Ok(SessionRecord {
            session_id: Uuid::parse_str(&id).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })?,
            parent_session_id,
            workspace_root,
            provider_id: provider_id.clone(),
            provider_display_name: fallback_display_name(provider_display_name, &provider_id),
            model_id: model_id.clone(),
            model_display_name: fallback_display_name(model_display_name, &model_id),
            title,
            created_at: parse_datetime(&created_at).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(error))
            })?,
            updated_at: parse_datetime(&updated_at).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
            })?,
            status: if status.trim().is_empty() {
                "active".to_string()
            } else {
                status
            },
            ended_at: match ended_at {
                Some(ref v) if !v.trim().is_empty() => {
                    Some(parse_datetime(v).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(10, Type::Text, Box::new(error))
                    })?)
                }
                _ => None,
            },
            context_summary: if context_summary.trim().is_empty() {
                None
            } else {
                Some(context_summary)
            },
            context_retained_from,
            system_prompt,
        })
    }
}

fn parse_datetime(value: &str) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn fallback_display_name(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

impl SessionStore {
    // ─── Internal query helpers ─────────────────────────────────────

    /// Execute a write query on the shared write connection.
    fn write_execute(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        self.write_conn
            .lock()
            .unwrap()
            .execute(sql, params)
            .map_err(anyhow::Error::from)
    }

    /// Prepare a query, map all rows, and collect into a Vec.
    fn read_query<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut stmt = self.read_conn.prepare(sql)?;
        let rows = stmt.query_map(params, f)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Query a single optional row from the read connection.
    fn read_query_opt<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        f: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>> {
        let mut stmt = self.read_conn.prepare(sql)?;
        stmt.query_row(params, f)
            .optional()
            .map_err(anyhow::Error::from)
    }

    /// Query a single row from the read connection (error if not found).
    fn read_query_row<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        f: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<T> {
        let mut stmt = self.read_conn.prepare(sql)?;
        stmt.query_row(params, f).map_err(anyhow::Error::from)
    }

    pub fn record_usage(
        &self,
        provider_id: &str,
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> Result<()> {
        let now = Utc::now();
        let time_bucket = Granularity::Hour.time_bucket(&now);
        let now_text = now.to_rfc3339();
        let total_tokens = input_tokens as i64 + output_tokens as i64;

        self.write_conn.lock().unwrap().execute(
            r#"
            INSERT INTO usage_stats (
                provider_id, model_id, time_bucket, granularity,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                total_tokens, request_count, created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'hour', ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9)
            ON CONFLICT(provider_id, model_id, time_bucket, granularity) DO UPDATE SET
                input_tokens = input_tokens + excluded.input_tokens,
                output_tokens = output_tokens + excluded.output_tokens,
                cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                total_tokens = total_tokens + excluded.total_tokens,
                request_count = request_count + 1,
                updated_at = excluded.updated_at
            "#,
            params![
                provider_id,
                model_id,
                time_bucket,
                input_tokens as i64,
                output_tokens as i64,
                cache_read_tokens as i64,
                cache_write_tokens as i64,
                total_tokens,
                now_text,
            ],
        )?;

        Ok(())
    }

    /// Save the user's thinking level preference for a specific model.
    pub fn save_model_thinking_level(
        &self,
        provider_id: &str,
        model_id: &str,
        thinking_level: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "INSERT OR REPLACE INTO model_thinking_levels (provider_id, model_id, thinking_level, updated_at) VALUES (:provider_id, :model_id, :thinking_level, :updated_at)",
            named_params! {
                ":provider_id": provider_id,
                ":model_id": model_id,
                ":thinking_level": thinking_level,
                ":updated_at": now,
            },
        )?;
        Ok(())
    }

    /// Load the user's thinking level preference for a specific model.
    pub fn load_model_thinking_level(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<Option<String>> {
        self.read_query_opt(
            "SELECT thinking_level FROM model_thinking_levels WHERE provider_id = :provider_id AND model_id = :model_id",
            named_params! {
                ":provider_id": provider_id,
                ":model_id": model_id,
            },
            |row| row.get::<_, String>(0),
        )
    }

    pub fn get_time_range_stats(
        &self,
        granularity: Granularity,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<TimeRangeStats> {
        let mut entries = Vec::new();
        let mut summary = UsageSummary::default();

        let granularity_str = granularity.as_str();

        let mut stmt = self.read_conn.prepare(
            r#"
            SELECT
                time_bucket,
                SUM(input_tokens) as input_tokens,
                SUM(output_tokens) as output_tokens,
                SUM(cache_read_tokens) as cache_read_tokens,
                SUM(cache_write_tokens) as cache_write_tokens,
                SUM(total_tokens) as total_tokens,
                SUM(request_count) as request_count
            FROM usage_stats
            WHERE granularity = ?1
            GROUP BY time_bucket
            HAVING time_bucket >= ?2 AND time_bucket <= ?3
            ORDER BY time_bucket ASC
            "#,
        )?;

        let start_bucket = granularity.time_bucket(&start);
        let end_bucket = granularity.time_bucket(&end);

        let rows = stmt.query_map(params![granularity_str, start_bucket, end_bucket], |row| {
            Ok(StatsEntry {
                time_bucket: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cache_read_tokens: row.get(3)?,
                cache_write_tokens: row.get(4)?,
                total_tokens: row.get(5)?,
                request_count: row.get(6)?,
            })
        })?;

        for row in rows {
            let entry = row?;
            summary.total_input_tokens += entry.input_tokens;
            summary.total_output_tokens += entry.output_tokens;
            summary.total_cache_read_tokens += entry.cache_read_tokens;
            summary.total_cache_write_tokens += entry.cache_write_tokens;
            summary.total_tokens += entry.total_tokens;
            summary.total_requests += entry.request_count;
            entries.push(entry);
        }

        let model_usage = self.get_model_usage_stats(start, end)?;

        Ok(TimeRangeStats {
            granularity,
            entries,
            summary,
            model_usage,
        })
    }

    pub fn get_model_usage_stats(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<ModelUsageEntry>> {
        let start_text = start.to_rfc3339();
        let end_text = end.to_rfc3339();

        self.read_query(
            r#"
            SELECT
                provider_id,
                model_id,
                SUM(input_tokens) as input_tokens,
                SUM(output_tokens) as output_tokens,
                SUM(cache_read_tokens) as cache_read_tokens,
                SUM(cache_write_tokens) as cache_write_tokens,
                SUM(total_tokens) as total_tokens,
                SUM(request_count) as request_count
            FROM usage_stats
            WHERE created_at >= :start AND created_at <= :end
            GROUP BY provider_id, model_id
            ORDER BY total_tokens DESC
            "#,
            named_params! {
                ":start": start_text,
                ":end": end_text,
            },
            |row| {
                Ok(map_row!(ModelUsageEntry, row,
                    provider_id: 0,
                    model_id: 1,
                    input_tokens: 2,
                    output_tokens: 3,
                    cache_read_tokens: 4,
                    cache_write_tokens: 5,
                    total_tokens: 6,
                    request_count: 7,
                ))
            },
        )
    }

    pub fn get_all_time_summary(&self) -> Result<UsageSummary> {
        let mut stmt = self.read_conn.prepare(
            r#"
            SELECT
                SUM(input_tokens),
                SUM(output_tokens),
                SUM(cache_read_tokens),
                SUM(cache_write_tokens),
                SUM(total_tokens),
                SUM(request_count)
            FROM usage_stats
            "#,
        )?;

        let row = stmt.query_row([], |row| {
            Ok(UsageSummary {
                total_input_tokens: row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                total_output_tokens: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                total_cache_read_tokens: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                total_cache_write_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                total_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                total_requests: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            })
        })?;

        Ok(row)
    }

    /// Record that a file was read by the model
    pub fn record_file_read(
        &self,
        session_id: Uuid,
        file_path: &str,
        read_at: DateTime<Utc>,
        mtime: Option<i64>,
        size: Option<i64>,
    ) -> Result<()> {
        self.write_conn.lock().unwrap().execute(
            r#"INSERT OR REPLACE INTO file_reads (session_id, file_path, read_at, mtime, size)
               VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![
                session_id.to_string(),
                file_path,
                read_at.to_rfc3339(),
                mtime,
                size,
            ],
        )?;
        Ok(())
    }

    /// Load all file reads for a session
    pub fn load_file_reads(&self, session_id: Uuid) -> Result<Vec<FileReadRecord>> {
        self.read_query(
            r#"SELECT file_path, read_at, mtime, size FROM file_reads WHERE session_id = :session_id"#,
            named_params! { ":session_id": session_id.to_string() },
            |row| {
                let read_at_str: String = row.get(1)?;
                Ok(FileReadRecord {
                    file_path: row.get(0)?,
                    read_at: DateTime::parse_from_rfc3339(&read_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    mtime: row.get(2)?,
                    size: row.get(3)?,
                })
            },
        )
    }

    /// Delete all file reads for a session
    pub fn clear_file_reads(&self, session_id: Uuid) -> Result<()> {
        self.write_execute(
            "DELETE FROM file_reads WHERE session_id = :session_id",
            named_params! { ":session_id": session_id.to_string() },
        )?;
        Ok(())
    }

    /// Export the specified sessions to a plain SQLite database file.
    ///
    /// The output database has the same schema but uses TEXT columns for
    /// `content`, `reasoning`, and `output_text` (i.e. no zstd compression).
    /// All other tables are copied verbatim. Only data belonging to the
    /// given sessions is included (plus global tables such as `meta`,
    /// `usage_stats`, and `memories`).
    pub fn export_to_sqlite(&self, session_ids: &[Uuid], output_path: &Path) -> Result<()> {
        // Remove existing file so we start fresh
        let _ = fs::remove_file(output_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create export directory {}", parent.display())
            })?;
        }

        let export_conn = Connection::open(output_path).with_context(|| {
            format!("failed to create export database {}", output_path.display())
        })?;
        export_conn.execute_batch("PRAGMA foreign_keys = OFF")?;
        export_conn.execute_batch(EXPORT_SCHEMA_SQL)?;

        // ── Session ID helpers ────────────────────────────────────────────
        let session_id_strs: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();

        let tx = export_conn.unchecked_transaction()?;
        // 1. meta ── copy all rows
        {
            let mut stmt = self.read_conn.prepare("SELECT key, value FROM meta")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut insert =
                tx.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)")?;
            for row in rows {
                let (key, value) = row?;
                insert.execute(params![key, value])?;
            }
        }

        // 2. sessions ── only the requested ones
        {
            let mut stmt = self.read_conn.prepare(
                "SELECT id, parent_session_id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, status, ended_at, context_summary, context_retained_from FROM sessions WHERE id = ?1",
            )?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO sessions (id, parent_session_id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, status, ended_at, context_summary, context_retained_from) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            for sid in &session_id_strs {
                let row = stmt.query_row(params![sid], |row| {
                    let parent: Option<String> = row.get(1)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        parent,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, i64>(12)?,
                    ))
                })?;
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10,
                    row.11, row.12
                ])?;
            }
        }

        // 3. session_workspaces
        {
            let mut stmt = self.read_conn.prepare(
                "SELECT session_id, workspace_root FROM session_workspaces WHERE session_id = ?1",
            )?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO session_workspaces (session_id, workspace_root) VALUES (?1, ?2)",
            )?;
            for sid in &session_id_strs {
                let row = stmt
                    .query_row(params![sid], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .optional()?;
                if let Some((sid, root)) = row {
                    insert.execute(params![sid, root])?;
                }
            }
        }

        // 4. session_instruction_sources
        {
            let mut stmt = self.read_conn.prepare(
                "SELECT session_id, source FROM session_instruction_sources WHERE session_id = ?1",
            )?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO session_instruction_sources (session_id, source) VALUES (?1, ?2)",
            )?;
            for sid in &session_id_strs {
                let rows = stmt.query_map(params![sid], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (sid, source) = row?;
                    insert.execute(params![sid, source])?;
                }
            }
        }

        // 5. session_reverts
        {
            let mut stmt = self
                .read_conn
                .prepare("SELECT session_id, message_id, redo_snapshot, created_at FROM session_reverts WHERE session_id = ?1")?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO session_reverts (session_id, message_id, redo_snapshot, created_at) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for sid in &session_id_strs {
                let row = stmt
                    .query_row(params![sid], |row| {
                        let redo: Option<String> =
                            read_opt_blob_maybe_text(row, 2)?.map(|bytes| decompress_text(&bytes));
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            redo,
                            row.get::<_, String>(3)?,
                        ))
                    })
                    .optional()?;
                if let Some((sid, msg_id, redo, created)) = row {
                    insert.execute(params![sid, msg_id, redo, created])?;
                }
            }
        }

        // 6. messages ── decompress content & reasoning
        {
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO messages (id, session_id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, rtk_rewritten, thinking_level) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            )?;
            for sid in session_ids {
                let messages = self.load_messages(*sid)?;
                for msg in &messages {
                    let tool_calls = serde_json::to_string(&msg.tool_calls)
                        .context("failed to serialize tool calls")?;
                    let attachments = serde_json::to_string(&msg.attachments)
                        .context("failed to serialize attachments")?;
                    let metadata = serde_json::to_string(&msg.metadata)
                        .context("failed to serialize metadata")?;
                    let mode = msg
                        .mode
                        .as_ref()
                        .map(|m| serde_json::to_string(&m).unwrap_or_default());
                    let thinking_level = msg.thinking_level.as_ref().map(|t| t.to_string());
                    insert.execute(params![
                        msg.id.to_string(),
                        sid.to_string(),
                        msg.role.db_value(),
                        msg.content, // plain text, already decompressed
                        attachments,
                        if msg.reasoning.is_empty() {
                            None
                        } else {
                            Some(&msg.reasoning)
                        },
                        tool_calls,
                        msg.tool_call_id,
                        msg.tool_name,
                        metadata,
                        msg.created_at.to_rfc3339(),
                        msg.completed_at.map(|t| t.to_rfc3339()),
                        if msg.streaming { 1_i64 } else { 0_i64 },
                        msg.input_tokens,
                        msg.output_tokens,
                        msg.total_tokens,
                        msg.cache_read_tokens,
                        msg.cache_write_tokens,
                        msg.model_id,
                        msg.tokens_per_second,
                        msg.snapshot_hash,
                        msg.patch_files,
                        msg.file_diffs,
                        mode,
                        if msg.rtk_rewritten { 1_i64 } else { 0_i64 },
                        thinking_level,
                    ])?;
                }
            }
        }

        // 8. todos
        {
            let mut stmt = self
                .read_conn
                .prepare("SELECT session_id, position, content, status FROM todos WHERE session_id = ?1")?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO todos (session_id, position, content, status) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for sid in &session_id_strs {
                let rows = stmt.query_map(params![sid], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    let (sid, pos, content, status) = row?;
                    insert.execute(params![sid, pos, content, status])?;
                }
            }
        }

        // 9. tool_permissions
        {
            let mut stmt = self
                .read_conn
                .prepare("SELECT session_id, tool_name, allowed, created_at FROM tool_permissions WHERE session_id = ?1")?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO tool_permissions (session_id, tool_name, allowed, created_at) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for sid in &session_id_strs {
                let rows = stmt.query_map(params![sid], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    let (sid, name, allowed, created) = row?;
                    insert.execute(params![sid, name, allowed, created])?;
                }
            }
        }

        // 10. file_reads
        {
            let mut stmt = self
                .read_conn
                .prepare("SELECT session_id, file_path, read_at, mtime, size FROM file_reads WHERE session_id = ?1")?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO file_reads (session_id, file_path, read_at, mtime, size) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for sid in &session_id_strs {
                let rows = stmt.query_map(params![sid], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                })?;
                for row in rows {
                    let (sid, path, read_at, mtime, size) = row?;
                    insert.execute(params![sid, path, read_at, mtime, size])?;
                }
            }
        }

        // 11. usage_stats ── global table, copy all
        {
            let mut stmt = self.read_conn.prepare(
                "SELECT id, provider_id, model_id, time_bucket, granularity, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, total_tokens, request_count, created_at, updated_at FROM usage_stats",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO usage_stats (id, provider_id, model_id, time_bucket, granularity, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, total_tokens, request_count, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            for row in rows {
                let (id, p, m, tb, g, it, ot, cr, cw, tt, rc, ca, ua) = row?;
                insert.execute(params![id, p, m, tb, g, it, ot, cr, cw, tt, rc, ca, ua])?;
            }
        }

        // 12. memories ── global table, copy all (decompress content for export)
        {
            let mut stmt = self.read_conn.prepare(
                "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active FROM memories",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    read_blob_maybe_text(row, 4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO memories (id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for row in rows {
                let (id, wr, mt, title, content, tags, src_sid, ca, ua, uc, active) = row?;
                insert.execute(params![
                    id, wr, mt, title, content, tags, src_sid, ca, ua, uc, active
                ])?;
            }
        }

        // 13. gateway_chat_sessions ── only for the exported sessions
        {
            let mut stmt = self
                .read_conn
                .prepare("SELECT platform, chat_key, session_id, updated_at FROM gateway_chat_sessions WHERE session_id = ?1")?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO gateway_chat_sessions (platform, chat_key, session_id, updated_at) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for sid in &session_id_strs {
                let rows = stmt.query_map(params![sid], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    let (platform, chat_key, sid, updated) = row?;
                    insert.execute(params![platform, chat_key, sid, updated])?;
                }
            }
        }

        // 14. gateway_chat_models ── global table, copy all
        {
            let mut stmt = self.read_conn.prepare(
                "SELECT platform, chat_key, provider_id, model_id, updated_at FROM gateway_chat_models",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO gateway_chat_models (platform, chat_key, provider_id, model_id, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in rows {
                let (platform, chat_key, p, m, u) = row?;
                insert.execute(params![platform, chat_key, p, m, u])?;
            }
        }

        tx.commit()?;
        export_conn.execute_batch("PRAGMA foreign_keys = ON")?;
        Ok(())
    }

    /// Export session messages to JSONL format.
    /// Each line is a JSON object representing one message.
    /// Returns the path to the exported file.
    pub fn export_session_to_jsonl(&self, session_id: Uuid, export_dir: &Path) -> Result<PathBuf> {
        // Create export directory if it doesn't exist
        fs::create_dir_all(export_dir).context("failed to create export directory")?;

        let file_path = export_dir.join(format!("{session_id}.jsonl"));

        let messages = self.load_messages(session_id)?;

        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&file_path)
            .context("failed to open export file")?;

        let mut writer = std::io::BufWriter::new(file);

        for message in messages {
            let json_line = serde_json::to_string(&message)?;
            use std::io::Write;
            writeln!(writer, "{}", json_line).context("failed to write JSON line")?;
        }

        Ok(file_path)
    }
}

/// Helper: read a NOT NULL column that may be BLOB or TEXT, returning raw bytes.
///
/// Unlike `read_blob_maybe_text` (which decompresses), this returns the
/// raw bytes so the caller can decompress later.  Falls back to reading
/// as TEXT for backwards compatibility with older databases.
pub fn load_session_messages(db: &Connection, session_id: Uuid) -> Result<Vec<Message>> {
    let mut stmt = db.prepare(
        "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, rtk_rewritten, thinking_level FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC"
    )?;

    let rows = stmt.query_map(
        rusqlite::params![session_id.to_string()],
        RawMessageRow::from_row,
    )?;

    let mut messages = Vec::new();
    for row in rows {
        let raw = row?;
        messages.push(raw.into_message()?);
    }
    Ok(messages)
}

fn read_blob_maybe_text_bytes(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Vec<u8>> {
    match row.get::<_, Vec<u8>>(idx) {
        Ok(v) => Ok(v),
        Err(_) => {
            let text: String = row.get(idx)?;
            Ok(text.into_bytes())
        }
    }
}

/// Helper: read a NOT NULL column that may be BLOB or TEXT, returning decompressed String.
///
/// Falls back to reading as TEXT (plain text) for backwards compatibility
/// with databases created before zstd compression was enabled for the column.
fn read_blob_maybe_text(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<String> {
    match row.get::<_, Vec<u8>>(idx) {
        Ok(bytes) => Ok(decompress_text(&bytes)),
        Err(_) => {
            // Fallback for old databases where the column is TEXT
            let text: String = row.get(idx)?;
            Ok(text)
        }
    }
}

/// Helper: read an optional column that may be BLOB or TEXT, returning raw bytes.
///
/// This handles backwards compatibility with older databases where the column
/// is still TEXT — rusqlite will fail to read TEXT as `Option<Vec<u8>>`, so
/// we fall back to reading as `Option<String>` and convert to bytes.
fn read_opt_blob_maybe_text(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Option<Vec<u8>>> {
    match row.get::<_, Option<Vec<u8>>>(idx) {
        Ok(v) => Ok(v),
        Err(_) => {
            // Fallback for old databases where the column is TEXT
            let text: Option<String> = row.get(idx)?;
            Ok(text.map(|s| s.into_bytes()))
        }
    }
}

/// Record of a file read by the model
#[derive(Debug, Clone)]
pub struct FileReadRecord {
    pub file_path: String,
    pub read_at: DateTime<Utc>,
    pub mtime: Option<i64>,
    pub size: Option<i64>,
}
