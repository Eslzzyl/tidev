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
    Connection, OptionalExtension, named_params, params, params_from_iter,
    types::{ToSql, Type},
};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use std::{
    collections::HashMap,
    fmt, fs,
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

/// Read a non-nullable BLOB (or TEXT) column, decompress it to text.
/// Returns an empty string if the value is empty or NULL.
fn blob_or_empty_to_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<String> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(String::new()),
        rusqlite::types::ValueRef::Blob(b) => {
            if b.is_empty() {
                Ok(String::new())
            } else {
                Ok(decompress_text(b))
            }
        }
        rusqlite::types::ValueRef::Text(t) => {
            if t.is_empty() {
                Ok(String::new())
            } else {
                Ok(decompress_text(t))
            }
        }
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            idx,
            other.data_type(),
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected BLOB, TEXT, or NULL",
            )),
        )),
    }
}

/// Read an optional BLOB (or TEXT) column, decompress it to optional text.
fn opt_blob_to_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Blob(b) => {
            if b.is_empty() {
                Ok(None)
            } else {
                let text = decompress_text(b);
                if text.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
        }
        rusqlite::types::ValueRef::Text(t) => {
            if t.is_empty() {
                Ok(None)
            } else {
                let text = decompress_text(t);
                if text.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
        }
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            idx,
            other.data_type(),
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected BLOB, TEXT, or NULL",
            )),
        )),
    }
}

mod instructions;
mod messages;
mod revert;
mod sessions;
mod snapshot;
#[cfg(test)]
mod tests;
mod todo;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption: Option<InterruptionData>,
}

/// Durable application state for an assistant stream that cannot continue.
///
/// It stays outside the protocol message payload. A corresponding assistant
/// message remains marked `streaming`, so context construction excludes it
/// from every later provider request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptionData {
    pub reason: InterruptionReason,
    pub request_id: u64,
    pub user_message_id: Option<Uuid>,
}

/// Why a streamed assistant message was terminally interrupted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionReason {
    UserCancelled,
    ProviderFailed,
    RuntimeRestarted,
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

        let metadata: tidev_llm::message::ToolMetadata = if self.metadata.is_empty() {
            tidev_llm::message::ToolMetadata::default()
        } else {
            serde_json::from_str(&decompress_text(&self.metadata)).unwrap_or_default()
        };

        let content = if self.content.is_empty() {
            String::new()
        } else {
            decompress_text(&self.content)
        };

        let attachments: Vec<tidev_llm::message::MessageAttachment> =
            serde_json::from_str(&self.attachments).unwrap_or_default();

        let reasoning = if self.reasoning.is_empty() {
            String::new()
        } else {
            decompress_text(&self.reasoning)
        };

        let tool_calls: Vec<tidev_llm::message::ToolCall> = if self.tool_calls.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&decompress_text(&self.tool_calls)).unwrap_or_default()
        };

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
        let context_summary = opt_blob_to_text(row, 11)?;
        let context_retained_from = row.get::<_, i64>(12)? as usize;
        let system_prompt = blob_or_empty_to_text(row, 13)?;
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
            context_summary,
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

/// Textual fields that can be searched without materializing complete
/// [`Message`] values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionSearchFields {
    pub session: bool,
    pub content: bool,
    pub reasoning: bool,
    pub tool_call: bool,
    pub metadata: bool,
    pub app_data: bool,
    pub attachment: bool,
    pub tool_output: bool,
}

impl SessionSearchFields {
    /// The message fields used by the default CLI search.
    pub fn message() -> Self {
        Self {
            content: true,
            reasoning: true,
            tool_call: true,
            metadata: true,
            app_data: true,
            ..Self::default()
        }
    }

    /// All searchable textual fields. Binary image data is deliberately
    /// excluded from this set.
    pub fn all() -> Self {
        Self {
            session: true,
            content: true,
            reasoning: true,
            tool_call: true,
            metadata: true,
            app_data: true,
            attachment: true,
            tool_output: true,
        }
    }

    fn includes_message(&self) -> bool {
        self.content
            || self.reasoning
            || self.tool_call
            || self.metadata
            || self.app_data
            || self.attachment
    }
}

/// Read-only options for searching stored session data.
#[derive(Clone, Debug)]
pub struct SessionSearchOptions {
    pub query: String,
    pub fields: SessionSearchFields,
    pub session_id: Option<Uuid>,
    pub workspace_root: Option<String>,
    pub roles: Vec<String>,
    pub case_sensitive: bool,
    pub context_chars: usize,
    pub limit: usize,
    pub offset: usize,
}

/// Kind of record containing a search match.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionSearchHitKind {
    Session,
    Message,
    ToolOutput,
}

