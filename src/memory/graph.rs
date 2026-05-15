use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::types::CompressedObservation;

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
    // Check for existing node
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM graph_nodes WHERE node_type = ?1 AND label = ?2",
            rusqlite::params![node_type, label],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO graph_nodes (id, node_type, label, properties, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, node_type, label, "{}", now],
    )
    .context("failed to insert graph node")?;

    Ok(id)
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
    let mut stmt = db.prepare(
        "SELECT id, node_type, label, properties, created_at FROM graph_nodes",
    )?;
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
    let existing: Option<(String, f64)> = db
        .query_row(
            "SELECT id, weight FROM graph_edges
             WHERE source_id = ?1 AND target_id = ?2 AND relation = ?3",
            rusqlite::params![source_id, target_id, relation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    if let Some((id, old_weight)) = existing {
        // Merge weight: running average to smooth over repeated co-occurrences
        let merged = (old_weight + weight) / 2.0;
        db.execute(
            "UPDATE graph_edges SET weight = ?1 WHERE id = ?2",
            rusqlite::params![merged, id],
        )?;
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO graph_edges (id, source_id, target_id, relation, weight, properties, created_at, session_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![id, source_id, target_id, relation, weight, "{}", now, session_id],
    )
    .context("failed to insert graph edge")?;

    Ok(id)
}

// ─── Extraction ───────────────────────────────────────────────────────

/// Reify a compressed observation into graph nodes + edges.
///
/// tidev's compression step already extracts structured `concepts` and `files`.
/// This function creates graph nodes from those fields and edges between
/// co-occurring entities — no additional LLM call needed.
pub fn extract_from_observation(db: &Connection, obs: &CompressedObservation) -> Result<()> {
    let mut entity_ids: Vec<(String, String)> = Vec::new(); // (id, type)

    // Create nodes for concepts
    for concept in &obs.concepts {
        let label = concept.trim();
        if label.is_empty() {
            continue;
        }
        let id = upsert_node(db, "concept", label)?;
        entity_ids.push((id, "concept".to_string()));
    }

    // Create nodes for files
    for file in &obs.files {
        let label = file.trim();
        if label.is_empty() {
            continue;
        }
        let id = upsert_node(db, "file", label)?;
        entity_ids.push((id, "file".to_string()));
    }

    // Create edges between all pairs that co-occur in this observation
    // Use different relation types based on entity type pairs:
    //   concept↔concept → "related_to"
    //   file↔file       → "co_occurs_with"
    //   concept↔file    → "mentioned_in"
    let session_id = Some(obs.session_id.to_string());
    for i in 0..entity_ids.len() {
        for j in (i + 1)..entity_ids.len() {
            let relation = match (entity_ids[i].1.as_str(), entity_ids[j].1.as_str()) {
                ("file", "file") => "co_occurs_with",
                ("concept", "concept") => "related_to",
                _ => "mentioned_in",
            };
            upsert_edge(
                db,
                &entity_ids[i].0,
                &entity_ids[j].0,
                relation,
                1.0,
                session_id.as_deref(),
            )?;
        }
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
