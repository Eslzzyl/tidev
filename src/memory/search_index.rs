use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashMap;
/// In-memory BM25 index. Used for RRF fusion scores (Phase 2+).
/// For Phase 1, the primary search path is SQLite FTS5.
/// This is kept minimal for now — will be extended in Phase 2.
#[derive(Debug)]
pub struct Bm25Index {
    entries: HashMap<String, Bm25Entry>,
    inverted_index: HashMap<String, HashMap<String, usize>>,
    total_docs: usize,
    total_doc_length: f64,
    k1: f64,
    b: f64,
}

#[derive(Debug)]
struct Bm25Entry {
    doc_length: usize,
    session_id: String,
}

impl Bm25Index {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            inverted_index: HashMap::new(),
            total_docs: 0,
            total_doc_length: 0.0,
            k1: 1.2,
            b: 0.75,
        }
    }

    #[allow(dead_code)]
    pub fn add(&mut self, id: &str, text: &str, session_id: &str) {
        let tokens = tokenize(text);
        let doc_length = tokens.len();
        self.total_docs += 1;
        self.total_doc_length += doc_length as f64;

        let mut local_tf: HashMap<String, usize> = HashMap::new();
        for token in &tokens {
            *local_tf.entry(token.clone()).or_default() += 1;
        }

        let id = id.to_string();
        for (token, tf) in local_tf {
            self.inverted_index
                .entry(token)
                .or_default()
                .insert(id.clone(), tf);
        }

        self.entries.insert(
            id,
            Bm25Entry {
                doc_length,
                session_id: session_id.to_string(),
            },
        );
    }

    #[allow(dead_code)]
    pub fn remove(&mut self, id: &str) {
        if let Some(entry) = self.entries.remove(id) {
            self.total_docs -= 1;
            self.total_doc_length -= entry.doc_length as f64;
        }
        for postings in self.inverted_index.values_mut() {
            postings.remove(id);
        }
    }

    #[allow(dead_code)]
    pub fn search(&self, query: &str, limit: usize) -> Vec<(String, f64)> {
        let tokens = tokenize(query);
        if tokens.is_empty() {
            return vec![];
        }

        let avg_doc_len = if self.total_docs > 0 {
            self.total_doc_length / self.total_docs as f64
        } else {
            1.0
        };

        let mut scores: HashMap<String, f64> = HashMap::new();
        for term in &tokens {
            let idf = self.idf(term);
            if idf == 0.0 {
                continue;
            }
            if let Some(postings) = self.inverted_index.get(term) {
                for (doc_id, tf) in postings {
                    if let Some(entry) = self.entries.get(doc_id) {
                        let doc_len = entry.doc_length as f64;
                        let bm25 = idf
                            * (*tf as f64 * (self.k1 + 1.0))
                            / (*tf as f64 + self.k1 * (1.0 - self.b + self.b * doc_len / avg_doc_len));
                        *scores.entry(doc_id.clone()).or_insert(0.0) += bm25;
                    }
                }
            }
        }

        let mut results: Vec<_> = scores.into_iter().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        results.truncate(limit);
        results
    }

    fn idf(&self, term: &str) -> f64 {
        let docs_with_term = self
            .inverted_index
            .get(term)
            .map(|m| m.len())
            .unwrap_or(0);
        if docs_with_term == 0 {
            return 0.0;
        }
        ((self.total_docs as f64 - docs_with_term as f64 + 0.5)
            / (docs_with_term as f64 + 0.5)
            + 1.0)
            .ln()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple whitespace + lowercasing tokenizer.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

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
          LIMIT ?3",    )?;

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