/// A compact, pipe-friendly search result that identifies the exact stored
/// record without returning the complete message payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSearchHit {
    pub kind: SessionSearchHitKind,
    pub session_id: Uuid,
    pub session_short_id: String,
    pub title: String,
    pub workspace_root: String,
    pub message_id: Option<Uuid>,
    pub sequence: Option<usize>,
    pub role: Option<String>,
    pub field: String,
    pub match_count: usize,
    pub snippet: String,
    pub tool_output_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

struct SearchSessionRow {
    session_id: Uuid,
    parent_session_id: Option<Uuid>,
    title: String,
    workspace_root: String,
    provider_id: String,
    provider_display_name: String,
    model_id: String,
    model_display_name: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    status: String,
    snapshot_start_hash: Option<String>,
    context_summary: Option<Vec<u8>>,
    system_prompt: Option<Vec<u8>>,
}

struct SearchMessageRow {
    message_id: Uuid,
    session_id: Uuid,
    title: String,
    workspace_root: String,
    role: String,
    created_at: DateTime<Utc>,
    content: Vec<u8>,
    reasoning: Vec<u8>,
    tool_calls: Vec<u8>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    metadata: Vec<u8>,
    mode: Option<String>,
    app_data: Vec<u8>,
    sequence: usize,
    attachments: Option<String>,
}

struct SearchableAttachment {
    kind: String,
    path: Option<String>,
    content: Option<String>,
    tool_output: Option<String>,
    tree: Option<String>,
    filename: Option<String>,
    mime: Option<String>,
}

impl<'de> Deserialize<'de> for SearchableAttachment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AttachmentVisitor;

        impl<'de> serde::de::Visitor<'de> for AttachmentVisitor {
            type Value = SearchableAttachment;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a message attachment object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut attachment = SearchableAttachment {
                    kind: String::new(),
                    path: None,
                    content: None,
                    tool_output: None,
                    tree: None,
                    filename: None,
                    mime: None,
                };

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => attachment.kind = map.next_value()?,
                        "path" => attachment.path = map.next_value()?,
                        "content" => attachment.content = map.next_value()?,
                        "tool_output" => attachment.tool_output = map.next_value()?,
                        "tree" => attachment.tree = map.next_value()?,
                        "filename" => attachment.filename = map.next_value()?,
                        "mime" => attachment.mime = map.next_value()?,
                        // Image bytes and other fields are consumed without
                        // allocating their values.
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }

                Ok(attachment)
            }
        }

        deserializer.deserialize_map(AttachmentVisitor)
    }
}

fn parse_search_datetime(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_default()
}

fn parse_search_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_default()
}

fn short_session_id(session_id: Uuid) -> String {
    tidev_llm::short_session_id(session_id)
}

fn make_search_snippet(
    text: &str,
    match_start: usize,
    match_chars: usize,
    context_chars: usize,
) -> String {
    let total_chars = text.chars().count();
    let start = match_start.saturating_sub(context_chars);
    let end = match_start
        .saturating_add(match_chars)
        .saturating_add(context_chars)
        .min(total_chars);
    let snippet: String = text.chars().skip(start).take(end - start).collect();
    let mut result = String::with_capacity(snippet.len() + 6);
    if start > 0 {
        result.push('…');
    }
    result.push_str(&snippet);
    if end < total_chars {
        result.push('…');
    }
    result
}

fn find_search_match(
    text: &str,
    query: &str,
    case_sensitive: bool,
    context_chars: usize,
) -> Option<(usize, String)> {
    if query.is_empty() {
        return None;
    }

    if case_sensitive {
        let first_byte = text.find(query)?;
        let match_chars = query.chars().count();
        let match_start = text[..first_byte].chars().count();
        let match_count = text.match_indices(query).count();
        return Some((
            match_count,
            make_search_snippet(text, match_start, match_chars, context_chars),
        ));
    }

    let lowered_text = text.to_lowercase();
    let lowered_query = query.to_lowercase();
    let first_byte = lowered_text.find(&lowered_query)?;
    let match_start = lowered_text[..first_byte].chars().count();
    let match_chars = lowered_query.chars().count();
    let match_count = lowered_text.match_indices(&lowered_query).count();

    Some((
        match_count,
        make_search_snippet(text, match_start, match_chars, context_chars),
    ))
}

#[allow(clippy::too_many_arguments)]
fn push_search_hit(
    hits: &mut Vec<SessionSearchHit>,
    kind: SessionSearchHitKind,
    session_id: Uuid,
    title: &str,
    workspace_root: &str,
    message_id: Option<Uuid>,
    sequence: Option<usize>,
    role: Option<&str>,
    field: &str,
    text: &str,
    tool_output_id: Option<&str>,
    created_at: DateTime<Utc>,
    options: &SessionSearchOptions,
) {
    let Some((match_count, snippet)) = find_search_match(
        text,
        &options.query,
        options.case_sensitive,
        options.context_chars,
    ) else {
        return;
    };

    hits.push(SessionSearchHit {
        kind,
        session_id,
        session_short_id: short_session_id(session_id),
        title: title.to_string(),
        workspace_root: workspace_root.to_string(),
        message_id,
        sequence,
        role: role.map(str::to_string),
        field: field.to_string(),
        match_count,
        snippet,
        tool_output_id: tool_output_id.map(str::to_string),
        created_at,
    });
}

