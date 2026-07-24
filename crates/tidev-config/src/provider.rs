use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::types::ApiType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderSource {
    User,
    Bundled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_type: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelConfig>,
}

impl ProviderConfig {
    /// Resolve the effective [`ApiType`] for a given model using cascade
    /// precedence: model-level → provider-level → default.
    pub fn resolve_api_type(&self, model: &ModelConfig) -> ApiType {
        model
            .api_type
            .as_deref()
            .or(self.api_type.as_deref())
            .map(ApiType::parse)
            .unwrap_or_default()
    }

    /// Resolve the effective base URL for a given model using cascade
    /// precedence: model-level → provider-level.
    pub fn resolve_base_url(&self, model: &ModelConfig) -> String {
        model
            .base_url
            .clone()
            .or_else(|| Some(self.base_url.clone()))
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    /// Per-model API type override.
    #[serde(default)]
    pub api_type: Option<String>,
    /// Per-model base URL override.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
    #[serde(default)]
    pub request_model_id: Option<String>,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn model(api_type: Option<String>, base_url: Option<String>) -> ModelConfig {
        ModelConfig {
            display_name: "test".into(),
            context_window: 100_000,
            max_output_tokens: 8_000,
            api_type,
            base_url,
            temperature: None,
            system_prompt: None,
            supports_streaming: true,
            supports_images: false,
            extra_body: None,
            request_model_id: None,
        }
    }

    fn provider(api_type: Option<String>, base_url: String) -> ProviderConfig {
        ProviderConfig {
            display_name: "Test".into(),
            base_url,
            api_type,
            models: BTreeMap::new(),
        }
    }

    // ── resolve_api_type ────────────────────────────────────────────────

    #[test]
    fn resolve_api_type_model_overrides_provider() {
        let provider = provider(Some("anthropic".into()), "https://api.anthropic.com".into());
        let model = model(Some("openai_chat_completions".into()), None);
        assert_eq!(
            provider.resolve_api_type(&model),
            ApiType::OpenAiChatCompletions
        );
    }

    #[test]
    fn resolve_api_type_falls_back_to_provider() {
        let provider = provider(Some("anthropic".into()), "https://api.anthropic.com".into());
        let model = model(None, None);
        assert_eq!(provider.resolve_api_type(&model), ApiType::Anthropic);
    }

    #[test]
    fn resolve_api_type_default_when_both_none() {
        let provider = provider(None, "https://api.test.com".into());
        let model = model(None, None);
        assert_eq!(provider.resolve_api_type(&model), ApiType::default());
    }

    // ── resolve_base_url ────────────────────────────────────────────────

    #[test]
    fn resolve_base_url_model_overrides_provider() {
        let provider = provider(None, "https://api.default.com".into());
        let model = model(None, Some("https://api.custom.com".into()));
        assert_eq!(
            provider.resolve_base_url(&model),
            "https://api.custom.com"
        );
    }

    #[test]
    fn resolve_base_url_falls_back_to_provider() {
        let provider = provider(None, "https://api.default.com".into());
        let model = model(None, None);
        assert_eq!(provider.resolve_base_url(&model), "https://api.default.com");
    }
}
