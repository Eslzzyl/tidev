//! Schema migration support for tidev.

use anyhow::{Context, Result};
use rusqlite::Connection;

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
    // Example:
    // Migration {
    //     version: 38,
    //     description: "Add some_column to sessions",
    //     sql: "ALTER TABLE sessions ADD COLUMN some_column TEXT NOT NULL DEFAULT ''",
    // },
];

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
            conn.execute_batch(migration.sql)
                .with_context(|| format!("migration v{} failed", migration.version))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        conn
    }

    #[test]
    fn fresh_database_gets_initial_version() {
        let conn = fresh_conn();
        run_migrations(&conn).unwrap();

        let version: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn noop_when_already_at_latest() {
        let conn = fresh_conn();
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', ?1)", rusqlite::params![SCHEMA_VERSION.to_string()]).unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn error_when_db_is_newer() {
        let conn = fresh_conn();
        let newer = format!("{}", SCHEMA_VERSION + 1);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', ?1)", rusqlite::params![newer]).unwrap();

        let err = run_migrations(&conn).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("newer"), "expected 'newer' error, got: {msg}");
    }
}
