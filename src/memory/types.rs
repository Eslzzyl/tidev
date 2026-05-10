use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::compression::{compress_text, decompress_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Project,
    Feedback,
    Reference,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Feedback => "feedback",
            Self::Reference => "reference",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            "feedback" => Some(Self::Feedback),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::User => "usr",
            Self::Project => "proj",
            Self::Feedback => "feed",
            Self::Reference => "ref",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub workspace_root: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source_session_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub usage_count: i64,
    pub active: bool,
}

use crate::storage::schema::MEMORIES_TABLE_SQL;

/// Thread-safe memory store. Not Clone.
/// Use `try_clone()` to open a fresh connection.
#[derive(Debug)]
pub struct MemoryStore {
    db_path: std::path::PathBuf,
    connection: std::sync::Mutex<rusqlite::Connection>,
    /// workspace_root -> cached entries
    cache: std::sync::RwLock<std::collections::HashMap<String, Vec<MemoryEntry>>>,
}

impl MemoryStore {
    pub fn open(db_path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let path = db_path.as_ref().to_path_buf();
        let connection = rusqlite::Connection::open(&path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(MEMORIES_TABLE_SQL)?;

        Ok(Self {
            db_path: path,
            connection: std::sync::Mutex::new(connection),
            cache: std::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.db_path
    }

    /// Load all active memories for a workspace into cache.
    pub fn load_for_workspace(&self, workspace_root: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        let entries = self.query_active(workspace_root)?;
        self.cache
            .write()
            .unwrap()
            .insert(workspace_root.to_string(), entries.clone());
        Ok(entries)
    }

    /// Get cached entries for a workspace, or load from DB if not cached.
    pub fn get_or_load(&self, workspace_root: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        {
            let cache = self.cache.read().unwrap();
            if let Some(entries) = cache.get(workspace_root) {
                return Ok(entries.clone());
            }
        }
        self.load_for_workspace(workspace_root)
    }

    /// Add a new memory entry.
    pub fn add(&self, entry: &MemoryEntry) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO memories (id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                entry.id.to_string(),
                entry.workspace_root,
                entry.memory_type.as_str(),
                entry.title,
                compress_text(&entry.content),
                serde_json::to_string(&entry.tags).unwrap_or_default(),
                entry.source_session_id.map(|id| id.to_string()),
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.usage_count,
                entry.active as i64,
            ],
        )?;
        self.invalidate_cache(&entry.workspace_root);
        Ok(())
    }

    /// Update an existing memory entry (by id).
    pub fn update(&self, entry: &MemoryEntry) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "UPDATE memories SET memory_type = ?1, title = ?2, content = ?3, tags = ?4, updated_at = ?5, usage_count = ?6, active = ?7
             WHERE id = ?8",
            rusqlite::params![
                entry.memory_type.as_str(),
                entry.title,
                compress_text(&entry.content),
                serde_json::to_string(&entry.tags).unwrap_or_default(),
                entry.updated_at.to_rfc3339(),
                entry.usage_count,
                entry.active as i64,
                entry.id.to_string(),
            ],
        )?;
        self.invalidate_cache(&entry.workspace_root);
        Ok(())
    }

    /// Soft-delete a memory entry.
    pub fn delete(&self, workspace_root: &str, id: Uuid) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "UPDATE memories SET active = 0, updated_at = ?1 WHERE id = ?2 AND workspace_root = ?3",
            rusqlite::params![
                chrono::Utc::now().to_rfc3339(),
                id.to_string(),
                workspace_root,
            ],
        )?;
        self.invalidate_cache(workspace_root);
        Ok(())
    }

    /// Increment usage_count for a memory.
    pub fn record_usage(&self, workspace_root: &str, id: Uuid) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "UPDATE memories SET usage_count = usage_count + 1 WHERE id = ?1 AND workspace_root = ?2",
            rusqlite::params![id.to_string(), workspace_root],
        )?;
        self.invalidate_cache(workspace_root);
        Ok(())
    }

    /// Search memories by keyword in title, content, and tags.
    pub fn search(&self, workspace_root: &str, query: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        let pattern = format!("%{}%", query);
        let guard = self.connection.lock().unwrap();
        let mut stmt = guard.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active
             FROM memories
             WHERE workspace_root = ?1 AND active = 1
               AND (title LIKE ?2 OR content LIKE ?2 OR tags LIKE ?2)
             ORDER BY usage_count DESC, updated_at DESC"
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![workspace_root, pattern],
                Self::row_to_entry,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// Get a single memory by id.
    pub fn get(&self, workspace_root: &str, id: Uuid) -> anyhow::Result<Option<MemoryEntry>> {
        let guard = self.connection.lock().unwrap();
        let mut stmt = guard.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active
             FROM memories WHERE id = ?1 AND workspace_root = ?2 AND active = 1"
        )?;
        let mut rows = stmt.query_map(
            rusqlite::params![id.to_string(), workspace_root],
            Self::row_to_entry,
        )?;
        Ok(rows.next().transpose()?)
    }

    /// Select hot memories for system prompt injection.
    /// Returns up to `max_count` entries, prioritized by usage_count and recency,
    /// fitting within `budget_chars` total.
    pub fn select_hot(
        &self,
        workspace_root: &str,
        max_count: usize,
        budget_chars: usize,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let all = self.get_or_load(workspace_root)?;
        let mut scored: Vec<(i64, &MemoryEntry)> = all
            .iter()
            .filter(|e| e.active)
            .map(|e| {
                let hours = (chrono::Utc::now() - e.updated_at).num_hours();
                let recency_bonus = if hours < 24 {
                    5
                } else if hours < 168 {
                    2
                } else {
                    0
                };
                (e.usage_count * 10 + recency_bonus, e)
            })
            .collect();

        scored.sort_by_key(|e| std::cmp::Reverse(e.0));

        let mut selected = Vec::new();
        let mut chars = 0usize;
        for (_, entry) in scored {
            if selected.len() >= max_count {
                break;
            }
            // Estimate formatted length: "- [type] title\n"
            let estimated = entry.memory_type.as_str().len() + entry.title.len() + 8;
            if chars + estimated > budget_chars {
                break;
            }
            chars += estimated;
            selected.push(entry.clone());
        }
        Ok(selected)
    }

    /// Format memories as a markdown snippet for system prompt injection.
    pub fn format_for_prompt(entries: &[MemoryEntry]) -> String {
        if entries.is_empty() {
            return String::new();
        }
        let mut out = String::from("\n\n## Workspace Memories\n");
        out.push_str(&format!("({} loaded)\n", entries.len()));
        for entry in entries {
            let preview: String = entry.content.chars().take(120).collect();
            let suffix = if entry.content.len() > 120 { "…" } else { "" };
            out.push_str(&format!(
                "- [{}] **{}**: {}{}\n",
                entry.memory_type.short_label(),
                entry.title,
                preview,
                suffix,
            ));
        }
        out.push_str("\nUse the `memory` tool with `operation: list` or `operation: search` to find more.\n");
        out
    }

    /// Clone of MemoryStore for sharing. Opens a new connection.
    /// This is cheap — rusqlite::Connection::open is fast on WAL mode.
    pub fn try_clone(&self) -> anyhow::Result<Self> {
        Self::open(&self.db_path)
    }

    // ─── helpers ──────────────────────────────────────────────

    fn query_active(&self, workspace_root: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        let guard = self.connection.lock().unwrap();
        let mut stmt = guard.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags, source_session_id, created_at, updated_at, usage_count, active
             FROM memories
             WHERE workspace_root = ?1 AND active = 1
             ORDER BY usage_count DESC, updated_at DESC"
        )?;
        let entries = stmt
            .query_map(rusqlite::params![workspace_root], Self::row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
        let tags_str: String = row.get(5)?;
        let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
        let session_id_str: Option<String> = row.get(6)?;

        // Read content as BLOB (compressed) or TEXT (legacy fallback)
        let content = match row.get::<_, Vec<u8>>(4) {
            Ok(bytes) => decompress_text(&bytes),
            Err(_) => {
                let text: String = row.get(4)?;
                text
            }
        };

        Ok(MemoryEntry {
            id: row.get::<_, String>(0)?.parse().unwrap_or_default(),
            workspace_root: row.get(1)?,
            memory_type: MemoryType::parse_str(&row.get::<_, String>(2)?)
                .unwrap_or(MemoryType::Reference),
            title: row.get(3)?,
            content,
            tags,
            source_session_id: session_id_str.and_then(|s| s.parse().ok()),
            created_at: row
                .get::<_, String>(7)?
                .parse()
                .unwrap_or_else(|_| chrono::Utc::now()),
            updated_at: row
                .get::<_, String>(8)?
                .parse()
                .unwrap_or_else(|_| chrono::Utc::now()),
            usage_count: row.get(9)?,
            active: row.get::<_, i64>(10)? != 0,
        })
    }

    fn invalidate_cache(&self, workspace_root: &str) {
        self.cache.write().unwrap().remove(workspace_root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (MemoryStore, TempDir) {
        let dir = TempDir::new().expect("temp dir should be created");
        let db_path = dir.path().join("test.db");
        // MemoryStore::open creates the table via MEMORIES_TABLE_SQL
        (MemoryStore::open(&db_path).unwrap(), dir)
    }

    fn sample_entry(ws: &str) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            workspace_root: ws.to_string(),
            memory_type: MemoryType::Project,
            title: "Test memory".to_string(),
            content: "This is a test memory entry".to_string(),
            tags: vec!["test".to_string()],
            source_session_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            usage_count: 0,
            active: true,
        }
    }

    #[test]
    fn test_add_and_load() {
        let (store, _dir) = test_store();
        let ws = "/test/workspace";
        let entry = sample_entry(ws);
        store.add(&entry).unwrap();

        let loaded = store.load_for_workspace(ws).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "Test memory");
    }

    #[test]
    fn test_delete() {
        let (store, _dir) = test_store();
        let ws = "/test/ws2";
        let entry = sample_entry(ws);
        store.add(&entry).unwrap();
        store.delete(ws, entry.id).unwrap();

        let loaded = store.load_for_workspace(ws).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_search() {
        let (store, _dir) = test_store();
        let ws = "/test/ws3";
        let mut entry = sample_entry(ws);
        entry.title = "SQLite WAL mode".to_string();
        entry.content = "Using SQLite with WAL journal mode".to_string();
        store.add(&entry).unwrap();

        let results = store.search(ws, "WAL").unwrap();
        assert_eq!(results.len(), 1);

        let no_results = store.search(ws, "PostgreSQL").unwrap();
        assert!(no_results.is_empty());
    }

    #[test]
    fn test_select_hot() {
        let (store, _dir) = test_store();
        let ws = "/test/ws4";
        for i in 0..5 {
            let mut entry = sample_entry(ws);
            entry.title = format!("Memory {}", i);
            entry.usage_count = i as i64;
            store.add(&entry).unwrap();
        }

        let hot = store.select_hot(ws, 100, 9999).unwrap();
        // Should have 5 entries sorted by usage_count desc
        assert_eq!(hot.len(), 5);
        assert_eq!(hot[0].title, "Memory 4");
    }

    #[test]
    fn test_format_for_prompt() {
        let ws = "/test";
        let entries = vec![MemoryEntry {
            id: Uuid::new_v4(),
            workspace_root: ws.to_string(),
            memory_type: MemoryType::Project,
            title: "Architecture".to_string(),
            content: "Using plugin-based design".to_string(),
            tags: vec![],
            source_session_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            usage_count: 5,
            active: true,
        }];

        let formatted = MemoryStore::format_for_prompt(&entries);
        assert!(formatted.contains("Architecture"));
        assert!(formatted.contains("[proj]"));
    }
}
