//! Schema migration support for tidev.
//!
//! Tidev uses a lightweight custom migration runner rather than an external
//! library.  Version state is stored in the existing `meta` table
//! (`key = 'schema_version'`).
//!
//! # How to add a migration
//!
//! 1. Append a `Migration` entry to the [`MIGRATIONS`] array (see doc comment
//!    there).
//! 2. Update [`SCHEMA_SQL`](super::schema::SCHEMA_SQL) so that fresh
//!    installations get the complete schema.
//! 3. Bump [`SCHEMA_VERSION`](super::schema::SCHEMA_VERSION).
//!
//! # Squashing old migrations
//!
//! When many small migrations have accumulated, you can "squash" them:
//!
//! 1. Update [`SCHEMA_SQL`](super::schema::SCHEMA_SQL) so it represents the
//!    combined state of all squashed migrations.
//! 2. Remove the squashed entries from [`MIGRATIONS`].
//! 3. For each existing database, update the `meta` table row
//!    `('schema_version', '<new_baseline_version>')` (e.g. via
//!    `tidev db set-version <version>`).

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::schema::SCHEMA_VERSION;

// ---------------------------------------------------------------------------
// Migration definition
// ---------------------------------------------------------------------------

/// A single versioned migration step.
pub struct Migration {
    /// Target schema version *after* this migration has been applied.
    ///
    /// Must be strictly greater than the version of the previous entry.
    pub version: i64,

    /// Human-readable description (shown by `tidev db status`).
    pub description: &'static str,

    /// SQL statements that bring the schema from the previous version to
    /// `version`.  Typically `ALTER TABLE`, `CREATE INDEX`, etc.
    ///
    /// The SQL is executed inside a single transaction so partial failures
    /// are rolled back atomically.
    pub sql: &'static str,
}

