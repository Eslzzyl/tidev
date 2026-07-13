//! Core data model — messages, tool calls, and backend events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// MessageAttachment
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAttachment {
    /// File reference with optional truncated tool output (for @ references).
    FileReference {
        path: String,
        content: Arc<String>,
        #[serde(default)]
        tool_output: Option<Arc<String>>,
        #[serde(default)]
        truncated: bool,
    },
    DirectoryReference { path: String, tree: Arc<String> },
    Image {
        filename: String,
        mime: String,
        data: Vec<u8>,
        file_size: u64,
    },
}

impl MessageAttachment {
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }

    /// Returns the prompt text in tool call result format.
    pub fn prompt_text(&self) -> Option<String> {
        match self {
            Self::FileReference {
                path,
                content,
                tool_output,
                truncated,
            } => {
                if let Some(output) = tool_output {
                    let truncated_hint = if *truncated {
                        "\n\n(The tool call succeeded but the output was truncated.)"
                    } else {
                        ""
                    };
                    return Some(format!(
                        "\n\n{tool_name} Tool: read\n{{\"path\":\"{args}\"}}\n\nOutput:\n{output}{truncated_hint}",
                        tool_name = "read",
                        args = path,
                        output = output.as_ref(),
                        truncated_hint = truncated_hint
                    ));
                }
                Some(format!(
                    "\n\nReferenced file: {}\n```text\n{}\n```",
                    path, content
                ))
            }
            Self::DirectoryReference { path, tree } => Some(format!(
                "\n\nReferenced directory: {}\n```text\n{}\n```",
                path, tree
            )),
            Self::Image { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}

fn tail_preview(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut shortened = value.chars().rev().take(max_chars).collect::<String>();
    shortened = shortened.chars().rev().collect();
    let mut preview = String::from("...");
    preview.push_str(&shortened);
    preview
}

pub fn tool_output_preview(tool_name: Option<&str>, output: &str) -> String {
    let output_char_count = output.chars().count();
    if output_char_count <= 8_000 {
        return output.to_string();
    }

    let tool_name = tool_name.unwrap_or("tool");
    let head = truncate_preview(output, 3_000);
    let tail = tail_preview(output, 1_000);

    format!(
        "[{tool_name} output truncated: {output_char_count} chars]\n\nFirst excerpt:\n{head}\n\nLast excerpt:\n{tail}"
    )
}

// ---------------------------------------------------------------------------
// ToolMetadata & FileChangeInfo
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeInfo {
    pub path: String,
    #[serde(default)]
    pub diff: Option<String>,
    pub operation: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolMetadata {
    #[serde(default)]
    pub filepath: Option<String>,
    #[serde(default)]
    pub diff: Option<String>,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub exists: Option<bool>,
    #[serde(default)]
    pub prior_summary: Option<String>,
    #[serde(default)]
    pub prior_retained_from: Option<usize>,
    #[serde(default)]
    pub child_session_id: Option<Uuid>,
    #[serde(default)]
    pub file_changes: Vec<FileChangeInfo>,
}

// ---------------------------------------------------------------------------
// ToolExecutionResult
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionResult {
    pub output: String,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default)]
    pub metadata: ToolMetadata,
    #[serde(default)]
    pub instruction_sources: Vec<String>,
    #[serde(default)]
    pub snapshot_hash: Option<String>,
    #[serde(default)]
    pub patch_files: Option<String>,
}

impl ToolExecutionResult {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            attachments: Vec::new(),
            metadata: ToolMetadata::default(),
            instruction_sources: Vec::new(),
            snapshot_hash: None,
            patch_files: None,
        }
    }

    pub fn preview_for_storage(&self, tool_name: Option<&str>) -> Self {
        let output = tool_output_preview(tool_name, &self.output);
        if output == self.output {
            return self.clone();
        }
        Self {
            output,
            attachments: self.attachments.clone(),
            metadata: self.metadata.clone(),
            instruction_sources: self.instruction_sources.clone(),
            snapshot_hash: self.snapshot_hash.clone(),
            patch_files: self.patch_files.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// MessageRole
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
    Error,
    Shell,
}

impl MessageRole {
    pub fn label(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Error => "error",
            Self::Shell => "shell",
        }
    }

    /// Database storage value.
    pub fn db_value(&self) -> &'static str {
        self.label()
    }

    /// Parse from a database storage value.
    pub fn from_db_value(value: &str) -> Self {
        match value {
            "system" => Self::System,
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "error" => Self::Error,
            "shell" => Self::Shell,
            _ => Self::User,
        }
    }
}

