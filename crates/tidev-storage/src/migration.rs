//! Schema migration support for tidev.

use anyhow::Result;
use rusqlite::Connection;
use serde::Deserialize;
use uuid::Uuid;

use super::schema::SCHEMA_VERSION;

/// A single versioned migration step.
pub struct Migration {
    pub version: i64,
    pub description: &'static str,
    pub sql: &'static str,
}

/// All registered migrations, ordered by version.
///
/// Migrations 1 … SCHEMA_VERSION are the cumulative current schema defined
/// by SCHEMA_SQL. They are not listed here — every database that ships with
/// this version of tidev already has them.
///
/// # Adding a migration
///
/// 1. Append a Migration entry here.
/// 2. Update SCHEMA_SQL so that fresh installations get the complete schema.
/// 3. Bump SCHEMA_VERSION.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 38,
        description: "Add reasoning_started_at to messages",
        sql: "ALTER TABLE messages ADD COLUMN reasoning_started_at TEXT",
    },
    Migration {
        version: 39,
        description: "Add reasoning_completed_at to messages",
        sql: "ALTER TABLE messages ADD COLUMN reasoning_completed_at TEXT",
    },
    Migration {
        version: 40,
        description: "Add child session id to messages",
        sql: "ALTER TABLE messages ADD COLUMN child_session_id TEXT",
    },
    Migration {
        version: 41,
        description: "Add provider error metadata to messages",
        sql: "ALTER TABLE messages ADD COLUMN provider_error TEXT",
    },
];

#[derive(Deserialize)]
struct LegacyMessageMetadata {
    child_session_id: Option<Uuid>,
}

/// Run all pending migrations on the given connection.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE((SELECT value FROM meta WHERE key = 'schema_version'), '0')",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    if current_version > SCHEMA_VERSION {
        anyhow::bail!(
            "Database schema version ({}) is newer than this binary ({}). \
             Please update tidev.",
            current_version,
            SCHEMA_VERSION
        );
    }

    for migration in MIGRATIONS {
        if migration.version > current_version {
            log::info!(
                "Applying migration v{}: {}",
                migration.version,
                migration.description
            );
            // Use execute() instead of execute_batch() so we can gracefully
            // handle the case where the column already exists (fresh DB
            // created by SCHEMA_SQL already includes it).
            if let Err(e) = conn.execute_batch(migration.sql) {
                let err_str = e.to_string();
                if err_str.contains("duplicate column")
                    || err_str.contains("already exists")
                    || err_str.contains("no such table")
                {
                    log::info!("migration v{} skipped: {err_str}", migration.version);
                } else {
                    anyhow::bail!("migration v{} failed: {e:#}", migration.version);
                }
            }
            if migration.version == 40 {
                backfill_child_session_ids(conn)?;
            }
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
                rusqlite::params![migration.version.to_string()],
            )?;
        }
    }

    // Always ensure meta has the latest version
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
        rusqlite::params![SCHEMA_VERSION.to_string()],
    )?;

    Ok(())
}

fn backfill_child_session_ids(conn: &Connection) -> Result<()> {
    let rows = match conn.prepare("SELECT id, metadata FROM messages") {
        Ok(mut stmt) => {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            let mut values = Vec::new();
            for row in rows {
                values.push(row?);
            }
            values
        }
        Err(error) if error.to_string().contains("no such table") => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    for (message_id, metadata_blob) in rows {
        let metadata = crate::compression::decompress_text(&metadata_blob);
        let Ok(metadata) = serde_json::from_str::<LegacyMessageMetadata>(&metadata) else {
            continue;
        };
        let Some(child_session_id) = metadata.child_session_id else {
            continue;
        };
        conn.execute(
            "UPDATE messages SET child_session_id = ?1 WHERE id = ?2 AND child_session_id IS NULL",
            rusqlite::params![child_session_id.to_string(), message_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn fresh_database_gets_initial_version() {
        let conn = fresh_conn();
        run_migrations(&conn).unwrap();

        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn noop_when_already_at_latest() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
            rusqlite::params![SCHEMA_VERSION.to_string()],
        )
        .unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn error_when_db_is_newer() {
        let conn = fresh_conn();
        let newer = format!("{}", SCHEMA_VERSION + 1);
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
            rusqlite::params![newer],
        )
        .unwrap();

        let err = run_migrations(&conn).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("newer"), "expected 'newer' error, got: {msg}");
    }

    #[test]
    fn v40_backfills_child_session_id_from_metadata() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             CREATE TABLE messages (id TEXT PRIMARY KEY, metadata BLOB NOT NULL);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '39')",
            [],
        )
        .unwrap();

        let message_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let metadata = serde_json::json!({ "child_session_id": child_id });
        conn.execute(
            "INSERT INTO messages (id, metadata) VALUES (?1, ?2)",
            rusqlite::params![
                message_id.to_string(),
                crate::compression::compress_text(&serde_json::to_string(&metadata).unwrap()),
            ],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let stored: String = conn
            .query_row(
                "SELECT child_session_id FROM messages WHERE id = ?1",
                rusqlite::params![message_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, child_id.to_string());
    }
}
