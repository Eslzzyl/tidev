use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

use crate::config::ActiveModel;
use tidev_llm::LlmClient;
use crate::memory::remember::RememberService;
use crate::memory::remember::map_memory_entry_from_row;
use crate::memory::types::{MemoryEntry, MemoryType};
use crate::memory::xml::clean_llm_xml_response;
use tidev_session::session::{Message, MessageRole};

// ─── Prompts ──────────────────────────────────────────────────────────

pub const REFLECT_SYSTEM: &str = r#"You are a higher-order reasoning engine for a coding AI assistant. Given a cluster of related facts extracted from past work sessions, synthesize cross-cutting insights that span multiple individual facts.

Output EXACTLY this XML format with no additional text:

<insights>
  <insight confidence="0.0-1.0" title="Short descriptive title (max 60 chars)">
    The higher-order observation or principle (1-3 sentences). Should be actionable
    and non-obvious — something that only becomes visible when viewing multiple
    facts together.
  </insight>
</insights>

Rules:
- Identify patterns, principles, or strategies that span 2+ source facts
- Confidence reflects how well-supported the insight is across the cluster
- Title should be a concise label (under 60 chars)
- Content should be the actual observation (1-3 sentences)
- Prefer actionable insights over abstract summaries
- Skip insights that merely restate a single fact
- Aim for 1-3 insights per cluster"#;

fn build_reflect_prompt(cluster_concepts: &[String], facts: &[MemoryEntry]) -> String {
    let mut parts = Vec::new();

    if !cluster_concepts.is_empty() {
        parts.push(format!(
            "## Concept Cluster\n{}",
            cluster_concepts.join(", ")
        ));
    }

    parts.push("\n## Related Facts".to_string());
    for fact in facts {
        let conf = fact.strength;
        parts.push(format!("- [confidence={:.2}] {}", conf, fact.content));
    }

    parts.join("\n")
}

// ─── Report ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct ReflectReport {
    pub insights_added: usize,
    pub clusters_processed: usize,
    pub skipped_reason: Option<String>,
}

// ─── Insight Service ──────────────────────────────────────────────────

pub struct ReflectService;

