use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params, types::Type};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

use crate::session::{Conversation, Message, MessageRole};

const SCHEMA_VERSION: i64 = 1;

pub struct SessionStore {
    connection: Connection,
    path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub session_id: Uuid,
    pub provider_id: String,
    pub model_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }

        let connection = Connection::open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.execute_batch(SCHEMA_SQL)?;
        connection.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;

        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn create_session(
        &self,
        session_id: Uuid,
        provider_id: &str,
        model_id: &str,
        title: &str,
    ) -> Result<SessionRecord> {
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let session_id_text = session_id.to_string();

        self.connection.execute(
            "INSERT INTO sessions (id, provider_id, model_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id_text, provider_id, model_id, title, now_text, now_text],
        )?;

        Ok(SessionRecord {
            session_id,
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn update_session_title(&self, session_id: Uuid, title: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, session_id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_session_model(
        &self,
        session_id: Uuid,
        provider_id: &str,
        model_id: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
            "UPDATE sessions SET provider_id = ?1, model_id = ?2, updated_at = ?3 WHERE id = ?4",
            params![provider_id, model_id, now, session_id.to_string()],
        )?;
        Ok(())
    }

    pub fn append_message(&self, session_id: Uuid, message: &Message) -> Result<()> {
        self.connection.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, streaming) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                message.id.to_string(),
                session_id.to_string(),
                message.role.db_value(),
                message.content,
                message.created_at.to_rfc3339(),
                if message.streaming { 1_i64 } else { 0_i64 },
            ],
        )?;

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn load_latest_session(&self) -> Result<Option<SessionRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, provider_id, model_id, title, created_at, updated_at FROM sessions ORDER BY updated_at DESC LIMIT 1",
        )?;

        let record = statement
            .query_row([], |row| Self::session_from_row(row))
            .optional()?;

        Ok(record)
    }

    pub fn load_conversation(&self, session_id: Uuid) -> Result<Option<Conversation>> {
        let record = self.load_session_record(session_id)?;

        let Some(record) = record else {
            return Ok(None);
        };

        let messages = self.load_messages(session_id)?;
        Ok(Some(Conversation {
            session_id: record.session_id,
            provider_id: record.provider_id,
            model_id: record.model_id,
            title: record.title,
            created_at: record.created_at,
            updated_at: record.updated_at,
            messages,
        }))
    }

    pub fn load_session_record(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, provider_id, model_id, title, created_at, updated_at FROM sessions WHERE id = ?1 LIMIT 1",
        )?;

        let record = statement
            .query_row(params![session_id.to_string()], |row| {
                Self::session_from_row(row)
            })
            .optional()?;

        Ok(record)
    }

    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT id, role, content, created_at, streaming FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;

        let rows = statement.query_map(params![session_id.to_string()], |row| {
            let id = row.get::<_, String>(0)?;
            let role = row.get::<_, String>(1)?;
            let content = row.get::<_, String>(2)?;
            let created_at = row.get::<_, String>(3)?;
            let streaming = row.get::<_, i64>(4)? != 0;

            Ok(Message {
                id: Uuid::parse_str(&id).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })?,
                role: MessageRole::from_db_value(&role),
                content,
                created_at: parse_datetime(&created_at).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
                })?,
                streaming,
            })
        })?;

        let mut messages = Vec::new();
        for message in rows {
            messages.push(message?);
        }

        Ok(messages)
    }

    fn touch_session(&self, session_id: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id.to_string()],
        )?;
        Ok(())
    }

    fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
        let id = row.get::<_, String>(0)?;
        let provider_id = row.get::<_, String>(1)?;
        let model_id = row.get::<_, String>(2)?;
        let title = row.get::<_, String>(3)?;
        let created_at = row.get::<_, String>(4)?;
        let updated_at = row.get::<_, String>(5)?;

        Ok(SessionRecord {
            session_id: Uuid::parse_str(&id).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })?,
            provider_id,
            model_id,
            title,
            created_at: parse_datetime(&created_at).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })?,
            updated_at: parse_datetime(&updated_at).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(error))
            })?,
        })
    }
}

fn parse_datetime(value: &str) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    streaming INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_messages_session_created_at
    ON messages(session_id, created_at);

CREATE TABLE IF NOT EXISTS tool_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    input_json TEXT NOT NULL,
    output_text TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_events_session_created_at
    ON tool_events(session_id, created_at);
"#;
