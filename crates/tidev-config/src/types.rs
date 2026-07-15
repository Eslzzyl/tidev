//! Shared types re-exported or defined locally for config crate.

use serde::{Deserialize, Serialize};

/// The API protocol used by a provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiType {
    #[serde(rename = "openai_chat_completions")]
    #[default]
    OpenAiChatCompletions,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "google_gemini")]
    GoogleGemini,
}

impl ApiType {
    /// Parse an API type string (case-insensitive).
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "openai_chat_completions" | "openai" | "chat" => Self::OpenAiChatCompletions,
            "openai_responses" | "responses" => Self::OpenAiResponses,
            "anthropic" | "claude" => Self::Anthropic,
            "google_gemini" | "gemini" | "google" => Self::GoogleGemini,
            _ => Self::OpenAiChatCompletions,
        }
    }
}

impl std::fmt::Display for ApiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAiChatCompletions => write!(f, "openai_chat_completions"),
            Self::OpenAiResponses => write!(f, "openai_responses"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::GoogleGemini => write!(f, "google_gemini"),
        }
    }
}
