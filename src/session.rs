use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    pub reasoning: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub streaming: bool,
}

impl Message {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: content.into(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            created_at: Utc::now(),
            streaming: false,
        }
    }

    pub fn streaming(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: content.into(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            created_at: Utc::now(),
            streaming: true,
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
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            created_at,
            streaming,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: MessageRole::Tool,
            content: content.into(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
            created_at: Utc::now(),
            streaming: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub session_id: Uuid,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<Message>,
}

impl Conversation {
    pub fn new(
        session_id: Uuid,
        provider_id: impl Into<String>,
        provider_display_name: impl Into<String>,
        model_id: impl Into<String>,
        model_display_name: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            provider_id: provider_id.into(),
            provider_display_name: provider_display_name.into(),
            model_id: model_id.into(),
            model_display_name: model_display_name.into(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        }
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
        self.updated_at = Utc::now();
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
    Delta(String),
    ReasoningDelta(String),
    Finished(AssistantTurn),
    Failed(String),
}
