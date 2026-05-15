use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::memory::types::{MemorySlot, SlotScope};

/// Default slots (from agentmemory's `DEFAULT_SLOTS`).
pub const DEFAULT_SLOTS: &[SlotDef] = &[
    SlotDef {
        label: "persona",
        content: "",
        size_limit: 1000,
        description: "How the agent should see itself: role, tone, behavioural guidelines.",
        pinned: true,
        read_only: false,
        scope: SlotScope::Global,
    },
    SlotDef {
        label: "user_preferences",
        content: "",
        size_limit: 2000,
        description: "Coding style, tool preferences, naming conventions, and other habits the user wants preserved across sessions.",
        pinned: true,
        read_only: false,
        scope: SlotScope::Global,
    },
    SlotDef {
        label: "tool_guidelines",
        content: "",
        size_limit: 1500,
        description: "Rules the agent should follow when picking or sequencing tools (e.g. prefer X over Y, never run Z without confirmation).",
        pinned: true,
        read_only: false,
        scope: SlotScope::Global,
    },
    SlotDef {
        label: "project_context",
        content: "",
        size_limit: 3000,
        description: "Architecture decisions, codebase conventions, build/test commands, and cross-cutting constraints for the current project.",
        pinned: true,
        read_only: false,
        scope: SlotScope::Project,
    },
    SlotDef {
        label: "guidance",
        content: "",
        size_limit: 1500,
        description: "Active advice for the next session: what to focus on, what to avoid, open risks.",
        pinned: true,
        read_only: false,
        scope: SlotScope::Project,
    },
    SlotDef {
        label: "pending_items",
        content: "",
        size_limit: 2000,
        description: "Unfinished work, explicit TODOs, and promises made but not yet delivered.",
        pinned: true,
        read_only: false,
        scope: SlotScope::Project,
    },
    SlotDef {
        label: "session_patterns",
        content: "",
        size_limit: 1500,
        description: "Recurring behaviours and common struggles observed across recent sessions.",
        pinned: false,
        read_only: false,
        scope: SlotScope::Project,
    },
    SlotDef {
        label: "self_notes",
        content: "",
        size_limit: 1500,
        description: "Free-form notes the agent keeps for itself: hypotheses, dead ends, things to revisit.",
        pinned: false,
        read_only: false,
        scope: SlotScope::Project,
    },
];

pub struct SlotDef {
    pub label: &'static str,
    pub content: &'static str,
    pub size_limit: usize,
    pub description: &'static str,
    pub pinned: bool,
    pub read_only: bool,
    pub scope: SlotScope,
}

/// Memory slot CRUD service.
/// Replicates agentmemory's slot-* functions.
pub struct SlotService;

