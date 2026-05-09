pub const SCHEMA_VERSION: i64 = 23;

/// The memories table SQL, exported so MemoryStore can create it independently.
pub const MEMORIES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    workspace_root TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    source_session_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    usage_count INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_memories_workspace_active
    ON memories(workspace_root, active);

CREATE INDEX IF NOT EXISTS idx_memories_type
    ON memories(workspace_root, memory_type, active);

CREATE INDEX IF NOT EXISTS idx_memories_usage
    ON memories(workspace_root, usage_count DESC);
"#;

pub const SESSION_SELECT_COLUMNS: &str = "s.id, s.parent_session_id, s.provider_id, s.provider_display_name, s.model_id, s.model_display_name, s.title, s.created_at, s.updated_at, s.context_summary, s.context_retained_from, COALESCE(sw.workspace_root, '')";

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
    context_summary TEXT NOT NULL DEFAULT '',
    context_retained_from INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS session_workspaces (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_root TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_instruction_sources (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    PRIMARY KEY(session_id, source)
);

CREATE INDEX IF NOT EXISTS idx_session_instruction_sources_session
    ON session_instruction_sources(session_id);

CREATE TABLE IF NOT EXISTS session_reverts (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    redo_snapshot BLOB,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content BLOB NOT NULL,
    attachments TEXT NOT NULL DEFAULT '[]',
    reasoning BLOB,
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
    snapshot_hash TEXT,
    patch_files BLOB,
    file_diffs BLOB,
    mode TEXT,
    rtk_rewritten INTEGER NOT NULL DEFAULT 0,
    thinking_level TEXT
);

CREATE INDEX IF NOT EXISTS idx_messages_session_created_at
    ON messages(session_id, created_at);

CREATE INDEX IF NOT EXISTS idx_messages_session_id
    ON messages(session_id);

CREATE TABLE IF NOT EXISTS tool_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    input_json BLOB NOT NULL,
    output_text BLOB NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_events_session_message
    ON tool_events(session_id, message_id);

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

CREATE TABLE IF NOT EXISTS gateway_chat_sessions (
    platform TEXT NOT NULL,
    chat_key TEXT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(platform, chat_key)
);

CREATE INDEX IF NOT EXISTS idx_gateway_chat_sessions_session
    ON gateway_chat_sessions(session_id);

CREATE TABLE IF NOT EXISTS gateway_chat_models (
    platform TEXT NOT NULL,
    chat_key TEXT,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(platform, chat_key)
);

CREATE TABLE IF NOT EXISTS usage_stats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    time_bucket TEXT NOT NULL,
    granularity TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    request_count INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider_id, model_id, time_bucket, granularity)
);

CREATE INDEX IF NOT EXISTS idx_usage_stats_time_bucket
    ON usage_stats(time_bucket, granularity);

CREATE INDEX IF NOT EXISTS idx_usage_stats_provider_model
    ON usage_stats(provider_id, model_id);

CREATE TABLE IF NOT EXISTS model_thinking_levels (
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    thinking_level TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, model_id)
);

CREATE TABLE IF NOT EXISTS file_reads (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    read_at TEXT NOT NULL,
    mtime INTEGER,
    size INTEGER,
    PRIMARY KEY(session_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_file_reads_session
    ON file_reads(session_id);

CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    workspace_root TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    source_session_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    usage_count INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_memories_workspace_active
    ON memories(workspace_root, active);

CREATE INDEX IF NOT EXISTS idx_memories_type
    ON memories(workspace_root, memory_type, active);

CREATE INDEX IF NOT EXISTS idx_memories_usage
    ON memories(workspace_root, usage_count DESC);
"#;

/// Schema for the export database (no zstd compression).
///
/// Mirrors `SCHEMA_SQL` but uses TEXT instead of BLOB for the columns
/// that are zstd-compressed in the main database:
///   - `messages.content`
///   - `messages.reasoning`
///   - `messages.patch_files`
///   - `messages.file_diffs`
///   - `tool_events.input_json`
///   - `tool_events.output_text`
///   - `session_reverts.redo_snapshot`
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
    context_summary TEXT NOT NULL DEFAULT '',
    context_retained_from INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS session_workspaces (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_root TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_instruction_sources (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    PRIMARY KEY(session_id, source)
);

CREATE INDEX IF NOT EXISTS idx_session_instruction_sources_session
    ON session_instruction_sources(session_id);

CREATE TABLE IF NOT EXISTS session_reverts (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    redo_snapshot TEXT,
    created_at TEXT NOT NULL
);

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
    snapshot_hash TEXT,
    patch_files TEXT,
    file_diffs TEXT,
    mode TEXT,
    rtk_rewritten INTEGER NOT NULL DEFAULT 0,
    thinking_level TEXT
);

CREATE INDEX IF NOT EXISTS idx_messages_session_created_at
    ON messages(session_id, created_at);

CREATE INDEX IF NOT EXISTS idx_messages_session_id
    ON messages(session_id);

CREATE TABLE IF NOT EXISTS tool_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    input_json TEXT NOT NULL,
    output_text TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_events_session_message
    ON tool_events(session_id, message_id);

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

CREATE TABLE IF NOT EXISTS gateway_chat_sessions (
    platform TEXT NOT NULL,
    chat_key TEXT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(platform, chat_key)
);

CREATE INDEX IF NOT EXISTS idx_gateway_chat_sessions_session
    ON gateway_chat_sessions(session_id);

CREATE TABLE IF NOT EXISTS gateway_chat_models (
    platform TEXT NOT NULL,
    chat_key TEXT,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(platform, chat_key)
);

CREATE TABLE IF NOT EXISTS usage_stats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    time_bucket TEXT NOT NULL,
    granularity TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    request_count INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider_id, model_id, time_bucket, granularity)
);

CREATE INDEX IF NOT EXISTS idx_usage_stats_time_bucket
    ON usage_stats(time_bucket, granularity);

CREATE INDEX IF NOT EXISTS idx_usage_stats_provider_model
    ON usage_stats(provider_id, model_id);

CREATE TABLE IF NOT EXISTS model_thinking_levels (
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    thinking_level TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, model_id)
);

CREATE TABLE IF NOT EXISTS file_reads (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    read_at TEXT NOT NULL,
    mtime INTEGER,
    size INTEGER,
    PRIMARY KEY(session_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_file_reads_session
    ON file_reads(session_id);

CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    workspace_root TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    source_session_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    usage_count INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_memories_workspace_active
    ON memories(workspace_root, active);

CREATE INDEX IF NOT EXISTS idx_memories_type
    ON memories(workspace_root, memory_type, active);

CREATE INDEX IF NOT EXISTS idx_memories_usage
    ON memories(workspace_root, usage_count DESC);
"#;
