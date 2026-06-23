use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAttachment {
    /// File reference with optional truncated tool output (for @ references)
    FileReference {
        path: String,
        /// Original full content (for display purposes)
        content: Arc<String>,
        /// Tool output format with truncation applied (like read tool results)
        tool_output: Option<Arc<String>>,
        /// Whether the content was truncated
        truncated: bool,
    },
    DirectoryReference {
        path: String,
        tree: Arc<String>,
    },
    Image {
        filename: String,
        mime: String,
        data_url: String,
        /// Exact file size in bytes (from `fs::read().len()`).
        file_size: u64,
    },
}

impl MessageAttachment {
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }

    pub fn summary(&self) -> String {
        match self {
            Self::FileReference {
                path,
                content,
                truncated,
                ..
            } => {
                let preview = content.lines().take(6).collect::<Vec<_>>().join(" ");
                let truncated_hint = if *truncated { " (truncated)" } else { "" };
                if preview.trim().is_empty() {
                    format!("[file:{}{}]", path, truncated_hint)
                } else {
                    format!(
                        "[file:{}{}] {}",
                        path,
                        truncated_hint,
                        truncate_preview(&preview, 120)
                    )
                }
            }
            Self::DirectoryReference { path, tree } => {
                let preview = tree.lines().take(8).collect::<Vec<_>>().join(" ");
                if preview.trim().is_empty() {
                    format!("[dir:{}]", path)
                } else {
                    format!("[dir:{}] {}", path, truncate_preview(&preview, 120))
                }
            }
            Self::Image { filename, mime, .. } => format!("[image:{} {}]", filename, mime),
        }
    }

    /// Returns the prompt text in tool call result format (like opencode).
    /// For FileReference with tool_output, returns read tool result format.
    /// For FileReference without tool_output, returns original format.
    pub fn prompt_text(&self) -> Option<String> {
        match self {
            Self::FileReference {
                path,
                content,
                tool_output,
                truncated,
            } => {
                // If we have tool output (from @ reference), return in tool result format
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
                // Fall back to original format
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

/// Describes a single file change within an `apply_patch` tool result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeInfo {
    /// Workspace-relative file path.
    pub path: String,
    /// Unified diff text for this file, if available (absent for deletions).
    #[serde(default)]
    pub diff: Option<String>,
    /// Operation type: "A" (added), "M" (modified), "D" (deleted).
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
    /// Context state before the compaction that produced this message.
    /// Used by undo to restore the conversation to its pre-compact state.
    #[serde(default)]
    pub prior_summary: Option<String>,
    #[serde(default)]
    pub prior_retained_from: Option<usize>,
    /// The child session ID spawned by a subagent (task) tool call.
    /// Persisted so click-to-enter-subsession works after restart.
    #[serde(default)]
    pub child_session_id: Option<Uuid>,
    /// Per-file change details for `apply_patch` results.
    /// Preserves insertion order from the patch and allows interleaved
    /// file-path + diff rendering in the TUI.
    #[serde(default)]
    pub file_changes: Vec<FileChangeInfo>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionResult {
    pub output: String,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default)]
    pub metadata: ToolMetadata,
    #[serde(default)]
    pub instruction_sources: Vec<String>,
}

impl ToolExecutionResult {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            attachments: Vec::new(),
            metadata: ToolMetadata::default(),
            instruction_sources: Vec::new(),
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
        }
    }
}

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

    pub fn db_value(&self) -> &'static str {
        self.label()
    }

    pub fn from_db_value(value: &str) -> Self {
        match value {
            "system" => Self::System,
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "error" => Self::Error,
            "shell" => Self::Shell,
            _ => Self::System,
        }
    }
}

pub const COMPACTION_MESSAGE_LABEL: &str = "Compaction";

