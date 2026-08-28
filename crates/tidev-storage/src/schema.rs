use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: i64 = 41;

pub const SESSION_SELECT_COLUMNS: &str = "s.id, s.parent_session_id, s.provider_id, s.provider_display_name, s.model_id, s.model_display_name, s.title, s.created_at, s.updated_at, s.status, s.ended_at, s.context_summary, s.context_retained_from, s.system_prompt, s.workspace_root, s.snapshot_start_hash";

pub const SCHEMA_SQL: &str = r#"
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
    updated_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    ended_at TEXT,
    context_summary TEXT NOT NULL DEFAULT '',
    context_retained_from INTEGER NOT NULL DEFAULT 0,
    system_prompt TEXT NOT NULL DEFAULT '',
    workspace_root TEXT NOT NULL DEFAULT '',
    snapshot_start_hash TEXT,
    instruction_sources TEXT NOT NULL DEFAULT '[]',
    todos TEXT NOT NULL DEFAULT '[]',
    revert_message_id TEXT,
    revert_redo_snapshot TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_workspace
    ON sessions(workspace_root);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content BLOB NOT NULL,
    attachments TEXT NOT NULL DEFAULT '[]',
    reasoning BLOB,
    tool_calls BLOB NOT NULL DEFAULT '[]',
    tool_call_id TEXT,
    tool_name TEXT,
    metadata BLOB NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    completed_at TEXT,
    streaming INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    model_id TEXT,
    tokens_per_second REAL,
    mode TEXT,
    thinking_level TEXT,
    app_data BLOB NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_messages_session_created_at
    ON messages(session_id, created_at);

CREATE INDEX IF NOT EXISTS idx_messages_session_id
    ON messages(session_id);

CREATE TABLE IF NOT EXISTS tool_outputs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    output BLOB,
    byte_size INTEGER NOT NULL,
    line_count INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_outputs_session
    ON tool_outputs(session_id);

CREATE INDEX IF NOT EXISTS idx_tool_outputs_created
    ON tool_outputs(created_at);

CREATE INDEX IF NOT EXISTS idx_tool_outputs_message
    ON tool_outputs(message_id);
"#;

/// Schema for the export database (no zstd compression).
pub const EXPORT_SCHEMA_SQL: &str = r#"
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
    updated_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    ended_at TEXT,
    context_summary TEXT NOT NULL DEFAULT '',
    context_retained_from INTEGER NOT NULL DEFAULT 0,
    system_prompt TEXT NOT NULL DEFAULT '',
    workspace_root TEXT NOT NULL DEFAULT '',
    snapshot_start_hash TEXT,
    instruction_sources TEXT NOT NULL DEFAULT '[]',
    todos TEXT NOT NULL DEFAULT '[]',
    revert_message_id TEXT,
    revert_redo_snapshot TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_workspace
    ON sessions(workspace_root);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    attachments TEXT NOT NULL DEFAULT '[]',
    reasoning TEXT,
    tool_calls TEXT NOT NULL DEFAULT '[]',
    tool_call_id TEXT,
    tool_name TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    completed_at TEXT,
    streaming INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    model_id TEXT,
    tokens_per_second REAL,
    mode TEXT,
    thinking_level TEXT,
    app_data TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_messages_session_created_at
    ON messages(session_id, created_at);

CREATE INDEX IF NOT EXISTS idx_messages_session_id
    ON messages(session_id);

CREATE TABLE IF NOT EXISTS tool_outputs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    output TEXT,
    byte_size INTEGER NOT NULL,
    line_count INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_outputs_session
    ON tool_outputs(session_id);

CREATE INDEX IF NOT EXISTS idx_tool_outputs_created
    ON tool_outputs(created_at);

CREATE INDEX IF NOT EXISTS idx_tool_outputs_message
    ON tool_outputs(message_id);
"#;

/// Conversation data — a session with its full message history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub session_id: uuid::Uuid,
    pub parent_session_id: Option<uuid::Uuid>,
    pub workspace_root: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub context_summary: Option<String>,
    pub context_retained_from: usize,
    pub messages: Vec<tidev_llm::message::Message>,
    pub revert_message_id: Option<uuid::Uuid>,
}

impl Conversation {
    pub fn new(
        session_id: uuid::Uuid,
        workspace_root: impl Into<String>,
        provider_id: impl Into<String>,
        provider_display_name: impl Into<String>,
        model_id: impl Into<String>,
        model_display_name: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            session_id,
            parent_session_id: None,
            workspace_root: workspace_root.into(),
            provider_id: provider_id.into(),
            provider_display_name: provider_display_name.into(),
            model_id: model_id.into(),
            model_display_name: model_display_name.into(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            context_summary: None,
            context_retained_from: 0,
            messages: Vec::new(),
            revert_message_id: None,
        }
    }
}
