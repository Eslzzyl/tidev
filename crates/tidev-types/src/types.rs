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
