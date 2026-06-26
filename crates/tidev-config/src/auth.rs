use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::paths::ConfigPaths;
use crate::reasoning::ThinkingLevelType;
use tidev_types::ApiType;

/// Web authentication configuration stored in auth.json
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebAuth {
    /// Optional token for web UI authentication (Bearer token)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// API keys for web search providers, keyed by provider name.
    /// E.g. `{ "brave": "BSA-xxx", "google": "AIza-xxx", "tavily": "tvly-xxx" }`
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub search_api_keys: BTreeMap<String, String>,
    /// Google Custom Search Engine ID (required for the `google` search provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_cx: Option<String>,
}

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

    pub fn remove_api_key(&mut self, provider_id: &str) {
        if let Some(provider) = self.providers.get_mut(provider_id) {
            provider.api_key = None;
        }
    }

    /// Remove all provider entries from auth whose ID does not appear in `known_ids`.
    /// Returns the number of entries removed.
    pub fn prune_orphan_providers(&mut self, known_ids: &[String]) -> usize {
        let before = self.providers.len();
        self.providers.retain(|id, _| known_ids.contains(id));
        before - self.providers.len()
    }

    pub fn web_token(&self) -> Option<&str> {
        self.web
            .auth_token
            .as_deref()
            .filter(|v| !v.trim().is_empty())
    }

    /// Get the API key for a named web search provider (brave, google, tavily).
    pub fn search_api_key(&self, provider: &str) -> Option<&str> {
        self.web
            .search_api_keys
            .get(provider)
            .map(|s| s.as_str())
            .filter(|v| !v.trim().is_empty())
    }

    /// Get the Google Custom Search Engine ID (cx).
    pub fn google_cx(&self) -> Option<&str> {
        self.web
            .google_cx
            .as_deref()
            .filter(|v| !v.trim().is_empty())
    }

    pub fn set_web_token(&mut self, token: impl Into<String>) {
        let token = token.into();
        if token.trim().is_empty() {
            self.web.auth_token = None;
        } else {
            self.web.auth_token = Some(token);
        }
    }

    pub fn clear_web_token(&mut self) {
        self.web.auth_token = None;
    }

    pub fn set_telegram_bot_token(&mut self, token: impl Into<String>) {
        let token = token.into();
        self.channels
            .entry("telegram".to_string())
            .or_default()
            .api_key = Some(token);
    }

    pub fn telegram_bot_token(&self) -> Option<&str> {
        self.channels
            .get("telegram")
            .and_then(|channel| channel.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn set_qq_credentials(&mut self, app_id: impl Into<String>, app_secret: impl Into<String>) {
        let auth = self.channels.entry("qq".to_string()).or_default();
        auth.api_key = Some(app_id.into());
        auth.extra.insert(
            "app_secret".to_string(),
            serde_json::Value::String(app_secret.into()),
        );
    }

    pub fn qq_app_id(&self) -> Option<&str> {
        self.channels
            .get("qq")
            .and_then(|channel| channel.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn qq_app_secret(&self) -> Option<&str> {
        self.channels
            .get("qq")
            .and_then(|channel| channel.extra.get("app_secret"))
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn set_discord_bot_token(&mut self, token: impl Into<String>) {
        let token = token.into();
        self.channels
            .entry("discord".to_string())
            .or_default()
            .api_key = Some(token);
    }

    pub fn discord_bot_token(&self) -> Option<&str> {
        self.channels
            .get("discord")
            .and_then(|channel| channel.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn lark_app_id(&self) -> Option<&str> {
        self.channels
            .get("lark")
            .and_then(|channel| channel.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn lark_app_secret(&self) -> Option<&str> {
        self.channels
            .get("lark")
            .and_then(|channel| channel.extra.get("app_secret"))
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelAuth {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderAuth {
    #[serde(default)]
    pub api_key: Option<String>,
}

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
    /// Thinking level type (depends on the model)
    pub thinking_level: ThinkingLevelType,
}

impl ActiveModel {
    /// Whether this model should use `apply_patch` instead of `edit`/`write`.
    ///
    /// GPT-5+ series (gpt-5, gpt-6, ...) get `apply_patch` as their primary edit tool;
    /// all other models (Claude, DeepSeek, GPT-4 variants, oss, etc.) get `edit`/`write`.
    /// GPT-4 variants (including gpt-4o) are excluded via the `!id.contains("gpt-4")` check.
    pub fn use_apply_patch(&self) -> bool {
        let id = self.model_id.to_ascii_lowercase();
        id.contains("gpt-") && !id.contains("oss") && !id.contains("gpt-4")
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_display_name, self.display_name)
    }

    pub fn api_key_present(&self) -> bool {
        self.api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }

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

    /// Gemini streaming endpoint (uses SSE via `streamGenerateContent`).
    pub fn gemini_stream_endpoint(&self) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base_url.trim_end_matches('/'),
            self.request_model_id
        )
    }

    /// 获取完整的 extra_body（合并基础配置 + 思考配置）
    pub fn merged_extra_body(&self) -> Option<serde_json::Value> {
        self.merged_extra_body_with_thinking(self.thinking_level.clone())
    }

    /// 获取完整的 extra_body（使用指定的 thinking level）
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

    /// 获取 thinking 配置（用于 OpenAI Responses API）
    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        self.thinking_level.thinking_config()
    }
}

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

// ── Bridge conversion: ActiveModel → LlmProviderConfig ──────────

impl From<ActiveModel> for tidev_llm::LlmProviderConfig {
    fn from(m: ActiveModel) -> Self {
        tidev_llm::LlmProviderConfig {
            provider_id: m.provider_id,
            api_type: m.api_type,
            api_key: m.api_key,
            base_url: m.base_url,
            model_id: m.model_id,
            request_model_id: Some(m.request_model_id).filter(|s| !s.is_empty()),
            system_prompt: Some(m.system_prompt).filter(|s| !s.is_empty()),
            thinking_level: m.thinking_level,
            extra_body: m.extra_body,
            max_output_tokens: m.max_output_tokens,
            temperature: m.temperature,
            supports_images: m.supports_images,
        }
    }
}