impl ReflectService {
    /// Run the reflection pipeline:
    /// 1. Load consolidated facts from DB
    /// 2. Cluster by Jaccard similarity on concepts
    /// 3. For each cluster ≥3, call LLM to synthesize insights
    /// 4. Save insights as MemoryEntry(type=insight)
    pub async fn run(
        db_path: &Path,
        llm: &LlmClient,
        model: &ActiveModel,
        project: &str,
    ) -> Result<ReflectReport> {
        let mut report = ReflectReport::default();

        // 1. Load consolidated facts (sync, connection dropped)
        let facts = {
            let db = Connection::open(db_path)?;
            Self::load_facts(&db, project)?
        };

        if facts.len() < 3 {
            report.skipped_reason = Some(format!(
                "need at least 3 facts for clustering, got {}",
                facts.len()
            ));
            return Ok(report);
        }

        // 2. Cluster facts by concept overlap (Jaccard)
        let clusters = Self::cluster_facts(&facts, 0.3);
        report.clusters_processed = clusters.len();

        // 3. Check cursor — skip already-processed clusters
        let cursor = Self::load_cursor(db_path, "reflect")?;
        // If cursor is a UUID (old format), treat all clusters as new
        let cursor_is_uuid = cursor.len() == 36 && cursor.contains('-');
        let mut last_cursor_time: String = if cursor_is_uuid {
            "1970-01-01T00:00:00+00:00".to_string()
        } else {
            cursor
        };

        for cluster in &clusters {
            // Extract shared concepts across the cluster
            let concepts = Self::cluster_concepts(cluster);

            if cluster.len() < 3 {
                continue; // skip small clusters
            }

            // Find the newest fact's created_at in this cluster
            let cluster_max_time = cluster
                .iter()
                .map(|f| f.created_at.to_rfc3339())
                .max()
                .unwrap_or_default();
            if cluster_max_time <= last_cursor_time {
                continue;
            }

            // Check persistent retry count — skip if failed too many times
            let retry_key = format!("reflect_retry_{}", cluster_max_time);
            let retry_count: i64 = {
                let db = Connection::open(db_path)?;
                db.query_row(
                    "SELECT value FROM meta WHERE key = ?1",
                    rusqlite::params![&retry_key],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
            };
            if retry_count >= 3 {
                log::warn!(
                    "reflect: skipping cluster at {} after {} consecutive failures",
                    cluster_max_time,
                    retry_count
                );
                last_cursor_time = cluster_max_time.clone();
                Self::save_cursor(db_path, "reflect", &cluster_max_time)?;
                continue;
            }

            // ─── STRICTER_SUFFIX for retry ───────────────────────────
            const STRICTER_SUFFIX: &str = r"
IMPORTANT: Your response MUST contain valid XML tags. Do NOT output any text outside the XML tags. Do NOT wrap XML in markdown code fences.";

            // 4. Build prompt and call LLM
            let prompt = build_reflect_prompt(&concepts, cluster);

            let mut insights = Vec::new();
            for attempt in 0..2 {
                let system = if attempt > 0 {
                    format!("{}{}", REFLECT_SYSTEM, STRICTER_SUFFIX)
                } else {
                    REFLECT_SYSTEM.to_string()
                };
                let messages = vec![
                    Message::new(MessageRole::System, system),
                    Message::new(MessageRole::User, prompt.clone()),
                ];

                let response = match llm
                    .complete_with_messages(
                        tidev_llm::LlmProviderConfig::from(model.clone()),
                        messages,
                        vec![],
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        log::warn!("reflect LLM call failed (attempt {}): {}", attempt, e);
                        continue;
                    }
                };

                // 5. Parse XML
                insights = Self::parse_insights_xml(&response);
                if !insights.is_empty() {
                    break;
                }
                if attempt == 0 {
                    log::warn!(
                        "reflect: unparseable response, retrying with stricter prompt"
                    );
                }
            }

            // 6. Save insights (transactional)
            let db = Connection::open(db_path)?;
            db.execute_batch("BEGIN TRANSACTION")?;
            let mut all_saved = true;
            for insight in &insights {
                // Build tags from cluster concepts
                let tags: Vec<String> = concepts
                    .iter()
                    .map(|c| format!("concept:{}", c))
                    .chain(std::iter::once("insight".to_string()))
                    .collect();

                if let Err(e) = RememberService::remember(
                    &db,
                    project,
                    MemoryType::Insight,
                    &insight.title,
                    &insight.content,
                    &concepts,
                    &[], // files
                    &tags,
                    None, // source_session_id
                ) {
                    log::warn!("failed to remember insight: {}", e);
                    all_saved = false;
                    break;
                }
            }

            if all_saved && !insights.is_empty() {
                db.execute_batch("COMMIT")?;
                report.insights_added += insights.len();
                // On success, reset retry count
                let _ = db.execute(
                    "DELETE FROM meta WHERE key = ?1",
                    rusqlite::params![&retry_key],
                );
            } else {
                db.execute_batch("ROLLBACK")?;
                // Persist retry count so we don't retry the same cluster forever
                let new_count = retry_count + 1;
                let db2 = Connection::open(db_path)?;
                let _ = db2.execute(
                    "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                    rusqlite::params![&retry_key, new_count.to_string()],
                );
                log::warn!(
                    "reflect: cluster at {} failed (attempt {}/3), will retry",
                    cluster_max_time,
                    new_count
                );
                continue;
            }

            // Update cursor to the newest fact time in this cluster
            if cluster_max_time > last_cursor_time {
                last_cursor_time = cluster_max_time.clone();
                Self::save_cursor(db_path, "reflect", &cluster_max_time)?;
            }
        }

        Ok(report)
    }