/// All registered migrations, ordered by version.
///
/// # Convention
///
/// Migrations **1 … `SCHEMA_VERSION`** are the *cumulative current schema*
/// defined by [`SCHEMA_SQL`](super::schema::SCHEMA_SQL).  They are **not**
/// listed here — every database that ships with this version of tidev already
/// has them.
///
/// Only **future** changes (version > `SCHEMA_VERSION`) are added here.
///
/// # Adding a migration
///
/// ```ignore
/// pub const MIGRATIONS: &[Migration] = &[
///     // Migration { version: 33, description: "Add collapsed column to messages",
///     //              sql: "ALTER TABLE messages ADD COLUMN collapsed INTEGER NOT NULL DEFAULT 0;" },
/// ];
/// ```
///
/// Then bump [`SCHEMA_VERSION`](super::schema::SCHEMA_VERSION) and update
/// [`SCHEMA_SQL`](super::schema::SCHEMA_SQL).
pub const MIGRATIONS: &[Migration] = &[
    // ── v33: Remove priority from todos ─────────────────────────────────
    //
    // The `priority` column was removed from the `todos` table.  SQLite's
    // ALTER TABLE DROP COLUMN does not support NOT NULL columns, so we
    // recreate the table via the create-copy-drop-rename pattern.
    Migration {
        version: 33,
        description: "Remove priority column from todos table",
        sql: r#"
DROP TABLE IF EXISTS todos_new;
CREATE TABLE IF NOT EXISTS todos_new (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL,
    PRIMARY KEY(session_id, position)
);
INSERT INTO todos_new (session_id, position, content, status)
    SELECT session_id, position, content, status FROM todos;
DROP TABLE IF EXISTS todos;
ALTER TABLE todos_new RENAME TO todos;
CREATE INDEX IF NOT EXISTS idx_todos_session_position
    ON todos(session_id, position);
"#,
    },
    // ── v34: Add tool_outputs table ─────────────────────────────────────
    //
    // Stores the full (zstd-compressed) output of tool calls so the TUI
    // can display the complete output when the user expands a tool result
    // card.  The output is compressed with zstd level 3 before writing.
    Migration {
        version: 34,
        description: "Add tool_outputs table for full tool output storage",
        sql: r#"
CREATE TABLE IF NOT EXISTS tool_outputs (
    message_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    output BLOB NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tool_outputs_session_created
    ON tool_outputs(session_id, created_at);
"#,
    },
    // ── v35: Add session_goals table ─────────────────────────────────────
    //
    // Stores a single persistent goal per session for the `/goal` command.
    // The table is created IF NOT EXISTS so fresh installations already
    // have it via SCHEMA_SQL.
    Migration {
        version: 35,
        description: "Add session_goals table for /goal command",
        sql: r#"
CREATE TABLE IF NOT EXISTS session_goals (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    objective TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    tokens_used INTEGER NOT NULL DEFAULT 0,
    time_used_seconds INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#,
    },
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read the current schema version from the `meta` table.
///
/// Returns `Ok(None)` when:
/// - The `meta` table does not exist yet (completely fresh database).
/// - The `meta` table exists but has no `schema_version` row.
pub fn current_version(conn: &Connection) -> Result<Option<i64>> {
    // First check if the meta table exists at all.
    let meta_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !meta_exists {
        return Ok(None);
    }

    // Try to read the schema_version key.
    match conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(v) => match v.parse::<i64>() {
            Ok(n) => Ok(Some(n)),
            Err(e) => anyhow::bail!("Invalid schema_version in meta table: {v:?} — {e}"),
        },
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Apply all pending migrations.
///
/// # Behaviour
///
/// | Scenario | Action |
/// |---|---|
/// | Fresh DB (no `schema_version` in `meta`) | Write `SCHEMA_VERSION`, return |
/// | DB version == `SCHEMA_VERSION` | No-op, return immediately |
/// | DB version < `SCHEMA_VERSION` | Apply each pending migration in order |
/// | DB version > `SCHEMA_VERSION` | **Error** — DB was opened by a newer tidev |
///
/// Each migration runs inside its own transaction.  After each successful
/// migration the `schema_version` in `meta` is updated to that migration's
/// version.
///
/// Returns the schema version after all pending migrations have been applied
/// (always `SCHEMA_VERSION` on success).
pub fn run_pending(conn: &Connection) -> Result<i64> {
    let current = match current_version(conn)? {
        Some(v) => v,
        None => {
            // Fresh database — write the initial schema version.
            log::info!("Fresh database, setting schema version to {SCHEMA_VERSION}");
            set_version(conn, SCHEMA_VERSION).context("failed to write initial schema version")?;
            return Ok(SCHEMA_VERSION);
        }
    };

    if current > SCHEMA_VERSION {
        anyhow::bail!(
            "Database schema version ({current}) is newer than this tidev binary ({SCHEMA_VERSION}). \
             Please upgrade tidev."
        );
    }

    if current == SCHEMA_VERSION {
        log::debug!("Database already at schema version {SCHEMA_VERSION}, nothing to do");
        return Ok(current);
    }

    // ── Apply pending migrations ────────────────────────────────────────
    log::info!(
        "Database at schema version {current}, target {SCHEMA_VERSION} — applying {} migration(s)",
        MIGRATIONS
            .iter()
            .filter(|m| m.version > current && m.version <= SCHEMA_VERSION)
            .count()
    );

    for m in MIGRATIONS {
        if m.version <= current {
            continue;
        }
        if m.version > SCHEMA_VERSION {
            break;
        }

        log::info!("Applying migration v{}: {} …", m.version, m.description);

        // Each migration is wrapped in its own transaction so partial
        // failures are rolled back atomically and do not corrupt the DB.
        let tx = conn
            .unchecked_transaction()
            .with_context(|| format!("failed to start transaction for migration v{}", m.version))?;

        tx.execute_batch(m.sql)
            .with_context(|| format!("migration v{} failed", m.version))?;

        set_version(&tx, m.version).context("failed to update schema_version after migration")?;

        tx.commit()
            .with_context(|| format!("failed to commit migration v{}", m.version))?;

        log::info!("Migration v{} applied successfully", m.version);
    }

    // One final sanity write (harmless if already set by the loop).
    set_version(conn, SCHEMA_VERSION)?;

    Ok(SCHEMA_VERSION)
}

/// Migration status information.
pub struct MigrationStatus {
    /// Version currently recorded in the database (0 if unset).
    pub current_version: i64,
    /// Version that this tidev binary expects.
    pub latest_version: i64,
    /// Number of pending migrations.
    pub pending_count: usize,
}

/// Return a snapshot of the migration status without applying anything.
pub fn status(conn: &Connection) -> Result<MigrationStatus> {
    let current = current_version(conn)?.unwrap_or(0);

    let pending_count = MIGRATIONS
        .iter()
        .filter(|m| m.version > current && m.version <= SCHEMA_VERSION)
        .count();

    Ok(MigrationStatus {
        current_version: current,
        latest_version: SCHEMA_VERSION,
        pending_count,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Write (or replace) the `schema_version` row in the `meta` table.
fn set_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
        rusqlite::params![version.to_string()],
    )
    .context("failed to write schema_version to meta table")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Helper: create a fresh in-memory database with the `meta` table.
    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        conn
    }

    /// Helper: set the schema_version in meta.
    fn set_ver(conn: &Connection, v: i64) {
        set_version(conn, v).unwrap();
    }

    #[test]
    fn fresh_database_gets_initial_version() {
        let conn = fresh_db();
        let result = run_pending(&conn).unwrap();
        assert_eq!(result, SCHEMA_VERSION);

        let current = current_version(&conn).unwrap().unwrap();
        assert_eq!(current, SCHEMA_VERSION);
    }

    #[test]
    fn noop_when_already_at_latest() {
        let conn = fresh_db();
        set_ver(&conn, SCHEMA_VERSION);

        let result = run_pending(&conn).unwrap();
        assert_eq!(result, SCHEMA_VERSION);
    }

    #[test]
    fn status_on_fresh_database() {
        let conn = fresh_db();
        let s = status(&conn).unwrap();
        assert_eq!(s.current_version, 0);
        assert_eq!(s.latest_version, SCHEMA_VERSION);
        // After adding migration v33, a fresh DB sees it as pending.
        assert!(s.pending_count > 0);
    }

    #[test]
    fn status_at_latest() {
        let conn = fresh_db();
        set_ver(&conn, SCHEMA_VERSION);
        let s = status(&conn).unwrap();
        assert_eq!(s.current_version, SCHEMA_VERSION);
    }

    #[test]
    fn current_version_none_when_no_meta_table() {
        let conn = Connection::open_in_memory().unwrap();
        // No meta table at all.
        let v = current_version(&conn).unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn current_version_none_when_no_row() {
        let conn = fresh_db();
        // meta exists but has no schema_version row.
        let v = current_version(&conn).unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn error_when_db_is_newer() {
        let conn = fresh_db();
        set_ver(&conn, SCHEMA_VERSION + 1);
        let err = run_pending(&conn).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("newer"), "expected 'newer' error, got: {msg}");
    }

    #[test]
    fn run_pending_applies_migration() {
        let conn = fresh_db();
        set_ver(&conn, SCHEMA_VERSION);

        // At latest version: no-op.
        let result = run_pending(&conn).unwrap();
        assert_eq!(result, SCHEMA_VERSION);
    }

    /// Create a database with the pre-v33 schema (including `priority` column).
    fn db_with_old_todos() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta(key, value) VALUES ('schema_version', '32');

             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 parent_session_id TEXT,
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
                 system_prompt TEXT NOT NULL DEFAULT ''
             );

             CREATE TABLE IF NOT EXISTS todos (
                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 position INTEGER NOT NULL,
                 content TEXT NOT NULL,
                 status TEXT NOT NULL,
                 priority TEXT NOT NULL,
                 PRIMARY KEY(session_id, position)
             );

             INSERT INTO sessions (id, provider_id, provider_display_name, model_id,
                 model_display_name, title, created_at, updated_at)
                 VALUES ('test-session', 'test', 'Test', 'test-model', 'Test Model',
                         'Test', '2024-01-01', '2024-01-01');

             INSERT INTO todos (session_id, position, content, status, priority)
                 VALUES ('test-session', 1, 'Test todo', 'pending', 'high');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn migration_v33_drops_priority_column() {
        let conn = db_with_old_todos();

        // Verify old schema has priority column
        let has_priority: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('todos') WHERE name='priority'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            has_priority,
            "pre-migration: todos should have priority column"
        );

        // Run migration
        let result = run_pending(&conn).unwrap();
        assert_eq!(result, SCHEMA_VERSION);

        // Verify priority column is gone
        let has_priority: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('todos') WHERE name='priority'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !has_priority,
            "post-migration: todos should NOT have priority column"
        );

        // Verify data survived
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM todos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "todo data should survive migration");

        let content: String = conn
            .query_row("SELECT content FROM todos WHERE position = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(content, "Test todo");
    }

    #[test]
    fn migration_v33_is_idempotent_across_runs() {
        let conn = db_with_old_todos();

        // First run
        run_pending(&conn).unwrap();
        let v1 = current_version(&conn).unwrap().unwrap();
        assert_eq!(v1, SCHEMA_VERSION);

        // Second run (should be no-op)
        let v2 = run_pending(&conn).unwrap();
        assert_eq!(v2, SCHEMA_VERSION);

        // Data still intact
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM todos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
