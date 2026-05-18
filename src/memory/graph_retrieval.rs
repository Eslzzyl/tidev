use anyhow::Result;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet, VecDeque};

use super::graph::{self, GraphNode};

/// A path through the graph, from a seed node to related entities.
#[derive(Debug, Clone)]
pub struct GraphEntityPath {
    pub seed_label: String,
    pub node_labels: Vec<String>,
    pub relations: Vec<String>,
    pub score: f64,
}

pub struct GraphRetrieval;

impl GraphRetrieval {
    /// Search for entities related to `query` by:
    /// 1. Finding nodes whose label matches the query (case-insensitive)
    /// 2. BFS traversal up to `max_depth`
    /// 3. Scoring paths by depth + edge weights
    pub fn search_related(
        db: &Connection,
        query: &str,
        max_depth: usize,
        max_results: usize,
    ) -> Result<Vec<GraphEntityPath>> {
        if query.is_empty() || max_depth == 0 {
            return Ok(Vec::new());
        }

        let all_nodes = graph::load_all_nodes(db)?;
        let all_edges = graph::load_all_edges(db)?;

        if all_nodes.is_empty() {
            return Ok(Vec::new());
        }

        let node_label_map: HashMap<&str, &GraphNode> =
            all_nodes.iter().map(|n| (n.id.as_str(), n)).collect();

        let query_lower = query.to_lowercase();
        let seed_matches: Vec<&GraphNode> = all_nodes
            .iter()
            .filter(|n| n.label.to_lowercase().contains(&query_lower))
            .collect();

        if seed_matches.is_empty() {
            return Ok(Vec::new());
        }

        let mut adjacency: HashMap<&str, Vec<(&str, &str, f64)>> = HashMap::new();
        for edge in &all_edges {
            adjacency
                .entry(edge.source_id.as_str())
                .or_default()
                .push((edge.target_id.as_str(), edge.relation.as_str(), edge.weight));
            adjacency
                .entry(edge.target_id.as_str())
                .or_default()
                .push((edge.source_id.as_str(), edge.relation.as_str(), edge.weight));
        }

        let mut all_paths: Vec<GraphEntityPath> = Vec::new();

        for seed in &seed_matches {
            let mut depth_map: HashMap<&str, usize> = HashMap::new();
            let mut parent_map: HashMap<&str, (&str, &str, f64)> = HashMap::new();
            let mut queue: VecDeque<&str> = VecDeque::new();

            depth_map.insert(seed.id.as_str(), 0);
            queue.push_back(seed.id.as_str());

            while let Some(current) = queue.pop_front() {
                let current_depth = depth_map[current];
                if current_depth >= max_depth {
                    continue;
                }

                if let Some(neighbors) = adjacency.get(current) {
                    for &(neighbor, relation, weight) in neighbors {
                        if depth_map.contains_key(neighbor) {
                            continue;
                        }
                        depth_map.insert(neighbor, current_depth + 1);
                        parent_map.insert(neighbor, (current, relation, weight));
                        queue.push_back(neighbor);
                    }
                }
            }

            for (&node_id, &depth) in &depth_map {
                if depth == 0 {
                    continue;
                }

                let mut node_labels = Vec::new();
                let mut relations = Vec::new();
                let mut score = 1.0;

                let mut current = node_id;
                loop {
                    if let Some(&node) = node_label_map.get(current) {
                        node_labels.push(node.label.clone());
                    }

                    if let Some(&(parent, relation, weight)) = parent_map.get(current) {
                        relations.push(relation.to_string());
                        score *= weight * 0.8;
                        current = parent;
                    } else {
                        break;
                    }
                }

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

        all_paths.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut seen_targets = HashSet::new();
        let paths: Vec<GraphEntityPath> = all_paths
            .into_iter()
            .filter(|p| {
                p.node_labels
                    .last()
                    .map(|target| seen_targets.insert(target.clone()))
                    .unwrap_or(false)
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
            let relation = path
                .relations
                .last()
                .map(|s| s.as_str())
                .unwrap_or("related_to");

            if label == target {
                lines.push(format!("- **{}** ({})", target, relation));
            } else {
                if path.node_labels.len() <= 3 {
                    let hops: Vec<&str> = path.node_labels.iter().map(|s| s.as_str()).collect();
                    lines.push(format!("- {}", hops.join(" → ")));
                } else {
                    lines.push(format!(
                        "- {} → ... → {} ({} hops)",
                        label,
                        target,
                        path.node_labels.len() - 1
                    ));
                }
            }
        }

        lines.join("\n")
    }
}
