use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, types::Type, Connection, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

use crate::{
    session::{Conversation, Message, MessageRole, ToolCall},
    tooling::TodoItem,
};

const SCHEMA_VERSION: i64 = 8;

pub struct SessionStore {
    connection: Connection,
    path: PathBuf,
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

        self.connection.execute(
            "INSERT INTO sessions (id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id_text.clone(),
                provider_id,
                provider_display_name,
                model_id,
                model_display_name,
                title,
                now_text,
                now_text,
            ],
        )?;

        self.connection.execute(
            "INSERT INTO session_workspaces (session_id, workspace_root) VALUES (?1, ?2)",
            params![session_id_text, workspace_root.clone()],
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
        })
    }

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

        self.connection.execute(
            "INSERT INTO sessions (id, parent_session_id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id_text.clone(),
                parent_session_id_text,
                provider_id,
                provider_display_name,
                model_id,
                model_display_name,
                title,
                now_text,
                now_text,
            ],
        )?;

        self.connection.execute(
            "INSERT INTO session_workspaces (session_id, workspace_root) VALUES (?1, ?2)",
            params![session_id_text, workspace_root.clone()],
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
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
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

    pub fn append_message(&self, session_id: Uuid, message: &Message) -> Result<()> {
        let tool_calls =
            serde_json::to_string(&message.tool_calls).context("failed to serialize tool calls")?;
        let attachments = serde_json::to_string(&message.attachments)
            .context("failed to serialize attachments")?;
        self.connection.execute(
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, created_at, streaming, input_tokens, output_tokens, total_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                message.id.to_string(),
                session_id.to_string(),
                message.role.db_value(),
                message.content,
                attachments,
                message.reasoning,
                tool_calls,
                message.tool_call_id,
                message.tool_name,
                message.created_at.to_rfc3339(),
                if message.streaming { 1_i64 } else { 0_i64 },
                message.input_tokens,
                message.output_tokens,
                message.total_tokens,
            ],
        )?;

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn delete_messages(&self, session_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        for message_id in message_ids {
            self.connection.execute(
                "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
                params![session_id.to_string(), message_id.to_string()],
            )?;
        }

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn append_tool_event(
        &self,
        session_id: Uuid,
        tool_name: &str,
        input_json: &str,
        output_text: &str,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO tool_events (id, session_id, tool_name, input_json, output_text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                Uuid::new_v4().to_string(),
                session_id.to_string(),
                tool_name,
                input_json,
                output_text,
                Utc::now().to_rfc3339(),
            ],
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
        self.connection.execute(
            "INSERT INTO tool_permissions (session_id, tool_name, allowed, created_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(session_id, tool_name) DO UPDATE SET allowed = excluded.allowed, created_at = excluded.created_at",
            params![
                session_id.to_string(),
                tool_name,
                if allowed { 1_i64 } else { 0_i64 },
                Utc::now().to_rfc3339(),
            ],
        )?;

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn load_tool_permission(&self, session_id: Uuid, tool_name: &str) -> Result<Option<bool>> {
        let mut statement = self.connection.prepare(
            "SELECT allowed FROM tool_permissions WHERE session_id = ?1 AND tool_name = ?2 LIMIT 1",
        )?;

        let value = statement
            .query_row(params![session_id.to_string(), tool_name], |row| {
                Ok(row.get::<_, i64>(0)? != 0)
            })
            .optional()?;

        Ok(value)
    }

    pub fn replace_todos(&self, session_id: Uuid, todos: &[TodoItem]) -> Result<()> {
        self.connection.execute(
            "DELETE FROM todos WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;

        for (position, todo) in todos.iter().enumerate() {
            self.connection.execute(
                "INSERT INTO todos (session_id, position, content, status, priority) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    session_id.to_string(),
                    position as i64,
                    &todo.content,
                    &todo.status,
                    &todo.priority,
                ],
            )?;
        }

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn load_todos(&self, session_id: Uuid) -> Result<Vec<TodoItem>> {
        let mut statement = self.connection.prepare(
            "SELECT content, status, priority FROM todos WHERE session_id = ?1 ORDER BY position ASC",
        )?;

        let rows = statement.query_map(params![session_id.to_string()], |row| {
            Ok(TodoItem {
                content: row.get::<_, String>(0)?,
                status: row.get::<_, String>(1)?,
                priority: row.get::<_, String>(2)?,
            })
        })?;

        let mut todos = Vec::new();
        for row in rows {
            todos.push(row?);
        }

        Ok(todos)
    }

    pub fn load_latest_session(&self) -> Result<Option<SessionRecord>> {
        let mut statement = self.connection.prepare(
              "SELECT s.id, s.parent_session_id, s.provider_id, s.provider_display_name, s.model_id, s.model_display_name, s.title, s.created_at, s.updated_at, COALESCE(sw.workspace_root, '') FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id ORDER BY s.updated_at DESC LIMIT 1",
        )?;

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
            messages,
            revert_message_id,
        }))
    }

    pub fn load_revert_message_id(&self, session_id: Uuid) -> Result<Option<Uuid>> {
        let mut statement = self
            .connection
            .prepare("SELECT message_id FROM session_reverts WHERE session_id = ?1 LIMIT 1")?;

        let message_id = statement
            .query_row(params![session_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .map(|value| {
                Uuid::parse_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })
            })
            .transpose()?;

        Ok(message_id)
    }

    pub fn set_revert_message_id(&self, session_id: Uuid, message_id: Option<Uuid>) -> Result<()> {
        match message_id {
            Some(message_id) => {
                self.connection.execute(
                    "INSERT INTO session_reverts (session_id, message_id, created_at) VALUES (?1, ?2, ?3) ON CONFLICT(session_id) DO UPDATE SET message_id = excluded.message_id, created_at = excluded.created_at",
                    params![
                        session_id.to_string(),
                        message_id.to_string(),
                        Utc::now().to_rfc3339(),
                    ],
                )?;
            }
            None => {
                self.connection.execute(
                    "DELETE FROM session_reverts WHERE session_id = ?1",
                    params![session_id.to_string()],
                )?;
            }
        }

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn clear_revert_message_id(&self, session_id: Uuid) -> Result<()> {
        self.set_revert_message_id(session_id, None)
    }

    pub fn load_session_record(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let mut statement = self.connection.prepare(
              "SELECT s.id, s.parent_session_id, s.provider_id, s.provider_display_name, s.model_id, s.model_display_name, s.title, s.created_at, s.updated_at, COALESCE(sw.workspace_root, '') FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.id = ?1 LIMIT 1",
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
            "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, created_at, streaming, input_tokens, output_tokens, total_tokens FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;

        let rows = statement.query_map(params![session_id.to_string()], |row| {
            let id = row.get::<_, String>(0)?;
            let role = row.get::<_, String>(1)?;
            let content = row.get::<_, String>(2)?;
            let attachments = row.get::<_, String>(3)?;
            let reasoning = row.get::<_, String>(4)?;
            let tool_calls = row.get::<_, String>(5)?;
            let tool_call_id = row.get::<_, Option<String>>(6)?;
            let tool_name = row.get::<_, Option<String>>(7)?;
            let created_at = row.get::<_, String>(8)?;
            let streaming = row.get::<_, i64>(9)? != 0;
            let input_tokens = row.get::<_, Option<u32>>(10)?;
            let output_tokens = row.get::<_, Option<u32>>(11)?;
            let total_tokens = row.get::<_, Option<u32>>(12)?;

            let attachments = serde_json::from_str(&attachments).unwrap_or_default();
            let tool_calls: Vec<ToolCall> = serde_json::from_str(&tool_calls).unwrap_or_default();

            let mut message = Message::persisted(
                Uuid::parse_str(&id).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })?,
                MessageRole::from_db_value(&role),
                content,
                parse_datetime(&created_at).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
                })?,
                streaming,
            );
            message.attachments = attachments;
            message.reasoning = reasoning;
            message.tool_calls = tool_calls;
            message.tool_call_id = tool_call_id;
            message.tool_name = tool_name;
            message.input_tokens = input_tokens;
            message.output_tokens = output_tokens;
            message.total_tokens = total_tokens;
            Ok(message)
        })?;

        let mut messages = Vec::new();
        for message in rows {
            messages.push(message?);
        }

        Ok(messages)
    }

    pub fn load_sessions_for_workspace(&self, workspace_root: &Path) -> Result<Vec<SessionRecord>> {
        let workspace_root = workspace_root.display().to_string();
        let mut statement = self.connection.prepare(
              "SELECT s.id, s.parent_session_id, s.provider_id, s.provider_display_name, s.model_id, s.model_display_name, s.title, s.created_at, s.updated_at, sw.workspace_root FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE sw.workspace_root = ?1 ORDER BY s.updated_at DESC, s.created_at DESC",
        )?;

        let rows = statement.query_map(params![workspace_root], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn load_child_sessions(&self, parent_session_id: Uuid) -> Result<Vec<SessionRecord>> {
        let mut statement = self.connection.prepare(
                "SELECT s.id, s.parent_session_id, s.provider_id, s.provider_display_name, s.model_id, s.model_display_name, s.title, s.created_at, s.updated_at, COALESCE(sw.workspace_root, '') FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.parent_session_id = ?1 ORDER BY s.updated_at DESC, s.created_at DESC",
            )?;

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
        let parent_session_id = row.get::<_, Option<String>>(1)?;
        let provider_id = row.get::<_, String>(2)?;
        let provider_display_name = row.get::<_, String>(3)?;
        let model_id = row.get::<_, String>(4)?;
        let model_display_name = row.get::<_, String>(5)?;
        let title = row.get::<_, String>(6)?;
        let created_at = row.get::<_, String>(7)?;
        let updated_at = row.get::<_, String>(8)?;
        let workspace_root = row.get::<_, String>(9)?;

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

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    provider_display_name TEXT NOT NULL,
    model_id TEXT NOT NULL,
    model_display_name TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_workspaces (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_root TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_reverts (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    attachments TEXT NOT NULL DEFAULT '[]',
    reasoning TEXT NOT NULL DEFAULT '',
    tool_calls TEXT NOT NULL DEFAULT '[]',
    tool_call_id TEXT,
    tool_name TEXT,
    created_at TEXT NOT NULL,
    streaming INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER
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

CREATE TABLE IF NOT EXISTS todos (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL,
    priority TEXT NOT NULL,
    PRIMARY KEY(session_id, position)
);

CREATE INDEX IF NOT EXISTS idx_todos_session_position
    ON todos(session_id, position);

CREATE TABLE IF NOT EXISTS tool_permissions (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    allowed INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(session_id, tool_name)
);

CREATE INDEX IF NOT EXISTS idx_tool_permissions_session_created_at
    ON tool_permissions(session_id, created_at);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_display_names_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "tidev-session-store-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));

        {
            let store = SessionStore::open(&path).expect("store should open");
            let session_id = uuid::Uuid::new_v4();

            let record = store
                .create_session(
                    session_id,
                    Path::new("/tmp/workspace"),
                    "deepseek",
                    "DeepSeek",
                    "deepseek-chat",
                    "DeepSeek Chat",
                    "Untitled session",
                )
                .expect("session should be created");

            assert_eq!(record.provider_id, "deepseek");
            assert_eq!(record.provider_display_name, "DeepSeek");
            assert_eq!(record.model_id, "deepseek-chat");
            assert_eq!(record.model_display_name, "DeepSeek Chat");
            assert_eq!(record.workspace_root, "/tmp/workspace");

            let loaded = store
                .load_session_record(session_id)
                .expect("session should load")
                .expect("session should exist");

            assert_eq!(loaded.provider_display_name, "DeepSeek");
            assert_eq!(loaded.model_display_name, "DeepSeek Chat");
            assert_eq!(loaded.workspace_root, "/tmp/workspace");

            let conversation = store
                .load_conversation(session_id)
                .expect("conversation should load")
                .expect("conversation should exist");

            assert_eq!(conversation.provider_display_name, "DeepSeek");
            assert_eq!(conversation.model_display_name, "DeepSeek Chat");
            assert_eq!(conversation.workspace_root, "/tmp/workspace");
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn child_session_round_trip_records_parent() {
        let path = std::env::temp_dir().join(format!(
            "tidev-session-store-child-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));

        {
            let store = SessionStore::open(&path).expect("store should open");
            let parent_session_id = uuid::Uuid::new_v4();
            let child_session_id = uuid::Uuid::new_v4();

            store
                .create_session(
                    parent_session_id,
                    Path::new("/tmp/workspace"),
                    "openai",
                    "OpenAI",
                    "gpt-4o",
                    "GPT-4o",
                    "Parent",
                )
                .expect("parent session should be created");

            let child_record = store
                .create_session_with_parent(
                    child_session_id,
                    parent_session_id,
                    Path::new("/tmp/workspace"),
                    "openai",
                    "OpenAI",
                    "gpt-4o",
                    "GPT-4o",
                    "Task: Child",
                )
                .expect("child session should be created");

            assert_eq!(child_record.parent_session_id, Some(parent_session_id));

            let loaded = store
                .load_session_record(child_session_id)
                .expect("child session should load")
                .expect("child session should exist");
            assert_eq!(loaded.parent_session_id, Some(parent_session_id));

            let children = store
                .load_child_sessions(parent_session_id)
                .expect("child sessions should load");
            assert_eq!(children.len(), 1);
            assert_eq!(children[0].session_id, child_session_id);
            assert_eq!(children[0].parent_session_id, Some(parent_session_id));
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workspace_session_listing_is_scoped_and_sorted() {
        let path = std::env::temp_dir().join(format!(
            "tidev-session-store-list-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));

        {
            let store = SessionStore::open(&path).expect("store should open");
            let shared_root = Path::new("/tmp/tidev-workspace-a");
            let other_root = Path::new("/tmp/tidev-workspace-b");

            let first = store
                .create_session(
                    uuid::Uuid::new_v4(),
                    shared_root,
                    "openai",
                    "OpenAI",
                    "gpt-4o",
                    "GPT-4o",
                    "First",
                )
                .expect("first session should be created");

            std::thread::sleep(std::time::Duration::from_millis(2));

            let second = store
                .create_session(
                    uuid::Uuid::new_v4(),
                    shared_root,
                    "openai",
                    "OpenAI",
                    "gpt-4o-mini",
                    "GPT-4o mini",
                    "Second",
                )
                .expect("second session should be created");

            store
                .create_session(
                    uuid::Uuid::new_v4(),
                    other_root,
                    "anthropic",
                    "Anthropic",
                    "claude",
                    "Claude",
                    "Other",
                )
                .expect("other session should be created");

            let sessions = store
                .load_sessions_for_workspace(shared_root)
                .expect("sessions should load");

            assert_eq!(sessions.len(), 2);
            assert_eq!(sessions[0].session_id, second.session_id);
            assert_eq!(sessions[1].session_id, first.session_id);
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn revert_marker_round_trips() {
        let path = std::env::temp_dir().join(format!(
            "tidev-session-store-revert-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));

        {
            let store = SessionStore::open(&path).expect("store should open");
            let session_id = uuid::Uuid::new_v4();
            let message_id = uuid::Uuid::new_v4();

            store
                .create_session(
                    session_id,
                    Path::new("/workspace"),
                    "deepseek",
                    "DeepSeek",
                    "deepseek-chat",
                    "DeepSeek Chat",
                    "Untitled session",
                )
                .expect("session should be created");

            assert_eq!(
                store
                    .load_revert_message_id(session_id)
                    .expect("revert should load"),
                None
            );

            store
                .set_revert_message_id(session_id, Some(message_id))
                .expect("revert should save");

            assert_eq!(
                store
                    .load_revert_message_id(session_id)
                    .expect("revert should load"),
                Some(message_id)
            );

            let conversation = store
                .load_conversation(session_id)
                .expect("conversation should load")
                .expect("conversation should exist");

            assert_eq!(conversation.revert_message_id, Some(message_id));

            store
                .clear_revert_message_id(session_id)
                .expect("revert should clear");

            assert_eq!(
                store
                    .load_revert_message_id(session_id)
                    .expect("revert should load"),
                None
            );
        }

        let _ = std::fs::remove_file(path);
    }
}
