//! Shared type definitions used across multiple tidev components.
//!
//! This module will become the foundation for the `tidev-types` crate
//! when the workspace is split.

use serde::{Deserialize, Serialize};
// ── Permission types (originally split across config + tooling) ─────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    Read,
    Search,
    Write,
    Edit,
    Execute,
    Session,
}

impl ToolPermission {
    pub fn is_allowed_in(
        self,
        mode: crate::prompts::SessionMode,
        permission_config: &PermissionConfig,
    ) -> bool {
        permission_config.is_allowed(mode, self)
    }

    pub fn needs_confirmation(self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PermissionSettings {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub edit: bool,
    #[serde(default)]
    pub execute: bool,
    #[serde(default)]
    pub session: bool,
}

impl PermissionSettings {
    pub fn is_allowed(&self, permission: ToolPermission) -> bool {
        match permission {
            ToolPermission::Read => self.read,
            ToolPermission::Search => self.search,
            ToolPermission::Write => self.write,
            ToolPermission::Edit => self.edit,
            ToolPermission::Execute => self.execute,
            ToolPermission::Session => self.session,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(default)]
    pub plan: PermissionSettings,
    #[serde(default)]
    pub build: PermissionSettings,
}

impl PermissionConfig {
    pub fn is_allowed(
        &self,
        mode: crate::prompts::SessionMode,
        permission: ToolPermission,
    ) -> bool {
        match mode {
            crate::prompts::SessionMode::Plan => self.plan.is_allowed(permission),
            crate::prompts::SessionMode::Build => self.build.is_allowed(permission),
        }
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            plan: PermissionSettings {
                read: true,
                search: true,
                write: false,
                edit: false,
                execute: true,
                session: true,
            },
            build: PermissionSettings {
                read: true,
                search: true,
                write: true,
                edit: true,
                execute: true,
                session: true,
            },
        }
    }
}

// ── TodoItem (moved from tooling to break tooling↔storage cycle) ─────

/// A task/todo item within a session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
}

// ── ApiType (moved from tidev-llm to break config→llm dependency) ────

/// Provider API protocol variant — used to dispatch to the correct
/// provider implementation when streaming/completing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

// ── ToolSchema (LLM-facing tool interface, replaces ToolDefinition) ───

/// The LLM-facing tool interface. Minimal — only what providers need.
/// Replaces `tidev_llm::types::ToolDefinition` and eliminates the
/// `llm_bridge.rs` conversion entirely.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSchema {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

// ── LlmProviderConfig (moved from tidev-llm to break config→llm dependency) ─

/// Provider configuration — a carrier for all the fields needed by the
/// LLM provider implementations.
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
    pub thinking_level: crate::reasoning::ThinkingLevelType,
    pub extra_body: Option<serde_json::Value>,
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
    pub fn merged_extra_body(&self) -> Option<serde_json::Value> {
        self.merged_extra_body_with_thinking(self.thinking_level.clone(), self.api_type)
    }

    /// Merge `extra_body` with a specific thinking level's extra body,
    /// filtering by API type.
    pub fn merged_extra_body_with_thinking(
        &self,
        thinking_level: crate::reasoning::ThinkingLevelType,
        api_type: ApiType,
    ) -> Option<serde_json::Value> {
        let thinking_extra = thinking_level.extra_body_for_api(api_type.as_str());

        match (&self.extra_body, thinking_extra) {
            (Some(base), Some(extra)) => {
                let mut merged = base.as_object().cloned().unwrap_or_default();
                if let Some(obj) = extra.as_object() {
                    merged.extend(obj.clone());
                }
                Some(serde_json::Value::Object(merged))
            }
            (Some(base), None) => Some(base.clone()),
            (None, Some(extra)) => Some(extra),
            (None, None) => None,
        }
    }

    /// Get thinking config for OpenAI Responses API.
    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        self.thinking_level.thinking_config()
    }

    /// Get the effective system prompt, or an empty string if none.
    pub fn system_prompt_str(&self) -> &str {
        self.system_prompt.as_deref().unwrap_or("")
    }
}
