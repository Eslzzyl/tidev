//! LLM-provider-agnostic config types that replace tidev-engine's `ActiveModel`
//! and tooling `ToolDefinition` in the LLM layer.

use serde_json::Value;

/// Provider API protocol variant — used to dispatch to the correct
/// provider implementation when streaming/completing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ApiType {
    #[default]
    OpenAiChatCompletions,
    Anthropic,
    OpenAiResponses,
    GoogleGemini,
}

impl ApiType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiChatCompletions => "openai_chat_completions",
            Self::Anthropic => "anthropic",
            Self::OpenAiResponses => "openai_responses",
            Self::GoogleGemini => "google_gemini",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "anthropic" => Self::Anthropic,
            "openai_responses" => Self::OpenAiResponses,
            "google_gemini" => Self::GoogleGemini,
            _ => Self::OpenAiChatCompletions,
        }
    }
}

/// Provider configuration — a carrier for all the fields needed by the
/// LLM provider implementations.  This replaces `ActiveModel` from the
/// engine's config module.
#[derive(Clone, Debug)]
pub struct LlmProviderConfig {
    pub provider_id: String,
    pub api_type: ApiType,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model_id: String,
    pub request_model_id: Option<String>,
    /// System prompt — `None` means no system prompt override.
    pub system_prompt: Option<String>,
    pub thinking_level: tidev_types::reasoning::ThinkingLevelType,
    pub extra_body: Option<Value>,
    pub max_output_tokens: usize,
    pub temperature: Option<f32>,
    pub supports_images: bool,
}

impl LlmProviderConfig {
    /// Provider-specific API endpoint URL.
    ///
    /// Each branch is idempotent: if `base_url` already ends with the expected
    /// path suffix, it is returned as-is.  This lets users configure either a
    /// bare base URL (e.g. `https://api.anthropic.com`) or a full URL
    /// (e.g. `https://opencode.ai/zen/go/v1/messages`) without double-path bugs.
    pub fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        match self.api_type {
            ApiType::Anthropic => {
                if base.ends_with("/v1/messages") {
                    base.to_string()
                } else {
                    format!("{}/v1/messages", base)
                }
            }
            ApiType::OpenAiChatCompletions => {
                if base.ends_with("/chat/completions") {
                    base.to_string()
                } else {
                    format!("{}/chat/completions", base)
                }
            }
            ApiType::OpenAiResponses => {
                if base.ends_with("/v1/responses") {
                    base.to_string()
                } else {
                    format!("{}/v1/responses", base)
                }
            }
            ApiType::GoogleGemini => {
                if base.ends_with(":generateContent") || base.contains(":streamGenerateContent") {
                    base.to_string()
                } else {
                    format!(
                        "{}/models/{}:generateContent",
                        base,
                        self.request_model_id.as_deref().unwrap_or(&self.model_id)
                    )
                }
            }
        }
    }

    /// Gemini streaming endpoint (uses SSE via `streamGenerateContent`).
    pub fn gemini_stream_endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.contains(":streamGenerateContent") {
            base.to_string()
        } else {
            format!(
                "{}/models/{}:streamGenerateContent?alt=sse",
                base,
                self.request_model_id.as_deref().unwrap_or(&self.model_id)
            )
        }
    }

    /// Merge provider-level `extra_body` with the thinking-level extra body.
    pub fn merged_extra_body(&self) -> Option<Value> {
        self.merged_extra_body_with_thinking(self.thinking_level.clone(), self.api_type)
    }

    /// Merge `extra_body` with a specific thinking level's extra body,
    /// filtering by API type.
    pub fn merged_extra_body_with_thinking(
        &self,
        thinking_level: tidev_types::reasoning::ThinkingLevelType,
        api_type: ApiType,
    ) -> Option<Value> {
        let thinking_extra = thinking_level.extra_body_for_api(api_type.as_str());

        match (&self.extra_body, thinking_extra) {
            (Some(base), Some(extra)) => {
                let mut merged = base.as_object().cloned().unwrap_or_default();
                if let Some(obj) = extra.as_object() {
                    merged.extend(obj.clone());
                }
                Some(Value::Object(merged))
            }
            (Some(base), None) => Some(base.clone()),
            (None, Some(extra)) => Some(extra),
            (None, None) => None,
        }
    }

    /// Get thinking config for OpenAI Responses API.
    pub fn thinking_config(&self) -> Option<Value> {
        self.thinking_level.thinking_config()
    }

    /// Get the effective system prompt, or an empty string if none.
    pub fn system_prompt_str(&self) -> &str {
        self.system_prompt.as_deref().unwrap_or("")
    }
}

/// Tool definition as needed by LLM providers — a subset of what the engine's
/// `ToolDefinition` carries.  We drop `ToolPermission`, `ToolOrigin` etc.
#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub parameters: Value,
}