impl SlotService {
    /// Ensure default slots exist for a project.
    pub fn ensure_defaults(db: &Connection, project: &str) -> Result<()> {
        for def in DEFAULT_SLOTS {
            let scope_str = def.scope.as_str();
            let project_str = match def.scope {
                SlotScope::Global => "",
                SlotScope::Project => project,
            };
            // Insert if not exists
            db.execute(
                "INSERT OR IGNORE INTO memory_slots (label, scope, project, content, size_limit, description, pinned, read_only, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    def.label,
                    scope_str,
                    project_str,
                    def.content,
                    def.size_limit as i64,
                    def.description,
                    def.pinned as i64,
                    def.read_only as i64,
                    Utc::now().to_rfc3339(),
                    Utc::now().to_rfc3339(),
                ],
            )?;
        }
        Ok(())
    }

    /// List slots, optionally filtered by scope and project.
    pub fn list(db: &Connection, scope: Option<SlotScope>, project: Option<&str>) -> Result<Vec<MemorySlot>> {
        let mut sql = String::from(
            "SELECT label, scope, project, content, size_limit, description, pinned, read_only, created_at, updated_at
             FROM memory_slots WHERE 1=1"
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = scope {
            sql.push_str(" AND scope = ?");
            params.push(Box::new(s.as_str().to_string()));
        }
        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project = '')");
            params.push(Box::new(p.to_string()));
        }

        sql.push_str(" ORDER BY label");

        let mut stmt = db.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| map_slot(row))?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    /// Get a single slot.
    pub fn get(db: &Connection, label: &str, scope: SlotScope, project: &str) -> Result<Option<MemorySlot>> {
        let project_str = match scope {
            SlotScope::Global => "",
            SlotScope::Project => project,
        };
        let mut stmt = db.prepare(
            "SELECT label, scope, project, content, size_limit, description, pinned, read_only, created_at, updated_at
             FROM memory_slots WHERE label = ?1 AND scope = ?2 AND project = ?3",
        )?;
        let result = stmt.query_row(
            rusqlite::params![label, scope.as_str(), project_str],
            |row| map_slot(row),
        );
        match result {
            Ok(slot) => Ok(Some(slot)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Create or update a slot.
    pub fn set(db: &Connection, slot: &MemorySlot) -> Result<()> {
        let scope_str = slot.scope.as_str();
        let project_str = match slot.scope {
            SlotScope::Global => "",
            SlotScope::Project => &slot.project,
        };
        db.execute(
            "INSERT OR REPLACE INTO memory_slots (label, scope, project, content, size_limit, description, pinned, read_only, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                slot.label,
                scope_str,
                project_str,
                slot.content,
                slot.size_limit as i64,
                slot.description,
                slot.pinned as i64,
                slot.read_only as i64,
                slot.created_at.to_rfc3339(),
                slot.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Append content to a slot (respects size limit).
    pub fn append(db: &Connection, label: &str, scope: SlotScope, project: &str, content: &str) -> Result<MemorySlot> {
        let existing = Self::get(db, label, scope, project)?
            .ok_or_else(|| anyhow::anyhow!("slot '{}' not found", label))?;

        let new_content = if existing.content.is_empty() {
            content.to_string()
        } else {
            format!("{}\n\n---\n\n{}", existing.content, content)
        };

        // Truncate to size limit
        let new_content = if new_content.len() > existing.size_limit {
            let truncated: String = new_content.chars().take(existing.size_limit).collect();
            format!("{}…", truncated)
        } else {
            new_content
        };

        let updated = MemorySlot {
            content: new_content,
            updated_at: Utc::now(),
            ..existing
        };

        Self::set(db, &updated)?;
        Ok(updated)
    }

    /// Delete a slot.
    pub fn delete(db: &Connection, label: &str, scope: SlotScope, project: &str) -> Result<()> {
        let project_str = match scope {
            SlotScope::Global => "",
            SlotScope::Project => project,
        };
        db.execute(
            "DELETE FROM memory_slots WHERE label = ?1 AND scope = ?2 AND project = ?3",
            rusqlite::params![label, scope.as_str(), project_str],
        )?;
        Ok(())
    }

    /// Render pinned slots as a formatted string for prompt injection.
    /// Automatically ensures default slots exist.
    pub fn render_pinned(db: &Connection, project: &str) -> Result<String> {
        // Ensure defaults first
        Self::ensure_defaults(db, project)?;

        let slots = Self::list(db, None, Some(project))?;
        let pinned: Vec<&MemorySlot> = slots.iter().filter(|s| s.pinned && !s.content.is_empty()).collect();

        if pinned.is_empty() {
            return Ok(String::new());
        }

        let mut parts = vec!["## Memory Slots\n".to_string()];
        for slot in &pinned {
            let scope_tag = match slot.scope {
                SlotScope::Global => "[global]",
                SlotScope::Project => "[project]",
            };
            parts.push(format!(
                "### {} {}\n{}\n",
                slot.label, scope_tag, slot.content
            ));
        }
        Ok(parts.join("\n"))
    }
}

fn map_slot(row: &rusqlite::Row) -> rusqlite::Result<MemorySlot> {
    let scope_str: String = row.get(1)?;
    Ok(MemorySlot {
        label: row.get(0)?,
        scope: SlotScope::parse_str(&scope_str).unwrap_or(SlotScope::Global),
        project: row.get(2)?,
        content: row.get(3)?,
        size_limit: row.get::<_, i64>(4)? as usize,
        description: row.get(5)?,
        pinned: row.get::<_, i64>(6)? != 0,
        read_only: row.get::<_, i64>(7)? != 0,
        created_at: row.get::<_, String>(8).ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
        updated_at: row.get::<_, String>(9).ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
    })
}
