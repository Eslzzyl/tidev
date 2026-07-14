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
use rusqlite::{Connection, OptionalExtension, named_params, params, params_from_iter, types::Type};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tidev_types::message::{Message, MessageRole};
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
            write_conn: Arc::clone(&self.write_conn),
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

    /// Query a single optional row from the read connection.
    fn read_query_opt<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        f: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(sql)?;
            stmt.query_row(params, f)
                .optional()
                .map_err(anyhow::Error::from)
        })
    }

    /// Query a single row from the read connection (error if not found).
    fn read_query_row<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        f: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<T> {
        self.read(|conn| {
            let mut stmt = conn.prepare(sql)?;
            stmt.query_row(params, f).map_err(anyhow::Error::from)
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

#[derive(Debug, Clone)]
pub struct FileReadRecord {
    pub file_path: String,
    pub read_at: DateTime<Utc>,
    pub mtime: Option<i64>,
    pub size: Option<i64>,
}

// ---------------------------------------------------------------------------
// Session CRUD
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Create a new session.
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
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![session_id.to_string(), parent_session_id.map(|id| id.to_string()), provider_id, provider_display_name, model_id, model_display_name, title, now, now],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO session_workspaces (session_id, workspace_root) VALUES (?1, ?2)",
            params![session_id.to_string(), workspace_root],
        )?;
        Ok(())
    }

    /// Load session record by ID.
    pub fn load_session(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             LEFT JOIN session_workspaces sw ON sw.session_id = s.id \
             WHERE s.id = ?1"
        );
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
                    context_summary: 11 => row.get::<_, Option<String>>(11)?,
                    context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                    system_prompt: 13 => row.get::<_, String>(13)?,
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

    /// List all sessions ordered by creation time (newest first).
    pub fn list_sessions(&self, limit: i64, offset: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             LEFT JOIN session_workspaces sw ON sw.session_id = s.id \
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
                context_summary: 11 => row.get::<_, Option<String>>(11)?,
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => row.get::<_, String>(13)?,
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
             LEFT JOIN session_workspaces sw ON sw.session_id = s.id \
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
                context_summary: 11 => row.get::<_, Option<String>>(11)?,
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => row.get::<_, String>(13)?,
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
             INNER JOIN session_workspaces sw ON sw.session_id = s.id \
             WHERE sw.workspace_root = ?1 AND s.parent_session_id IS NULL \
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
                        context_summary: 11 => row.get::<_, Option<String>>(11)?,
                        context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                        system_prompt: 13 => row.get::<_, String>(13)?,
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
                 INNER JOIN session_workspaces sw ON sw.session_id = s.id \
                 WHERE s.updated_at < :cutoff AND s.parent_session_id IS NULL \
                 ORDER BY sw.workspace_root, s.updated_at DESC"
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
                 INNER JOIN session_workspaces sw ON sw.session_id = s.id \
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
    pub fn delete_sessions_in_workspace(&self, workspace_root: &Path) -> Result<Vec<SessionRecord>> {
        let root = workspace_root.display().to_string();

        let records: Vec<SessionRecord> = self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 INNER JOIN session_workspaces sw ON sw.session_id = s.id \
                 WHERE sw.workspace_root = :workspace_root AND s.parent_session_id IS NULL \
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
    pub fn export_session_to_jsonl(
        &self,
        session_id: Uuid,
        export_dir: &Path,
    ) -> Result<PathBuf> {
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

    /// Count sessions in a workspace.
    pub fn get_current_workspace_sessions_count(&self, workspace_root: &Path) -> Result<i64> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM session_workspaces WHERE workspace_root = ?1",
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

    /// Load tool output content for a message from tool_outputs table.
    pub fn load_tool_output(&self, message_id: Uuid) -> Result<Option<String>> {
        self.read_query_opt(
            "SELECT output FROM tool_outputs WHERE message_id = :message_id",
            named_params! { ":message_id": message_id.to_string() },
            |row| {
                let blob: Vec<u8> = row.get(0)?;
                Ok(decompress_text(&blob))
            },
        )
    }

    /// Save tool output for a message.
    pub fn save_tool_output(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        tool_name: &str,
        output: &str,
    ) -> Result<()> {
        let compressed = compress_text(output);
        self.write_execute(
            "INSERT OR REPLACE INTO tool_outputs \
             (message_id, session_id, tool_name, output, created_at) \
             VALUES (:message_id, :session_id, :tool_name, :output, :created_at)",
            named_params! {
                ":message_id": message_id.to_string(),
                ":session_id": session_id.to_string(),
                ":tool_name": tool_name,
                ":output": compressed,
                ":created_at": Utc::now().to_rfc3339(),
            },
        )?;
        Ok(())
    }

    /// Delete tool outputs older than `max_age_days`.
    pub fn delete_expired_tool_outputs(&self, max_age_days: i64) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(max_age_days)).to_rfc3339();
        self.write_execute(
            "DELETE FROM tool_outputs WHERE created_at < :cutoff",
            named_params! { ":cutoff": cutoff },
        )
    }

    /// Start a background thread that periodically deletes tool outputs
    /// older than `max_age_days`.  The thread runs every `interval` and
    /// uses a cloned database connection so it does not block the main
    /// thread.
    pub fn start_output_cleanup(&self, max_age_days: i64, interval: std::time::Duration) {
        let conn = self.write_conn.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(interval);
            let cutoff =
                (Utc::now() - Duration::days(max_age_days)).to_rfc3339();
            match conn
                .lock()
                .unwrap()
                .execute(
                    "DELETE FROM tool_outputs WHERE created_at < :cutoff",
                    rusqlite::named_params! { ":cutoff": cutoff },
                ) {
                Ok(count) if count > 0 => {
                    log::info!("Cleaned up {count} old tool output(s)");
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!("Failed to clean old tool outputs: {e}");
                }
            }
        });
    }

    /// Remember a tool permission (allow/deny) for a session.
    pub fn remember_tool_permission(
        &self,
        session_id: Uuid,
        tool_name: &str,
        allowed: bool,
    ) -> Result<()> {
        self.write_execute(
            "INSERT INTO tool_permissions (session_id, tool_name, allowed, created_at) \
             VALUES (:session_id, :tool_name, :allowed, :created_at) \
             ON CONFLICT(session_id, tool_name) DO UPDATE \
             SET allowed = excluded.allowed, created_at = excluded.created_at",
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

    /// Save the user's thinking level preference for a specific model.
    pub fn save_model_thinking_level(
        &self,
        provider_id: &str,
        model_id: &str,
        thinking_level: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "INSERT OR REPLACE INTO model_thinking_levels \
             (provider_id, model_id, thinking_level, updated_at) \
             VALUES (:provider_id, :model_id, :thinking_level, :updated_at)",
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
            "SELECT thinking_level FROM model_thinking_levels \
             WHERE provider_id = :provider_id AND model_id = :model_id \
             LIMIT 1",
            named_params! {
                ":provider_id": provider_id,
                ":model_id": model_id,
            },
            |row| row.get::<_, String>(0),
        )
    }

    /// Load a single tool permission by permission key (tool_name) for a session.
    pub fn load_tool_permission(
        &self,
        session_id: Uuid,
        permission_key: &str,
    ) -> Result<Option<bool>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT allowed FROM tool_permissions \
                 WHERE session_id = ?1 AND tool_name = ?2 \
                 ORDER BY created_at DESC LIMIT 1",
            )?;
            let mut rows = stmt.query_map(
                params![session_id.to_string(), permission_key],
                |row| row.get::<_, i64>(0),
            )?;
            match rows.next() {
                Some(Ok(val)) => Ok(Some(val != 0)),
                _ => Ok(None),
            }
        })
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
                "SELECT sw.workspace_root, COUNT(*) as cnt FROM sessions s \
                 LEFT JOIN session_workspaces sw ON sw.session_id = s.id \
                 GROUP BY sw.workspace_root ORDER BY cnt DESC",
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
             LEFT JOIN session_workspaces sw ON sw.session_id = s.id \
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
                context_summary: 11 => row.get::<_, Option<String>>(11)?,
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => row.get::<_, String>(13)?,
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
                "SELECT DISTINCT workspace_root FROM session_workspaces \
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
    /// Append a message to a session.
    pub fn append_message(&self, session_id: Uuid, msg: &Message) -> Result<()> {
        let now = msg.created_at.to_rfc3339();
        let completed = msg.completed_at.map(|t| t.to_rfc3339());
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, \
             tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, \
             streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, \
             cache_write_tokens, model_id, tokens_per_second, snapshot_hash, patch_files, \
             file_diffs, mode, thinking_level) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
             ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
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
                msg.snapshot_hash,
                compress_text(msg.patch_files.as_deref().unwrap_or("")),
                compress_text(msg.file_diffs.as_deref().unwrap_or("")),
                msg.mode
                    .as_ref()
                    .map(|m| serde_json::to_string(m).unwrap_or_default()),
                msg.thinking_level
                    .as_ref()
                    .map(|t| serde_json::to_string(t).unwrap_or_default()),
            ],
        )?;
        // Update session timestamp
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load all messages for a session, ordered by creation time.
    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, \
             tool_name, metadata, created_at, completed_at, streaming, input_tokens, \
             output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, \
             tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, thinking_level \
             FROM messages WHERE session_id = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                let role_str: String = match row.get(1) {
                    Ok(s) => s,
                    Err(_) => return Ok(Message::new(MessageRole::User, "")),
                };
                let metadata_raw: Vec<u8> = row.get(8).unwrap_or_default();
                let metadata: tidev_types::message::ToolMetadata =
                    serde_json::from_str(&decompress_text(&metadata_raw)).unwrap_or_default();

                let content = row
                    .get::<_, Vec<u8>>(2)
                    .map(|b| decompress_text(&b))
                    .unwrap_or_else(|_| row.get::<_, String>(2).unwrap_or_default());

                let attachments_raw: String = row.get(3).unwrap_or_default();
                let attachments: Vec<tidev_types::message::MessageAttachment> =
                    serde_json::from_str(&attachments_raw).unwrap_or_default();

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

                let patch_files = row
                    .get::<_, Vec<u8>>(20)
                    .ok()
                    .filter(|b| !b.is_empty())
                    .map(|b| decompress_text(&b));

                let file_diffs = row
                    .get::<_, Vec<u8>>(21)
                    .ok()
                    .filter(|b| !b.is_empty())
                    .map(|b| decompress_text(&b));

                let mode: Option<String> = row.get(22).ok().flatten();
                let thinking_level: Option<String> = row.get(23).ok().flatten();

                Ok(Message {
                    id: Uuid::parse_str(&row.get::<_, String>(0).unwrap_or_default())
                        .unwrap_or_default(),
                    role: MessageRole::from_db_value(&role_str),
                    content,
                    attachments,
                    reasoning,
                    tool_calls: serde_json::from_str(&tool_calls_json).unwrap_or_default(),
                    tool_call_id: row.get(6).ok().flatten(),
                    tool_name: row.get(7).ok().flatten(),
                    metadata,
                    created_at: DateTime::parse_from_rfc3339(
                        &row.get::<_, String>(9).unwrap_or_default(),
                    )
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
                    snapshot_hash: row.get(19).ok().flatten(),
                    patch_files,
                    file_diffs,
                    mode: mode.and_then(|m| serde_json::from_str(&m).ok()),
                    thinking_level: thinking_level.and_then(|t| serde_json::from_str(&t).ok()),
                })
            })?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
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
        tool_calls: &[tidev_types::message::ToolCall],
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
        metadata: &tidev_types::message::ToolMetadata,
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

    /// Update message completion status.
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

    /// Get the most recent message for a session.
    pub fn get_last_message(&self, session_id: Uuid) -> Result<Option<Message>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, \
                 tool_name, metadata, created_at, completed_at, streaming, input_tokens, \
                 output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, \
                 tokens_per_second, snapshot_hash, patch_files, file_diffs, mode, thinking_level \
                 FROM messages WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 1",
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
        let attachments: Vec<tidev_types::message::MessageAttachment> =
            serde_json::from_str(&attachments_raw).unwrap_or_default();
        let metadata_raw: Vec<u8> = row.get(8).unwrap_or_default();
        let metadata: tidev_types::message::ToolMetadata =
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

        let patch_files = row
            .get::<_, Vec<u8>>(20)
            .ok()
            .filter(|b| !b.is_empty())
            .map(|b| decompress_text(&b));

        let file_diffs = row
            .get::<_, Vec<u8>>(21)
            .ok()
            .filter(|b| !b.is_empty())
            .map(|b| decompress_text(&b));

        let mode: Option<String> = row.get(22).ok().flatten();
        let thinking_level: Option<String> = row.get(23).ok().flatten();

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
            snapshot_hash: row.get(19).ok().flatten(),
            patch_files,
            file_diffs,
            mode: mode.and_then(|m| serde_json::from_str(&m).ok()),
            thinking_level: thinking_level.and_then(|t| serde_json::from_str(&t).ok()),
        }
    }

    /// Export one or more sessions to an uncompressed SQLite database.
    ///
    /// The output database uses the [`EXPORT_SCHEMA_SQL`] schema where all
    /// text columns are stored as plain TEXT (no zstd compression), suitable
    /// for inspection with standard SQLite tools or re-import.
    pub fn export_to_sqlite(
        &self,
        session_ids: &[Uuid],
        output_path: &Path,
    ) -> Result<()> {
        // Remove existing file so we start fresh.
        let _ = fs::remove_file(output_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create export directory {}", parent.display()))?;
        }

        let export_conn = Connection::open(output_path)
            .with_context(|| format!("failed to create export database {}", output_path.display()))?;
        export_conn.execute_batch("PRAGMA foreign_keys = OFF")?;
        export_conn.execute_batch(crate::schema::EXPORT_SCHEMA_SQL)?;

        let sid_strs: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
        let placeholder = sid_strs.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let tx = export_conn.unchecked_transaction()?;

        // ── 1. meta ── copy all rows ─────────────────────────────────────
        {
            let rows = self.read_query(
                "SELECT key, value FROM meta",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            )?;
            for (key, value) in &rows {
                insert.execute(params![key, value])?;
            }
        }

        // ── 2. sessions ── only the requested ones ───────────────────────
        {
            let sql = format!(
                "SELECT id, parent_session_id, provider_id, provider_display_name, \
                        model_id, model_display_name, title, created_at, updated_at, \
                        status, ended_at, context_summary, context_retained_from, system_prompt \
                 FROM sessions WHERE id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let parent: Option<String> = row.get(1)?;
                Ok((
                    row.get::<_, String>(0)?, parent,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?, row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?, row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?, row.get::<_, i64>(12)?,
                    row.get::<_, String>(13)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO sessions \
                 (id, parent_session_id, provider_id, provider_display_name, \
                  model_id, model_display_name, title, created_at, updated_at, \
                  status, ended_at, context_summary, context_retained_from, system_prompt) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7,
                    row.8, row.9, row.10, row.11, row.12, row.13,
                ])?;
            }
        }

        // ── 3. session_workspaces ────────────────────────────────────────
        {
            let sql = format!(
                "SELECT session_id, workspace_root FROM session_workspaces \
                 WHERE session_id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO session_workspaces (session_id, workspace_root) \
                 VALUES (?1, ?2)",
            )?;
            for (sid, root) in &rows {
                insert.execute(params![sid, root])?;
            }
        }

        // ── 4. session_instruction_sources ───────────────────────────────
        {
            let sql = format!(
                "SELECT session_id, source FROM session_instruction_sources \
                 WHERE session_id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO session_instruction_sources (session_id, source) \
                 VALUES (?1, ?2)",
            )?;
            for (sid, src) in &rows {
                insert.execute(params![sid, src])?;
            }
        }

        // ── 5. session_reverts ── decompress redo_snapshot BLOB → TEXT ───
        {
            let sql = format!(
                "SELECT session_id, message_id, CAST(redo_snapshot AS BLOB), created_at \
                 FROM session_reverts WHERE session_id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let blob: Option<Vec<u8>> = row.get(2)?;
                let redo_text = blob.as_deref().map(decompress_text);
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    redo_text,
                    row.get::<_, String>(3)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO session_reverts \
                 (session_id, message_id, redo_snapshot, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (sid, mid, snap, created) in &rows {
                insert.execute(params![sid, mid, snap, created])?;
            }
        }

        // ── 6. messages ── decompress BLOB columns → TEXT ────────────────
        {
            let sql = format!(
                "SELECT id, session_id, role, CAST(content AS BLOB), attachments, \
                        CAST(reasoning AS BLOB), CAST(tool_calls AS BLOB), tool_call_id, \
                        tool_name, CAST(metadata AS BLOB), created_at, completed_at, \
                        streaming, input_tokens, output_tokens, total_tokens, \
                        cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, \
                        snapshot_hash, CAST(patch_files AS BLOB), CAST(file_diffs AS BLOB), \
                        mode, thinking_level \
                 FROM messages WHERE session_id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let content = blob_or_empty_to_text(row, 3)?;
                let reasoning = opt_blob_to_text(row, 5)?;
                let tool_calls = blob_or_empty_to_text(row, 6)?;
                let metadata = blob_or_empty_to_text(row, 9)?;
                let patch_files = opt_blob_to_text(row, 21)?;
                let file_diffs = opt_blob_to_text(row, 22)?;
                Ok((
                    row.get::<_, String>(0)?,  // id
                    row.get::<_, String>(1)?,  // session_id
                    row.get::<_, String>(2)?,  // role
                    content,
                    row.get::<_, String>(4)?,  // attachments
                    reasoning,
                    tool_calls,
                    row.get::<_, Option<String>>(7)?,  // tool_call_id
                    row.get::<_, Option<String>>(8)?,  // tool_name
                    metadata,
                    row.get::<_, String>(10)?, // created_at
                    row.get::<_, Option<String>>(11)?, // completed_at
                    row.get::<_, i64>(12)?,    // streaming
                    row.get::<_, Option<i64>>(13)?, row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<i64>>(15)?, row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<i64>>(17)?, row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<f64>>(19)?, row.get::<_, Option<String>>(20)?,
                    patch_files, file_diffs,
                    row.get::<_, Option<String>>(23)?, // mode
                    row.get::<_, Option<String>>(24)?, // thinking_level
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO messages \
                 (id, session_id, role, content, attachments, reasoning, tool_calls, \
                  tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                  input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                  cache_write_tokens, model_id, tokens_per_second, snapshot_hash, \
                  patch_files, file_diffs, mode, thinking_level) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7,
                    row.8, row.9, row.10, row.11, row.12, row.13, row.14,
                    row.15, row.16, row.17, row.18, row.19, row.20, row.21,
                    row.22, row.23, row.24,
                ])?;
            }
        }

        // ── 7. todos ──────────────────────────────────────────────────────
        {
            let sql = format!(
                "SELECT session_id, position, content, status FROM todos \
                 WHERE session_id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO todos (session_id, position, content, status) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (sid, pos, content, status) in &rows {
                insert.execute(params![sid, pos, content, status])?;
            }
        }

        // ── 8. tool_permissions ───────────────────────────────────────────
        {
            let sql = format!(
                "SELECT session_id, tool_name, allowed, created_at FROM tool_permissions \
                 WHERE session_id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                sid_strs.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?, row.get::<_, String>(3)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO tool_permissions \
                 (session_id, tool_name, allowed, created_at) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (sid, name, allowed, created) in &rows {
                insert.execute(params![sid, name, allowed, created])?;
            }
        }

        tx.commit()?;
        Ok(())
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


impl SessionStore {
    /// Record that a file was read.
    pub fn record_file_read(
        &self,
        session_id: Uuid,
        file_path: &str,
        read_at: DateTime<Utc>,
        mtime: Option<i64>,
        size: Option<i64>,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO file_reads (session_id, file_path, read_at, mtime, size) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
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

    /// Load file reads for a session.
    pub fn load_file_reads(&self, session_id: Uuid) -> Result<Vec<FileReadRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT file_path, read_at, mtime, size FROM file_reads \
                 WHERE session_id = ?1 ORDER BY read_at DESC",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                Ok(FileReadRecord {
                    file_path: row.get(0)?,
                    read_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_default(),
                    mtime: row.get(2)?,
                    size: row.get(3)?,
                })
            })?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row?);
            }
            Ok(records)
        })
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
        todos: &[tidev_types::tools::TodoItem],
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "DELETE FROM todos WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;
        for (i, item) in todos.iter().enumerate() {
            conn.execute(
                "INSERT INTO todos (session_id, position, content, status) VALUES (?1, ?2, ?3, ?4)",
                params![session_id.to_string(), i as i64, item.content, item.status],
            )?;
        }
        Ok(())
    }

    /// Load todo items for a session.
    pub fn load_todos(&self, session_id: Uuid) -> Result<Vec<tidev_types::tools::TodoItem>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT content, status FROM todos WHERE session_id = ?1 ORDER BY position ASC",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                Ok(tidev_types::tools::TodoItem {
                    content: row.get(0)?,
                    status: row.get(1)?,
                })
            })?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }
}

