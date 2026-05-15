use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Existing Memory Types (enhanced) ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Project,
    Feedback,
    Reference,
    Pattern,
    Preference,
    Architecture,
    Bug,
    Workflow,
    Fact,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Feedback => "feedback",
            Self::Reference => "reference",
            Self::Pattern => "pattern",
            Self::Preference => "preference",
            Self::Architecture => "architecture",
            Self::Bug => "bug",
            Self::Workflow => "workflow",
            Self::Fact => "fact",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            "feedback" => Some(Self::Feedback),
            "reference" => Some(Self::Reference),
            "pattern" => Some(Self::Pattern),
            "preference" => Some(Self::Preference),
            "architecture" => Some(Self::Architecture),
            "bug" => Some(Self::Bug),
            "workflow" => Some(Self::Workflow),
            "fact" => Some(Self::Fact),
            _ => None,
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::User => "usr",
            Self::Project => "proj",
            Self::Feedback => "feed",
            Self::Reference => "ref",
            Self::Pattern => "pat",
            Self::Preference => "pref",
            Self::Architecture => "arch",
            Self::Bug => "bug",
            Self::Workflow => "flow",
            Self::Fact => "fact",
        }
    }
}

/// Enhanced memory entry — backward-compatible with old fields,
/// plus agentmemory-style fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub workspace_root: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source_session_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub usage_count: i64,
    pub active: bool,
    // New agentmemory-style fields
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub strength: f64,
    pub importance: u8,
    pub version: i64,
    pub parent_id: Option<Uuid>,
    pub supersedes: Vec<Uuid>,
    pub related_ids: Vec<Uuid>,
    pub is_latest: bool,
}

impl MemoryEntry {
    /// Create a new MemoryEntry with sensible defaults for new fields.
    pub fn new(
        id: Uuid,
        workspace_root: String,
        memory_type: MemoryType,
        title: String,
        content: String,
        tags: Vec<String>,
        source_session_id: Option<Uuid>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        usage_count: i64,
        active: bool,
    ) -> Self {
        Self {
            id, workspace_root, memory_type, title, content, tags,
            source_session_id, created_at, updated_at, usage_count, active,
            concepts: vec![],
            files: vec![],
            strength: 0.0,
            importance: 5,
            version: 1,
            parent_id: None,
            supersedes: vec![],
            related_ids: vec![],
            is_latest: true,
        }
    }
}

// ─── AgentMemory Data Model ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookType {
    SessionStart,
    PromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolFailure,
    PreCompact,
    SubagentStart,
    SubagentStop,
    Notification,
    TaskCompleted,
    Stop,
    SessionEnd,
}

impl HookType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::PromptSubmit => "prompt_submit",
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::PostToolFailure => "post_tool_failure",
            Self::PreCompact => "pre_compact",
            Self::SubagentStart => "subagent_start",
            Self::SubagentStop => "subagent_stop",
            Self::Notification => "notification",
            Self::TaskCompleted => "task_completed",
            Self::Stop => "stop",
            Self::SessionEnd => "session_end",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "session_start" => Some(Self::SessionStart),
            "prompt_submit" => Some(Self::PromptSubmit),
            "pre_tool_use" => Some(Self::PreToolUse),
            "post_tool_use" => Some(Self::PostToolUse),
            "post_tool_failure" => Some(Self::PostToolFailure),
            "pre_compact" => Some(Self::PreCompact),
            "subagent_start" => Some(Self::SubagentStart),
            "subagent_stop" => Some(Self::SubagentStop),
            "notification" => Some(Self::Notification),
            "task_completed" => Some(Self::TaskCompleted),
            "stop" => Some(Self::Stop),
            "session_end" => Some(Self::SessionEnd),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationType {
    FileRead,
    FileWrite,
    FileEdit,
    CommandRun,
    Search,
    WebFetch,
    Conversation,
    Error,
    Decision,
    Discovery,
    Subagent,
    Notification,
    Task,
    Image,
    Other,
}

