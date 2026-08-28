//! SQLite storage layer — sessions, messages, schema, and tool metadata.
//!
//! The main entry point is [`Database::open`] which creates the database,
//! runs schema migrations, and provides factory methods for [`SessionStore`].

pub mod compression;
pub mod database;
pub mod migration;
pub mod schema;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rayon::prelude::*;
use rusqlite::{
    Connection, OptionalExtension, named_params, params, params_from_iter, types::Type,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tidev_llm::message::{Message, MessageRole};
use uuid::Uuid;

use crate::compression::{compress_text, decompress_text};
use crate::schema::SESSION_SELECT_COLUMNS;

/// Build a struct literal from a SQLite row.
macro_rules! map_row {
    ($struct:tt, $row:expr, $($field:ident: $idx:expr => $conv:expr),+ $(,)?) => {
        $struct {
            $($field: $conv),+
        }
    };
    ($struct:tt, $row:expr, $($field:ident: $idx:expr),+ $(,)?) => {
        $struct {
            $($field: $row.get($idx)?),+
        }
    };
}

pub struct SessionStore {
    /// Shared write connection (behind Mutex for thread-safety).
    write_conn: Arc<Mutex<Connection>>,
    /// Connection for read operations (SELECT, behind Mutex for Sync).
    read_conn: Mutex<Connection>,
    path: PathBuf,
}

impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self {
            write_conn: Arc::new(Mutex::new(
                crate::database::open_write_conn(&self.path)
                    .expect("failed to clone SessionStore write_conn"),
            )),
            read_conn: Mutex::new(
                Connection::open(&self.path).expect("failed to clone SessionStore read_conn"),
            ),
            path: self.path.clone(),
        }
    }
}

fn parse_datetime(value: &str) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

/// Convert a string to `Some(s)` only if non-empty; otherwise `None`.
///
/// Used when reading `NOT NULL DEFAULT ''` columns where the empty string
/// should be treated as absence (i.e. equivalent to SQL `NULL`).
fn opt_non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

/// Raw per-row data from the messages table, collected before
/// any CPU-intensive decompression or JSON parsing.
///
/// Phase 1 of [`SessionStore::load_messages`] populates this from SQLite;
/// Phase 2 processes rows in parallel via rayon.
struct RawMessageRow {
    id: String,
    role: String,
    content: Vec<u8>,
    attachments: String,
    reasoning: Vec<u8>,
    tool_calls: Vec<u8>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    metadata: Vec<u8>,
    created_at: String,
    completed_at: Option<String>,
    streaming: bool,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    model_id: Option<String>,
    tokens_per_second: Option<f64>,
    thinking_level: Option<String>,
    app_data: Vec<u8>,
}

/// Application-owned fields stored alongside a protocol message.
///
/// These values are intentionally kept out of the LLM message payload. The
/// mode value remains in its database JSON representation so old databases
/// can be read without rewriting rows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageAppData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_files: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_diffs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error: Option<ProviderErrorData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_completed_at: Option<DateTime<Utc>>,
}

/// Application-owned details for a provider failure shown in the chat.
///
/// The associated user message identifies the turn that can be retried. This
/// metadata is kept outside the protocol [`Message`] so it cannot affect LLM
/// request bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderErrorData {
    pub message: String,
    pub retryable: bool,
    pub request_id: u64,
    pub user_message_id: Option<Uuid>,
}

impl RawMessageRow {
    /// Decompress zstd blobs, parse JSON, and build a [`Message`].
    fn decompress_and_parse(self) -> Message {
        // If role is empty, return a default message
        // (mirrors original early-return for corrupt rows).
        if self.role.is_empty() {
            return Message::new(MessageRole::User, "");
        }

        let metadata: tidev_llm::message::ToolMetadata =
            serde_json::from_str(&decompress_text(&self.metadata)).unwrap_or_default();

        let content = decompress_text(&self.content);

        let attachments: Vec<tidev_llm::message::MessageAttachment> =
            serde_json::from_str(&self.attachments).unwrap_or_default();

        let reasoning = decompress_text(&self.reasoning);

        let tool_calls: Vec<tidev_llm::message::ToolCall> =
            serde_json::from_str(&decompress_text(&self.tool_calls)).unwrap_or_default();

        let thinking_level = self
            .thinking_level
            .and_then(|t| serde_json::from_str(&t).ok());

        let app_data: MessageAppData = if self.app_data.is_empty() {
            MessageAppData::default()
        } else {
            serde_json::from_str(&decompress_text(&self.app_data)).unwrap_or_default()
        };

        Message {
            id: Uuid::parse_str(&self.id).unwrap_or_default(),
            role: MessageRole::from_db_value(&self.role),
            content,
            attachments,
            reasoning,
            tool_calls,
            tool_call_id: self.tool_call_id,
            tool_name: self.tool_name,
            metadata,
            created_at: DateTime::parse_from_rfc3339(&self.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_default(),
            completed_at: self.completed_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            streaming: self.streaming,
            input_tokens: self.input_tokens.map(|v| v as u32),
            output_tokens: self.output_tokens.map(|v| v as u32),
            total_tokens: self.total_tokens.map(|v| v as u32),
            cache_read_tokens: self.cache_read_tokens.map(|v| v as u32),
            cache_write_tokens: self.cache_write_tokens.map(|v| v as u32),
            model_id: self.model_id,
            tokens_per_second: self.tokens_per_second.map(|v| v as f32),
            thinking_level,
            reasoning_started_at: app_data.reasoning_started_at,
            reasoning_completed_at: app_data.reasoning_completed_at,
        }
    }
}

impl SessionStore {
    /// Execute a read operation against the read connection.
    ///
    /// The closure receives a `&Connection` and runs while holding the
    /// read lock. Because every query method returns owned data (no
    /// references escape the closure), this preserves the borrow checker's
    /// safety guarantees.
    fn read<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        let conn = self.read_conn.lock().unwrap();
        f(&conn)
    }

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
        self.read(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params, f)?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
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
            provider_display_name: if provider_display_name.trim().is_empty() {
                provider_id.clone()
            } else {
                provider_display_name
            },
            model_id: model_id.clone(),
            model_display_name: if model_display_name.trim().is_empty() {
                model_id.clone()
            } else {
                model_display_name
            },
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
            snapshot_start_hash: None,
        })
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub snapshot_start_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSessionCount {
    pub workspace_root: String,
    pub session_count: i64,
}

/// A tool output retained separately from the protocol message content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolOutputRecord {
    pub id: String,
    pub session_id: Uuid,
    pub message_id: Uuid,
    pub tool_call_id: String,
    pub tool_name: String,
    pub byte_size: usize,
    pub line_count: usize,
    pub created_at: String,
}

/// Status and optional content of a stored tool output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ToolOutputContent {
    Available {
        record: ToolOutputRecord,
        output: String,
    },
    Expired {
        record: ToolOutputRecord,
    },
}

/// A message together with application-owned fields needed for inspection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredMessageView {
    pub sequence: usize,
    pub message: Message,
    pub app_data: MessageAppData,
    pub tool_output: Option<ToolOutputRecord>,
}

/// Complete read-only inspection data for one session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInspection {
    pub session: SessionRecord,
    pub messages: Vec<StoredMessageView>,
}

#[derive(Serialize)]
struct JsonlMessageRecord<'a> {
    session_id: Uuid,
    sequence: usize,
    message: &'a Message,
}

