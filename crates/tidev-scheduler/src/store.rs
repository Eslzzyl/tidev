use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::schedule::{next_run_for_schedule, schedule_cron_expression, validate_schedule};
use crate::types::{
    CronJob, CronJobPatch, CronRun, DeliveryConfig, JobResult, JobType, Schedule, SessionTarget,
};

const SCHEDULER_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS scheduler_jobs (
    id               TEXT PRIMARY KEY,
    expression       TEXT NOT NULL DEFAULT '',
    command          TEXT NOT NULL DEFAULT '',
    schedule         TEXT,
    job_type         TEXT NOT NULL DEFAULT 'shell',
    prompt           TEXT,
    name             TEXT,
    session_target   TEXT NOT NULL DEFAULT 'isolated',
    model            TEXT,
    agent_alias      TEXT NOT NULL DEFAULT 'default',
    enabled          INTEGER NOT NULL DEFAULT 1,
    delivery         TEXT,
    delete_after_run INTEGER NOT NULL DEFAULT 0,
    allowed_tools    TEXT,
    uses_memory      INTEGER NOT NULL DEFAULT 1,
    source           TEXT NOT NULL DEFAULT 'imperative',
    created_at       TEXT NOT NULL,
    next_run         TEXT NOT NULL,
    last_run         TEXT,
    last_status      TEXT,
    last_output      TEXT
);
CREATE INDEX IF NOT EXISTS idx_scheduler_jobs_next_run ON scheduler_jobs(next_run);

