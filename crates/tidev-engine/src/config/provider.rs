use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tidev_llm::ApiType;

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
    /// Per-model API type override.  When `None`, falls back to the
    /// provider-level `api_type`, then to the default (`openai_chat_completions`).
    #[serde(default)]
    pub api_type: Option<String>,
    /// Per-model base URL override.  When `None`, falls back to the
    /// provider-level `base_url`.
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