/// Token statistics for a session.
#[derive(Debug, Clone)]
pub struct SessionTokenStats {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Token usage recorded for one completed assistant response.
///
/// This deliberately contains metadata only. Message content is not loaded by
/// the statistics endpoints, which keeps the read path small and avoids
/// exposing conversation payloads to the web layer.
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub session_id: String,
    pub title: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub session_created_at: String,
    pub session_updated_at: String,
    pub created_at: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// Session CRUD
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Create a new session.
    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &self,
        session_id: Uuid,
        workspace_root: &str,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
        parent_session_id: Option<Uuid>,
        snapshot_start_hash: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, workspace_root, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, snapshot_start_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![session_id.to_string(), parent_session_id.map(|id| id.to_string()), workspace_root, provider_id, provider_display_name, model_id, model_display_name, title, now, now, snapshot_start_hash],
        )?;
        Ok(())
    }

    /// Load session record by ID.
    pub fn load_session(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let sql = format!("SELECT {SESSION_SELECT_COLUMNS} FROM sessions s WHERE s.id = ?1");
        self.read(|conn| {
            conn.query_row(&sql, params![session_id.to_string()], |row| {
                Ok(map_row!(SessionRecord, row,
                    session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                    workspace_root: 14 => row.get::<_, String>(14)?,
                    provider_id: 2 => row.get::<_, String>(2)?,
                    provider_display_name: 3 => row.get::<_, String>(3)?,
                    model_id: 4 => row.get::<_, String>(4)?,
                    model_display_name: 5 => row.get::<_, String>(5)?,
                    title: 6 => row.get::<_, String>(6)?,
                    created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                    updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                    status: 9 => row.get::<_, String>(9)?,
                    ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                    context_summary: 11 => opt_non_empty(row.get::<_, String>(11)?),
                    context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                    system_prompt: 13 => row.get::<_, String>(13)?,
                    snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
                ))
            })
            .optional()
            .map_err(Into::into)
        })
    }

    /// Load a conversation (session + messages).
    pub fn load_conversation(&self, session_id: Uuid) -> Result<Option<schema::Conversation>> {
        let Some(session) = self.load_session(session_id)? else {
            return Ok(None);
        };
        let messages = self.load_messages(session_id)?;
        Ok(Some(schema::Conversation {
            session_id: session.session_id,
            parent_session_id: session.parent_session_id,
            workspace_root: session.workspace_root,
            provider_id: session.provider_id,
            provider_display_name: session.provider_display_name,
            model_id: session.model_id,
            model_display_name: session.model_display_name,
            title: session.title,
            created_at: session.created_at,
            updated_at: session.updated_at,
            context_summary: session.context_summary,
            context_retained_from: session.context_retained_from,
            messages,
            revert_message_id: None,
        }))
    }

    /// Load a session and all read-only data needed to inspect its history.
    pub fn load_session_inspection(&self, session_id: Uuid) -> Result<Option<SessionInspection>> {
        let Some(session) = self.load_session(session_id)? else {
            return Ok(None);
        };

        let messages = self.load_messages(session_id)?;
        let app_data = self.load_message_app_data(session_id)?;
        let tool_outputs = self.load_tool_outputs(session_id)?;
        let messages = messages
            .into_iter()
            .enumerate()
            .map(|(sequence, message)| {
                let message_id = message.id;
                StoredMessageView {
                    sequence,
                    message,
                    app_data: app_data.get(&message_id).cloned().unwrap_or_default(),
                    tool_output: tool_outputs.get(&message_id).cloned(),
                }
            })
            .collect();

        Ok(Some(SessionInspection { session, messages }))
    }

    /// List all sessions ordered by creation time (newest first).
    pub fn list_sessions(&self, limit: i64, offset: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE s.parent_session_id IS NULL \
             ORDER BY s.created_at DESC LIMIT ?1 OFFSET ?2"
        );
        self.read(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(map_row!(SessionRecord, row,
                session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                workspace_root: 14 => row.get::<_, String>(14)?,
                provider_id: 2 => row.get::<_, String>(2)?,
                provider_display_name: 3 => row.get::<_, String>(3)?,
                model_id: 4 => row.get::<_, String>(4)?,
                model_display_name: 5 => row.get::<_, String>(5)?,
                title: 6 => row.get::<_, String>(6)?,
                created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                status: 9 => row.get::<_, String>(9)?,
                ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                context_summary: 11 => opt_non_empty(row.get::<_, String>(11)?),
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => row.get::<_, String>(13)?,
                snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
        })
    }

    /// List all sessions including children (no parent_session_id filter).
    /// Used internally for subsession navigation; session panel should use list_sessions instead.
    pub fn list_sessions_unfiltered(&self, limit: i64, offset: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             ORDER BY s.created_at DESC LIMIT ?1 OFFSET ?2"
        );
        self.read(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(map_row!(SessionRecord, row,
                session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                workspace_root: 14 => row.get::<_, String>(14)?,
                provider_id: 2 => row.get::<_, String>(2)?,
                provider_display_name: 3 => row.get::<_, String>(3)?,
                model_id: 4 => row.get::<_, String>(4)?,
                model_display_name: 5 => row.get::<_, String>(5)?,
                title: 6 => row.get::<_, String>(6)?,
                created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                status: 9 => row.get::<_, String>(9)?,
                ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                context_summary: 11 => opt_non_empty(row.get::<_, String>(11)?),
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => row.get::<_, String>(13)?,
                    snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
        })
    }

    /// List sessions for a specific workspace, ordered by creation time (newest first).
    pub fn list_sessions_for_workspace(
        &self,
        workspace_root: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE s.workspace_root = ?1 AND s.parent_session_id IS NULL \
             ORDER BY s.created_at DESC LIMIT ?2 OFFSET ?3"
        );
        self.read(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![workspace_root, limit, offset],
                |row| {
                    Ok(map_row!(SessionRecord, row,
                        session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                        parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                        workspace_root: 14 => row.get::<_, String>(14)?,
                        provider_id: 2 => row.get::<_, String>(2)?,
                        provider_display_name: 3 => row.get::<_, String>(3)?,
                        model_id: 4 => row.get::<_, String>(4)?,
                        model_display_name: 5 => row.get::<_, String>(5)?,
                        title: 6 => row.get::<_, String>(6)?,
                        created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                        updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                        status: 9 => row.get::<_, String>(9)?,
                        ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                        context_summary: 11 => opt_non_empty(row.get::<_, String>(11)?),
                        context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                        system_prompt: 13 => row.get::<_, String>(13)?,
                    snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
                    ))
                },
            )?;
            let mut sessions = Vec::new();
            for row in rows {
                sessions.push(row?);
            }
            Ok(sessions)
        })
    }

    /// Update session metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn update_session(
        &self,
        session_id: Uuid,
        title: Option<&str>,
        status: Option<&str>,
        context_summary: Option<&str>,
        context_retained_from: Option<usize>,
        system_prompt: Option<&str>,
        provider_id: Option<&str>,
        provider_display_name: Option<&str>,
        model_id: Option<&str>,
        model_display_name: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        let mut sets = vec!["updated_at = ?1".to_string()];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
        let mut idx = 2;

        if let Some(v) = title {
            sets.push(format!("title = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = status {
            sets.push(format!("status = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = context_summary {
            sets.push(format!("context_summary = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = context_retained_from {
            sets.push(format!("context_retained_from = ?{idx}"));
            params.push(Box::new(v as i64));
            idx += 1;
        }
        if let Some(v) = system_prompt {
            sets.push(format!("system_prompt = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = provider_id {
            sets.push(format!("provider_id = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = provider_display_name {
            sets.push(format!("provider_display_name = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = model_id {
            sets.push(format!("model_id = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = model_display_name {
            sets.push(format!("model_display_name = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }

        let sql = format!("UPDATE sessions SET {} WHERE id = ?{idx}", sets.join(", "));
        params.push(Box::new(session_id.to_string()));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        stmt.execute(param_refs.as_slice())?;
        Ok(())
    }

    /// Persist the snapshot start hash for a session.
    pub fn update_session_start_hash(
        &self,
        session_id: Uuid,
        snapshot_start_hash: &str,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET snapshot_start_hash = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                snapshot_start_hash,
                Utc::now().to_rfc3339(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// End a session (set status to 'ended' and ended_at).
    pub fn end_session(&self, session_id: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'ended', ended_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Delete a session and all related data (CASCADE).
    pub fn delete_session(&self, session_id: Uuid) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
    }

    /// Delete multiple sessions at once.
    pub fn delete_sessions(&self, session_ids: &[Uuid]) -> Result<()> {
        let ids: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
        self.delete_sessions_by_ids(&ids)
    }

    /// Return sessions older than the given duration.
    pub fn get_sessions_older_than_preview(
        &self,
        duration: Duration,
    ) -> Result<Vec<SessionRecord>> {
        let cutoff = Utc::now() - duration;
        let cutoff_text = cutoff.to_rfc3339();
        self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 WHERE s.updated_at < :cutoff AND s.parent_session_id IS NULL \
                 ORDER BY s.workspace_root, s.updated_at DESC"
            ),
            named_params! { ":cutoff": cutoff_text },
            Self::session_from_row,
        )
    }

    /// Delete sessions older than the given duration. Returns the deleted records.
    pub fn delete_sessions_older_than(&self, duration: Duration) -> Result<Vec<SessionRecord>> {
        let cutoff = Utc::now() - duration;
        let cutoff_text = cutoff.to_rfc3339();

        let records: Vec<SessionRecord> = self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 WHERE s.updated_at < :cutoff AND s.parent_session_id IS NULL \
                 ORDER BY s.updated_at DESC"
            ),
            named_params! { ":cutoff": cutoff_text },
            Self::session_from_row,
        )?;

        let session_ids: Vec<String> = records.iter().map(|r| r.session_id.to_string()).collect();
        self.delete_sessions_by_ids(&session_ids)?;

        Ok(records)
    }

    /// Delete all sessions in a workspace. Returns the deleted records.
    pub fn delete_sessions_in_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<SessionRecord>> {
        let root = workspace_root.display().to_string();

        let records: Vec<SessionRecord> = self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 WHERE s.workspace_root = :workspace_root AND s.parent_session_id IS NULL \
                 ORDER BY s.updated_at DESC"
            ),
            named_params! { ":workspace_root": root },
            Self::session_from_row,
        )?;

        let session_ids: Vec<String> = records.iter().map(|r| r.session_id.to_string()).collect();
        self.delete_sessions_by_ids(&session_ids)?;

        Ok(records)
    }

    /// Export a session as JSONL file. Returns the file path.
    pub fn export_session_to_jsonl(&self, session_id: Uuid, export_dir: &Path) -> Result<PathBuf> {
        let messages = self.load_messages(session_id)?;
        std::fs::create_dir_all(export_dir)?;
        let file_path = export_dir.join(format!("session_{session_id}.jsonl"));
        let mut file = std::fs::File::create(&file_path)?;
        for msg in &messages {
            let line = serde_json::to_string(msg)?;
            writeln!(file, "{line}")?;
        }
        Ok(file_path)
    }

    /// Export one or more sessions to a single JSONL message stream.
    ///
    /// Each line contains the session ID, a stable per-session sequence, and
    /// the protocol message. This format is intended for CLI inspection and
    /// scripting; the existing TUI export keeps its legacy one-message shape.
    pub fn export_to_jsonl(&self, session_ids: &[Uuid], output_path: &Path) -> Result<usize> {
        for session_id in session_ids {
            if self.load_session(*session_id)?.is_none() {
                anyhow::bail!("session not found: {session_id}");
            }
        }

        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create export directory {}", parent.display())
            })?;
        }

        let mut file = fs::File::create(output_path)
            .with_context(|| format!("failed to create JSONL export {}", output_path.display()))?;
        let mut message_count = 0;

        for session_id in session_ids {
            let messages = self.load_messages(*session_id)?;
            for (sequence, message) in messages.iter().enumerate() {
                let record = JsonlMessageRecord {
                    session_id: *session_id,
                    sequence,
                    message,
                };
                serde_json::to_writer(&mut file, &record)?;
                file.write_all(b"\n")?;
                message_count += 1;
            }
        }

        file.flush()?;
        Ok(message_count)
    }

    /// Count sessions in a workspace.
    pub fn get_current_workspace_sessions_count(&self, workspace_root: &Path) -> Result<i64> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE workspace_root = ?1 AND parent_session_id IS NULL",
                params![workspace_root.to_string_lossy().to_string()],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
    }

    /// Alias for load_session (compatibility).
    pub fn load_session_record(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        self.load_session(session_id)
    }

    /// Load all retained tool outputs for a session.
    pub fn load_tool_outputs(&self, session_id: Uuid) -> Result<HashMap<Uuid, ToolOutputRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, byte_size, line_count, created_at \
                 FROM tool_outputs WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                let id: String = row.get(0)?;
                let stored_session_id =
                    Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(session_id);
                let message_id = Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default();
                let tool_call_id: String = row.get(3)?;
                let tool_name: String = row.get(4)?;
                let byte_size: usize = row.get::<_, i64>(5)? as usize;
                let line_count: usize = row.get::<_, i64>(6)? as usize;
                let created_at: String = row.get(7)?;

                Ok(ToolOutputRecord {
                    id,
                    session_id: stored_session_id,
                    message_id,
                    tool_call_id,
                    tool_name,
                    byte_size,
                    line_count,
                    created_at,
                })
            })?;
            let mut tool_outputs = HashMap::new();
            for row in rows {
                let record = row?;
                tool_outputs.insert(record.message_id, record);
            }
            Ok(tool_outputs)
        })
    }

    /// Load tool output content by output id, message_id, or tool_call_id.
    pub fn load_tool_output(&self, id_or_alias: &str) -> Result<Option<ToolOutputContent>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at \
                 FROM tool_outputs \
                 WHERE id = ?1 OR message_id = ?1 OR tool_call_id = ?1 \
                 LIMIT 1",
            )?;
            let mut rows = stmt.query_map(params![id_or_alias], |row| {
                let id: String = row.get(0)?;
                let session_id = Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default();
                let message_id = Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default();
                let tool_call_id: String = row.get(3)?;
                let tool_name: String = row.get(4)?;
                let output_blob: Option<Vec<u8>> = row.get(5)?;
                let byte_size: usize = row.get::<_, i64>(6)? as usize;
                let line_count: usize = row.get::<_, i64>(7)? as usize;
                let created_at: String = row.get(8)?;

                let record = ToolOutputRecord {
                    id,
                    session_id,
                    message_id,
                    tool_call_id,
                    tool_name,
                    byte_size,
                    line_count,
                    created_at,
                };

                match output_blob {
                    Some(blob) => Ok(ToolOutputContent::Available {
                        record,
                        output: decompress_text(&blob),
                    }),
                    None => Ok(ToolOutputContent::Expired { record }),
                }
            })?;
            match rows.next() {
                Some(Ok(content)) => Ok(Some(content)),
                _ => Ok(None),
            }
        })
    }

    /// Save tool output for a tool call.
    pub fn save_tool_output(
        &self,
        id: &str,
        session_id: Uuid,
        message_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        raw_output: &str,
    ) -> Result<()> {
        let compressed = compress_text(raw_output);
        let byte_size = raw_output.len() as i64;
        let line_count = raw_output.lines().count() as i64;
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "INSERT OR REPLACE INTO tool_outputs \
             (id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at) \
             VALUES (:id, :session_id, :message_id, :tool_call_id, :tool_name, :output, :byte_size, :line_count, :created_at)",
            named_params! {
                ":id": id,
                ":session_id": session_id.to_string(),
                ":message_id": message_id.to_string(),
                ":tool_call_id": tool_call_id,
                ":tool_name": tool_name,
                ":output": compressed,
                ":byte_size": byte_size,
                ":line_count": line_count,
                ":created_at": now,
            },
        )?;
        Ok(())
    }

    /// Clear big payloads for tool outputs older than `max_age_days` (tombstone pattern).
    /// Sets `output` column to NULL while preserving metadata for inspection.
    pub fn clear_expired_tool_outputs(&self, max_age_days: i64) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(max_age_days)).to_rfc3339();
        self.write_execute(
            "UPDATE tool_outputs SET output = NULL WHERE created_at < :cutoff AND output IS NOT NULL",
            named_params! { ":cutoff": cutoff },
        )
    }

    /// Delete tombstone records older than `max_age_days`.
    pub fn delete_tombstones_older_than(&self, max_age_days: i64) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(max_age_days)).to_rfc3339();
        self.write_execute(
            "DELETE FROM tool_outputs WHERE created_at < :cutoff AND output IS NULL",
            named_params! { ":cutoff": cutoff },
        )
    }

    /// Count total number of sessions.
    pub fn count_sessions(&self) -> Result<i64> {
        self.read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
                .map_err(Into::into)
        })
    }

    /// Count sessions per workspace.
    pub fn count_sessions_by_workspace(&self) -> Result<Vec<WorkspaceSessionCount>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.workspace_root, COUNT(*) as cnt FROM sessions s \
                 WHERE s.workspace_root != '' \
                 GROUP BY s.workspace_root ORDER BY cnt DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(WorkspaceSessionCount {
                    workspace_root: row.get(0)?,
                    session_count: row.get(1)?,
                })
            })?;
            let mut counts = Vec::new();
            for row in rows {
                counts.push(row?);
            }
            Ok(counts)
        })
    }

    /// Search sessions by title.
    pub fn search_sessions(&self, query: &str, limit: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE s.title LIKE ?1 OR s.id = ?2 \
             ORDER BY s.created_at DESC LIMIT ?3"
        );
        let pattern = format!("%{}%", query);
        let id_match = Uuid::parse_str(query)
            .map(|id| id.to_string())
            .unwrap_or_default();
        self.read(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, id_match, limit], |row| {
            Ok(map_row!(SessionRecord, row,
                session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                workspace_root: 14 => row.get::<_, String>(14)?,
                provider_id: 2 => row.get::<_, String>(2)?,
                provider_display_name: 3 => row.get::<_, String>(3)?,
                model_id: 4 => row.get::<_, String>(4)?,
                model_display_name: 5 => row.get::<_, String>(5)?,
                title: 6 => row.get::<_, String>(6)?,
                created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                status: 9 => row.get::<_, String>(9)?,
                ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                context_summary: 11 => opt_non_empty(row.get::<_, String>(11)?),
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => row.get::<_, String>(13)?,
                    snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
        })
    }

    /// Get workspaces that have sessions.
    pub fn list_workspaces(&self, limit: i64, offset: i64) -> Result<Vec<String>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT workspace_root FROM sessions \
                 WHERE workspace_root != '' \
                 ORDER BY workspace_root LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt.query_map(params![limit, offset], |row| row.get(0))?;
            let mut workspaces = Vec::new();
            for row in rows {
                workspaces.push(row?);
            }
            Ok(workspaces)
        })
    }
}