CREATE TABLE IF NOT EXISTS scheduler_runs (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    status      TEXT NOT NULL,
    output      TEXT,
    duration_ms INTEGER,
    delivered   INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (job_id) REFERENCES scheduler_jobs(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_scheduler_runs_job_id ON scheduler_runs(job_id);
CREATE INDEX IF NOT EXISTS idx_scheduler_runs_started_at ON scheduler_runs(started_at);
CREATE INDEX IF NOT EXISTS idx_scheduler_runs_job_started ON scheduler_runs(job_id, started_at);
"#;

/// Persistent storage for cron jobs and run history.
///
/// Uses the same shared write connection as the main tidev database,
/// ensuring all tables live in a single SQLite file.
pub struct CronStore {
    write_conn: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    path: PathBuf,
    max_tasks: usize,
    max_run_history: usize,
}

impl CronStore {
    /// Open (or create) the scheduler tables using a shared write connection.
    ///
    /// `write_conn` should come from tidev's `Database` (the shared
    /// `Arc<Mutex<Connection>>`).  The schema is created idempotently.
    pub fn new(
        write_conn: Arc<Mutex<Connection>>,
        path: &Path,
        max_tasks: usize,
        max_run_history: usize,
    ) -> Result<Self> {
        let store = Self {
            write_conn,
            path: path.to_path_buf(),
            max_tasks,
            max_run_history,
        };
        store.initialize_schema()?;
        Ok(store)
    }

    fn initialize_schema(&self) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute_batch(SCHEDULER_SCHEMA_SQL)
            .context("Failed to initialize scheduler schema")?;
        Ok(())
    }

    // ── CRUD ───────────────────────────────────────────────────────────────

    /// Add a new shell job.
    pub fn add_shell_job(
        &self,
        name: Option<String>,
        schedule: Schedule,
        command: &str,
        delivery: Option<DeliveryConfig>,
    ) -> Result<CronJob> {
        let now = Utc::now();
        validate_schedule(&schedule, now)?;
        let next_run = next_run_for_schedule(&schedule, now)?;
        let id = Uuid::new_v4().to_string();
        let expression = schedule_cron_expression(&schedule).unwrap_or_default();
        let schedule_json = serde_json::to_string(&schedule)?;
        let delivery_json = delivery
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduler_jobs
                (id, expression, command, schedule, job_type, prompt, name, session_target, model,
                 agent_alias, enabled, delivery, delete_after_run, allowed_tools, uses_memory, source,
                 created_at, next_run, last_run, last_status, last_output)
             VALUES (?1, ?2, ?3, ?4, 'shell', NULL, ?5, 'isolated', NULL,
                     'default', 1, ?6, 0, NULL, 1, 'imperative',
                     ?7, ?8, NULL, NULL, NULL)",
            params![
                id,
                expression,
                command,
                schedule_json,
                name,
                delivery_json,
                now.to_rfc3339(),
                next_run.to_rfc3339(),
            ],
        )?;

        Ok(CronJob {
            id,
            expression,
            schedule,
            command: command.to_string(),
            prompt: None,
            name,
            job_type: JobType::Shell,
            session_target: SessionTarget::Isolated,
            model: None,
            agent_alias: "default".to_string(),
            enabled: true,
            delivery: delivery.unwrap_or_default(),
            delete_after_run: false,
            allowed_tools: None,
            uses_memory: true,
            source: "imperative".to_string(),
            created_at: now,
            next_run,
            last_run: None,
            last_status: None,
            last_output: None,
        })
    }

    /// Add a new agent job.
    pub fn add_agent_job(
        &self,
        name: Option<String>,
        schedule: Schedule,
        prompt: &str,
        model: Option<String>,
        delivery: Option<DeliveryConfig>,
    ) -> Result<CronJob> {
        let now = Utc::now();
        validate_schedule(&schedule, now)?;
        let next_run = next_run_for_schedule(&schedule, now)?;
        let id = Uuid::new_v4().to_string();
        let expression = schedule_cron_expression(&schedule).unwrap_or_default();
        let schedule_json = serde_json::to_string(&schedule)?;
        let delivery_json = delivery
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduler_jobs
                (id, expression, command, schedule, job_type, prompt, name, session_target, model,
                 agent_alias, enabled, delivery, delete_after_run, allowed_tools, uses_memory, source,
                 created_at, next_run, last_run, last_status, last_output)
             VALUES (?1, '', '', ?2, 'agent', ?3, ?4, 'isolated', ?5,
                     'default', 1, ?6, 0, NULL, 1, 'imperative',
                     ?7, ?8, NULL, NULL, NULL)",
            params![
                id,
                schedule_json,
                prompt,
                name,
                model,
                delivery_json,
                now.to_rfc3339(),
                next_run.to_rfc3339(),
            ],
        )?;

        Ok(CronJob {
            id,
            expression,
            schedule,
            command: String::new(),
            prompt: Some(prompt.to_string()),
            name,
            job_type: JobType::Agent,
            session_target: SessionTarget::Isolated,
            model,
            agent_alias: "default".to_string(),
            enabled: true,
            delivery: delivery.unwrap_or_default(),
            delete_after_run: false,
            allowed_tools: None,
            uses_memory: true,
            source: "imperative".to_string(),
            created_at: now,
            next_run,
            last_run: None,
            last_status: None,
            last_output: None,
        })
    }

    /// List all jobs.
    pub fn list_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = self.write_conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, expression, command, schedule, job_type, prompt, name, session_target, model,
                    enabled, delivery, delete_after_run, created_at, next_run, last_run, last_status,
                    last_output, allowed_tools, source, uses_memory, agent_alias
             FROM scheduler_jobs
             ORDER BY created_at DESC",
        )?;

        let rows = stmt.query_map([], map_cron_job_row)?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row?);
        }
        Ok(jobs)
    }

    /// Get a single job by ID.
    pub fn get_job(&self, job_id: &str) -> Result<CronJob> {
        let conn = self.write_conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, expression, command, schedule, job_type, prompt, name, session_target, model,
                    enabled, delivery, delete_after_run, created_at, next_run, last_run, last_status,
                    last_output, allowed_tools, source, uses_memory, agent_alias
             FROM scheduler_jobs WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![job_id])?;
        if let Some(row) = rows.next()? {
            map_cron_job_row(row).map_err(Into::into)
        } else {
            anyhow::bail!("Cron job '{job_id}' not found")
        }
    }

    /// Update a job with partial fields.
    pub fn update_job(&self, job_id: &str, patch: &CronJobPatch) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();

        if let Some(ref schedule) = patch.schedule {
            let schedule_json = serde_json::to_string(schedule)?;
            let expression = schedule_cron_expression(schedule).unwrap_or_default();
            let next_run = next_run_for_schedule(schedule, Utc::now())?;
            conn.execute(
                "UPDATE scheduler_jobs SET schedule = ?1, expression = ?2, next_run = ?3 WHERE id = ?4",
                params![schedule_json, expression, next_run.to_rfc3339(), job_id],
            )?;
        }
        if let Some(ref command) = patch.command {
            conn.execute(
                "UPDATE scheduler_jobs SET command = ?1 WHERE id = ?2",
                params![command, job_id],
            )?;
        }
        if let Some(ref prompt) = patch.prompt {
            conn.execute(
                "UPDATE scheduler_jobs SET prompt = ?1 WHERE id = ?2",
                params![prompt, job_id],
            )?;
        }
        if let Some(ref name) = patch.name {
            conn.execute(
                "UPDATE scheduler_jobs SET name = ?1 WHERE id = ?2",
                params![name, job_id],
            )?;
        }
        if let Some(enabled) = patch.enabled {
            conn.execute(
                "UPDATE scheduler_jobs SET enabled = ?1 WHERE id = ?2",
                params![enabled as i64, job_id],
            )?;
        }
        if let Some(ref delivery) = patch.delivery {
            let json = serde_json::to_string(delivery)?;
            conn.execute(
                "UPDATE scheduler_jobs SET delivery = ?1 WHERE id = ?2",
                params![json, job_id],
            )?;
        }

        Ok(())
    }

    /// Delete a job.
    pub fn remove_job(&self, job_id: &str) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let changed = conn.execute(
            "DELETE FROM scheduler_jobs WHERE id = ?1",
            params![job_id],
        )?;
        if changed == 0 {
            anyhow::bail!("Cron job '{job_id}' not found");
        }
        Ok(())
    }

    // ── Query due jobs ────────────────────────────────────────────────────

    /// Get all enabled jobs whose `next_run` is <= `now`.
    pub fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<CronJob>> {
        let lim = i64::try_from(self.max_tasks.max(1))
            .context("max_tasks overflows i64")?;

        let conn = self.write_conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, expression, command, schedule, job_type, prompt, name, session_target, model,
                    enabled, delivery, delete_after_run, created_at, next_run, last_run, last_status,
                    last_output, allowed_tools, source, uses_memory, agent_alias
             FROM scheduler_jobs
             WHERE enabled = 1 AND next_run <= ?1
             ORDER BY next_run ASC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![now.to_rfc3339(), lim], map_cron_job_row)?;
        let mut jobs = Vec::new();
        for row in rows {
            match row {
                Ok(job) => jobs.push(job),
                Err(e) => log::warn!("Failed to map cron job row: {e}"),
            }
        }
        Ok(jobs)
    }

    // ── Run history ────────────────────────────────────────────────────────

    /// Record a job execution result.
    pub fn record_run(&self, job: &CronJob, result: &JobResult) -> Result<CronRun> {
        let id = Uuid::new_v4().to_string();

        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduler_runs (id, job_id, started_at, finished_at, status, output, duration_ms, delivered)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![
                id,
                job.id,
                result.started_at.to_rfc3339(),
                result.finished_at.to_rfc3339(),
                if result.success { "success" } else { "failure" },
                result.output,
                result.duration_ms,
            ],
        )?;

        // Prune old run history
        self.prune_run_history(&job.id)?;

        Ok(CronRun {
            id,
            job_id: job.id.clone(),
            started_at: result.started_at,
            finished_at: result.finished_at,
            status: if result.success {
                "success".to_string()
            } else {
                "failure".to_string()
            },
            output: Some(result.output.clone()),
            duration_ms: Some(result.duration_ms),
            delivered: false,
        })
    }

    /// Update `last_run`, `last_status`, `last_output`, and `next_run` after
    /// a job completes.
    pub fn reschedule_after_run(
        &self,
        job: &CronJob,
        next_run: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let now = Utc::now();
        let conn = self.write_conn.lock().unwrap();

        if let Some(next) = next_run {
            conn.execute(
                "UPDATE scheduler_jobs SET last_run = ?1, last_status = ?2, last_output = ?3, next_run = ?4 WHERE id = ?5",
                params![
                    now.to_rfc3339(),
                    "success",
                    "",
                    next.to_rfc3339(),
                    job.id,
                ],
            )?;
        } else {
            // Job is done (At-type) or should be disabled
            conn.execute(
                "UPDATE scheduler_jobs SET last_run = ?1, last_status = ?2, last_output = ?3, enabled = 0 WHERE id = ?4",
                params![now.to_rfc3339(), "completed", "", job.id],
            )?;
        }
        Ok(())
    }

    /// List runs for a specific job, most recent first.
    pub fn list_runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRun>> {
        let conn = self.write_conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_id, started_at, finished_at, status, output, duration_ms, delivered
             FROM scheduler_runs
             WHERE job_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;

        let lim = i64::try_from(limit.max(1)).context("limit overflows i64")?;
        let rows = stmt.query_map(params![job_id, lim], |row| {
            Ok(CronRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at: parse_rfc3339(&row.get::<_, String>(2)?)
                    .map_err(to_sql_err)?,
                finished_at: parse_rfc3339(&row.get::<_, String>(3)?)
                    .map_err(to_sql_err)?,
                status: row.get(4)?,
                output: row.get(5)?,
                duration_ms: row.get(6)?,
                delivered: row.get::<_, i64>(7)? != 0,
            })
        })?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// Mark a run as delivered.
    pub fn mark_delivered(&self, run_id: &str) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduler_runs SET delivered = 1 WHERE id = ?1",
            params![run_id],
        )?;
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn prune_run_history(&self, job_id: &str) -> Result<()> {
        let limit = i64::try_from(self.max_run_history.max(1))
            .context("max_run_history overflows i64")?;
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "DELETE FROM scheduler_runs
             WHERE id IN (
                 SELECT id FROM scheduler_runs
                 WHERE job_id = ?1
                 ORDER BY started_at DESC
                 LIMIT -1 OFFSET ?2
             )",
            params![job_id, limit],
        )?;
        Ok(())
    }
}

