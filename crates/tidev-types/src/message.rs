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
    DirectoryReference {
        path: String,
        tree: Arc<String>,
    },
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
    #[serde(default)]
    pub reasoning_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reasoning_completed_at: Option<DateTime<Utc>>,
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
    pub reasoning_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reasoning_completed_at: Option<DateTime<Utc>>,
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
            reasoning_started_at: None,
            reasoning_completed_at: None,
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
            reasoning_started_at: None,
            reasoning_completed_at: None,
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
            reasoning_started_at: None,
            reasoning_completed_at: None,
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
            reasoning_started_at: None,
            reasoning_completed_at: None,
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
    ToolStarting {
        session_id: Uuid,
        request_id: u64,
        tool_call: ToolCall,
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
        tool_call_id: String,
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
    UserMessageCreated {
        session_id: Uuid,
        message: Message,
    },
    UndoCompleted {
        session_id: Uuid,
        target_id: Uuid,
        message_content: String,
    },
    SidebarSnapshotReady {
        session_id: Uuid,
        request_id: u64,
        tool_call_id: String,
        file_diffs_json: String,
    },
    ShellOutput {
        session_id: Uuid,
        tool_call_id: String,
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
        reasoning_started_at: Option<DateTime<Utc>>,
        reasoning_completed_at: Option<DateTime<Utc>>,
    },
    MessagesTruncated {
        session_id: Uuid,
        kept_count: usize,
    },
}

// ---------------------------------------------------------------------------
// QueuedUserMessage
// ---------------------------------------------------------------------------