// ---------------------------------------------------------------------------
// ToolCall
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    /// Opaque signature from Gemini thought/reasoning that must be echoed back
    /// in subsequent conversation turns (required for Gemini 3+ models).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

// ---------------------------------------------------------------------------
// AssistantTurn
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AssistantTurn {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_tokens: Option<u32>,
    #[serde(default)]
    pub cache_write_tokens: Option<u32>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub tokens_per_second: Option<f32>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
}

impl AssistantTurn {
    pub fn upsert_tool_call(&mut self, tool_call: ToolCall) {
        if let Some(existing) = self
            .tool_calls
            .iter_mut()
            .find(|existing| existing.id == tool_call.id)
        {
            *existing = tool_call;
        } else {
            self.tool_calls.push(tool_call);
        }
    }
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

pub const COMPACTION_MESSAGE_LABEL: &str = "Compaction";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub metadata: ToolMetadata,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub streaming: bool,
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_tokens: Option<u32>,
    #[serde(default)]
    pub cache_write_tokens: Option<u32>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub tokens_per_second: Option<f32>,
    #[serde(default)]
    pub snapshot_hash: Option<String>,
    #[serde(default)]
    pub patch_files: Option<String>,
    #[serde(default)]
    pub file_diffs: Option<String>,
    #[serde(default)]
    pub mode: Option<crate::prompts::SessionMode>,
    #[serde(default)]
    pub thinking_level: Option<crate::reasoning::ThinkingLevelType>,
}

impl Message {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: content.into(),
            attachments: Vec::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            metadata: ToolMetadata::default(),
            created_at: Utc::now(),
            completed_at: None,
            streaming: false,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            model_id: None,
            tokens_per_second: None,
            snapshot_hash: None,
            patch_files: None,
            file_diffs: None,
            mode: None,
            thinking_level: None,
        }
    }

    pub fn compaction(summary: impl Into<String>) -> Self {
        Self::new(
            MessageRole::System,
            format!("{COMPACTION_MESSAGE_LABEL}\n\n{}", summary.into()),
        )
    }

    /// Create a streaming message (role + content, streaming = true).
    pub fn streaming(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: content.into(),
            attachments: Vec::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            metadata: ToolMetadata::default(),
            created_at: Utc::now(),
            completed_at: None,
            streaming: true,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            model_id: None,
            tokens_per_second: None,
            snapshot_hash: None,
            patch_files: None,
            file_diffs: None,
            mode: None,
            thinking_level: None,
        }
    }

    pub fn persisted(
        id: Uuid,
        role: MessageRole,
        content: impl Into<String>,
        created_at: DateTime<Utc>,
        streaming: bool,
    ) -> Self {
        Self {
            id,
            role,
            content: content.into(),
            attachments: Vec::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            metadata: ToolMetadata::default(),
            created_at,
            completed_at: None,
            streaming,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            model_id: None,
            tokens_per_second: None,
            snapshot_hash: None,
            patch_files: None,
            file_diffs: None,
            mode: None,
            thinking_level: None,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        result: ToolExecutionResult,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: MessageRole::Tool,
            content: result.output,
            attachments: result.attachments,
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
            metadata: result.metadata,
            created_at: Utc::now(),
            completed_at: None,
            streaming: false,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            model_id: None,
            tokens_per_second: None,
            snapshot_hash: result.snapshot_hash,
            patch_files: result.patch_files,
            file_diffs: None,
            mode: None,
            thinking_level: None,
        }
    }

    pub fn upsert_tool_call(&mut self, tool_call: ToolCall) {
        if let Some(existing) = self
            .tool_calls
            .iter_mut()
            .find(|existing| existing.id == tool_call.id)
        {
            *existing = tool_call;
        } else {
            self.tool_calls.push(tool_call);
        }
    }
}

