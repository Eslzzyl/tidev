use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::memory::types::AuditEntry;

/// Immutable audit log service.
/// Replicates agentmemory's `recordAudit` and `queryAudit` functions.
pub struct AuditService;

impl AuditService {
    /// Record an audit entry (immutable append).
    pub fn record(
        db: &Connection,
        operation: &str,
        entity_type: &str,
        entity_id: &str,
        actor: Option<&str>,
        details: Option<&serde_json::Value>,
        session_id: Option<Uuid>,
    ) -> Result<()> {
        db.execute(
            "INSERT INTO audit_log (id, timestamp, operation, entity_type, entity_id, actor, details, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                Uuid::new_v4().to_string(),
                Utc::now().to_rfc3339(),
                operation,
                entity_type,
                entity_id,
                actor,
                details.map(|d| d.to_string()),
                session_id.map(|id| id.to_string()),
            ],
        )?;
        Ok(())
    }

    /// Query audit log with filters.
    pub fn query(
        db: &Connection,
        opts: &AuditQuery,
    ) -> Result<Vec<AuditEntry>> {
        let mut sql = String::from(
            "SELECT id, timestamp, operation, entity_type, entity_id, actor, details, session_id
             FROM audit_log WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref op) = opts.operation {
            sql.push_str(" AND operation = ?");
            params.push(Box::new(op.clone()));
        }
        if let Some(ref et) = opts.entity_type {
            sql.push_str(" AND entity_type = ?");
            params.push(Box::new(et.clone()));
        }
        if let Some(ref eid) = opts.entity_id {
            sql.push_str(" AND entity_id = ?");
            params.push(Box::new(eid.clone()));
        }
        if let Some(sid) = opts.session_id {
            sql.push_str(" AND session_id = ?");
            params.push(Box::new(sid.to_string()));
        }
        if let Some((start, end)) = opts.time_range {
            sql.push_str(" AND timestamp >= ? AND timestamp <= ?");
            params.push(Box::new(start.to_rfc3339()));
            params.push(Box::new(end.to_rfc3339()));
        }

        sql.push_str(" ORDER BY timestamp DESC LIMIT ? OFFSET ?");
        let limit = opts.limit.unwrap_or(50) as i64;
        let offset = opts.offset.unwrap_or(0) as i64;
        params.push(Box::new(limit));
        params.push(Box::new(offset));

        let mut stmt = db.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let entries = stmt.query_map(param_refs.as_slice(), |row| {
            let details_str: Option<String> = row.get(6)?;
            let details = details_str.and_then(|s| serde_json::from_str(&s).ok());
            Ok(AuditEntry {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
                timestamp: row.get::<_, String>(1).ok()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                operation: row.get(2)?,
                entity_type: row.get(3)?,
                entity_id: row.get(4)?,
                actor: row.get(5)?,
                details,
                session_id: row.get::<_, Option<String>>(7)?
                    .and_then(|s| Uuid::parse_str(&s).ok()),
            })
        })?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry?);
        }
        Ok(result)
    }
}

/// Audit query parameters.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub operation: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub session_id: Option<Uuid>,
    pub time_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}
