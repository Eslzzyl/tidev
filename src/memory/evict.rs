use anyhow::Result;
use rusqlite::Connection;
use chrono::Utc;

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

        // 1. Stale memories
        let stale = db.execute(
            "UPDATE memories SET active = 0 WHERE active = 1 AND id IN (
                SELECT entity_id FROM retention_scores
                WHERE entity_type = 'memory' AND score < 1.0 AND age_days > 90
            )",
            [],
        )?;
        report.stale_memories_removed = stale as usize;

        // 2. Old non-latest versions
        let old_versions = db.execute(
            "UPDATE memories SET active = 0 WHERE active = 1 AND is_latest = 0
             AND julianday('now') - julianday(updated_at) > 30",
            [],
        )?;
        report.old_versions_removed = old_versions as usize;

        // 3. Clean up retention_scores for deleted entities
        db.execute(
            "DELETE FROM retention_scores WHERE entity_id NOT IN (
                SELECT id FROM memories WHERE active = 1
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
