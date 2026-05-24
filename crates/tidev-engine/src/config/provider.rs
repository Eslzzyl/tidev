use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
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