// ---------------------------------------------------------------------------
// Tool permissions
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save a tool permission for a session.
    pub fn save_tool_permission(
        &self,
        session_id: Uuid,
        tool_name: &str,
        allowed: bool,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO tool_permissions (session_id, tool_name, allowed, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id.to_string(), tool_name, allowed as i64, now],
        )?;
        Ok(())
    }

    /// Load all tool permissions for a session.
    pub fn load_tool_permissions(&self, session_id: Uuid) -> Result<Vec<(String, bool)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT tool_name, allowed FROM tool_permissions \
                 WHERE session_id = ?1 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0))
            })?;
            let mut permissions = Vec::new();
            for row in rows {
                permissions.push(row?);
            }
            Ok(permissions)
        })
    }
}

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
        conn.execute(
            "UPDATE messages SET snapshot_hash = ?1, patch_files = ?2, file_diffs = ?3 \
             WHERE id = ?4 AND session_id = ?5",
            params![
                snapshot_hash,
                compress_text(patch_files),
                compress_text(file_diffs),
                message_id.to_string(),
                session_id.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Load snapshot patch data for a message.
    pub fn load_snapshot(&self, message_id: Uuid) -> Result<Option<(String, String, String)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT snapshot_hash, patch_files, file_diffs FROM messages WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![message_id.to_string()], |row| {
                let hash: String = row.get(0).unwrap_or_default();
                let patches = row
                    .get::<_, Vec<u8>>(1)
                    .ok()
                    .filter(|b| !b.is_empty())
                    .map(|b| decompress_text(&b))
                    .unwrap_or_default();
                let diffs = row
                    .get::<_, Vec<u8>>(2)
                    .ok()
                    .filter(|b| !b.is_empty())
                    .map(|b| decompress_text(&b))
                    .unwrap_or_default();
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
        conn.execute(
            "DELETE FROM session_instruction_sources WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;
        for source in sources {
            conn.execute(
                "INSERT INTO session_instruction_sources (session_id, source) VALUES (?1, ?2)",
                params![session_id.to_string(), source],
            )?;
        }
        Ok(())
    }

    /// Load instruction sources for a session.
    pub fn load_instruction_sources(&self, session_id: Uuid) -> Result<Vec<String>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT source FROM session_instruction_sources WHERE session_id = ?1 \
                 ORDER BY source",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| row.get(0))?;
            let mut sources = Vec::new();
            for row in rows {
                sources.push(row?);
            }
            Ok(sources)
        })
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
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO session_reverts (session_id, message_id, redo_snapshot, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id.to_string(), message_id.to_string(), redo_snapshot, now],
        )?;
        Ok(())
    }

    /// Load revert state for a session.
    pub fn load_revert_state(&self, session_id: Uuid) -> Result<Option<(Uuid, Option<Vec<u8>>)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, redo_snapshot FROM session_reverts WHERE session_id = ?1",
            )?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                let msg_id: String = row.get(0)?;
                let snapshot: Option<Vec<u8>> = row.get(1)?;
                Ok((Uuid::parse_str(&msg_id).unwrap_or_default(), snapshot))
            })?;
            match rows.next() {
                Some(Ok(result)) => Ok(Some(result)),
                _ => Ok(None),
            }
        })
    }
}
