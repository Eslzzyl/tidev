use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use chrono::{DateTime, Utc};

use crate::storage::SessionStore;

/// Tracks which files have been read by the model in each session.
/// This enforces the rule that a file must be read before editing.
#[derive(Debug)]
pub struct FileReadTracker {
    // session_id -> (normalized_path -> stamp)
    reads: RwLock<HashMap<uuid::Uuid, HashMap<PathBuf, FileReadStamp>>>,
}

#[derive(Debug, Clone)]
pub struct FileReadStamp {
    pub read_at: DateTime<Utc>,
    pub mtime: Option<i64>,
    pub size: Option<i64>,
}

impl FileReadTracker {
    pub fn new() -> Self {
        Self {
            reads: RwLock::new(HashMap::new()),
        }
    }

    /// Load file reads from database for a session
    pub fn load_from_store(&self, store: &SessionStore, session_id: uuid::Uuid) -> anyhow::Result<()> {
        let records = store.load_file_reads(session_id)?;
        
        let mut writes = self.reads.write().unwrap();
        let mut session_map = HashMap::new();
        
        for record in records {
            let path = PathBuf::from(&record.file_path);
            session_map.insert(path, FileReadStamp {
                read_at: record.read_at,
                mtime: record.mtime,
                size: record.size,
            });
        }
        
        writes.insert(session_id, session_map);
        Ok(())
    }

    /// Record a file read
    pub fn record_read(
        &self,
        store: &SessionStore,
        session_id: uuid::Uuid,
        path: &Path,
    ) -> anyhow::Result<()> {
        let metadata = std::fs::metadata(path).ok();
        
        let mtime = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        
        let size = metadata.as_ref().map(|m| m.len() as i64);
        
        let stamp = FileReadStamp {
            read_at: Utc::now(),
            mtime,
            size,
        };
        
        // Update in-memory cache
        {
            let mut writes = self.reads.write().unwrap();
            writes
                .entry(session_id)
                .or_default()
                .insert(path.to_path_buf(), stamp.clone());
        }
        
        // Persist to database
        store.record_file_read(
            session_id,
            &path.to_string_lossy(),
            stamp.read_at,
            stamp.mtime,
            stamp.size,
        )?;
        
        Ok(())
    }

    /// Check if a file has been read and not modified since
    pub fn check_read(&self, session_id: uuid::Uuid, path: &Path) -> Result<(), String> {
        let reads = self.reads.read().unwrap();
        
        let Some(stamp) = reads
            .get(&session_id)
            .and_then(|m| m.get(path))
        else {
            return Err(format!(
                "You must read file {} before editing it. Use the Read tool first.",
                path.display()
            ));
        };

        // Check if file still exists
        if !path.exists() {
            return Err(format!(
                "File {} was deleted after it was read. Cannot edit.",
                path.display()
            ));
        }

        // Get current file metadata
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) => return Err(format!("Failed to read file metadata: {}", e)),
        };

        let current_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        
        let current_size = metadata.len() as i64;

        // Check if file was modified
        let mtime_changed = match (stamp.mtime, current_mtime) {
            (Some(stored), Some(current)) => stored != current,
            (None, None) => false,
            _ => true, // If we don't have mtime data, assume changed
        };
        
        let size_changed = stamp.size != Some(current_size);

        if mtime_changed || size_changed {
            return Err(format!(
                "File {} has been modified since it was last read.\n\
                 Last modification: {:?}\n\
                 Last read: {}\n\n\
                 Please read the file again before editing it.",
                path.display(),
                current_mtime,
                stamp.read_at.to_rfc3339()
            ));
        }

        Ok(())
    }

    /// Clear all reads for a session (e.g., when session is deleted)
    pub fn clear_session(&self, session_id: uuid::Uuid) {
        let mut writes = self.reads.write().unwrap();
        writes.remove(&session_id);
    }
}

impl Default for FileReadTracker {
    fn default() -> Self {
        Self::new()
    }
}
