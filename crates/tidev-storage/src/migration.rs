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
