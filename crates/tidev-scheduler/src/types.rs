use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── JobType ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    #[default]
    Shell,
    Agent,
}

impl TryFrom<&str> for JobType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "shell" => Ok(JobType::Shell),
            "agent" => Ok(JobType::Agent),
            _ => Err(format!(
                "Invalid job type '{}'. Expected one of: 'shell', 'agent'",
                value
            )),
        }
    }
}

// ── Schedule ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Schedule {
    Cron {
        expr: String,
        #[serde(default)]
        tz: Option<String>,
    },
    At {
        at: DateTime<Utc>,
    },
    Every {
        every_ms: u64,
    },
}

impl Schedule {
    /// Return a human-readable summary of this schedule.
    pub fn summary(&self) -> String {
        match self {
            Schedule::Cron { expr, tz } => {
                if let Some(tz) = tz {
                    format!("cron({expr}, tz={tz})")
                } else {
                    format!("cron({expr})")
                }
            }
            Schedule::At { at } => format!("at({at})"),
            Schedule::Every { every_ms } => format!("every({every_ms}ms)"),
        }
    }
}

// ── SessionTarget ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionTarget {
    #[default]
    Isolated,
    Main,
}

impl SessionTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Isolated => "isolated",
            Self::Main => "main",
        }
    }

    pub fn parse(raw: &str) -> Self {
        if raw.eq_ignore_ascii_case("main") {
            Self::Main
        } else {
            Self::Isolated
        }
    }
}

// ── DeliveryConfig ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryConfig {
    /// Delivery mode: "announce" or "none".
    #[serde(default)]
    pub mode: String,
    /// Target channel type: "telegram", "qq", "discord", "lark".
    #[serde(default)]
    pub channel: Option<String>,
    /// Target recipient ID (chat_id, channel_id, user_id).
    #[serde(default)]
    pub to: Option<String>,
    /// Optional thread/conversation identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Best-effort delivery: don't fail the job if delivery fails.
    #[serde(default = "default_true")]
    pub best_effort: bool,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            mode: "none".to_string(),
            channel: None,
            to: None,
            thread_id: None,
            best_effort: true,
        }
    }
}

pub fn default_true() -> bool {
    true
}

fn default_source() -> String {
    "imperative".to_string()
}

// ── CronJob ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub expression: String,
    pub schedule: Schedule,
    pub command: String,
    pub prompt: Option<String>,
    pub name: Option<String>,
    pub job_type: JobType,
    pub session_target: SessionTarget,
    pub model: Option<String>,
    /// Agent alias this job runs under.
    #[serde(default)]
    pub agent_alias: String,
    pub enabled: bool,
    pub delivery: DeliveryConfig,
    pub delete_after_run: bool,
    /// Optional allowlist of tool names for agent jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    /// Whether to inject memory context for agent jobs.
    #[serde(default = "default_true")]
    pub uses_memory: bool,
    /// How the job was created: "imperative" (CLI/API) or "declarative" (config).
    #[serde(default = "default_source")]
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub next_run: DateTime<Utc>,
    #[serde(default)]
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_output: Option<String>,
}

// ── CronRun ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub output: Option<String>,
    pub duration_ms: Option<i64>,
    pub delivered: bool,
}

// ── CronJobPatch ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJobPatch {
    pub schedule: Option<Schedule>,
    pub command: Option<String>,
    pub prompt: Option<String>,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub delivery: Option<DeliveryConfig>,
    pub model: Option<String>,
    pub session_target: Option<SessionTarget>,
    pub delete_after_run: Option<bool>,
    pub allowed_tools: Option<Vec<String>>,
    pub uses_memory: Option<bool>,
}

// ── CronDeliveryMessage (sent from scheduler → gateway channels) ──────────

/// A message produced by the scheduler after a job execution, intended for
/// delivery through a gateway channel (Telegram, QQ, etc.).
#[derive(Debug, Clone)]
pub struct CronDeliveryMessage {
    pub job_id: String,
    pub job_name: String,
    pub output: String,
    pub delivery: DeliveryConfig,
    pub success: bool,
    pub executed_at: DateTime<Utc>,
}

// ── Job result (internal) ──────────────────────────────────────────────────

/// The result of executing a single cron job.
#[derive(Debug, Clone)]
pub struct JobResult {
    pub success: bool,
    pub output: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: i64,
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_type_try_from() {
        assert_eq!(JobType::try_from("shell").unwrap(), JobType::Shell);
        assert_eq!(JobType::try_from("agent").unwrap(), JobType::Agent);
        assert!(JobType::try_from("invalid").is_err());
    }

    #[test]
    fn test_session_target_parse() {
        assert_eq!(SessionTarget::parse("main"), SessionTarget::Main);
        assert_eq!(SessionTarget::parse("isolated"), SessionTarget::Isolated);
        assert_eq!(SessionTarget::parse("unknown"), SessionTarget::Isolated);
    }

    #[test]
    fn test_delivery_config_default() {
        let cfg = DeliveryConfig::default();
        assert_eq!(cfg.mode, "none");
        assert!(cfg.best_effort);
    }

    #[test]
    fn test_schedule_summary() {
        let s = Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
        };
        assert!(s.summary().contains("*/5"));

        let s = Schedule::Every { every_ms: 5000 };
        assert!(s.summary().contains("5000"));
    }
}