/// Find the prior context state stored on the first compaction message
/// whose content starts with [`COMPACTION_MESSAGE_LABEL`] in the range
/// after `revert_to_id`.  Returns `(prior_summary, prior_retained_from)`
/// if such a compaction message carries prior state in its metadata.
///
/// Used by undo to restore the context to its pre-compaction state.
pub fn find_compaction_prior_state(
    messages: &[Message],
    revert_to_id: Uuid,
) -> Option<(Option<String>, usize)> {
    let mut found_target = false;
    for message in messages {
        if message.id == revert_to_id {
            found_target = true;
            continue;
        }
        if !found_target {
            continue;
        }
        if message.content.starts_with(COMPACTION_MESSAGE_LABEL)
            && let Some(prior_retained_from) = message.metadata.prior_retained_from
        {
            return Some((message.metadata.prior_summary.clone(), prior_retained_from));
        }
    }
    None
}

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
    /// Token usage captured from UsageStats event during streaming.
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
    /// When the first stream delta was received (start of turn).
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// When the turn finished (after usage stats).
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
    pub mode: Option<tidev_types::prompts::SessionMode>,
    #[serde(default)]
    pub thinking_level: Option<tidev_types::reasoning::ThinkingLevelType>,
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
            snapshot_hash: None,
            patch_files: None,
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

    /// Get token usage from this message's token fields.
    pub fn token_usage(&self) -> crate::utils::TokenUsage {
        crate::utils::TokenUsage::new(
            self.input_tokens.unwrap_or(0),
            self.output_tokens.unwrap_or(0),
            self.cache_read_tokens.unwrap_or(0),
            self.cache_write_tokens.unwrap_or(0),
        )
    }
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub session_id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub workspace_root: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub context_summary: Option<String>,
    pub context_retained_from: usize,
    pub messages: Vec<Message>,
    pub revert_message_id: Option<Uuid>,
}

impl Conversation {
    pub fn new(
        session_id: Uuid,
        workspace_root: impl Into<String>,
        provider_id: impl Into<String>,
        provider_display_name: impl Into<String>,
        model_id: impl Into<String>,
        model_display_name: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            parent_session_id: None,
            workspace_root: workspace_root.into(),
            provider_id: provider_id.into(),
            provider_display_name: provider_display_name.into(),
            model_id: model_id.into(),
            model_display_name: model_display_name.into(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            context_summary: None,
            context_retained_from: 0,
            messages: Vec::new(),
            revert_message_id: None,
        }
    }

    pub fn set_context_state(&mut self, summary: Option<String>, retained_from: usize) {
        self.context_summary = summary;
        self.context_retained_from = retained_from;
    }

    pub fn clear_context_state(&mut self) {
        self.set_context_state(None, 0);
    }

    pub fn set_model(
        &mut self,
        provider_id: impl Into<String>,
        provider_display_name: impl Into<String>,
        model_id: impl Into<String>,
        model_display_name: impl Into<String>,
    ) {
        self.provider_id = provider_id.into();
        self.provider_display_name = provider_display_name.into();
        self.model_id = model_id.into();
        self.model_display_name = model_display_name.into();
    }

    pub fn model_label(&self) -> String {
        format!(
            "{} / {}",
            self.provider_display_name, self.model_display_name
        )
    }

    pub fn push(&mut self, message: Message) {
        self.updated_at = Utc::now();
        self.messages.push(message);
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.revert_message_id = None;
        self.clear_context_state();
        self.updated_at = Utc::now();
    }

    pub fn message_index(&self, message_id: Uuid) -> Option<usize> {
        self.messages
            .iter()
            .position(|message| message.id == message_id)
    }

    pub fn visible_message_count(&self) -> usize {
        self.revert_message_id
            .and_then(|message_id| self.message_index(message_id))
            .unwrap_or(self.messages.len())
    }

    pub fn visible_messages(&self) -> &[Message] {
        let visible_count = self.visible_message_count();
        &self.messages[..visible_count]
    }

    pub fn take_hidden_messages(&mut self) -> Vec<Message> {
        let visible_count = self.visible_message_count();
        if visible_count >= self.messages.len() {
            return Vec::new();
        }

        self.messages.split_off(visible_count)
    }

    pub fn is_reverted(&self) -> bool {
        self.revert_message_id.is_some()
    }