    /// Load insights for prompt injection.
    pub fn load_insights(db: &Connection, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type = 'insight'
               AND active = 1 AND is_latest = 1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![project, limit as i64], |row| {
            map_memory_entry_from_row(row)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Reinforce an insight (same pattern as lesson reinforcement).
    pub fn reinforce_insight(db: &Connection, id: &Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        db.execute(
            "UPDATE memories SET
                usage_count = usage_count + 1,
                strength = MIN(1.0, strength + 0.1 * (1.0 - strength)),
                updated_at = ?1
             WHERE id = ?2 AND memory_type = 'insight'",
            rusqlite::params![now, id.to_string()],
        )?;
        Ok(())
    }

    // ─── Internal ──────────────────────────────────────────────────

    fn load_facts(db: &Connection, project: &str) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type = 'fact'
               AND active = 1 AND is_latest = 1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![project], |row| {
            map_memory_entry_from_row(row)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Cluster facts by Jaccard similarity of their concepts + content.
    /// Each fact is compared; if similarity ≥ threshold, they go in the same cluster.
    fn cluster_facts(facts: &[MemoryEntry], threshold: f64) -> Vec<Vec<MemoryEntry>> {
        use std::collections::HashMap;

        // Union-find for clustering
        let mut parent: HashMap<usize, usize> = HashMap::new();
        let mut rank: HashMap<usize, usize> = HashMap::new();

        fn find(parent: &mut HashMap<usize, usize>, x: usize) -> usize {
            let p = *parent.get(&x).unwrap_or(&x);
            if p != x {
                let root = find(parent, p);
                parent.insert(x, root);
                root
            } else {
                x
            }
        }

        fn union(
            parent: &mut HashMap<usize, usize>,
            rank: &mut HashMap<usize, usize>,
            a: usize,
            b: usize,
        ) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra == rb {
                return;
            }
            let rank_a = *rank.get(&ra).unwrap_or(&0);
            let rank_b = *rank.get(&rb).unwrap_or(&0);
            if rank_a < rank_b {
                parent.insert(ra, rb);
            } else if rank_a > rank_b {
                parent.insert(rb, ra);
            } else {
                parent.insert(rb, ra);
                rank.insert(ra, rank_a + 1);
            }
        }

        // Initialize each fact as its own parent
        for i in 0..facts.len() {
            parent.insert(i, i);
        }

        // Compare all pairs
        for i in 0..facts.len() {
            for j in (i + 1)..facts.len() {
                let sim = crate::memory::remember::jaccard_similarity(
                    &facts[i].content.to_lowercase(),
                    &facts[j].content.to_lowercase(),
                );
                // Also check concept overlap
                let concept_overlap =
                    if facts[i].concepts.is_empty() || facts[j].concepts.is_empty() {
                        0.0
                    } else {
                        let set_i: std::collections::HashSet<&str> =
                            facts[i].concepts.iter().map(|s| s.as_str()).collect();
                        let set_j: std::collections::HashSet<&str> =
                            facts[j].concepts.iter().map(|s| s.as_str()).collect();
                        let intersection = set_i.intersection(&set_j).count();
                        intersection as f64 / (set_i.len() + set_j.len() - intersection) as f64
                    };

                if sim.max(concept_overlap) >= threshold {
                    union(&mut parent, &mut rank, i, j);
                }
            }
        }

        // Build clusters from union-find roots
        let mut clusters: HashMap<usize, Vec<MemoryEntry>> = HashMap::new();
        for (i, fact) in facts.iter().enumerate() {
            let root = find(&mut parent, i);
            clusters.entry(root).or_default().push(fact.clone());
        }

        clusters.into_values().collect()
    }

    /// Extract shared concepts across a cluster.
    fn cluster_concepts(cluster: &[MemoryEntry]) -> Vec<String> {
        let mut all_concepts = Vec::new();
        for fact in cluster {
            for concept in &fact.concepts {
                if !all_concepts.contains(concept) {
                    all_concepts.push(concept.clone());
                }
            }
        }
        all_concepts.sort();
        all_concepts.truncate(10); // limit to 10 concepts
        all_concepts
    }

    /// Parse <insights> XML from LLM response.
    fn parse_insights_xml(raw: &str) -> Vec<InsightEntry> {
        let cleaned = clean_llm_xml_response(raw);
        let mut insights = Vec::new();

        // Extract each <insight> block using regex (case-insensitive on tag names)
        let pattern =
            r#"(?i)<insight\s+confidence="([^"]*)"\s+title="([^"]*)"[^>]*>([\s\S]*?)</insight>"#;
        if let Ok(re) = fancy_regex::Regex::new(pattern) {
            for c in re.captures_iter(&cleaned).flatten() {
                let title = c
                    .get(2)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                let content = c
                    .get(3)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();

                if !title.is_empty() && !content.is_empty() {
                    insights.push(InsightEntry { title, content });
                }
            }
        }

        insights
    }

    // ─── Cursor (same pattern as ConsolidationService) ────────────

    fn load_cursor(db_path: &Path, cursor_key: &str) -> Result<String> {
        let db = Connection::open(db_path)?;
        let key = format!("reflect_cursor_{}", cursor_key);
        let val: Option<String> = db
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .ok();
        Ok(val.unwrap_or_default())
    }

    fn save_cursor(db_path: &Path, cursor_key: &str, value: &str) -> Result<()> {
        let db = Connection::open(db_path)?;
        let key = format!("reflect_cursor_{}", cursor_key);
        db.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }
}

// ─── Internal Types ─────────────────────────────────────────────────

struct InsightEntry {
    title: String,
    content: String,
}