/// A user message queued while the agent loop is busy.
///
/// When the agent loop finishes its current turn and finds queued messages,
/// it processes them one at a time, continuing the loop instead of exiting.
#[derive(Clone, Debug)]
pub struct QueuedUserMessage {
    pub content: String,
    pub attachments: Vec<MessageAttachment>,
    pub mode: crate::prompts::SessionMode,
    pub thinking_level: Option<crate::reasoning::ThinkingLevelType>,
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
            | Self::ToolStarting { session_id, .. }
            | Self::ToolCompleted { session_id, .. }
            | Self::SubagentStatus { session_id, .. }
            | Self::SubagentCompleted { session_id, .. }
            | Self::UsageStats { session_id, .. }
            | Self::InstructionsLoaded { session_id, .. }
            | Self::ContextCompacted { session_id, .. }
            | Self::UndoCompleted { session_id, .. }
            | Self::UserMessageCreated { session_id, .. }
            | Self::SidebarSnapshotReady { session_id, .. }
            | Self::ShellOutput { session_id, .. }
            | Self::TurnStarting { session_id, .. }
            | Self::StreamEnd { session_id, .. }
            | Self::MessagesTruncated { session_id, .. } => *session_id,
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
            | Self::ToolStarting { request_id, .. }
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
            | Self::UserMessageCreated { .. }
            | Self::ShellOutput { .. }
            | Self::MessagesTruncated { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── MessageAttachment ───────────────────────────────────────────────

    #[test]
    fn is_image_returns_true_for_image_variant() {
        let img = MessageAttachment::Image {
            filename: "x.png".into(),
            mime: "image/png".into(),
            data: vec![0u8; 16],
            file_size: 16,
        };
        assert!(img.is_image());
    }

    #[test]
    fn is_image_returns_false_for_file_reference() {
        let fr = MessageAttachment::FileReference {
            path: "a.rs".into(),
            content: Arc::new("fn main() {}".into()),
            tool_output: None,
            truncated: false,
        };
        assert!(!fr.is_image());
    }

    #[test]
    fn is_image_returns_false_for_directory_reference() {
        let dr = MessageAttachment::DirectoryReference {
            path: "src".into(),
            tree: Arc::new("src/\n  main.rs".into()),
        };
        assert!(!dr.is_image());
    }

    #[test]
    fn prompt_text_file_reference_without_tool_output() {
        let fr = MessageAttachment::FileReference {
            path: "src/main.rs".into(),
            content: Arc::new("let x = 1;".into()),
            tool_output: None,
            truncated: false,
        };
        let text = fr.prompt_text().unwrap();
        assert!(text.contains("Referenced file: src/main.rs"));
        assert!(text.contains("let x = 1;"));
    }

    #[test]
    fn prompt_text_file_reference_with_tool_output() {
        let fr = MessageAttachment::FileReference {
            path: "Cargo.toml".into(),
            content: Arc::new("[package]".into()),
            tool_output: Some(Arc::new("[package]\nname = \"foo\"".into())),
            truncated: false,
        };
        let text = fr.prompt_text().unwrap();
        assert!(text.contains("read")); // tool_name = "read"
        assert!(text.contains("\"path\":\"Cargo.toml\"")); // serialised arg
        assert!(text.contains("name = \"foo\""));
        assert!(!text.contains("truncated")); // truncated_hint is empty
    }

    #[test]
    fn prompt_text_file_reference_with_truncated_output() {
        let fr = MessageAttachment::FileReference {
            path: "big.log".into(),
            content: Arc::new("".into()),
            tool_output: Some(Arc::new("lots of data".into())),
            truncated: true,
        };
        let text = fr.prompt_text().unwrap();
        assert!(text.contains("The tool call succeeded but the output was truncated."));
    }

    #[test]
    fn prompt_text_directory_reference() {
        let dr = MessageAttachment::DirectoryReference {
            path: "src".into(),
            tree: Arc::new("src/\n  lib.rs".into()),
        };
        let text = dr.prompt_text().unwrap();
        assert!(text.contains("Referenced directory: src"));
        assert!(text.contains("lib.rs"));
    }

    #[test]
    fn prompt_text_image_returns_none() {
        let img = MessageAttachment::Image {
            filename: "x.png".into(),
            mime: "image/png".into(),
            data: vec![0u8; 16],
            file_size: 16,
        };
        assert!(img.prompt_text().is_none());
    }

    // ── truncate_preview / tail_preview ─────────────────────────────────

    #[test]
    fn truncate_preview_short_input() {
        assert_eq!(truncate_preview("hello", 100), "hello");
    }

    #[test]
    fn truncate_preview_exact_fit() {
        assert_eq!(truncate_preview("hello", 5), "hello");
    }

    #[test]
    fn truncate_preview_truncates_long_input() {
        let result = truncate_preview("hello world", 5);
        assert_eq!(result, "hello...");
        assert_eq!(result.chars().count(), 8); // 5 + 3 dots
    }

    #[test]
    fn truncate_preview_empty_input() {
        assert_eq!(truncate_preview("", 10), "");
    }

    #[test]
    fn truncate_preview_zero_max() {
        let result = truncate_preview("hello", 0);
        assert_eq!(result, "...");
        assert_eq!(result.chars().count(), 3);
    }

    #[test]
    fn tail_preview_short_input() {
        assert_eq!(tail_preview("hello", 100), "hello");
    }

    #[test]
    fn tail_preview_truncates_long_input() {
        let result = tail_preview("hello world", 5);
        assert_eq!(result, "...world");
        assert_eq!(result.chars().count(), 8); // 3 dots + 5 chars
    }

    #[test]
    fn tail_preview_empty_input() {
        assert_eq!(tail_preview("", 10), "");
    }

    #[test]
    fn tail_preview_zero_max() {
        let result = tail_preview("hello", 0);
        assert_eq!(result, "...");
    }

    #[test]
    fn truncate_preview_multibyte() {
        // "你好世界" is 4 CJK characters (12 bytes, 4 chars)
        let result = truncate_preview("你好世界", 2);
        assert_eq!(result, "你好...");
        assert_eq!(result.chars().count(), 5);
    }

    #[test]
    fn tail_preview_multibyte() {
        let result = tail_preview("你好世界", 2);
        assert_eq!(result, "...世界");
        assert_eq!(result.chars().count(), 5);
    }

    // ── tool_output_preview ─────────────────────────────────────────────

    #[test]
    fn tool_output_preview_short_output() {
        let result = tool_output_preview(Some("read"), "short");
        assert_eq!(result, "short");
    }

    #[test]
    fn tool_output_preview_at_threshold() {
        let s = "a".repeat(8_000);
        let result = tool_output_preview(None, &s);
        assert_eq!(result, s); // not truncated
    }

    #[test]
    fn tool_output_preview_over_threshold() {
        let s = "a".repeat(8_001);
        let result = tool_output_preview(Some("shell"), &s);
        assert!(result.starts_with("[shell output truncated: 8001 chars]"));
        assert!(result.contains("First excerpt:"));
        assert!(result.contains("Last excerpt:"));
    }

    #[test]
    fn tool_output_preview_none_tool_name() {
        let s = "b".repeat(8_001);
        let result = tool_output_preview(None, &s);
        // Falls back to "tool" when tool_name is None
        assert!(result.starts_with("[tool output truncated: 8001 chars]"));
    }

    // ── ToolExecutionResult ─────────────────────────────────────────────

    #[test]
    fn tool_execution_result_new() {
        let r = ToolExecutionResult::new("hello");
        assert_eq!(r.output, "hello");
        assert!(r.attachments.is_empty());
        assert!(r.snapshot_hash.is_none());
    }

    #[test]
    fn preview_for_storage_no_truncation_needed() {
        let r = ToolExecutionResult::new("short");
        let previewed = r.preview_for_storage(Some("read"));
        // No truncation → returns clone, same content
        assert_eq!(previewed.output, "short");
    }

    #[test]
    fn preview_for_storage_with_truncation() {
        let s = "x".repeat(9_000);
        let r = ToolExecutionResult::new(s);
        let previewed = r.preview_for_storage(Some("shell"));
        assert!(
            previewed
                .output
                .starts_with("[shell output truncated: 9000 chars]")
        );
    }

    // ── MessageRole ─────────────────────────────────────────────────────

    #[test]
    fn message_role_from_db_value_known() {
        assert_eq!(MessageRole::from_db_value("system"), MessageRole::System);
        assert_eq!(MessageRole::from_db_value("user"), MessageRole::User);
        assert_eq!(
            MessageRole::from_db_value("assistant"),
            MessageRole::Assistant
        );
        assert_eq!(MessageRole::from_db_value("tool"), MessageRole::Tool);
        assert_eq!(MessageRole::from_db_value("error"), MessageRole::Error);
        assert_eq!(MessageRole::from_db_value("shell"), MessageRole::Shell);
    }

    #[test]
    fn message_role_from_db_value_unknown_falls_back_to_user() {
        assert_eq!(MessageRole::from_db_value("unknown"), MessageRole::User);
        assert_eq!(MessageRole::from_db_value(""), MessageRole::User);
    }

    #[test]
    fn message_role_db_value_roundtrip() {
        for role in &[
            MessageRole::System,
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::Error,
            MessageRole::Shell,
        ] {
            let db = role.db_value();
            let back = MessageRole::from_db_value(db);
            assert_eq!(*role, back);
        }
    }

    // ── AssistantTurn ───────────────────────────────────────────────────

    #[test]
    fn assistant_turn_upsert_tool_call_updates_existing() {
        let mut turn = AssistantTurn::default();
        let tc1 = ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: r#"{"file_path": "a.rs"}"#.into(),
            thought_signature: None,
        };
        turn.upsert_tool_call(tc1.clone());
        assert_eq!(turn.tool_calls.len(), 1);

        let tc2 = ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: r#"{"file_path": "b.rs"}"#.into(),
            thought_signature: None,
        };
        turn.upsert_tool_call(tc2);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].arguments, r#"{"file_path": "b.rs"}"#);
    }

    #[test]
    fn assistant_turn_upsert_tool_call_appends_new() {
        let mut turn = AssistantTurn::default();
        let tc1 = ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        let tc2 = ToolCall {
            id: "call_2".into(),
            name: "write".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        turn.upsert_tool_call(tc1);
        turn.upsert_tool_call(tc2);
        assert_eq!(turn.tool_calls.len(), 2);
    }

    // ── Message ─────────────────────────────────────────────────────────

    #[test]
    fn message_new_creates_correct_role() {
        let msg = Message::new(MessageRole::User, "hello");
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "hello");
        assert!(!msg.streaming);
        assert!(msg.metadata.prior_summary.is_none());
    }

    #[test]
    fn message_compaction_creates_system_message() {
        let msg = Message::compaction("summary text");
        assert_eq!(msg.role, MessageRole::System);
        assert!(msg.content.starts_with("Compaction"));
        assert!(msg.content.contains("summary text"));
    }

    #[test]
    fn message_streaming_has_streaming_flag() {
        let msg = Message::streaming(MessageRole::Assistant, "partial");
        assert!(msg.streaming);
        assert_eq!(msg.content, "partial");
    }

    #[test]
    fn message_persisted_roundtrip() {
        let id = Uuid::new_v4();
        let ts = Utc::now();
        let msg = Message::persisted(id, MessageRole::User, "content", ts, true);
        assert_eq!(msg.id, id);
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "content");
        assert_eq!(msg.created_at, ts);
        assert!(msg.streaming);
    }

    #[test]
    fn message_tool_result_sets_fields() {
        let result = ToolExecutionResult::new("done");
        let msg = Message::tool_result("tc_1", "shell", result);
        assert_eq!(msg.role, MessageRole::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("tc_1"));
        assert_eq!(msg.tool_name.as_deref(), Some("shell"));
        assert_eq!(msg.content, "done");
    }

    #[test]
    fn message_upsert_tool_call_updates_existing() {
        let mut msg = Message::new(MessageRole::Assistant, "");
        let tc1 = ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        msg.upsert_tool_call(tc1);
        let tc2 = ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: r#"{"file_path": "x"}"#.into(),
            thought_signature: None,
        };
        msg.upsert_tool_call(tc2);
        assert_eq!(msg.tool_calls.len(), 1);
        assert_eq!(msg.tool_calls[0].arguments, r#"{"file_path": "x"}"#);
    }

    #[test]
    fn message_upsert_tool_call_appends_new() {
        let mut msg = Message::new(MessageRole::Assistant, "");
        msg.upsert_tool_call(ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        });
        msg.upsert_tool_call(ToolCall {
            id: "c2".into(),
            name: "write".into(),
            arguments: "{}".into(),
            thought_signature: None,
        });
        assert_eq!(msg.tool_calls.len(), 2);
    }

    // ── Message serialisation ───────────────────────────────────────────

    #[test]
    fn message_serde_roundtrip() {
        let msg = Message::new(MessageRole::User, "hello");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.id, deserialized.id);
        assert_eq!(msg.role, deserialized.role);
        assert_eq!(msg.content, deserialized.content);
    }

    #[test]
    fn tool_call_serde_roundtrip() {
        let tc = ToolCall {
            id: "call_abc".into(),
            name: "read".into(),
            arguments: r#"{"file_path":"/tmp/x"}"#.into(),
            thought_signature: Some("sig123".into()),
        };
        let json = serde_json::to_string(&tc).unwrap();
        let back: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(tc, back);
    }

    #[test]
    fn tool_call_serde_thought_signature_optional() {
        // When thought_signature is None it should be skipped in JSON
        let tc = ToolCall {
            id: "c".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        let json = serde_json::to_string(&tc).unwrap();
        assert!(!json.contains("thought_signature"));
        // roundtrip should still work
        let back: ToolCall = serde_json::from_str(&json).unwrap();
        assert!(back.thought_signature.is_none());
    }

    // ── BackendEvent ────────────────────────────────────────────────────

    #[test]
    fn backend_event_session_id_and_request_id() {
        let sid = Uuid::new_v4();
        let events: Vec<BackendEvent> = vec![
            BackendEvent::Delta {
                session_id: sid,
                request_id: 1,
                content: "a".into(),
            },
            BackendEvent::ReasoningDelta {
                session_id: sid,
                request_id: 1,
                content: "b".into(),
            },
            BackendEvent::ToolCallUpdated {
                session_id: sid,
                request_id: 1,
                tool_call: ToolCall::default(),
            },
            BackendEvent::Finished {
                session_id: sid,
                request_id: 1,
                turn: AssistantTurn::default(),
            },
            BackendEvent::Failed {
                session_id: sid,
                request_id: 1,
                error: "err".into(),
            },
            BackendEvent::Retrying {
                session_id: sid,
                request_id: 1,
                attempt: 1,
                max_attempts: 3,
                reason: "timeout".into(),
                retry_after_secs: None,
            },
            BackendEvent::InstructionsLoaded {
                session_id: sid,
                sources: vec!["AGENTS.md".into()],
            },
            BackendEvent::ToolStarting {
                session_id: sid,
                request_id: 1,
                tool_call: ToolCall::default(),
            },
            BackendEvent::ToolCompleted {
                session_id: sid,
                request_id: 1,
                tool_call: ToolCall::default(),
                result: ToolExecutionResult::new("ok"),
            },
            BackendEvent::SubagentStatus {
                session_id: sid,
                request_id: 1,
                tool_call_id: "t1".into(),
                child_session_id: Uuid::new_v4(),
                status_text: "running".into(),
                current_tool_call: None,
                assistant_message: None,
                content_delta: None,
                reasoning_delta: None,
            },
            BackendEvent::SubagentCompleted {
                session_id: sid,
                request_id: 1,
                tool_call: ToolCall::default(),
                child_session_id: Uuid::new_v4(),
                result: ToolExecutionResult::new("ok"),
            },
            BackendEvent::UsageStats {
                session_id: sid,
                request_id: 1,
                input_tokens: 10,
                output_tokens: 20,
                total_tokens: 30,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                model_id: "gpt-4".into(),
                duration_ms: None,
            },
            BackendEvent::ContextCompacted {
                session_id: sid,
                compacted: true,
                manual: false,
                summary: Some("sum".into()),
                retained_from: 5,
                model_id: None,
                completed_at: None,
                error: None,
            },
            BackendEvent::UserMessageCreated {
                session_id: sid,
                message: Message::new(MessageRole::User, "hi"),
            },
            BackendEvent::UndoCompleted {
                session_id: sid,
                target_id: Uuid::new_v4(),
                message_content: "restored".into(),
            },
            BackendEvent::SidebarSnapshotReady {
                session_id: sid,
                request_id: 1,
                tool_call_id: "t1".into(),
                file_diffs_json: "[]".into(),
            },
            BackendEvent::ShellOutput {
                session_id: sid,
                tool_call_id: "t1".into(),
                content: "out".into(),
                finished: false,
                exit_code: None,
            },
            BackendEvent::TurnStarting {
                session_id: sid,
                request_id: 1,
            },
            BackendEvent::StreamEnd {
                session_id: sid,
                request_id: 1,
                reasoning_started_at: None,
                reasoning_completed_at: None,
            },
            BackendEvent::MessagesTruncated {
                session_id: sid,
                kept_count: 10,
            },
        ];
        for ev in &events {
            assert_eq!(ev.session_id(), sid, "session_id mismatch for {ev:?}");
        }
    }

    #[test]
    fn backend_event_request_id_present() {
        let sid = Uuid::new_v4();
        assert_eq!(
            BackendEvent::Delta {
                session_id: sid,
                request_id: 42,
                content: "a".into()
            }
            .request_id(),
            Some(42)
        );
    }

    #[test]
    fn backend_event_request_id_absent() {
        let sid = Uuid::new_v4();
        assert_eq!(
            BackendEvent::InstructionsLoaded {
                session_id: sid,
                sources: vec![]
            }
            .request_id(),
            None
        );
        assert_eq!(
            BackendEvent::ContextCompacted {
                session_id: sid,
                compacted: true,
                manual: false,
                summary: None,
                retained_from: 0,
                model_id: None,
                completed_at: None,
                error: None
            }
            .request_id(),
            None
        );
        assert_eq!(
            BackendEvent::UndoCompleted {
                session_id: sid,
                target_id: Uuid::new_v4(),
                message_content: "".into()
            }
            .request_id(),
            None
        );
        assert_eq!(
            BackendEvent::UserMessageCreated {
                session_id: sid,
                message: Message::new(MessageRole::User, "")
            }
            .request_id(),
            None
        );
        assert_eq!(
            BackendEvent::ShellOutput {
                session_id: sid,
                tool_call_id: "t1".into(),
                content: "".into(),
                finished: false,
                exit_code: None
            }
            .request_id(),
            None
        );
        assert_eq!(
            BackendEvent::MessagesTruncated {
                session_id: sid,
                kept_count: 0
            }
            .request_id(),
            None
        );
    }

    // ── Serialisation: MessageAttachment tagged enum ────────────────────

    #[test]
    fn message_attachment_file_reference_json_tag() {
        let fr = MessageAttachment::FileReference {
            path: "f".into(),
            content: Arc::new("c".into()),
            tool_output: None,
            truncated: false,
        };
        let json = serde_json::to_value(&fr).unwrap();
        assert_eq!(json["type"], "file_reference");
    }

    #[test]
    fn message_attachment_directory_reference_json_tag() {
        let dr = MessageAttachment::DirectoryReference {
            path: "d".into(),
            tree: Arc::new("t".into()),
        };
        let json = serde_json::to_value(&dr).unwrap();
        assert_eq!(json["type"], "directory_reference");
    }

    #[test]
    fn message_attachment_image_json_tag() {
        let img = MessageAttachment::Image {
            filename: "x.png".into(),
            mime: "image/png".into(),
            data: vec![1, 2, 3],
            file_size: 3,
        };
        let json = serde_json::to_value(&img).unwrap();
        assert_eq!(json["type"], "image");
    }
}