// ---------------------------------------------------------------------------
// Message CRUD
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Insert a single message row (no session timestamp update).
    fn insert_message(conn: &Connection, session_id: Uuid, msg: &Message) -> Result<()> {
        Self::insert_message_with_app_data(conn, session_id, msg, &MessageAppData::default())
    }

    fn insert_message_with_app_data(
        conn: &Connection,
        session_id: Uuid,
        msg: &Message,
        app_data: &MessageAppData,
    ) -> Result<()> {
        let now = msg.created_at.to_rfc3339();
        let completed = msg.completed_at.map(|t| t.to_rfc3339());
        let mut combined_app_data = app_data.clone();
        if combined_app_data.reasoning_started_at.is_none() {
            combined_app_data.reasoning_started_at = msg.reasoning_started_at;
        }
        if combined_app_data.reasoning_completed_at.is_none() {
            combined_app_data.reasoning_completed_at = msg.reasoning_completed_at;
        }
        let app_data_json =
            serde_json::to_string(&combined_app_data).unwrap_or_else(|_| "{}".to_string());

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, \
             tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, \
             streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, \
             cache_write_tokens, model_id, tokens_per_second, mode, thinking_level, \
             app_data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
             ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                msg.id.to_string(),
                session_id.to_string(),
                msg.role.db_value(),
                compress_text(&msg.content),
                serde_json::to_string(&msg.attachments).unwrap_or_else(|_| "[]".to_string()),
                compress_text(&msg.reasoning),
                compress_text(
                    &serde_json::to_string(&msg.tool_calls).unwrap_or_else(|_| "[]".to_string())
                ),
                msg.tool_call_id.as_deref(),
                msg.tool_name.as_deref(),
                compress_text(
                    &serde_json::to_string(&msg.metadata).unwrap_or_else(|_| "{}".to_string())
                ),
                now,
                completed,
                msg.streaming as i64,
                msg.input_tokens,
                msg.output_tokens,
                msg.total_tokens,
                msg.cache_read_tokens,
                msg.cache_write_tokens,
                msg.model_id,
                msg.tokens_per_second,
                app_data.mode.as_deref(),
                msg.thinking_level
                    .as_ref()
                    .map(|t| serde_json::to_string(t).unwrap_or_default()),
                compress_text(&app_data_json),
            ],
        )?;
        Ok(())
    }

    /// Append multiple messages in a single transaction.
    ///
    /// More efficient than calling [`append_message`] in a loop because it
    /// acquires the write lock once and wraps all INSERTs + session timestamp
    /// update in one SQLite transaction.
    pub fn append_messages(&self, session_id: Uuid, messages: &[Message]) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        for msg in messages {
            Self::insert_message(&tx, session_id, msg)?;
        }
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Append protocol messages with their application-owned fields.
    pub fn append_messages_with_app_data(
        &self,
        session_id: Uuid,
        messages: &[Message],
        app_data: &HashMap<Uuid, MessageAppData>,
    ) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        for msg in messages {
            let fallback;
            let data = match app_data.get(&msg.id) {
                Some(data) => data,
                None => {
                    fallback = MessageAppData::default();
                    &fallback
                }
            };
            Self::insert_message_with_app_data(&tx, session_id, msg, data)?;
        }
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Append a single message to a session.
    pub fn append_message(&self, session_id: Uuid, msg: &Message) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        Self::insert_message(&conn, session_id, msg)?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Delete specific messages from a session.
    pub fn delete_messages(&self, session_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        for id in message_ids {
            self.write_execute(
                "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
                params![session_id.to_string(), id.to_string()],
            )?;
        }
        // Update session timestamp
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load all messages for a session, ordered by creation time.
    ///
    /// **Phase 1** — collect raw column data from SQLite (fast, serial).
    /// **Phase 2** — decompress zstd blobs and parse JSON in parallel via
    /// rayon, utilising all available CPU cores for these CPU-bound steps.
    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, \
                 tool_name, metadata, created_at, completed_at, streaming, input_tokens, \
                 output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, \
                 tokens_per_second, thinking_level, app_data \
                 FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
            )?;

            // ── Phase 1: collect raw rows (no decompression) ──────────
            let raw_rows: Vec<RawMessageRow> = {
                let rows = stmt.query_map(params![session_id.to_string()], |row| {
                    Ok(RawMessageRow {
                        id: row.get::<_, String>(0).unwrap_or_default(),
                        role: row.get::<_, String>(1).unwrap_or_default(),
                        content: row.get::<_, Vec<u8>>(2).unwrap_or_default(),
                        attachments: row.get::<_, String>(3).unwrap_or_default(),
                        reasoning: row.get::<_, Vec<u8>>(4).unwrap_or_default(),
                        tool_calls: row.get::<_, Vec<u8>>(5).unwrap_or_default(),
                        tool_call_id: row.get(6).ok().flatten(),
                        tool_name: row.get(7).ok().flatten(),
                        metadata: row.get::<_, Vec<u8>>(8).unwrap_or_default(),
                        created_at: row.get::<_, String>(9).unwrap_or_default(),
                        completed_at: row.get(10).ok().flatten(),
                        streaming: row.get::<_, i64>(11).unwrap_or(0) != 0,
                        input_tokens: row.get(12).ok().flatten(),
                        output_tokens: row.get(13).ok().flatten(),
                        total_tokens: row.get(14).ok().flatten(),
                        cache_read_tokens: row.get(15).ok().flatten(),
                        cache_write_tokens: row.get(16).ok().flatten(),
                        model_id: row.get(17).ok().flatten(),
                        tokens_per_second: row.get(18).ok().flatten(),
                        thinking_level: row.get(19).ok().flatten(),
                        app_data: row.get::<_, Vec<u8>>(20).unwrap_or_default(),
                    })
                })?;
                let mut raw = Vec::new();
                for row in rows {
                    raw.push(row?);
                }
                raw
            };

            // ── Phase 2: parallel decompress and parse ───────────────
            let messages: Vec<Message> = raw_rows
                .into_par_iter()
                .map(|raw| raw.decompress_and_parse())
                .collect();

            Ok(messages)
        })
    }

    /// Load application-owned fields for all messages in a session.
    pub fn load_message_app_data(&self, session_id: Uuid) -> Result<HashMap<Uuid, MessageAppData>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, mode, app_data FROM messages WHERE session_id = ?1")?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                let id = Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default();
                let mode: Option<String> = row.get(1)?;
                let app_data_blob: Vec<u8> = row.get(2).unwrap_or_default();
                let mut app_data: MessageAppData = if app_data_blob.is_empty() {
                    MessageAppData::default()
                } else {
                    serde_json::from_str(&decompress_text(&app_data_blob)).unwrap_or_default()
                };
                if mode.is_some() {
                    app_data.mode = mode;
                }
                Ok((id, app_data))
            })?;
            let mut app_data = HashMap::new();
            for row in rows {
                let (id, data) = row?;
                app_data.insert(id, data);
            }
            Ok(app_data)
        })
    }

    /// Update a message's content (used for streaming).
    pub fn update_message_content(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        content: &str,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET content = ?1 WHERE id = ?2 AND session_id = ?3",
            params![
                compress_text(content),
                message_id.to_string(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// Update a message's tool calls (used for streaming updates).
    pub fn update_message_tool_calls(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        tool_calls: &[tidev_llm::message::ToolCall],
    ) -> Result<()> {
        let json = serde_json::to_string(tool_calls)?;
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET tool_calls = ?1 WHERE id = ?2 AND session_id = ?3",
            params![
                compress_text(&json),
                message_id.to_string(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// Update a message's metadata (used by subagent tracking and tool result enrichment).
    pub fn update_message_metadata(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        metadata: &tidev_llm::message::ToolMetadata,
    ) -> Result<()> {
        let json = serde_json::to_string(metadata)?;
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET metadata = ?1 WHERE id = ?2 AND session_id = ?3",
            params![
                compress_text(&json),
                message_id.to_string(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// Update a message's child-session association without touching protocol metadata.
    pub fn update_message_child_session_id(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        child_session_id: Uuid,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let app_data_blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT app_data FROM messages WHERE id = ?1 AND session_id = ?2",
                params![message_id.to_string(), session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let mut app_data: MessageAppData = app_data_blob
            .filter(|b| !b.is_empty())
            .map(|b| serde_json::from_str(&decompress_text(&b)).unwrap_or_default())
            .unwrap_or_default();
        app_data.child_session_id = Some(child_session_id);
        let json = serde_json::to_string(&app_data).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "UPDATE messages SET app_data = ?1 WHERE id = ?2 AND session_id = ?3",
            params![
                compress_text(&json),
                message_id.to_string(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// Update message completion status.
    #[allow(clippy::too_many_arguments)]
    pub fn update_message_completed(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        completed_at: DateTime<Utc>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        total_tokens: Option<u32>,
        cache_read_tokens: Option<u32>,
        cache_write_tokens: Option<u32>,
        model_id: Option<String>,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET completed_at = ?1, input_tokens = ?2, output_tokens = ?3, \
             total_tokens = ?4, cache_read_tokens = ?5, cache_write_tokens = ?6, model_id = ?7 \
             WHERE id = ?8 AND session_id = ?9",
            params![
                completed_at.to_rfc3339(),
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                message_id.to_string(),
                session_id.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Mark a message as no longer streaming.
    pub fn finish_streaming(&self, session_id: Uuid, message_id: Uuid) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET streaming = 0 WHERE id = ?1 AND session_id = ?2",
            params![message_id.to_string(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Get token statistics for a session.
    pub fn get_session_token_stats(&self, session_id: Uuid) -> Result<SessionTokenStats> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(input_tokens), 0) as input_tokens, \
                 COALESCE(SUM(output_tokens), 0) as output_tokens \
                 FROM messages WHERE session_id = :session_id",
                named_params! { ":session_id": session_id.to_string() },
                |row| {
                    Ok(SessionTokenStats {
                        input_tokens: row.get::<_, i64>(0)? as u32,
                        output_tokens: row.get::<_, i64>(1)? as u32,
                    })
                },
            )
            .map_err(Into::into)
        })
    }

    /// Load token usage records without loading message content or metadata.
    pub fn load_usage_records(&self) -> Result<Vec<UsageRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.session_id, s.title, s.provider_id, s.provider_display_name, \
                 s.model_id, s.model_display_name, s.created_at, s.updated_at, m.created_at, \
                 COALESCE(m.input_tokens, 0), COALESCE(m.output_tokens, 0), \
                 COALESCE(m.cache_read_tokens, 0), COALESCE(m.cache_write_tokens, 0), \
                 COALESCE(m.total_tokens, COALESCE(m.input_tokens, 0) + COALESCE(m.output_tokens, 0)) \
                 FROM messages m INNER JOIN sessions s ON s.id = m.session_id \
                 WHERE m.role = 'assistant' \
                   AND (m.total_tokens IS NOT NULL OR m.input_tokens IS NOT NULL OR m.output_tokens IS NOT NULL) \
                 ORDER BY m.created_at ASC, m.rowid ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(UsageRecord {
                    session_id: row.get(0)?,
                    title: row.get(1)?,
                    provider_id: row.get(2)?,
                    provider_display_name: row.get(3)?,
                    model_id: row.get(4)?,
                    model_display_name: row.get(5)?,
                    session_created_at: row.get(6)?,
                    session_updated_at: row.get(7)?,
                    created_at: row.get(8)?,
                    input_tokens: row.get::<_, i64>(9)?.max(0) as u64,
                    output_tokens: row.get::<_, i64>(10)?.max(0) as u64,
                    cache_read_tokens: row.get::<_, i64>(11)?.max(0) as u64,
                    cache_write_tokens: row.get::<_, i64>(12)?.max(0) as u64,
                    total_tokens: row.get::<_, i64>(13)?.max(0) as u64,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Get the most recent message for a session.
    pub fn get_last_message(&self, session_id: Uuid) -> Result<Option<Message>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, \
                 tool_name, metadata, created_at, completed_at, streaming, input_tokens, \
                 output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, \
                 tokens_per_second, thinking_level, app_data \
                 FROM messages WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
            )?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                Ok(Self::build_message_from_row(row))
            })?;
            match rows.next() {
                Some(Ok(msg)) => Ok(Some(msg)),
                _ => Ok(None),
            }
        })
    }

    /// Build a Message from a SQLite row (used by load_messages and get_last_message).
    fn build_message_from_row(row: &rusqlite::Row) -> Message {
        let role_str: String = row.get(1).unwrap_or_default();
        let content = row
            .get::<_, Vec<u8>>(2)
            .map(|b| decompress_text(&b))
            .unwrap_or_else(|_| row.get::<_, String>(2).unwrap_or_default());
        let attachments_raw: String = row.get(3).unwrap_or_default();
        let attachments: Vec<tidev_llm::message::MessageAttachment> =
            serde_json::from_str(&attachments_raw).unwrap_or_default();
        let metadata_raw: Vec<u8> = row.get(8).unwrap_or_default();
        let metadata: tidev_llm::message::ToolMetadata =
            serde_json::from_str(&decompress_text(&metadata_raw)).unwrap_or_default();
        let completed_at: Option<String> = row.get(10).ok().flatten();
        let streaming: bool = row.get::<_, i64>(11).unwrap_or(0) != 0;

        let reasoning = row
            .get::<_, Vec<u8>>(4)
            .map(|b| decompress_text(&b))
            .unwrap_or_default();

        let tool_calls_json = row
            .get::<_, Vec<u8>>(5)
            .map(|b| decompress_text(&b))
            .unwrap_or_else(|_| "[]".to_string());

        let thinking_level: Option<String> = row.get(19).ok().flatten();
        let app_data_blob: Vec<u8> = row.get(20).unwrap_or_default();
        let app_data: MessageAppData = if app_data_blob.is_empty() {
            MessageAppData::default()
        } else {
            serde_json::from_str(&decompress_text(&app_data_blob)).unwrap_or_default()
        };

        Message {
            id: Uuid::parse_str(&row.get::<_, String>(0).unwrap_or_default()).unwrap_or_default(),
            role: MessageRole::from_db_value(&role_str),
            content,
            attachments,
            reasoning,
            tool_calls: serde_json::from_str(&tool_calls_json).unwrap_or_default(),
            tool_call_id: row.get(6).ok().flatten(),
            tool_name: row.get(7).ok().flatten(),
            metadata,
            created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(9).unwrap_or_default())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_default(),
            completed_at: completed_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            streaming,
            input_tokens: row.get(12).ok().flatten(),
            output_tokens: row.get(13).ok().flatten(),
            total_tokens: row.get(14).ok().flatten(),
            cache_read_tokens: row.get(15).ok().flatten(),
            cache_write_tokens: row.get(16).ok().flatten(),
            model_id: row.get(17).ok().flatten(),
            tokens_per_second: row.get(18).ok().flatten(),
            thinking_level: thinking_level.and_then(|t| serde_json::from_str(&t).ok()),
            reasoning_started_at: app_data.reasoning_started_at,
            reasoning_completed_at: app_data.reasoning_completed_at,
        }
    }

    /// Export one or more sessions to an uncompressed SQLite database.
    ///
    /// The output database uses the [`EXPORT_SCHEMA_SQL`] schema where all
    /// text columns are stored as plain TEXT (no zstd compression), suitable
    /// for inspection with standard SQLite tools or re-import.
    pub fn export_to_sqlite(&self, session_ids: &[Uuid], output_path: &Path) -> Result<()> {
        // Remove existing file so we start fresh.
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
        export_conn.execute_batch(crate::schema::EXPORT_SCHEMA_SQL)?;

        let sid_strs: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
        let placeholder = sid_strs.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let tx = export_conn.unchecked_transaction()?;

        // ── 1. meta ── copy all rows ─────────────────────────────────────
        {
            let rows = self.read_query("SELECT key, value FROM meta", [], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut insert =
                tx.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)")?;
            for (key, value) in &rows {
                insert.execute(params![key, value])?;
            }
        }

        // ── 2. sessions ── only the requested ones ───────────────────────
        {
            let sql = format!(
                "SELECT id, parent_session_id, provider_id, provider_display_name, \
                        model_id, model_display_name, title, created_at, updated_at, \
                        status, ended_at, context_summary, context_retained_from, system_prompt, \
                        workspace_root, snapshot_start_hash, instruction_sources, todos, \
                        revert_message_id, revert_redo_snapshot \
                 FROM sessions WHERE id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
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
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO sessions \
                 (id, parent_session_id, provider_id, provider_display_name, \
                  model_id, model_display_name, title, created_at, updated_at, \
                  status, ended_at, context_summary, context_retained_from, system_prompt, \
                  workspace_root, snapshot_start_hash, instruction_sources, todos, \
                  revert_message_id, revert_redo_snapshot) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10,
                    row.11, row.12, row.13, row.14, row.15, row.16, row.17, row.18, row.19,
                ])?;
            }
        }

        // ── 3. messages ── decompress BLOB columns → TEXT ────────────────
        {
            let sql = format!(
                "SELECT id, session_id, role, CAST(content AS BLOB), attachments, \
                        CAST(reasoning AS BLOB), CAST(tool_calls AS BLOB), tool_call_id, \
                        tool_name, CAST(metadata AS BLOB), created_at, completed_at, \
                        streaming, input_tokens, output_tokens, total_tokens, \
                        cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, \
                        mode, thinking_level, CAST(app_data AS BLOB) \
                 FROM messages WHERE session_id IN ({placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let content = blob_or_empty_to_text(row, 3)?;
                let reasoning = opt_blob_to_text(row, 5)?;
                let tool_calls = blob_or_empty_to_text(row, 6)?;
                let metadata = blob_or_empty_to_text(row, 9)?;
                let app_data = blob_or_empty_to_text(row, 22)?;
                Ok((
                    row.get::<_, String>(0)?, // id
                    row.get::<_, String>(1)?, // session_id
                    row.get::<_, String>(2)?, // role
                    content,
                    row.get::<_, String>(4)?, // attachments
                    reasoning,
                    tool_calls,
                    row.get::<_, Option<String>>(7)?, // tool_call_id
                    row.get::<_, Option<String>>(8)?, // tool_name
                    metadata,
                    row.get::<_, String>(10)?,         // created_at
                    row.get::<_, Option<String>>(11)?, // completed_at
                    row.get::<_, i64>(12)?,            // streaming
                    row.get::<_, Option<i64>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<i64>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<i64>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<f64>>(19)?,
                    row.get::<_, Option<String>>(20)?, // mode
                    row.get::<_, Option<String>>(21)?, // thinking_level
                    app_data,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO messages \
                 (id, session_id, role, content, attachments, reasoning, tool_calls, \
                  tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                  input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                  cache_write_tokens, model_id, tokens_per_second, mode, thinking_level, app_data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10,
                    row.11, row.12, row.13, row.14, row.15, row.16, row.17, row.18, row.19, row.20,
                    row.21, row.22,
                ])?;
            }
        }

        // ── 4. tool_outputs ── decompress BLOB columns → TEXT ────────────
        {
            let sql = format!(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, CAST(output AS BLOB), \
                        byte_size, line_count, created_at \
                 FROM tool_outputs WHERE session_id IN ({placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let output = opt_blob_to_text(row, 5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    output,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO tool_outputs \
                 (id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Import sessions from an uncompressed SQLite file created by
    /// [`export_to_sqlite`].
    ///
    /// Returns the list of session UUIDs that were imported.
    ///
    /// * `import_path` — path to the export SQLite file.
    /// * `session_ids` — if `Some`, only import these sessions from the file;
    ///   if `None`, import all sessions found.
    /// * `replace` — if `true`, overwrite existing sessions with the same UUID;
    ///   if `false`, skip sessions that already exist.
    pub fn import_from_sqlite(
        &self,
        import_path: &Path,
        session_ids: Option<&[Uuid]>,
        replace: bool,
    ) -> Result<Vec<Uuid>> {
        let import_conn = Connection::open(import_path)
            .with_context(|| format!("failed to open import file {}", import_path.display()))?;

        // Determine which sessions to import.
        let sid_strs: Vec<String> = if let Some(ids) = session_ids {
            ids.iter().map(|id| id.to_string()).collect()
        } else {
            import_conn
                .prepare("SELECT id FROM sessions")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if sid_strs.is_empty() {
            return Ok(Vec::new());
        }

        let placeholder = sid_strs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sid_params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        // Filter out sessions that already exist (unless replace).
        let existing: Vec<String> = if replace {
            Vec::new()
        } else {
            let sql = format!("SELECT id FROM sessions WHERE id IN ({placeholder})");
            self.read_query(&sql, sid_params.as_slice(), |row| row.get::<_, String>(0))?
        };

        let to_import: Vec<&str> = sid_strs
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !existing.contains(&s.to_string()))
            .collect();

        if to_import.is_empty() {
            return Ok(Vec::new());
        }

        let imp_placeholder = to_import.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let imp_params: Vec<&dyn rusqlite::types::ToSql> = to_import
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        // ── 1. sessions ───────────────────────────────────────────────────
        {
            let sql = format!(
                "SELECT id, parent_session_id, provider_id, provider_display_name, \
                        model_id, model_display_name, title, created_at, updated_at, \
                        status, ended_at, context_summary, context_retained_from, system_prompt, \
                        workspace_root, snapshot_start_hash, instruction_sources, todos, \
                        revert_message_id, revert_redo_snapshot \
                 FROM sessions WHERE id IN ({imp_placeholder})"
            );
            let mut stmt = import_conn.prepare(&sql)?;
            let rows = stmt
                .query_map(imp_params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
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
                        row.get::<_, String>(13)?,
                        row.get::<_, String>(14)?,
                        row.get::<_, Option<String>>(15)?,
                        row.get::<_, String>(16)?,
                        row.get::<_, String>(17)?,
                        row.get::<_, Option<String>>(18)?,
                        row.get::<_, Option<String>>(19)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let conn = self.write_conn.lock().unwrap();
            for r in &rows {
                conn.execute(
                    "INSERT OR REPLACE INTO sessions \
                     (id, parent_session_id, provider_id, provider_display_name, model_id, \
                      model_display_name, title, created_at, updated_at, status, ended_at, \
                      context_summary, context_retained_from, system_prompt, \
                      workspace_root, snapshot_start_hash, instruction_sources, todos, \
                      revert_message_id, revert_redo_snapshot) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                    params![
                        r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, r.11, r.12, r.13,
                        r.14, r.15, r.16, r.17, r.18, r.19,
                    ],
                )?;
            }
        }

        // ── 2. messages ── compress TEXT columns → BLOB ───────────────────
        {
            let sql = format!(
                "SELECT id, session_id, role, content, attachments, reasoning, tool_calls, \
                        tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                        input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                        cache_write_tokens, model_id, tokens_per_second, mode, \
                        thinking_level, app_data \
                 FROM messages WHERE session_id IN ({imp_placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = import_conn.prepare(&sql)?;
            let rows = stmt
                .query_map(imp_params.as_slice(), |row| {
                    let content: String = row.get(3)?;
                    let reasoning: Option<String> = row.get(5)?;
                    let tool_calls: String = row.get(6)?;
                    let metadata: String = row.get(9)?;
                    let app_data: String = row.get(22).unwrap_or_else(|_| "{}".to_string());
                    Ok((
                        row.get::<_, String>(0)?, // id
                        row.get::<_, String>(1)?, // session_id
                        row.get::<_, String>(2)?, // role
                        compress_text(&content),
                        row.get::<_, String>(4)?, // attachments (JSON, stored as TEXT in both)
                        reasoning.map(|r| compress_text(&r)),
                        compress_text(&tool_calls),
                        row.get::<_, Option<String>>(7)?, // tool_call_id
                        row.get::<_, Option<String>>(8)?, // tool_name
                        compress_text(&metadata),
                        row.get::<_, String>(10)?,         // created_at
                        row.get::<_, Option<String>>(11)?, // completed_at
                        row.get::<_, i64>(12)?,            // streaming
                        row.get::<_, Option<i64>>(13)?,
                        row.get::<_, Option<i64>>(14)?,
                        row.get::<_, Option<i64>>(15)?,
                        row.get::<_, Option<i64>>(16)?,
                        row.get::<_, Option<i64>>(17)?,
                        row.get::<_, Option<String>>(18)?, // model_id
                        row.get::<_, Option<f64>>(19)?,    // tokens_per_second
                        row.get::<_, Option<String>>(20)?, // mode
                        row.get::<_, Option<String>>(21)?, // thinking_level
                        compress_text(&app_data),
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let conn = self.write_conn.lock().unwrap();
            for r in &rows {
                conn.execute(
                    "INSERT OR REPLACE INTO messages \
                     (id, session_id, role, content, attachments, reasoning, tool_calls, \
                      tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                      input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                      cache_write_tokens, model_id, tokens_per_second, mode, \
                      thinking_level, app_data) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                             ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                    params![
                        r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, r.11, r.12, r.13,
                        r.14, r.15, r.16, r.17, r.18, r.19, r.20, r.21, r.22,
                    ],
                )?;
            }
        }

        // ── 3. tool_outputs ── compress TEXT columns → BLOB ───────────────
        {
            let sql = format!(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, output, \
                        byte_size, line_count, created_at \
                 FROM tool_outputs WHERE session_id IN ({imp_placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = import_conn.prepare(&sql)?;
            let rows = stmt
                .query_map(imp_params.as_slice(), |row| {
                    let output_text: Option<String> = row.get(5)?;
                    let output_blob = output_text.as_deref().map(compress_text);
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        output_blob,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let conn = self.write_conn.lock().unwrap();
            for r in &rows {
                conn.execute(
                    "INSERT OR REPLACE INTO tool_outputs \
                     (id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8,
                    ],
                )?;
            }
        }

        let imported = to_import
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();
        Ok(imported)
    }
}

// ── Export helpers ───────────────────────────────────────────────────

/// Read a non-nullable BLOB column, decompress it to text.
/// Returns an empty string if the BLOB is empty.
fn blob_or_empty_to_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<String> {
    let blob: Vec<u8> = row.get(idx)?;
    if blob.is_empty() {
        Ok(String::new())
    } else {
        Ok(decompress_text(&blob))
    }
}

/// Read an optional BLOB column, decompress it to optional text.
fn opt_blob_to_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    let blob: Option<Vec<u8>> = row.get(idx)?;
    match blob {
        Some(b) if !b.is_empty() => Ok(Some(decompress_text(&b))),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Todo persistence
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save/overwrite all todo items for a session.
    pub fn save_todos(
        &self,
        session_id: Uuid,
        todos: &[tidev_tools::types::TodoItem],
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let json = serde_json::to_string(todos)?;
        conn.execute(
            "UPDATE sessions SET todos = ?1 WHERE id = ?2",
            params![json, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load todo items for a session.
    pub fn load_todos(&self, session_id: Uuid) -> Result<Vec<tidev_tools::types::TodoItem>> {
        self.read(|conn| {
            let mut stmt = conn.prepare("SELECT todos FROM sessions WHERE id = ?1")?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?;
            if let Some(Ok(json)) = rows.next() {
                let items: Vec<tidev_tools::types::TodoItem> =
                    serde_json::from_str(&json).unwrap_or_default();
                Ok(items)
            } else {
                Ok(Vec::new())
            }
        })
    }
}

// ===========================================================================
// End of SessionStore implementation.
// ===========================================================================

// ---------------------------------------------------------------------------
// Snapshot storage
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save snapshot data for a message.
    pub fn save_snapshot(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        snapshot_hash: &str,
        patch_files: &str,
        file_diffs: &str,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let app_data_blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT app_data FROM messages WHERE id = ?1 AND session_id = ?2",
                params![message_id.to_string(), session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let mut app_data: MessageAppData = app_data_blob
            .filter(|b| !b.is_empty())
            .map(|b| serde_json::from_str(&decompress_text(&b)).unwrap_or_default())
            .unwrap_or_default();
        app_data.snapshot_hash = Some(snapshot_hash.to_string());
        app_data.patch_files = if patch_files.is_empty() {
            None
        } else {
            Some(patch_files.to_string())
        };
        app_data.file_diffs = if file_diffs.is_empty() {
            None
        } else {
            Some(file_diffs.to_string())
        };
        let json = serde_json::to_string(&app_data).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "UPDATE messages SET app_data = ?1 WHERE id = ?2 AND session_id = ?3",
            params![
                compress_text(&json),
                message_id.to_string(),
                session_id.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Load snapshot patch data for a message.
    pub fn load_snapshot(&self, message_id: Uuid) -> Result<Option<(String, String, String)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare("SELECT app_data FROM messages WHERE id = ?1")?;
            let mut rows = stmt.query_map(params![message_id.to_string()], |row| {
                let blob: Vec<u8> = row.get(0).unwrap_or_default();
                let app_data: MessageAppData = if blob.is_empty() {
                    MessageAppData::default()
                } else {
                    serde_json::from_str(&decompress_text(&blob)).unwrap_or_default()
                };
                let hash = app_data.snapshot_hash.unwrap_or_default();
                let patches = app_data.patch_files.unwrap_or_default();
                let diffs = app_data.file_diffs.unwrap_or_default();
                Ok((hash, patches, diffs))
            })?;
            match rows.next() {
                Some(Ok(result)) => {
                    if result.0.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(result))
                    }
                }
                _ => Ok(None),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Session instruction sources
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save instruction sources for a session (replaces all existing).
    pub fn save_instruction_sources(&self, session_id: Uuid, sources: &[String]) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let json = serde_json::to_string(sources)?;
        conn.execute(
            "UPDATE sessions SET instruction_sources = ?1 WHERE id = ?2",
            params![json, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Append instruction sources for a session (deduplicates against existing items
    /// so the same source is never stored twice).
    pub fn append_instruction_sources(&self, session_id: Uuid, sources: &[String]) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let current_json: Option<String> = conn
            .query_row(
                "SELECT instruction_sources FROM sessions WHERE id = ?1",
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let mut existing: Vec<String> = current_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        for source in sources {
            if !existing.contains(source) {
                existing.push(source.clone());
            }
        }
        let json = serde_json::to_string(&existing)?;
        conn.execute(
            "UPDATE sessions SET instruction_sources = ?1 WHERE id = ?2",
            params![json, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load instruction sources for a session.
    pub fn load_instruction_sources(&self, session_id: Uuid) -> Result<Vec<String>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT instruction_sources FROM sessions WHERE id = ?1")?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?;
            if let Some(Ok(json)) = rows.next() {
                let mut sources: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                sources.sort();
                Ok(sources)
            } else {
                Ok(Vec::new())
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::database::Database;
    use tempfile::TempDir;

    fn test_store() -> (SessionStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path().join("test.db")).unwrap();
        let store = db.create_store().unwrap();
        (store, tmp)
    }

    fn create_test_session(store: &SessionStore, workspace: &str, title: &str) -> Uuid {
        let id = Uuid::new_v4();
        store
            .create_session(
                id,
                workspace,
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                title,
                None,
                None,
            )
            .unwrap();
        id
    }

    #[test]
    fn session_create_and_load_round_trip() {
        let (store, _tmp) = test_store();
        let id = create_test_session(&store, "/workspace", "Test session");

        let loaded = store
            .load_session(id)
            .unwrap()
            .expect("session should exist");
        assert_eq!(loaded.title, "Test session");
        assert_eq!(loaded.provider_id, "deepseek");
        assert_eq!(loaded.model_id, "deepseek-v4-flash");
        assert_eq!(loaded.workspace_root, "/workspace");
        assert_eq!(loaded.status, "active");
        assert!(loaded.parent_session_id.is_none());
    }

    #[test]
    fn context_state_round_trip() {
        let (store, _tmp) = test_store();
        let id = create_test_session(&store, "/workspace", "ctx test");

        store
            .update_session(
                id,
                None,
                None,
                Some("Summary: refactored main.rs"),
                Some(7),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let loaded = store.load_session(id).unwrap().unwrap();
        assert_eq!(
            loaded.context_summary.as_deref(),
            Some("Summary: refactored main.rs")
        );
        assert_eq!(loaded.context_retained_from, 7);
    }

    #[test]
    fn parent_child_session_relationship() {
        let (store, _tmp) = test_store();
        let parent = create_test_session(&store, "/workspace", "Parent");
        let child = Uuid::new_v4();
        store
            .create_session(
                child,
                "/workspace",
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                "Child",
                Some(parent),
                None,
            )
            .unwrap();

        let loaded = store.load_session(child).unwrap().unwrap();
        assert_eq!(loaded.parent_session_id, Some(parent));
    }

    #[test]
    fn child_sessions_excluded_from_listing() {
        let (store, _tmp) = test_store();
        let parent = create_test_session(&store, "/workspace", "Parent");
        let child = Uuid::new_v4();
        store
            .create_session(
                child,
                "/workspace",
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                "Child",
                Some(parent),
                None,
            )
            .unwrap();

        let sessions = store.list_sessions(10, 0).unwrap();
        let ids: Vec<Uuid> = sessions.iter().map(|s| s.session_id).collect();
        assert!(ids.contains(&parent));
        assert!(!ids.contains(&child));
    }

    #[test]
    fn workspace_session_scoping() {
        let (store, _tmp) = test_store();
        let _a = create_test_session(&store, "/ws-a", "A");
        let _b = create_test_session(&store, "/ws-b", "B");

        let ws_a_sessions = store.list_sessions_for_workspace("/ws-a", 10, 0).unwrap();
        assert_eq!(ws_a_sessions.len(), 1);
        assert_eq!(ws_a_sessions[0].title, "A");

        let ws_b_sessions = store.list_sessions_for_workspace("/ws-b", 10, 0).unwrap();
        assert_eq!(ws_b_sessions.len(), 1);
        assert_eq!(ws_b_sessions[0].title, "B");
    }

    #[test]
    fn system_prompt_round_trip() {
        let (store, _tmp) = test_store();
        let id = create_test_session(&store, "/workspace", "prompt test");

        let loaded = store.load_session(id).unwrap().unwrap();
        assert_eq!(loaded.system_prompt, "");

        store
            .update_session(
                id,
                None,
                None,
                None,
                None,
                Some("You are a helpful AI."),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let loaded = store.load_session(id).unwrap().unwrap();
        assert_eq!(loaded.system_prompt, "You are a helpful AI.");
    }

    #[test]
    fn message_append_and_load_round_trip() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "msg test");

        let msg = Message::new(MessageRole::User, "Hello, world!");
        store.append_message(sid, &msg).unwrap();

        let messages = store.load_messages(sid).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[0].content, "Hello, world!");
    }

    #[test]
    fn message_reload_preserves_insert_order_for_equal_timestamps() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "equal timestamp test");
        let timestamp = Utc::now();
        let mut messages = Vec::new();
        for content in ["first", "second", "third"] {
            let mut message = Message::new(MessageRole::User, content);
            message.created_at = timestamp;
            messages.push(message);
        }

        store.append_messages(sid, &messages).unwrap();
        let loaded = store.load_messages(sid).unwrap();

        let contents: Vec<&str> = loaded
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(contents, ["first", "second", "third"]);
    }

    #[test]
    fn message_app_data_round_trip() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "app data test");
        let msg = Message::new(MessageRole::User, "hello");
        let mut app_data = HashMap::new();
        app_data.insert(
            msg.id,
            MessageAppData {
                snapshot_hash: Some("snap-1".into()),
                patch_files: Some(r#"[{"files":["src/main.rs"]}]"#.into()),
                file_diffs: Some("[]".into()),
                mode: Some("plan".into()),
                child_session_id: None,
                provider_error: Some(ProviderErrorData {
                    message: "HTTP 503 overloaded".into(),
                    retryable: true,
                    request_id: 7,
                    user_message_id: Some(msg.id),
                }),
                ..Default::default()
            },
        );
        store
            .append_messages_with_app_data(sid, std::slice::from_ref(&msg), &app_data)
            .unwrap();

        let loaded = store.load_message_app_data(sid).unwrap();
        assert_eq!(loaded.get(&msg.id), app_data.get(&msg.id));
        let child_id = Uuid::new_v4();
        store
            .update_message_child_session_id(sid, msg.id, child_id)
            .unwrap();
        assert_eq!(
            store.load_message_app_data(sid).unwrap()[&msg.id].child_session_id,
            Some(child_id)
        );
        let protocol = store.load_messages(sid).unwrap();
        assert_eq!(protocol[0].content, "hello");
        let serialized = serde_json::to_value(&protocol[0]).unwrap();
        assert!(serialized.get("snapshot_hash").is_none());
        assert!(serialized.get("patch_files").is_none());
        assert!(serialized.get("file_diffs").is_none());
        assert!(serialized.get("mode").is_none());
    }

    #[test]
    fn session_inspection_includes_application_data_and_tool_output() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "inspection test");
        let message = Message::tool_result(
            "call-1",
            "shell",
            tidev_llm::message::ToolExecutionResult::new("stored preview"),
        );
        let child_id = Uuid::new_v4();
        let mut app_data = HashMap::new();
        app_data.insert(
            message.id,
            MessageAppData {
                mode: Some("plan".into()),
                child_session_id: Some(child_id),
                ..Default::default()
            },
        );
        store
            .append_messages_with_app_data(sid, std::slice::from_ref(&message), &app_data)
            .unwrap();
        store
            .save_tool_output(
                "out-test-1",
                sid,
                message.id,
                "call-1",
                "shell",
                "complete tool output",
            )
            .unwrap();

        let inspection = store
            .load_session_inspection(sid)
            .unwrap()
            .expect("session should exist");
        assert_eq!(inspection.session.session_id, sid);
        assert_eq!(inspection.messages.len(), 1);
        assert_eq!(inspection.messages[0].sequence, 0);
        assert_eq!(inspection.messages[0].app_data, app_data[&message.id]);
        let tool_out = inspection.messages[0].tool_output.as_ref().unwrap();
        assert_eq!(tool_out.id, "out-test-1");
        assert_eq!(tool_out.tool_name, "shell");
        assert_eq!(tool_out.byte_size, "complete tool output".len());
    }

    #[test]
    fn jsonl_export_contains_session_id_and_message_sequence() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "jsonl test");
        let messages = vec![
            Message::new(MessageRole::User, "first"),
            Message::new(MessageRole::Assistant, "second"),
        ];
        store.append_messages(sid, &messages).unwrap();

        let export_tmp = TempDir::new().unwrap();
        let export_path = export_tmp.path().join("messages.jsonl");
        let count = store.export_to_jsonl(&[sid], &export_path).unwrap();
        assert_eq!(count, 2);

        let lines = std::fs::read_to_string(export_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["session_id"], sid.to_string());
        assert_eq!(lines[0]["sequence"], 0);
        assert_eq!(lines[0]["message"]["content"], "first");
        assert_eq!(lines[1]["sequence"], 1);
        assert_eq!(lines[1]["message"]["content"], "second");
    }

    #[test]
    fn sqlite_export_import_preserves_child_session_id() {
        let (source, _source_tmp) = test_store();
        let sid = create_test_session(&source, "/workspace", "export app data test");
        let msg = Message::new(MessageRole::Tool, "subagent finished");
        let child_id = Uuid::new_v4();
        let mut app_data = HashMap::new();
        app_data.insert(
            msg.id,
            MessageAppData {
                child_session_id: Some(child_id),
                ..Default::default()
            },
        );
        source
            .append_messages_with_app_data(sid, std::slice::from_ref(&msg), &app_data)
            .unwrap();

        let export_tmp = TempDir::new().unwrap();
        let export_path = export_tmp.path().join("session-export.db");
        source.export_to_sqlite(&[sid], &export_path).unwrap();

        let (target, _target_tmp) = test_store();
        assert_eq!(
            target
                .import_from_sqlite(&export_path, None, false)
                .unwrap(),
            vec![sid]
        );
        let loaded = target.load_message_app_data(sid).unwrap();
        assert_eq!(loaded[&msg.id].child_session_id, Some(child_id));
    }

    #[test]
    fn message_streaming_update_content() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "stream test");
        let msg = Message::new(MessageRole::Assistant, "");
        store.append_message(sid, &msg).unwrap();

        store
            .update_message_content(sid, msg.id, "streamed content")
            .unwrap();

        let messages = store.load_messages(sid).unwrap();
        assert_eq!(messages[0].content, "streamed content");
    }

    #[test]
    fn message_streaming_update_tool_calls() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "toolcall test");
        let msg = Message::new(MessageRole::Assistant, "");
        store.append_message(sid, &msg).unwrap();

        let calls = vec![tidev_llm::message::ToolCall {
            id: "tc-1".into(),
            name: "shell".into(),
            arguments: r#"{"command":"ls"}"#.into(),
            thought_signature: None,
        }];
        store
            .update_message_tool_calls(sid, msg.id, &calls)
            .unwrap();

        let messages = store.load_messages(sid).unwrap();
        assert_eq!(messages[0].tool_calls.len(), 1);
        assert_eq!(messages[0].tool_calls[0].name, "shell");
    }

    #[test]
    fn message_update_metadata() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "meta test");
        let msg = Message::new(MessageRole::User, "hello");
        store.append_message(sid, &msg).unwrap();

        let meta = tidev_llm::message::ToolMetadata {
            prior_summary: Some("old summary".into()),
            prior_retained_from: Some(42),
            ..Default::default()
        };
        store.update_message_metadata(sid, msg.id, &meta).unwrap();

        let messages = store.load_messages(sid).unwrap();
        assert_eq!(
            messages[0].metadata.prior_summary.as_deref(),
            Some("old summary")
        );
        assert_eq!(messages[0].metadata.prior_retained_from, Some(42));
    }

    #[test]
    fn tool_output_save_and_load() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "toolout test");
        let msg_id = Uuid::new_v4();
        store
            .save_tool_output(
                "out-abc12345",
                sid,
                msg_id,
                "call-xyz",
                "read",
                "file content here",
            )
            .unwrap();

        // Lookup by ID
        let loaded = store.load_tool_output("out-abc12345").unwrap().unwrap();
        match loaded {
            ToolOutputContent::Available { record, output } => {
                assert_eq!(record.id, "out-abc12345");
                assert_eq!(record.session_id, sid);
                assert_eq!(record.message_id, msg_id);
                assert_eq!(record.tool_name, "read");
                assert_eq!(record.byte_size, 17);
                assert_eq!(record.line_count, 1);
                assert_eq!(output, "file content here");
            }
            ToolOutputContent::Expired { .. } => panic!("should be available"),
        }

        // Lookup by message_id
        assert!(
            store
                .load_tool_output(&msg_id.to_string())
                .unwrap()
                .is_some()
        );
        // Lookup by tool_call_id
        assert!(store.load_tool_output("call-xyz").unwrap().is_some());
    }

    #[test]
    fn tool_output_tombstone_expiration() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "toolout expire test");
        let msg_id = Uuid::new_v4();
        store
            .save_tool_output(
                "out-expire-1",
                sid,
                msg_id,
                "call-exp",
                "bash",
                "big output",
            )
            .unwrap();

        // Clear output older than -1 days (clears everything up to tomorrow)
        let cleared = store.clear_expired_tool_outputs(-1).unwrap();
        assert_eq!(cleared, 1);

        // Record still exists in Expired state
        let loaded = store.load_tool_output("out-expire-1").unwrap().unwrap();
        match loaded {
            ToolOutputContent::Available { .. } => panic!("should be expired"),
            ToolOutputContent::Expired { record } => {
                assert_eq!(record.id, "out-expire-1");
                assert_eq!(record.tool_name, "bash");
                assert_eq!(record.byte_size, 10);
            }
        }

        // Delete tombstones
        let deleted = store.delete_tombstones_older_than(-1).unwrap();
        assert_eq!(deleted, 1);
        assert!(store.load_tool_output("out-expire-1").unwrap().is_none());
    }

    #[test]
    fn todo_save_and_load() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "todo test");
        let todos = vec![
            tidev_tools::types::TodoItem {
                content: "Fix bug".into(),
                status: "pending".into(),
            },
            tidev_tools::types::TodoItem {
                content: "Write tests".into(),
                status: "completed".into(),
            },
        ];
        store.save_todos(sid, &todos).unwrap();

        let loaded = store.load_todos(sid).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "Fix bug");
        assert_eq!(loaded[1].status, "completed");
    }

    #[test]
    fn session_delete_removes_session() {
        let (store, _tmp) = test_store();
        let id = create_test_session(&store, "/workspace", "delete test");
        assert!(store.load_session(id).unwrap().is_some());

        store.delete_session(id).unwrap();
        assert!(store.load_session(id).unwrap().is_none());
    }

    #[test]
    fn session_update_title_and_status() {
        let (store, _tmp) = test_store();
        let id = create_test_session(&store, "/workspace", "original");
        store
            .update_session(
                id,
                Some("updated"),
                Some("ended"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let loaded = store.load_session(id).unwrap().unwrap();
        assert_eq!(loaded.title, "updated");
        assert_eq!(loaded.status, "ended");
    }

    #[test]
    fn session_token_stats() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "token test");

        let mut msg = Message::new(MessageRole::User, "hello");
        msg.input_tokens = Some(10);
        msg.output_tokens = Some(20);
        store.append_message(sid, &msg).unwrap();

        let stats = store.get_session_token_stats(sid).unwrap();
        assert_eq!(stats.input_tokens, 10);
        assert_eq!(stats.output_tokens, 20);
    }

    #[test]
    fn session_search_by_title() {
        let (store, _tmp) = test_store();
        create_test_session(&store, "/ws", "Refactor database layer");
        create_test_session(&store, "/ws", "Add unit tests");

        let results = store.search_sessions("Refactor", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Refactor database layer");
    }

    #[test]
    fn instruction_sources_save_and_load() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "instr test");
        store
            .save_instruction_sources(
                sid,
                &["/path/to/AGENTS.md".into(), "/path/to/PROJECT.md".into()],
            )
            .unwrap();

        let loaded = store.load_instruction_sources(sid).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], "/path/to/AGENTS.md");
        assert_eq!(loaded[1], "/path/to/PROJECT.md");

        store
            .append_instruction_sources(
                sid,
                &["/path/to/AGENTS.md".into(), "/path/to/EXTRA.md".into()],
            )
            .unwrap();
        let loaded2 = store.load_instruction_sources(sid).unwrap();
        assert_eq!(loaded2.len(), 3);
        assert_eq!(loaded2[0], "/path/to/AGENTS.md");
        assert_eq!(loaded2[1], "/path/to/EXTRA.md");
        assert_eq!(loaded2[2], "/path/to/PROJECT.md");
    }

    #[test]
    fn sqlite_export_import_roundtrip() {
        let (store, tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "export import test");
        let todos = vec![tidev_tools::types::TodoItem {
            content: "Task 1".into(),
            status: "pending".into(),
        }];
        store.save_todos(sid, &todos).unwrap();
        store
            .save_instruction_sources(sid, &["/workspace/AGENTS.md".into()])
            .unwrap();

        let revert_target_msg = Uuid::new_v4();
        store
            .save_revert_state(sid, revert_target_msg, Some(b"snap-hash-123"))
            .unwrap();

        let msg = Message::new(MessageRole::Tool, "truncated snippet");
        store.append_message(sid, &msg).unwrap();
        store
            .save_tool_output(
                "out-export-1",
                sid,
                msg.id,
                "call-exp-1",
                "shell",
                "full shell output text",
            )
            .unwrap();

        let export_path = tmp.path().join("export.db");
        store.export_to_sqlite(&[sid], &export_path).unwrap();

        let (store2, _tmp2) = test_store();
        let imported = store2.import_from_sqlite(&export_path, None, true).unwrap();
        assert_eq!(imported, vec![sid]);

        let loaded_session = store2.load_session(sid).unwrap().unwrap();
        assert_eq!(loaded_session.workspace_root, "/workspace");
        assert_eq!(loaded_session.title, "export import test");

        let loaded_todos = store2.load_todos(sid).unwrap();
        assert_eq!(loaded_todos.len(), 1);
        assert_eq!(loaded_todos[0].content, "Task 1");

        let loaded_instr = store2.load_instruction_sources(sid).unwrap();
        assert_eq!(loaded_instr, vec!["/workspace/AGENTS.md"]);

        let loaded_revert = store2.load_revert_state(sid).unwrap().unwrap();
        assert_eq!(loaded_revert.0, revert_target_msg);
        assert_eq!(
            loaded_revert.1.as_deref(),
            Some(b"snap-hash-123".as_slice())
        );

        let loaded_tool_out = store2.load_tool_output("out-export-1").unwrap().unwrap();
        match loaded_tool_out {
            ToolOutputContent::Available { record, output } => {
                assert_eq!(record.id, "out-export-1");
                assert_eq!(record.tool_name, "shell");
                assert_eq!(output, "full shell output text");
            }
            ToolOutputContent::Expired { .. } => panic!("should be available"),
        }
    }

    #[test]
    fn revert_state_crud() {
        let (store, _tmp) = test_store();
        let sid = create_test_session(&store, "/workspace", "revert test");

        // Initially None
        assert!(store.load_revert_state(sid).unwrap().is_none());

        // Set revert state
        let target_msg = Uuid::new_v4();
        store
            .save_revert_state(sid, target_msg, Some(b"redo-hash-456"))
            .unwrap();
        let state = store.load_revert_state(sid).unwrap().unwrap();
        assert_eq!(state.0, target_msg);
        assert_eq!(state.1.as_deref(), Some(b"redo-hash-456".as_slice()));

        // Clear revert state
        store.save_revert_state(sid, Uuid::nil(), None).unwrap();
        assert!(store.load_revert_state(sid).unwrap().is_none());
    }
}
// ---------------------------------------------------------------------------
// Session revert support
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save revert state for a session.
    pub fn save_revert_state(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        redo_snapshot: Option<&[u8]>,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let msg_id_opt = if message_id.is_nil() {
            None
        } else {
            Some(message_id.to_string())
        };
        let snap_opt = redo_snapshot.map(|s| String::from_utf8_lossy(s).to_string());
        conn.execute(
            "UPDATE sessions SET revert_message_id = ?1, revert_redo_snapshot = ?2 WHERE id = ?3",
            params![msg_id_opt, snap_opt, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load revert state for a session.
    pub fn load_revert_state(&self, session_id: Uuid) -> Result<Option<(Uuid, Option<Vec<u8>>)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT revert_message_id, revert_redo_snapshot FROM sessions WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                let msg_id_opt: Option<String> = row.get(0)?;
                let snapshot_opt: Option<String> = row.get(1)?;
                Ok((msg_id_opt, snapshot_opt))
            })?;
            match rows.next() {
                Some(Ok((Some(msg_id_str), snap))) => {
                    let id = Uuid::parse_str(&msg_id_str).unwrap_or_default();
                    if id.is_nil() {
                        Ok(None)
                    } else {
                        Ok(Some((id, snap.map(|s| s.into_bytes()))))
                    }
                }
                _ => Ok(None),
            }
        })
    }
}