// ---------------------------------------------------------------------------
// BackendEvent
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum BackendEvent {
    Delta {
        session_id: Uuid,
        request_id: u64,
        content: String,
    },
    ReasoningDelta {
        session_id: Uuid,
        request_id: u64,
        content: String,
    },
    ToolCallUpdated {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
    },
    Finished {
        session_id: Uuid,
        request_id: u64,
        turn: AssistantTurn,
    },
    Failed {
        session_id: Uuid,
        request_id: u64,
        error: String,
    },
    Retrying {
        session_id: Uuid,
        request_id: u64,
        attempt: u32,
        max_attempts: u32,
        reason: String,
        retry_after_secs: Option<u32>,
    },
    InstructionsLoaded {
        session_id: Uuid,
        sources: Vec<String>,
    },
    ToolCompleted {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
        result: ToolExecutionResult,
    },
    SubagentStatus {
        session_id: Uuid,
        request_id: u64,
        child_session_id: Uuid,
        status_text: String,
        current_tool_call: Option<ToolCall>,
        assistant_message: Option<Message>,
        content_delta: Option<String>,
        reasoning_delta: Option<String>,
    },
    SubagentCompleted {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
        child_session_id: Uuid,
        result: ToolExecutionResult,
    },
    UsageStats {
        session_id: Uuid,
        request_id: u64,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
        model_id: String,
        duration_ms: Option<u64>,
    },
    ContextCompacted {
        session_id: Uuid,
        compacted: bool,
        manual: bool,
        summary: Option<String>,
        retained_from: usize,
        model_id: Option<String>,
        completed_at: Option<DateTime<Utc>>,
        error: Option<String>,
    },
    UndoCompleted {
        session_id: Uuid,
        target_id: Uuid,
        message_content: String,
    },
    SidebarSnapshotReady {
        session_id: Uuid,
        request_id: u64,
        message_id: Uuid,
        file_diffs_json: String,
    },
    ShellOutput {
        session_id: Uuid,
        content: String,
        finished: bool,
        exit_code: Option<i32>,
    },
    TurnStarting {
        session_id: Uuid,
        request_id: u64,
    },
    StreamEnd {
        session_id: Uuid,
        request_id: u64,
    },
}

impl BackendEvent {
    pub fn session_id(&self) -> Uuid {
        match self {
            Self::Delta { session_id, .. }
            | Self::ReasoningDelta { session_id, .. }
            | Self::ToolCallUpdated { session_id, .. }
            | Self::Finished { session_id, .. }
            | Self::Failed { session_id, .. }
            | Self::Retrying { session_id, .. }
            | Self::ToolCompleted { session_id, .. }
            | Self::SubagentStatus { session_id, .. }
            | Self::SubagentCompleted { session_id, .. }
            | Self::UsageStats { session_id, .. }
            | Self::InstructionsLoaded { session_id, .. }
            | Self::ContextCompacted { session_id, .. }
            | Self::UndoCompleted { session_id, .. }
            | Self::SidebarSnapshotReady { session_id, .. }
            | Self::ShellOutput { session_id, .. }
            | Self::TurnStarting { session_id, .. }
            | Self::StreamEnd { session_id, .. } => *session_id,
        }
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Delta { request_id, .. }
            | Self::ReasoningDelta { request_id, .. }
            | Self::ToolCallUpdated { request_id, .. }
            | Self::Finished { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Retrying { request_id, .. }
            | Self::ToolCompleted { request_id, .. }
            | Self::SubagentStatus { request_id, .. }
            | Self::SubagentCompleted { request_id, .. }
            | Self::UsageStats { request_id, .. }
            | Self::SidebarSnapshotReady { request_id, .. }
            | Self::TurnStarting { request_id, .. }
            | Self::StreamEnd { request_id, .. } => Some(*request_id),
            Self::InstructionsLoaded { .. }
            | Self::ContextCompacted { .. }
            | Self::UndoCompleted { .. }
            | Self::ShellOutput { .. } => None,
        }
    }
}
