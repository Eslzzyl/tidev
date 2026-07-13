use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::paths::ConfigPaths;
use crate::reasoning::ThinkingLevelType;
use crate::types::ApiType;

// ---------------------------------------------------------------------------
// AuthStore
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderAuth>,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelAuth>,
    #[serde(default)]
    pub web: WebAuth,
}

impl AuthStore {
    /// Load from auth file, creating a default one if it doesn't exist.
    pub fn load_or_create(paths: &ConfigPaths) -> Result<Self> {
        paths.ensure_directories()?;

        if !paths.auth_file.exists() {
            let auth = Self::default();
            auth.save(paths)?;
            return Ok(auth);
        }

        let contents = std::fs::read_to_string(&paths.auth_file)
            .with_context(|| format!("failed to read {}", paths.auth_file.display()))?;
        let auth: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.auth_file.display()))?;
        Ok(auth)
    }

    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents =
            serde_json::to_string_pretty(self).context("failed to serialize auth store")?;
        std::fs::write(&paths.auth_file, contents)
            .with_context(|| format!("failed to write {}", paths.auth_file.display()))?;
        Ok(())
    }

    pub fn set_api_key(&mut self, provider_id: impl Into<String>, api_key: impl Into<String>) {
        let provider_id = provider_id.into();
        let api_key = api_key.into();
        self.providers.entry(provider_id).or_default().api_key = Some(api_key);
    }

    pub fn api_key(&self, provider_id: &str) -> Option<&str> {
        self.providers
            .get(provider_id)
            .and_then(|provider| provider.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    /// Remove all provider entries whose ID does not appear in `known_ids`.
    pub fn prune_orphan_providers(&mut self, known_ids: &[String]) -> usize {
        let before = self.providers.len();
        self.providers.retain(|id, _| known_ids.contains(id));
        before - self.providers.len()
    }
}

// ---------------------------------------------------------------------------
// ProviderAuth
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderAuth {
    #[serde(default)]
    pub api_key: Option<String>,
}

// ---------------------------------------------------------------------------
// ChannelAuth
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelAuth {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// WebAuth
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebAuth {
    /// Optional token for web UI authentication (Bearer token)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// API keys for web search providers, keyed by provider name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub search_api_keys: BTreeMap<String, String>,
    /// Google Custom Search Engine ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_cx: Option<String>,
}

// ---------------------------------------------------------------------------
// ActiveModel
// ---------------------------------------------------------------------------

/// A fully resolved model configuration ready for use by the LLM layer.
#[derive(Clone, Debug)]
pub struct ActiveModel {
    pub provider_id: String,
    pub provider_display_name: String,
    pub base_url: String,
    pub api_type: ApiType,
    pub model_id: String,
    pub request_model_id: String,
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: Option<f32>,
    pub supports_images: bool,
    pub system_prompt: String,
    pub api_key: Option<String>,
    pub extra_body: Option<serde_json::Value>,
    pub thinking_level: ThinkingLevelType,
}

impl ActiveModel {
    /// Build the API endpoint URL for this model.
    pub fn endpoint(&self) -> String {
        match self.api_type {
            ApiType::Anthropic => {
                format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
            }
            ApiType::OpenAiChatCompletions => {
                format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
            }
            ApiType::OpenAiResponses => {
                format!("{}/v1/responses", self.base_url.trim_end_matches('/'))
            }
            ApiType::GoogleGemini => {
                format!(
                    "{}/models/{}:generateContent",
                    self.base_url.trim_end_matches('/'),
                    self.request_model_id
                )
            }
        }
    }

    /// Gemini streaming endpoint.
    pub fn gemini_stream_endpoint(&self) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base_url.trim_end_matches('/'),
            self.request_model_id
        )
    }

    /// Merged extra_body with thinking level configuration.
    pub fn merged_extra_body(&self) -> Option<serde_json::Value> {
        self.merged_extra_body_with_thinking(self.thinking_level.clone())
    }

    pub fn merged_extra_body_with_thinking(
        &self,
        thinking_level: ThinkingLevelType,
    ) -> Option<serde_json::Value> {
        let thinking_extra = thinking_level.extra_body();

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

    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_display_name, self.display_name)
    }

    /// Determine whether this model should receive `apply_patch` instead of `write`/`edit`.
    ///
    /// GPT models (gpt-4o, gpt-4o-mini, gpt-4.1, gpt-5, etc.) get `apply_patch`.
    /// All other models (Claude, DeepSeek, Gemini, GPT-4, any OSS model) get `write`/`edit`.
    pub fn use_apply_patch(&self) -> bool {
        let id = self.model_id.to_ascii_lowercase();
        id.contains("gpt-") && !id.contains("oss") && !id.contains("gpt-4")
    }

    pub fn api_key_present(&self) -> bool {
        self.api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }

    /// Thinking configuration for OpenAI Responses API.
    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        self.thinking_level.thinking_config()
    }
}

// ---------------------------------------------------------------------------
// ModelSummary
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ModelSummary {
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub base_url: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
}

impl ModelSummary {
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_id, self.model_id)
    }
}
