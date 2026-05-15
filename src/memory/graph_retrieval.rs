use anyhow::Result;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Bfs;
use rusqlite::Connection;
use std::collections::HashMap;

use super::graph::{self, GraphEdge, GraphNode};

// ─── Types ────────────────────────────────────────────────────────────

/// A path through the graph, from a seed node to related entities.
#[derive(Debug, Clone)]
pub struct GraphEntityPath {
    /// Seed label that matched the query.
    pub seed_label: String,
    /// Ordered list of node labels along the path (seed → ... → target).
    pub node_labels: Vec<String>,
    /// Relation types between consecutive nodes.
    pub relations: Vec<String>,
    /// Combined score (higher = more relevant).
    pub score: f64,
}

// ─── Retrieval ────────────────────────────────────────────────────────

pub struct GraphRetrieval;

impl GraphRetrieval {
    /// Search for entities related to `query` by:
    /// 1. Finding nodes whose label matches the query
    /// 2. BFS traversal up to `max_depth`
    /// 3. Scoring paths by depth + edge weights
    pub fn search_related(
        db: &Connection,
        query: &str,
        max_depth: usize,
        max_results: usize,
    ) -> Result<Vec<GraphEntityPath>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Load graph into memory
        let all_nodes = graph::load_all_nodes(db)?;
        let all_edges = graph::load_all_edges(db)?;

        if all_nodes.is_empty() || all_edges.is_empty() {
            return Ok(Vec::new());
        }

        // 2. Build petgraph DiGraph
        let mut pet = DiGraph::<&GraphNode, &GraphEdge>::new();
        let mut node_index_map: HashMap<&str, NodeIndex> = HashMap::new();

        for node in &all_nodes {
            let idx = pet.add_node(node);
            node_index_map.insert(node.id.as_str(), idx);
        }

        for edge in &all_edges {
            if let (Some(&src), Some(&tgt)) =
                (node_index_map.get(edge.source_id.as_str()), node_index_map.get(edge.target_id.as_str()))
            {
                pet.add_edge(src, tgt, edge);
                // Add reverse edge for undirected traversal
                pet.add_edge(tgt, src, edge);
            }
        }

        // 3. Find seed nodes matching the query (case-insensitive)
        let query_lower = query.to_lowercase();
        let seed_matches: Vec<&GraphNode> = all_nodes
            .iter()
            .filter(|n| n.label.to_lowercase().contains(&query_lower))
            .collect();

        if seed_matches.is_empty() {
            return Ok(Vec::new());
        }

        // 4. BFS from each seed node, collect paths
        let mut all_paths: Vec<GraphEntityPath> = Vec::new();

        for seed in &seed_matches {
            let seed_idx = match node_index_map.get(seed.id.as_str()) {
                Some(idx) => *idx,
                None => continue,
            };

            let mut bfs = Bfs::new(&pet, seed_idx);
            let mut depth_map: HashMap<NodeIndex, usize> = HashMap::new();
            let mut parent_map: HashMap<NodeIndex, (NodeIndex, &GraphEdge)> = HashMap::new();
            depth_map.insert(seed_idx, 0);

            while let Some(nx) = bfs.next(&pet) {
                let current_depth = depth_map[&nx];
                if current_depth >= max_depth {
                    continue;
                }

                // Find neighbors via outgoing edges
                for neighbor in pet.neighbors(nx) {
                    if depth_map.contains_key(&neighbor) {
                        continue; // already visited
                    }
                    depth_map.insert(neighbor, current_depth + 1);

                    // Find the edge between nx and neighbor
                    let edge = pet
                        .edges_connecting(nx, neighbor)
                        .next()
                        .map(|e| *e.weight());
                    if let Some(e) = edge {
                        parent_map.insert(neighbor, (nx, e));
                    }
                }
            }

            // Reconstruct paths for all visited nodes
            for (&node_idx, &depth) in &depth_map {
            if node_idx == seed_idx || depth == 0 {
                continue;
            }

            let mut node_labels = Vec::new();                let mut relations = Vec::new();
                let mut score = 1.0;

                // Walk back to seed
                let mut current = node_idx;
                while current != seed_idx {
                    node_labels.push(pet[current].label.clone());
                    if let Some(&(parent, edge)) = parent_map.get(&current) {
                        relations.push(edge.relation.clone());
                        score *= edge.weight * 0.8; // decay per hop
                        current = parent;
                    } else {
                        break;
                    }
                }
                node_labels.push(pet[seed_idx].label.clone());

                // Reverse because we walked backwards
                node_labels.reverse();
                relations.reverse();

                all_paths.push(GraphEntityPath {
                    seed_label: seed.label.clone(),
                    node_labels,
                    relations,
                    score,
                });
            }
        }

        // 5. Sort by score descending, deduplicate by target label, take top N
        all_paths.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        let mut seen_targets = std::collections::HashSet::new();
        let paths: Vec<GraphEntityPath> = all_paths
            .into_iter()
            .filter(|p| {
                if let Some(target) = p.node_labels.last() {
                    seen_targets.insert(target.clone())
                } else {
                    false
                }
            })
            .take(max_results)
            .collect();

        Ok(paths)
    }

    /// Format entity paths for system prompt injection.
    ///
    /// Output format:
    /// ```text
    /// ## Knowledge Graph Context
    /// - tokio → related_to → async runtime
    /// - schema.rs → co_occurs_with → engine.rs
    /// ```
    pub fn format_for_prompt(paths: &[GraphEntityPath], max_items: usize) -> String {
        if paths.is_empty() {
            return String::new();
        }

        let mut lines: Vec<String> = Vec::new();
        lines.push("## Knowledge Graph Context".to_string());

        let count = paths.len().min(max_items);
        for path in &paths[..count] {
            let label = &path.seed_label;
            let target = path.node_labels.last().map(|s| s.as_str()).unwrap_or("?");
            let relation = path.relations.last().map(|s| s.as_str()).unwrap_or("related_to");

            if label == target {
                // Single-node path: just mention the entity
                lines.push(format!("- **{}** ({})", target, relation));
            } else {
                // Multi-node path: seed → relation → target
                // We show the full path if short (≤3 hops), otherwise just seed → target
                if path.node_labels.len() <= 3 {
                    let hops: Vec<&str> = path.node_labels.iter().map(|s| s.as_str()).collect();
                    lines.push(format!("- {}", hops.join(" → ")));
                } else {
                    lines.push(format!("- {} → ... → {} ({} hops)", label, target, path.node_labels.len() - 1));
                }
            }
        }

        lines.join("\n")
    }
}
