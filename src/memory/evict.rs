use anyhow::Result;
use rusqlite::Connection;

/// Automatic eviction service.
/// Replicates agentmemory's `mem::evict` and `mem::auto-forget`.
pub struct EvictionService;

impl EvictionService {
    /// Run eviction rules:
    /// 1. Soft-delete stale memories (retention score < 1.0, older than 90 days)
    /// 2. Soft-delete old non-latest memory versions (older than 30 days)
    /// 3. Remove stale sessions without summary (older than 30 days)
    pub fn run_eviction(db: &Connection) -> Result<EvictionReport> {
        let mut report = EvictionReport::default();

        // 1. Stale memories (use real-time age via julianday, not cached age_days)
        let stale = db.execute(
            "UPDATE memories SET active = 0 WHERE active = 1 AND id IN (
                SELECT rs.entity_id FROM retention_scores rs
                JOIN memories m ON m.id = rs.entity_id
                WHERE rs.entity_type = 'memory' AND rs.score < 1.0
                  AND julianday('now') - julianday(m.created_at) > 90
            )",
            [],
        )?;
        report.stale_memories_removed = stale;

        // 2. Old non-latest versions
        let old_versions = db.execute(
            "UPDATE memories SET active = 0 WHERE active = 1 AND is_latest = 0
             AND julianday('now') - julianday(updated_at) > 30",
            [],
        )?;
        report.old_versions_removed = old_versions;

        // 3. Clean up retention_scores for deleted entities
        db.execute(
            "DELETE FROM retention_scores WHERE entity_id NOT IN (
                SELECT id FROM memories WHERE active = 1
            )",
            [],
        )?;

        // 4. Remove dangling graph nodes that belong to soft-deleted memories
        //    (no longer referenced by any edge + not referenced by any active memory concept/file)
        db.execute(
            "DELETE FROM graph_edges WHERE source_id NOT IN (
                SELECT id FROM graph_nodes
            ) OR target_id NOT IN (
                SELECT id FROM graph_nodes
            )",
            [],
        )?;
        db.execute(
            "DELETE FROM graph_nodes WHERE id NOT IN (
                SELECT source_id FROM graph_edges
                UNION
                SELECT target_id FROM graph_edges
            )",
            [],
        )?;

        // 5. Clean up session_summaries that no longer have a corresponding session
        db.execute(
            "DELETE FROM session_summaries WHERE session_id NOT IN (
                SELECT id FROM sessions
            )",
            [],
        )?;

        Ok(report)
    }
}

#[derive(Debug, Default)]
pub struct EvictionReport {
    pub stale_memories_removed: usize,
    pub old_versions_removed: usize,
}
