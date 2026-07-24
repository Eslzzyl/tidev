use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use rusqlite::{Connection, functions::FunctionFlags};

use crate::migration;
use crate::{SessionStore, schema::SCHEMA_SQL};

/// Open a write connection to the database and configure it.
///
/// Shared by [`Database::open`], [`Database::create_store`], and
/// [`SessionStore::clone`] so that every write connection is independently
/// configured (WAL mode, timeouts, helper functions, etc.).
pub(crate) fn open_write_conn(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("failed to open write connection to {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("failed to set journal_mode")?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("failed to enable foreign_keys")?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("failed to set synchronous")?;
    conn.pragma_update(None, "mmap_size", "268435456")
        .context("failed to set mmap_size")?;
    conn.pragma_update(None, "cache_size", "-64000")
        .context("failed to set cache_size")?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .context("failed to set temp_store")?;
    conn.busy_timeout(Duration::from_secs(5))
        .context("failed to set busy_timeout")?;

    // Register zstd_decode(blob) → text for CLI debugging
    conn.create_scalar_function("zstd_decode", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let blob = ctx.get::<Vec<u8>>(0)?;
        let text = crate::compression::decompress_text(&blob);
        Ok(text)
    })
    .context("failed to register zstd_decode function")?;

    Ok(conn)
}

/// Unified database manager.
///
/// Opens the SQLite file, runs the full schema, and provides a factory method
/// for [`SessionStore`].
pub struct Database {
    path: PathBuf,
    write_conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (or create) the database and initialise the full schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }

        let write_conn = open_write_conn(&path)?;

        let write_conn = Arc::new(Mutex::new(write_conn));

        // Run schema and migrations
        {
            let conn = write_conn.lock().unwrap();
            conn.execute_batch(SCHEMA_SQL)
                .context("failed to execute initial schema")?;
            migration::run_migrations(&conn).context("failed to run schema migrations")?;
        }

        Ok(Self { path, write_conn })
    }

    /// Create a new [`SessionStore`] for this database.
    ///
    /// Each store gets its **own write connection** (independent `Mutex`).
    /// With WAL mode and `busy_timeout` this is safe for concurrent writers
    /// writing to different sessions.
    pub fn create_store(&self) -> Result<SessionStore> {
        let read_conn = Connection::open(&self.path).with_context(|| {
            format!("failed to open read connection to {}", self.path.display())
        })?;
        read_conn.pragma_update(None, "journal_mode", "WAL").ok();
        read_conn.pragma_update(None, "foreign_keys", "ON").ok();
        read_conn
            .pragma_update(None, "query_only", "true")
            .context("failed to set query_only on read connection")?;

        let write_conn = open_write_conn(&self.path)?;

        Ok(SessionStore {
            write_conn: Arc::new(Mutex::new(write_conn)),
            read_conn: Mutex::new(read_conn),
            path: self.path.clone(),
        })
    }

    /// Return the database path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run database maintenance (VACUUM, ANALYZE).
    pub fn maintain(&self) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();

        // WAL checkpoint before VACUUM
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
            .context("failed to checkpoint WAL")?;

        conn.execute_batch("VACUUM; ANALYZE;")
            .context("failed to run maintenance")?;

        log::info!("database maintenance completed for {}", self.path.display());
        Ok(())
    }

    /// Run VACUUM on the database, optionally with a timeout.
    pub fn vacuum(&self, timeout: Duration) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.busy_timeout(timeout)?;
        conn.execute_batch("VACUUM;")
            .context("failed to VACUUM database")?;
        Ok(())
    }
}
