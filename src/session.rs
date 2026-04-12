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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub streaming: bool,
}

impl Message {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: content.into(),
            created_at: Utc::now(),
            streaming: false,
        }
    }

    pub fn streaming(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: content.into(),
            created_at: Utc::now(),
            streaming: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub session_id: Uuid,
    pub provider_id: String,
    pub model_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<Message>,
}

impl Conversation {
    pub fn new(
        session_id: Uuid,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        }
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
    Finished(String),
    Failed(String),
}
