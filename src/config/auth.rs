use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::paths::ConfigPaths;
use super::provider::ApiType;
use super::reasoning::ThinkingLevelType;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderAuth>,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelAuth>,
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
    pub temperature: f32,
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
    /// Matches opencode's logic: GPT models (claude/deepseek/etc. excluded) get
    /// `apply_patch` as their primary edit tool; all other models get `edit`/`write`.
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
        }
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
