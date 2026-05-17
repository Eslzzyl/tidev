use anyhow::Result;
use rusqlite::Connection;

// ─── FTS5 Query Helpers ──────────────────────────────────────────────

/// Escape special FTS5 characters in a user query string.
pub fn escape_fts5_query(query: &str) -> String {
    // FTS5 special chars: ^ * " ( ) ~
    // We need to escape them so user queries don't break FTS5 syntax
    let mut escaped = String::with_capacity(query.len());
    for ch in query.chars() {
        match ch {
            '"' | '(' | ')' | '^' | '*' | '~' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
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