    pub fn last_visible_user_message(&self) -> Option<&Message> {
        self.visible_messages()
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::User))
    }

    pub fn prev_user_message_before(&self, message_id: Uuid) -> Option<&Message> {
        let end_index = self.message_index(message_id)?;
        self.messages
            .iter()
            .take(end_index)
            .rev()
            .find(|message| matches!(message.role, MessageRole::User))
    }

    pub fn next_user_message_after(&self, message_id: Uuid) -> Option<&Message> {
        let start_index = self.message_index(message_id)?;
        self.messages
            .iter()
            .skip(start_index.saturating_add(1))
            .find(|message| matches!(message.role, MessageRole::User))
    }

    pub fn title_from_prompt(prompt: &str) -> String {
        let first_line = prompt.lines().next().unwrap_or("Untitled session").trim();

        if first_line.is_empty() {
            return "Untitled session".to_string();
        }

        let mut title = first_line.chars().take(48).collect::<String>();
        if first_line.chars().count() > 48 {
            title.push_str("...");
        }

        title
    }

    pub fn update_title_from_prompt(&mut self, prompt: &str) {
        self.title = Self::title_from_prompt(prompt);
    }
}

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
    SubagentToolResult {
        session_id: Uuid,
        request_id: u64,
        child_session_id: Uuid,
        message: Message,
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
        error: Option<String>,
    },
    /// Async sidebar snapshot computation completed.
    /// Contains full FileDiff data (with patches) computed off the main thread.
    SidebarSnapshotReady {
        session_id: Uuid,
        request_id: u64,
        message_id: Uuid,
        file_diffs_json: String,
    },
    /// Streaming output from a shell command execution.
    ShellOutput {
        session_id: Uuid,
        content: String,
        finished: bool,
        exit_code: Option<i32>,
    },
    /// Emitted by the agent loop when a new turn starts (after executing
    /// tool calls from the previous turn).  Frontends should use this to
    /// update their active request ID and create a new streaming message.
    TurnStarting { session_id: Uuid, request_id: u64 },
    /// Emitted when the agent loop has fully completed all turns and is
    /// waiting for user input.  Frontends should use this to signal that
    /// the stream is done (e.g. show per-round stats).
    StreamEnd { session_id: Uuid, request_id: u64 },
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
            | Self::SubagentToolResult { session_id, .. }
            | Self::SubagentCompleted { session_id, .. }
            | Self::UsageStats { session_id, .. }
            | Self::InstructionsLoaded { session_id, .. }
            | Self::ContextCompacted { session_id, .. }
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
            | Self::SubagentToolResult { request_id, .. }
            | Self::SubagentCompleted { request_id, .. }
            | Self::UsageStats { request_id, .. }
            | Self::SidebarSnapshotReady { request_id, .. }
            | Self::TurnStarting { request_id, .. }
            | Self::StreamEnd { request_id, .. } => Some(*request_id),
            Self::InstructionsLoaded { .. }
            | Self::ContextCompacted { .. }
            | Self::ShellOutput { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_messages_stop_at_revert_marker() {
        let mut conversation = Conversation::new(
            Uuid::new_v4(),
            "/workspace",
            "provider",
            "Provider",
            "model",
            "Model",
            "Untitled session",
        );

        let first_user = Message::new(MessageRole::User, "first prompt");
        let first_assistant = Message::new(MessageRole::Assistant, "first answer");
        let second_user = Message::new(MessageRole::User, "second prompt");
        let second_assistant = Message::new(MessageRole::Assistant, "second answer");

        conversation.push(first_user.clone());
        conversation.push(first_assistant);
        conversation.push(second_user.clone());
        conversation.push(second_assistant);
        conversation.revert_message_id = Some(second_user.id);

        assert_eq!(conversation.visible_messages().len(), 2);
        assert_eq!(
            conversation
                .last_visible_user_message()
                .map(|message| message.id),
            Some(first_user.id)
        );
        assert_eq!(conversation.visible_messages()[0].id, first_user.id);
        assert_eq!(conversation.visible_messages()[1].content, "first answer");
    }

    #[test]
    fn tool_output_preview_truncates_large_outputs() {
        let output = "abcdef\n".repeat(2_000);

        let preview = tool_output_preview(Some("grep"), &output);

        assert!(preview.contains("grep output truncated"));
        assert!(preview.contains("First excerpt"));
        assert!(preview.contains("Last excerpt"));
        assert!(preview.len() < output.len());
    }

    #[test]
    fn backend_event_exposes_session_id() {
        let session_id = Uuid::new_v4();
        let event = BackendEvent::Failed {
            session_id,
            request_id: 7,
            error: "boom".to_string(),
        };

        assert_eq!(event.session_id(), session_id);
    }

    #[test]
    fn assistant_turn_upserts_tool_calls_by_id() {
        let mut turn = AssistantTurn::default();

        turn.upsert_tool_call(ToolCall {
            id: "tool-call-1".to_string(),
            name: "bash".to_string(),
            arguments: "{\"command\":\"ls\"}".to_string(),
            thought_signature: None,
        });
        turn.upsert_tool_call(ToolCall {
            id: "tool-call-1".to_string(),
            name: "bash".to_string(),
            arguments: "{\"command\":\"ls -la\"}".to_string(),
            thought_signature: None,
        });
        turn.upsert_tool_call(ToolCall {
            id: "tool-call-2".to_string(),
            name: "read".to_string(),
            arguments: "{\"path\":\"README.md\"}".to_string(),
            thought_signature: None,
        });

        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].arguments, "{\"command\":\"ls -la\"}");
        assert_eq!(turn.tool_calls[1].name, "read");
    }

    #[test]
    fn find_compaction_prior_state_none_when_no_compaction() {
        let id1 = Uuid::new_v4();
        let msgs = vec![
            Message::new(MessageRole::User, "hello"),
            Message::new(MessageRole::Assistant, "hi"),
        ];
        assert!(find_compaction_prior_state(&msgs, id1).is_none());
    }

    #[test]
    fn find_compaction_prior_state_returns_state_when_present() {
        let revert_to = Uuid::new_v4();
        let mut compaction = Message::compaction("new summary");
        compaction.metadata.prior_summary = Some("old summary".to_string());
        compaction.metadata.prior_retained_from = Some(42);

        let msgs = vec![
            Message {
                id: revert_to,
                ..Message::new(MessageRole::User, "last user msg")
            },
            compaction,
        ];
        let result = find_compaction_prior_state(&msgs, revert_to);
        assert!(result.is_some());
        let (prior_summary, prior_retained) = result.unwrap();
        assert_eq!(prior_summary, Some("old summary".to_string()));
        assert_eq!(prior_retained, 42);
    }

    #[test]
    fn find_compaction_prior_state_ignores_compaction_before_target() {
        let revert_to = Uuid::new_v4();
        let mut early_compact = Message::compaction("early");
        early_compact.metadata.prior_retained_from = Some(10);

        let mut late_compact = Message::compaction("late");
        late_compact.metadata.prior_retained_from = Some(20);

        let msgs = vec![
            early_compact,
            Message {
                id: revert_to,
                ..Message::new(MessageRole::User, "msg")
            },
            late_compact,
        ];
        // Should find "late" (after revert_to), not "early" (before)
        let result = find_compaction_prior_state(&msgs, revert_to);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, 20);
    }

    #[test]
    fn find_compaction_prior_state_skips_compaction_without_metadata() {
        let revert_to = Uuid::new_v4();
        let no_metadata = Message::compaction("no prior state");
        let mut with_metadata = Message::compaction("with prior");
        with_metadata.metadata.prior_retained_from = Some(99);

        let msgs = vec![
            Message {
                id: revert_to,
                ..Message::new(MessageRole::User, "msg")
            },
            no_metadata,
            with_metadata,
        ];
        // Should skip the one without metadata and find the one with
        let result = find_compaction_prior_state(&msgs, revert_to);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, 99);
    }

    #[test]
    fn tool_metadata_prior_state_round_trips_through_json() {
        let meta = ToolMetadata {
            prior_summary: Some("old summary".to_string()),
            prior_retained_from: Some(42),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: ToolMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.prior_summary, Some("old summary".to_string()));
        assert_eq!(deserialized.prior_retained_from, Some(42));
    }

    #[test]
    fn tool_metadata_prior_state_defaults_to_none() {
        let json = r#"{}"#;
        let deserialized: ToolMetadata = serde_json::from_str(json).unwrap();
        assert!(deserialized.prior_summary.is_none());
        assert!(deserialized.prior_retained_from.is_none());
    }

    #[test]
    fn compaction_message_accepts_metadata() {
        let mut msg = Message::compaction("new summary text");
        msg.metadata.prior_summary = Some("old summary".to_string());
        msg.metadata.prior_retained_from = Some(77);

        assert!(msg.content.starts_with(COMPACTION_MESSAGE_LABEL));
        assert_eq!(msg.metadata.prior_summary.as_deref(), Some("old summary"));
        assert_eq!(msg.metadata.prior_retained_from, Some(77));
    }
}