// ── Row mapping ───────────────────────────────────────────────────────────

fn map_cron_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    let expression: String = row.get(1)?;
    let schedule_raw: Option<String> = row.get(3)?;
    let schedule = decode_schedule(schedule_raw.as_deref(), &expression)
        .map_err(to_sql_err)?;

    let delivery_raw: Option<String> = row.get(10)?;
    let delivery = decode_delivery(delivery_raw.as_deref())
        .map_err(to_sql_err)?;

    let next_run_raw: String = row.get(13)?;
    let last_run_raw: Option<String> = row.get(14)?;
    let created_at_raw: String = row.get(12)?;
    let allowed_tools_raw: Option<String> = row.get(17)?;
    let source: Option<String> = row.get(18)?;
    let uses_memory: Option<i64> = row.get(19)?;
    let agent_alias: Option<String> = row.get(20)?;

    Ok(CronJob {
        id: row.get(0)?,
        expression,
        schedule,
        command: row.get(2)?,
        job_type: row.get::<_, String>(4)?.as_str().try_into().unwrap_or(JobType::Shell),
        prompt: row.get(5)?,
        name: row.get(6)?,
        session_target: SessionTarget::parse(&row.get::<_, String>(7)?),
        model: row.get(8)?,
        agent_alias: agent_alias.unwrap_or_default(),
        enabled: row.get::<_, i64>(9)? != 0,
        delivery,
        delete_after_run: row.get::<_, i64>(11)? != 0,
        source: source.unwrap_or_else(|| "imperative".to_string()),
        uses_memory: uses_memory != Some(0),
        created_at: parse_rfc3339(&created_at_raw)
            .map_err(to_sql_err)?,
        next_run: parse_rfc3339(&next_run_raw)
            .map_err(to_sql_err)?,
        last_run: match last_run_raw {
            Some(raw) => Some(
                parse_rfc3339(&raw)
                    .map_err(to_sql_err)?,
            ),
            None => None,
        },
        last_status: row.get(15)?,
        last_output: row.get(16)?,
        allowed_tools: decode_allowed_tools(allowed_tools_raw.as_deref())
            .map_err(to_sql_err)?,
    })
}

