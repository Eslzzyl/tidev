use anyhow::Result;
use rusqlite::Connection;

// ─── FTS5 Query Helpers ──────────────────────────────────────────────

/// Escape special FTS5 characters in a user query string.
/// Wraps the entire query in double quotes so that all special operators
/// (^, *, ~, (, ), NEAR) are treated as literal text. Internal double
/// quotes are escaped by doubling them (FTS5 convention inside quoted strings).
pub fn escape_fts5_query(query: &str) -> String {
    // Inside FTS5 double-quoted strings, only " needs escaping (as "")
    let inner = query.replace('"', "\"\"");
    format!("\"{}\"", inner)
}

/// Search observations using FTS5 with BM25 ranking.
/// Search memories using FTS5 with BM25 ranking.
pub fn fts5_search_memories(
    db: &Connection,
    query: &str,
    workspace_root: &str,
    limit: usize,
) -> Result<Vec<(Uuid, String, f64)>> {
    let safe_query = escape_fts5_query(query);

    let mut stmt = db.prepare(
        "SELECT m.id, m.title, rank
          FROM memories_fts f
          JOIN memories m ON m.rowid = f.rowid
          WHERE memories_fts MATCH ?1 AND m.workspace_root = ?2 AND m.active = 1 AND m.is_latest = 1
          ORDER BY rank
          LIMIT ?3",
    )?;

    let results = stmt.query_map(
        rusqlite::params![safe_query, workspace_root, limit as i64],
        |row| {
            let id_str: String = row.get(0)?;
            let id = uuid::Uuid::parse_str(&id_str).unwrap_or(uuid::Uuid::nil());
            let title: String = row.get(1)?;
            let score: f64 = row.get(2)?;
            Ok((id, title, score))
        },
    )?;

    let mut out = Vec::new();
    for r in results {
        out.push(r?);
    }
    Ok(out)
}

use uuid::Uuid;
