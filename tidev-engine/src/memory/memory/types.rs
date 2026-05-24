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
    Lesson,
    Insight,
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
            Self::Lesson => "lesson",
            Self::Insight => "insight",
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
            "lesson" => Some(Self::Lesson),
            "insight" => Some(Self::Insight),
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
            Self::Lesson => "less",
            Self::Insight => "insight",
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
    #[allow(clippy::too_many_arguments)]
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
            id,
            workspace_root,
            memory_type,
            title,
            content,
            tags,
            source_session_id,
            created_at,
            updated_at,
            usage_count,
            active,
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
