use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::memory::remember::RememberService;
use crate::memory::types::{MemoryEntry, MemoryType};

use super::remember::map_memory_entry_from_row;

// ─── Lesson Service ───────────────────────────────────────────────────

pub struct LessonService;

impl LessonService {
    /// Save a lesson. Reuses RememberService for storage.
    ///
    /// Mapping:
    /// - `title` → context/trigger condition
    /// - `content` → lesson body
    /// - `strength` → confidence (set during save)
    /// - `usage_count` → reinforcements (starts at 0)
    /// - `tags` → categorization + source tag
    pub fn save_lesson(
        db: &Connection,
        project: &str,
        content: &str,
        context: &str,
        confidence: f64,
        tags: &[String],
    ) -> Result<MemoryEntry> {
        let mut all_tags = tags.to_vec();
        if !all_tags.iter().any(|t| t == "lesson") {
            all_tags.push("lesson".to_string());
        }

        let mut entry = RememberService::remember(
            db,
            project,
            MemoryType::Lesson,
            context, // title = context/trigger
            content,
            &[], // concepts
            &[], // files
            &all_tags,
            None, // source_session_id
        )?;

        // Overwrite strength with confidence
        entry.strength = confidence.clamp(0.0, 1.0);
        db.execute(
            "UPDATE memories SET strength = ?1 WHERE id = ?2",
            rusqlite::params![entry.strength, entry.id.to_string()],
        )?;

        Ok(entry)
    }

    /// Recall lessons by query (title/content LIKE).
    pub fn recall_lessons(
        db: &Connection,
        project: &str,
        query: &str,
        limit: usize,
        min_confidence: f64,
    ) -> Result<Vec<MemoryEntry>> {
        let pattern = format!("%{}%", query);
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type = 'lesson'
               AND active = 1 AND is_latest = 1
               AND strength >= ?2
               AND (title LIKE ?3 OR content LIKE ?3)
             ORDER BY strength DESC, usage_count DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![project, min_confidence, pattern, limit as i64],
            |row| map_memory_entry_from_row(row),
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List recent lessons, most reinforced first.
    pub fn list_lessons(db: &Connection, project: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut stmt = db.prepare(
            "SELECT id, workspace_root, memory_type, title, content, tags,
                    source_session_id, created_at, updated_at, usage_count, active,
                    concepts, files, strength, importance, version, parent_id,
                    supersedes, related_ids, is_latest
             FROM memories
             WHERE workspace_root = ?1
               AND memory_type = 'lesson'
               AND active = 1 AND is_latest = 1
             ORDER BY usage_count DESC, strength DESC, updated_at DESC
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

    /// Reinforce a lesson: increment usage_count, boost strength.
    pub fn reinforce_lesson(db: &Connection, id: &Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        db.execute(
            "UPDATE memories SET
                usage_count = usage_count + 1,
                strength = MIN(1.0, strength + 0.1 * (1.0 - strength)),
                updated_at = ?1
             WHERE id = ?2 AND memory_type = 'lesson'",
            rusqlite::params![now, id.to_string()],
        )?;
        Ok(())
    }

    /// Decay lessons: reduce strength over time if not reinforced.
    /// Call periodically (e.g., once per hour).
    pub fn decay_lessons(db: &Connection, project: &str) -> Result<usize> {
        let affected = db.execute(
            "UPDATE memories SET
                strength = MAX(0.0, strength - 0.02)
             WHERE workspace_root = ?1
               AND memory_type = 'lesson'
               AND active = 1 AND is_latest = 1
               AND strength > 0.0",
            rusqlite::params![project],
        )?;
        Ok(affected)
    }

    /// Delete (soft-deactivate) a lesson.
    pub fn delete_lesson(db: &Connection, id: &Uuid) -> Result<()> {
        db.execute(
            "UPDATE memories SET active = 0 WHERE id = ?1 AND memory_type = 'lesson'",
            rusqlite::params![id.to_string()],
        )?;
        Ok(())
    }
}
