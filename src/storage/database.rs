use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::memory::MemoryStore;

use super::{
    schema::{SCHEMA_SQL, SCHEMA_VERSION},
    SessionStore,
};

/// Unified database manager.
///
/// Opens the SQLite file, runs the full schema (including memory system tables),
/// creates a **shared write connection**, and provides factory methods for
/// [`SessionStore`] and [`MemoryStore`].
///
/// # Shared write connection
///
/// Both stores receive the same `Arc<Mutex<Connection>>` for writes.  This
/// guarantees that only one thread writes at a time — which is all SQLite
/// allows anyway.  Each store still gets its own read connection so that
/// reads never block writes and vice versa.
pub struct Database {
    path: PathBuf,
    /// Shared write connection used by both SessionStore and MemoryStore.
    write_conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (or create) the database and initialise the full schema.
    ///
    /// All `CREATE TABLE IF NOT EXISTS` statements are idempotent, so this
    /// is safe to call on an existing database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create database directory {}", parent.display()))?;
        }

        // ── Shared write connection ──────────────────────────────────────
        let write_conn = Connection::open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        write_conn.pragma_update(None, "journal_mode", "WAL")
            .context("failed to set journal_mode")?;
        write_conn.pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable foreign_keys")?;
        write_conn.pragma_update(None, "synchronous", "NORMAL")
            .context("failed to set synchronous")?;
        write_conn.pragma_update(None, "mmap_size", "268435456")
            .context("failed to set mmap_size")?;
        write_conn.pragma_update(None, "cache_size", "-64000")
            .context("failed to set cache_size")?;
        write_conn.pragma_update(None, "temp_store", "MEMORY")
            .context("failed to set temp_store")?;
        write_conn.busy_timeout(Duration::from_secs(5))?;

        // Register zstd_decode(blob) → text for CLI debugging
        write_conn
            .create_scalar_function(
                "zstd_decode",
                1,
                rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                |ctx| {
                    let blob = ctx.get::<Vec<u8>>(0)?;
                    let text = crate::storage::compression::decompress_text(&blob);
                    Ok(text)
                },
            )
            .context("failed to register zstd_decode function")?;

        // Create / migrate all tables
        write_conn
            .execute_batch(SCHEMA_SQL)
            .context("failed to initialise database schema")?;

        write_conn
            .execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )
            .context("failed to record schema version")?;

        let shared_conn = Arc::new(Mutex::new(write_conn));

        Ok(Self {
            path,
            write_conn: shared_conn,
        })
    }

    /// Create a [`SessionStore`] that shares the write connection.
    pub fn create_session_store(&self) -> Result<SessionStore> {
        SessionStore::open_with_shared_write(&self.path, Arc::clone(&self.write_conn))
    }

    /// Create a [`MemoryStore`] that shares the write connection.
    pub fn create_memory_store(&self) -> Result<MemoryStore> {
        MemoryStore::open_with_shared_write(&self.path, Arc::clone(&self.write_conn))
    }

    /// Return the path to the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}
