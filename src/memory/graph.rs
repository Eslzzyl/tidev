use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub properties: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub weight: f64,
    pub properties: serde_json::Value,
    pub created_at: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GraphStats {
    pub node_count: u64,
    pub edge_count: u64,
    pub by_type: Vec<(String, u64)>,
}

// ─── Node CRUD ───────────────────────────────────────────────────────

/// Insert a node, or if one with the same (node_type, label) exists, return its id.
pub fn upsert_node(db: &Connection, node_type: &str, label: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO graph_nodes (id, node_type, label, properties, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(node_type, label) DO NOTHING",
        rusqlite::params![id, node_type, label, "{}", now],
    )
    .context("failed to insert graph node")?;

    let existing: String = db.query_row(
        "SELECT id FROM graph_nodes WHERE node_type = ?1 AND label = ?2",
        rusqlite::params![node_type, label],
        |row| row.get(0),
    )?;
    Ok(existing)
}

/// Lookup a node by exact type + label.
pub fn find_node(db: &Connection, node_type: &str, label: &str) -> Result<Option<GraphNode>> {
    let mut stmt = db.prepare(
        "SELECT id, node_type, label, properties, created_at
         FROM graph_nodes WHERE node_type = ?1 AND label = ?2",
    )?;
    let mut rows = stmt.query_map(rusqlite::params![node_type, label], map_node_row)?;
    match rows.next() {
        Some(Ok(node)) => Ok(Some(node)),
        _ => Ok(None),
    }
}

/// Search nodes by label prefix (case-insensitive via LIKE).
pub fn search_nodes(db: &Connection, query: &str, limit: usize) -> Result<Vec<GraphNode>> {
    let pattern = format!("%{}%", query);
    let mut stmt = db.prepare(
        "SELECT id, node_type, label, properties, created_at
         FROM graph_nodes
         WHERE label LIKE ?1
         ORDER BY label ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![pattern, limit as i64], map_node_row)?;
    let mut nodes = Vec::new();
    for row in rows {
        nodes.push(row?);
    }
    Ok(nodes)
}

/// Load ALL nodes from DB (used by graph retrieval to build in-memory graph).
pub fn load_all_nodes(db: &Connection) -> Result<Vec<GraphNode>> {
    let mut stmt =
        db.prepare("SELECT id, node_type, label, properties, created_at FROM graph_nodes")?;
    let rows = stmt.query_map([], map_node_row)?;
    let mut nodes = Vec::new();
    for row in rows {
        nodes.push(row?);
    }
    Ok(nodes)
}

/// Load ALL edges from DB.
pub fn load_all_edges(db: &Connection) -> Result<Vec<GraphEdge>> {
    let mut stmt = db.prepare(
        "SELECT id, source_id, target_id, relation, weight, properties, created_at, session_id
         FROM graph_edges",
    )?;
    let rows = stmt.query_map([], map_edge_row)?;
    let mut edges = Vec::new();
    for row in rows {
        edges.push(row?);
    }
    Ok(edges)
}

// ─── Edge CRUD ────────────────────────────────────────────────────────

/// Insert an edge, or update weight if an exact match exists.
pub fn upsert_edge(
    db: &Connection,
    source_id: &str,
    target_id: &str,
    relation: &str,
    weight: f64,
    session_id: Option<&str>,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let affected = db.execute(
        "INSERT INTO graph_edges (id, source_id, target_id, relation, weight, properties, created_at, session_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(source_id, target_id, relation) DO NOTHING",
        rusqlite::params![id, source_id, target_id, relation, weight, "{}", now, session_id],
    )
    .context("failed to insert graph edge")?;

    if affected > 0 {
        return Ok(id);
    }

    let existing: (String, f64) = db.query_row(
        "SELECT id, weight FROM graph_edges
         WHERE source_id = ?1 AND target_id = ?2 AND relation = ?3",
        rusqlite::params![source_id, target_id, relation],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let merged = (existing.1 + weight) / 2.0;
    db.execute(
        "UPDATE graph_edges SET weight = ?1 WHERE id = ?2",
        rusqlite::params![merged, existing.0],
    )?;
    Ok(existing.0)
}

// ─── Knowledge Graph Extraction ───────────────────────────────────────

/// Extract nodes and edges from a `SessionSummary`.
///
/// Uses a star pattern: a session node is created, each concept connects
/// to the session node, and the session node connects to each file.
/// This avoids the O(concepts × files) Cartesian product.
pub fn extract_from_session_summary(
    db: &Connection,
    summary: &crate::memory::types::SessionSummary,
    session_id: &str,
) -> Result<()> {
    let session_node_id = upsert_node(db, "session", session_id)?;

    for concept in &summary.concepts {
        let cid = upsert_node(db, "concept", concept)?;
        upsert_edge(db, &cid, &session_node_id, "relates_to", 1.0, Some(session_id))?;
    }

    for file in &summary.files_modified {
        let fid = upsert_node(db, "file", file)?;
        upsert_edge(db, &session_node_id, &fid, "modifies", 1.0, Some(session_id))?;
    }

    Ok(())
}

/// Extract nodes and edges from a `MemoryEntry`.
///
/// Uses a star pattern: a memory node is created, each concept connects
/// to the memory node, and the memory node connects to each file.
/// This avoids the O(concepts × files) Cartesian product.
pub fn extract_from_memory_entry(
    db: &Connection,
    entry: &crate::memory::types::MemoryEntry,
) -> Result<()> {
    let memory_label = entry.id.to_string();
    let memory_node_id = upsert_node(db, "memory_entry", &memory_label)?;

    for concept in &entry.concepts {
        let cid = upsert_node(db, "concept", concept)?;
        upsert_edge(db, &cid, &memory_node_id, "relates_to", 1.0, Some(&memory_label))?;
    }

    for file in &entry.files {
        let fid = upsert_node(db, "file", file)?;
        upsert_edge(db, &memory_node_id, &fid, "modifies", 1.0, Some(&memory_label))?;
    }

    Ok(())
}

// ─── Stats ────────────────────────────────────────────────────────────

pub fn get_stats(db: &Connection) -> Result<GraphStats> {
    let node_count: i64 = db
        .query_row("SELECT COUNT(*) FROM graph_nodes", [], |row| row.get(0))
        .unwrap_or(0);

    let edge_count: i64 = db
        .query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))
        .unwrap_or(0);

    let mut stmt = db.prepare(
        "SELECT node_type, COUNT(*) as cnt FROM graph_nodes GROUP BY node_type ORDER BY cnt DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let count: i64 = row.get(1)?;
        Ok((row.get::<_, String>(0)?, count as u64))
    })?;
    let mut by_type = Vec::new();
    for row in rows {
        by_type.push(row?);
    }

    Ok(GraphStats {
        node_count: node_count as u64,
        edge_count: edge_count as u64,
        by_type,
    })
}

// ─── Row Mappers ─────────────────────────────────────────────────────

fn map_node_row(row: &rusqlite::Row) -> rusqlite::Result<GraphNode> {
    Ok(GraphNode {
        id: row.get(0)?,
        node_type: row.get(1)?,
        label: row.get(2)?,
        properties: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
        created_at: row.get(4)?,
    })
}

fn map_edge_row(row: &rusqlite::Row) -> rusqlite::Result<GraphEdge> {
    Ok(GraphEdge {
        id: row.get(0)?,
        source_id: row.get(1)?,
        target_id: row.get(2)?,
        relation: row.get(3)?,
        weight: row.get(4)?,
        properties: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
        created_at: row.get(6)?,
        session_id: row.get(7)?,
    })
}