impl ObservationType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::FileEdit => "file_edit",
            Self::CommandRun => "command_run",
            Self::Search => "search",
            Self::WebFetch => "web_fetch",
            Self::Conversation => "conversation",
            Self::Error => "error",
            Self::Decision => "decision",
            Self::Discovery => "discovery",
            Self::Subagent => "subagent",
            Self::Notification => "notification",
            Self::Task => "task",
            Self::Image => "image",
            Self::Other => "other",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "file_read" => Some(Self::FileRead),
            "file_write" => Some(Self::FileWrite),
            "file_edit" => Some(Self::FileEdit),
            "command_run" => Some(Self::CommandRun),
            "search" => Some(Self::Search),
            "web_fetch" => Some(Self::WebFetch),
            "conversation" => Some(Self::Conversation),
            "error" => Some(Self::Error),
            "decision" => Some(Self::Decision),
            "discovery" => Some(Self::Discovery),
            "subagent" => Some(Self::Subagent),
            "notification" => Some(Self::Notification),
            "task" => Some(Self::Task),
            "image" => Some(Self::Image),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Mixed,
}

/// Hook payload — received from HookEngine on tool use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPayload {
    pub session_id: Uuid,
    pub hook_type: HookType,
    pub timestamp: DateTime<Utc>,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub tool_output: Option<String>,
    pub user_prompt: Option<String>,
    pub assistant_response: Option<String>,
}

/// Raw observation — stored before LLM compression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawObservation {
    pub id: Uuid,
    pub session_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub hook_type: HookType,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub tool_output: Option<String>,
    pub user_prompt: Option<String>,
    pub assistant_response: Option<String>,
    pub modality: Modality,
    pub image_data: Option<String>,
}

/// Compressed observation — after LLM compression.
/// Same id as the raw observation (observe → compress is in-place).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedObservation {
    pub id: Uuid,
    pub session_id: Uuid,
    pub obs_type: ObservationType,
    pub title: String,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub importance: u8,
    pub confidence: Option<f64>,
    pub created_at: DateTime<Utc>,
}

impl CompressedObservation {
    /// Text to index in BM25.
    pub fn to_search_text(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.title,
            self.narrative,
            self.facts.join(" "),
            self.concepts.join(" "),
            self.files.join(" ")
        )
    }
}

/// Session summary — LLM-generated session digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: Uuid,
    pub project: String,
    pub created_at: DateTime<Utc>,
    pub title: Option<String>,
    pub narrative: Option<String>,
    pub key_decisions: Vec<String>,
    pub files_modified: Vec<String>,
    pub concepts: Vec<String>,
    pub observation_count: i64,
}

/// Audit entry — immutable operation log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub operation: String,
    pub entity_type: String,
    pub entity_id: String,
    pub actor: Option<String>,
    pub details: Option<serde_json::Value>,
    pub session_id: Option<Uuid>,
}

/// Search result — from BM25 / FTS5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: Uuid,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    pub source: SearchSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchSource {
    Observation,
    Memory,
}

/// Observation result — returned from observe().
#[derive(Debug, Clone)]
pub enum ObservationResult {
    New(Uuid),
    Deduplicated,
}

// ─── Phase 2 Types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySlot {
    pub label: String,
    pub content: String,
    pub size_limit: usize,
    pub description: String,
    pub pinned: bool,
    pub read_only: bool,
    pub scope: SlotScope,
    pub project: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotScope {
    Global,
    Project,
}

impl SlotScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "global" => Some(Self::Global),
            "project" => Some(Self::Project),
            _ => None,
        }
    }
}

/// Embedding provider trait.
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}

/// Hybrid search result with score components.
#[derive(Debug, Clone)]
pub struct HybridSearchResult {
    pub id: String,
    pub combined_score: f64,
    pub bm25_score: Option<f64>,
    pub vector_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionScore {
    pub entity_id: String,
    pub entity_type: String,
    pub importance: f64,
    pub access_frequency: f64,
    pub age_days: f64,
    pub score: f64,
    pub computed_at: DateTime<Utc>,
}
