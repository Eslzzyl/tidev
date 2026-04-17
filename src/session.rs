use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAttachment {
    FileReference {
        path: String,
        content: String,
    },
    DirectoryReference {
        path: String,
        tree: String,
    },
    Image {
        filename: String,
        mime: String,
        data_url: String,
    },
}

impl MessageAttachment {
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }

    pub fn summary(&self) -> String {
        match self {
            Self::FileReference { path, content } => {
                let preview = content.lines().take(6).collect::<Vec<_>>().join(" ");
                if preview.trim().is_empty() {
                    format!("[file:{}]", path)
                } else {
                    format!("[file:{}] {}", path, truncate_preview(&preview, 120))
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

    pub fn prompt_text(&self) -> Option<String> {
        match self {
            Self::FileReference { path, content } => Some(format!(
                "\n\nReferenced file: {}\n```text\n{}\n```",
                path, content
            )),
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionResult {
    pub output: String,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
}

impl ToolExecutionResult {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            attachments: Vec::new(),
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
}

impl MessageRole {
    pub fn label(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Error => "error",
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
            _ => Self::System,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
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
    pub created_at: DateTime<Utc>,
    pub streaming: bool,
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub snapshot_hash: Option<String>,
    #[serde(default)]
    pub patch_files: Option<String>,
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
            created_at: Utc::now(),
            streaming: false,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            snapshot_hash: None,
            patch_files: None,
        }
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
            created_at: Utc::now(),
            streaming: true,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            snapshot_hash: None,
            patch_files: None,
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
            created_at,
            streaming,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            snapshot_hash: None,
            patch_files: None,
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
            created_at: Utc::now(),
            streaming: false,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            snapshot_hash: None,
            patch_files: None,
        }
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
    },
    ContextCompacted {
        session_id: Uuid,
        compacted: bool,
        summary: Option<String>,
        retained_from: usize,
        error: Option<String>,
    },
}

impl BackendEvent {
    pub fn session_id(&self) -> Uuid {
        match self {
            Self::Delta { session_id, .. }
            | Self::ReasoningDelta { session_id, .. }
            | Self::Finished { session_id, .. }
            | Self::Failed { session_id, .. }
            | Self::Retrying { session_id, .. }
            | Self::ToolCompleted { session_id, .. }
            | Self::SubagentStatus { session_id, .. }
            | Self::SubagentToolResult { session_id, .. }
            | Self::SubagentCompleted { session_id, .. }
            | Self::UsageStats { session_id, .. }
            | Self::ContextCompacted { session_id, .. } => *session_id,
        }
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Delta { request_id, .. }
            | Self::ReasoningDelta { request_id, .. }
            | Self::Finished { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::Retrying { request_id, .. }
            | Self::ToolCompleted { request_id, .. }
            | Self::SubagentStatus { request_id, .. }
            | Self::SubagentToolResult { request_id, .. }
            | Self::SubagentCompleted { request_id, .. }
            | Self::UsageStats { request_id, .. } => Some(*request_id),
            Self::ContextCompacted { .. } => None,
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
}
