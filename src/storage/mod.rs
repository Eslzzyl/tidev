mod schema;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Type};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    session::{Conversation, Message, MessageRole, ToolCall},
    stats::{Granularity, ModelUsageEntry, StatsEntry, TimeRangeStats, UsageSummary},
    tooling::TodoItem,
};

use schema::{SCHEMA_SQL, SCHEMA_VERSION, SESSION_SELECT_COLUMNS};

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
    pub context_summary: Option<String>,
    pub context_retained_from: usize,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSessionCount {
    pub workspace_root: String,
    pub session_count: i64,
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
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.busy_timeout(Duration::from_secs(5))?;
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

        self.connection.execute(
            "INSERT INTO sessions (id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, context_summary, context_retained_from) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                session_id_text.clone(),
                provider_id,
                provider_display_name,
                model_id,
                model_display_name,
                title,
                now_text,
                now_text,
                "",
                0_i64,
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
            context_summary: None,
            context_retained_from: 0,
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

        self.connection.execute(
            "INSERT INTO sessions (id, parent_session_id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, context_summary, context_retained_from) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                "",
                0_i64,
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
            context_summary: None,
            context_retained_from: 0,
        })
    }

    pub fn update_session_context_state(
        &self,
        session_id: Uuid,
        summary: Option<&str>,
        retained_from: usize,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
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
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, snapshot_hash, patch_files) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
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
                message.completed_at.map(|t| t.to_rfc3339()),
                if message.streaming { 1_i64 } else { 0_i64 },
                message.input_tokens,
                message.output_tokens,
                message.total_tokens,
                message.cache_read_tokens,
                message.cache_write_tokens,
                message.model_id,
                message.snapshot_hash,
                message.patch_files,
            ],
        )?;

        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn update_message(&self, session_id: Uuid, message: &Message) -> Result<()> {
        let tool_calls =
            serde_json::to_string(&message.tool_calls).context("failed to serialize tool calls")?;
        let attachments = serde_json::to_string(&message.attachments)
            .context("failed to serialize attachments")?;

        self.connection.execute(
            "UPDATE messages SET role = ?3, content = ?4, attachments = ?5, reasoning = ?6, tool_calls = ?7, tool_call_id = ?8, tool_name = ?9, created_at = ?10, completed_at = ?11, streaming = ?12, input_tokens = ?13, output_tokens = ?14, total_tokens = ?15, cache_read_tokens = ?16, cache_write_tokens = ?17, model_id = ?18, snapshot_hash = ?19, patch_files = ?20 WHERE session_id = ?1 AND id = ?2",
            params![
                session_id.to_string(),
                message.id.to_string(),
                message.role.db_value(),
                message.content,
                attachments,
                message.reasoning,
                tool_calls,
                message.tool_call_id,
                message.tool_name,
                message.created_at.to_rfc3339(),
                message.completed_at.map(|t| t.to_rfc3339()),
                if message.streaming { 1_i64 } else { 0_i64 },
                message.input_tokens,
                message.output_tokens,
                message.total_tokens,
                message.cache_read_tokens,
                message.cache_write_tokens,
                message.model_id,
                message.snapshot_hash,
                message.patch_files,
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
                "DELETE FROM tool_events WHERE session_id = ?1 AND message_id = ?2",
                params![session_id.to_string(), message_id.to_string()],
            )?;
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
        message_id: Uuid,
        tool_name: &str,
        input_json: &str,
        output_text: &str,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO tool_events (id, session_id, message_id, tool_name, input_json, output_text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                Uuid::new_v4().to_string(),
                session_id.to_string(),
                message_id.to_string(),
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

    pub fn copy_tool_permissions(&self, from_session_id: Uuid, to_session_id: Uuid) -> Result<()> {
        self.connection.execute(
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
        self.connection.execute(
            "DELETE FROM todos WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;

        if !todos.is_empty() {
            let mut stmt = self.connection.prepare(
                "INSERT INTO todos (session_id, position, content, status, priority) VALUES (?1, ?2, ?3, ?4, ?5)"
            )?;

            for (position, todo) in todos.iter().enumerate() {
                stmt.execute(params![
                    session_id.to_string(),
                    position as i64,
                    &todo.content,
                    &todo.status,
                    &todo.priority,
                ])?;
            }
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
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.parent_session_id IS NULL ORDER BY s.updated_at DESC LIMIT 1"
        );
        let mut statement = self.connection.prepare(&sql)?;

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

    pub fn set_revert_message_id(
        &self,
        session_id: Uuid,
        message_id: Option<Uuid>,
        redo_snapshot: Option<&str>,
    ) -> Result<()> {
        match message_id {
            Some(message_id) => {
                self.connection.execute(
                    "INSERT INTO session_reverts (session_id, message_id, redo_snapshot, created_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(session_id) DO UPDATE SET message_id = excluded.message_id, redo_snapshot = excluded.redo_snapshot, created_at = excluded.created_at",
                    params![
                        session_id.to_string(),
                        message_id.to_string(),
                        redo_snapshot,
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
        self.set_revert_message_id(session_id, None, None)
    }

    pub fn load_redo_snapshot(&self, session_id: Uuid) -> Result<Option<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT redo_snapshot FROM session_reverts WHERE session_id = ?1 LIMIT 1")?;

        let snapshot = statement
            .query_row(params![session_id.to_string()], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten();

        Ok(snapshot)
    }

    pub fn load_tool_event_output(
        &self,
        session_id: Uuid,
        message_id: Uuid,
    ) -> Result<Option<String>> {
        let mut statement = self.connection.prepare(
            "SELECT output_text FROM tool_events WHERE session_id = ?1 AND message_id = ?2 ORDER BY created_at DESC LIMIT 1",
        )?;

        let output = statement
            .query_row(
                params![session_id.to_string(), message_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        Ok(output)
    }

    pub fn update_message_snapshot(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        snapshot_hash: &str,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE messages SET snapshot_hash = ?1 WHERE session_id = ?2 AND id = ?3",
            params![
                snapshot_hash,
                session_id.to_string(),
                message_id.to_string()
            ],
        )?;
        Ok(())
    }

    pub fn update_message_patch(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        patch_files: &str,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE messages SET patch_files = ?1 WHERE session_id = ?2 AND id = ?3",
            params![patch_files, session_id.to_string(), message_id.to_string()],
        )?;
        Ok(())
    }

    pub fn load_session_record(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s LEFT JOIN session_workspaces sw ON sw.session_id = s.id WHERE s.id = ?1 LIMIT 1"
        );
        let mut statement = self.connection.prepare(&sql)?;

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
        self.connection.execute(
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
        let mut statement = self.connection.prepare(
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
        self.connection.execute(
            "DELETE FROM gateway_chat_sessions WHERE platform = ?1 AND chat_key = ?2",
            params![platform, chat_key],
        )?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, tool_name, created_at, completed_at, streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, snapshot_hash, patch_files FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
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
            let completed_at = row.get::<_, Option<String>>(9)?;
            let streaming = row.get::<_, i64>(10)? != 0;
            let input_tokens = row.get::<_, Option<u32>>(11)?;
            let output_tokens = row.get::<_, Option<u32>>(12)?;
            let total_tokens = row.get::<_, Option<u32>>(13)?;
            let cache_read_tokens = row.get::<_, Option<u32>>(14)?;
            let cache_write_tokens = row.get::<_, Option<u32>>(15)?;
            let model_id = row.get::<_, Option<String>>(16)?;
            let snapshot_hash = row.get::<_, Option<String>>(17)?;
            let patch_files = row.get::<_, Option<String>>(18)?;

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
            message.cache_read_tokens = cache_read_tokens;
            message.cache_write_tokens = cache_write_tokens;
            message.model_id = model_id;
            message.snapshot_hash = snapshot_hash;
            message.patch_files = patch_files;
            message.completed_at = completed_at.and_then(|s| parse_datetime(&s).ok());
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
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s INNER JOIN session_workspaces sw ON sw.session_id = s.id WHERE sw.workspace_root = ?1 AND s.parent_session_id IS NULL ORDER BY s.updated_at DESC, s.created_at DESC"
        );
        let mut statement = self.connection.prepare(&sql)?;

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
        let mut statement = self.connection.prepare(&sql)?;

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
        let mut statement = self.connection.prepare(&sql)?;

        let rows = statement.query_map([], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn delete_session(&self, session_id: Uuid) -> Result<()> {
        self.connection.execute(
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
        self.connection.execute(&sql, params_from_iter(params))?;

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
        let mut statement = self.connection.prepare(&sql)?;

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
        let mut statement = self.connection.prepare(&sql)?;

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
        let mut statement = self.connection.prepare(
            "SELECT sw.workspace_root, COUNT(*) as cnt FROM session_workspaces sw INNER JOIN sessions s ON s.id = sw.session_id WHERE s.parent_session_id IS NULL GROUP BY sw.workspace_root ORDER BY cnt DESC",
        )?;

        let rows = statement.query_map([], |row| {
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
        let mut statement = self.connection.prepare(&sql)?;

        let rows = statement.query_map(params![cutoff_text], Self::session_from_row)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }

        Ok(records)
    }

    pub fn get_current_workspace_sessions_count(&self, workspace_root: &Path) -> Result<i64> {
        let workspace_root = workspace_root.display().to_string();
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM session_workspaces sw INNER JOIN sessions s ON s.id = sw.session_id WHERE sw.workspace_root = ?1 AND s.parent_session_id IS NULL",
            params![workspace_root],
            |row| row.get(0),
        )?;
        Ok(count)
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
        let context_summary = row.get::<_, String>(9)?;
        let context_retained_from = row.get::<_, i64>(10)? as usize;
        let workspace_root = row.get::<_, String>(11)?;

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
            context_summary: if context_summary.trim().is_empty() {
                None
            } else {
                Some(context_summary)
            },
            context_retained_from,
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

        self.connection.execute(
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

    pub fn get_time_range_stats(
        &self,
        granularity: Granularity,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<TimeRangeStats> {
        let mut entries = Vec::new();
        let mut summary = UsageSummary::default();

        let granularity_str = granularity.as_str();

        let mut stmt = self.connection.prepare(
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
        let mut entries = Vec::new();

        let start_text = start.to_rfc3339();
        let end_text = end.to_rfc3339();

        let mut stmt = self.connection.prepare(
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
            WHERE created_at >= ?1 AND created_at <= ?2
            GROUP BY provider_id, model_id
            ORDER BY total_tokens DESC
            "#,
        )?;

        let rows = stmt.query_map(params![start_text, end_text], |row| {
            Ok(ModelUsageEntry {
                provider_id: row.get(0)?,
                model_id: row.get(1)?,
                input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                cache_read_tokens: row.get(4)?,
                cache_write_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                request_count: row.get(7)?,
            })
        })?;

        for row in rows {
            entries.push(row?);
        }

        Ok(entries)
    }

    pub fn get_all_time_summary(&self) -> Result<UsageSummary> {
        let mut stmt = self.connection.prepare(
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
        self.connection.execute(
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
        let mut stmt = self.connection.prepare(
            r#"SELECT file_path, read_at, mtime, size FROM file_reads WHERE session_id = ?1"#,
        )?;

        let records = stmt
            .query_map(params![session_id.to_string()], |row| {
                let read_at_str: String = row.get(1)?;
                Ok(FileReadRecord {
                    file_path: row.get(0)?,
                    read_at: DateTime::parse_from_rfc3339(&read_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    mtime: row.get(2)?,
                    size: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(records)
    }

    /// Delete all file reads for a session
    pub fn clear_file_reads(&self, session_id: Uuid) -> Result<()> {
        self.connection.execute(
            "DELETE FROM file_reads WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
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