fn push_attachment_search_hits(
    hits: &mut Vec<SessionSearchHit>,
    row: &SearchMessageRow,
    raw_attachments: &str,
    options: &SessionSearchOptions,
) {
    let Ok(attachments) = serde_json::from_str::<Vec<SearchableAttachment>>(raw_attachments) else {
        return;
    };

    for attachment in attachments {
        let mut fields = Vec::new();
        match attachment.kind.as_str() {
            "file_reference" => {
                if let Some(value) = attachment.path {
                    fields.push(("path", value));
                }
                if let Some(value) = attachment.content {
                    fields.push(("content", value));
                }
                if let Some(value) = attachment.tool_output {
                    fields.push(("tool_output", value));
                }
            }
            "directory_reference" => {
                if let Some(value) = attachment.path {
                    fields.push(("path", value));
                }
                if let Some(value) = attachment.tree {
                    fields.push(("tree", value));
                }
            }
            "image" => {
                if let Some(value) = attachment.filename {
                    fields.push(("filename", value));
                }
                if let Some(value) = attachment.mime {
                    fields.push(("mime", value));
                }
            }
            _ => {}
        }

        for (field_name, value) in fields {
            let field = format!("message.attachment.{field_name}");
            push_search_hit(
                hits,
                SessionSearchHitKind::Message,
                row.session_id,
                &row.title,
                &row.workspace_root,
                Some(row.message_id),
                Some(row.sequence),
                Some(&row.role),
                &field,
                &value,
                None,
                row.created_at,
                options,
            );
        }
    }
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

/// Daily activity totals derived directly from assistant messages with usage.
///
/// This intentionally exposes no message content. The web statistics API uses
/// it to render a fixed-size activity calendar without loading raw messages.
#[derive(Debug, Clone)]
pub struct UsageActivityDay {
    pub date: String,
    pub request_count: u64,
    pub total_tokens: u64,
}

/// Distinct sessions with at least one token-bearing assistant response in a time bucket.
#[derive(Debug, Clone)]
pub struct UsageActiveSessionBucket {
    pub time_bucket: String,
    pub active_sessions: u64,
}

/// Aggregate response activity for one weekday and hour pair.
#[derive(Debug, Clone)]
pub struct UsageRhythmCell {
    pub weekday: u8,
    pub hour: u8,
    pub request_count: u64,
    pub total_tokens: u64,
}

/// Token usage for one model series in a time bucket.
#[derive(Debug, Clone)]
pub struct UsageModelMixBucket {
    pub time_bucket: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub is_other: bool,
    pub total_tokens: u64,
}

/// Aggregate response counts for one total-token size range.
#[derive(Debug, Clone)]
pub struct UsageRequestSizeBucket {
    pub lower_bound: u64,
    pub upper_bound: Option<u64>,
    pub request_count: u64,
    pub total_tokens: u64,
}

const USAGE_MESSAGE_FILTER: &str = "m.role = 'assistant' AND (m.total_tokens IS NOT NULL OR m.input_tokens IS NOT NULL OR m.output_tokens IS NOT NULL)";
const USAGE_REQUEST_SIZE_RANGES: [(u64, Option<u64>); 5] = [
    (0, Some(2_048)),
    (2_048, Some(8_192)),
    (8_192, Some(32_768)),
    (32_768, Some(131_072)),
    (131_072, None),
];

fn usage_range_filter(
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
) -> (String, Vec<Box<dyn ToSql>>) {
    let mut clauses = Vec::new();
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(start) = start {
        clauses.push("m.created_at >= ?".to_owned());
        values.push(Box::new(start.to_rfc3339()));
    }
    if let Some(end) = end {
        clauses.push("m.created_at < ?".to_owned());
        values.push(Box::new(end.to_rfc3339()));
    }
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", clauses.join(" AND "))
    };
    (filter, values)
}

fn usage_time_bucket_expression(granularity: &str, timestamp: &str) -> String {
    match granularity {
        "hour" => format!("strftime('%Y-%m-%dT%H:00:00Z', {timestamp})"),
        "week" => format!(
            "strftime('%Y-%m-%dT00:00:00Z', date({timestamp}, '-' || ((CAST(strftime('%w', {timestamp}) AS INTEGER) + 6) % 7) || ' days'))"
        ),
        "month" => format!("strftime('%Y-%m-01T00:00:00Z', {timestamp})"),
        _ => format!("strftime('%Y-%m-%dT00:00:00Z', {timestamp})"),
    }
}