fn decode_schedule(schedule_raw: Option<&str>, expression: &str) -> Result<Schedule> {
    if let Some(raw) = schedule_raw {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return serde_json::from_str(trimmed)
                .with_context(|| format!("Failed to parse cron schedule JSON: {trimmed}"));
        }
    }
    if expression.trim().is_empty() {
        anyhow::bail!("Missing schedule and legacy expression for cron job");
    }
    Ok(Schedule::Cron {
        expr: expression.to_string(),
        tz: None,
    })
}

fn decode_delivery(delivery_raw: Option<&str>) -> Result<DeliveryConfig> {
    if let Some(raw) = delivery_raw {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && trimmed != "null" {
            return serde_json::from_str(trimmed)
                .with_context(|| format!("Failed to parse delivery config JSON: {trimmed}"));
        }
    }
    Ok(DeliveryConfig::default())
}

fn decode_allowed_tools(raw: Option<&str>) -> Result<Option<Vec<String>>> {
    match raw {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            serde_json::from_str(s)
                .with_context(|| format!("Failed to parse allowed_tools JSON: {s}"))
                .map(Some)
        }
        _ => Ok(None),
    }
}

fn to_sql_err(e: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(
        Box::new(std::io::Error::other(e.to_string()))
    )
}

fn parse_rfc3339(s: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("Invalid RFC 3339 timestamp: {s}"))?
        .with_timezone(&Utc))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_store() -> (CronStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();
        let write_conn = Arc::new(Mutex::new(conn));
        let store = CronStore::new(write_conn, &db_path, 10, 10).unwrap();
        (store, dir)
    }

    #[test]
    fn test_add_and_list_shell_job() {
        let (store, _dir) = setup_store();
        let schedule = Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
        };
        let job = store
            .add_shell_job(Some("test".into()), schedule, "echo hello", None)
            .unwrap();
        assert_eq!(job.name, Some("test".into()));
        assert_eq!(job.job_type, JobType::Shell);

        let jobs = store.list_jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, job.id);
    }

    #[test]
    fn test_due_jobs() {
        let (store, _dir) = setup_store();
        let schedule = Schedule::Every { every_ms: 100 };
        store
            .add_shell_job(Some("frequent".into()), schedule, "echo test", None)
            .unwrap();

        let now = Utc::now();
        let due = store.due_jobs(now).unwrap();
        assert_eq!(due.len(), 0); // not due yet (next_run is in the future)

        let far_future = now + chrono::Duration::hours(1);
        let due = store.due_jobs(far_future).unwrap();
        assert_eq!(due.len(), 1);
    }

    #[test]
    fn test_remove_job() {
        let (store, _dir) = setup_store();
        let schedule = Schedule::Every { every_ms: 5000 };
        let job = store
            .add_shell_job(None, schedule, "echo hi", None)
            .unwrap();

        assert!(store.list_jobs().unwrap().len() == 1);
        store.remove_job(&job.id).unwrap();
        assert!(store.list_jobs().unwrap().is_empty());
    }

    #[test]
    fn test_record_and_list_runs() {
        let (store, _dir) = setup_store();
        let schedule = Schedule::Every { every_ms: 5000 };
        let job = store
            .add_shell_job(None, schedule, "echo test", None)
            .unwrap();

        let result = JobResult {
            success: true,
            output: "hello".into(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            duration_ms: 42,
        };
        store.record_run(&job, &result).unwrap();

        let runs = store.list_runs(&job.id, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "success");
    }
}
